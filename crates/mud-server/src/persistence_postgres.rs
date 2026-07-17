//! PostgreSQL implementation of the transport-independent character store.
//!
//! `postgres::Client` is synchronous, so one dedicated owner thread performs
//! database work behind the same bounded Tokio channel used by SQLite. Argon2
//! runs in a separately bounded blocking service, and no PostgreSQL call runs
//! on an async executor thread.

use std::{
    collections::BTreeSet,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use postgres::{Client, GenericClient, IsolationLevel};
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, BoardKey, CharacterGameplaySnapshot, CharacterId,
    CharacterSnapshot, DomainEvent, DurableCharacterSnapshot, MAX_BOARD_BODY_BYTES,
    MAX_BOARD_REPORT_REASON_BYTES, MAX_BOARD_SUBJECT_BYTES, MAX_MAIL_BODY_BYTES,
    MAX_MAIL_REPORT_REASON_BYTES, MAX_MAIL_SUBJECT_BYTES, SocialChannelName, StaffCapability,
    StaffProfileKey, StaffReportResolution, WorldRuntimeSnapshot, valid_board_public_id,
    valid_board_subject, valid_board_visible_text, valid_mail_public_id, valid_mail_visible_text,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};

use crate::{
    auth::{
        AuthenticatedSession, DEFAULT_SESSION_TTL, PasswordService, PasswordWorkError,
        SessionToken, digest_token, validate_password,
    },
    persistence::{
        BOARD_ACCOUNT_CAPACITY, BOARD_CAPACITY_PER_BOARD, BOARD_MAX_PAGES, BOARD_PAGE_SIZE,
        BoardEntry, BoardPage, BoardPostSummary, BoardReportEvidence, CharacterStore,
        DOMAIN_EVENT_RETENTION, DurableAreaCheckpoint, DurableAreaGeneration,
        DurableCharacterGameplay, MailMessage, MailMessageSummary, MailPage, MailReportEvidence,
        MailSendResult, ModerationAction, PreparedLogin, Request, SocialProfile,
        SocialRelationKind, StaffAccess, StaffAuditEntry, StaffAuditRecord, StaffReport,
        StartupGameplayLoadLimits, StartupWorldRecovery, StoreError, decode_runtime_snapshot,
        decode_startup_domain_events, decode_startup_gameplay, ensure_all_character_ids_exist,
        ensure_all_startup_characters_loaded, normalise_character_identity_ids,
        normalise_startup_character_ids, validate_area_checkpoint, validate_area_generation,
        validate_area_generation_transition, validate_area_rehydration_commit,
        validate_area_rehydration_previous_runtime, validate_area_retirement_commit,
        validate_area_retirement_previous_runtime,
    },
};

#[cfg(test)]
use crate::persistence::{AuditEntry, area_checkpoint_unit_id};

const MAX_IDENTITY_NAME_BYTES: usize = 64;
const MAX_ACTIVE_SESSIONS: i64 = 16;
const MAX_CHARACTER_JSON_BYTES: usize = 65_536;
const MAX_EVENT_JSON_BYTES: usize = 65_536;
const MAX_RUNTIME_JSON_BYTES: usize = tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
const MAX_SOCIAL_REASON_BYTES: usize = 1_024;
const MAX_REPORT_REASON_BYTES: usize = 2_048;
const MAX_AUDIT_DETAILS_BYTES: usize = 4_096;
const MAX_STAFF_ACTION_BYTES: usize = 64;
const MAX_STAFF_LORE_TITLE_BYTES: usize = 128;
const STAFF_LIST_LIMIT: i64 = 10;
const MAIL_PAGE_SIZE: i64 = 10;
const MAIL_MAX_PAGES: i64 = 7;
const MAILBOX_CAPACITY: i64 = 64;
const MAILBOX_BYTE_CAPACITY: i64 = 128 * 1024;
const MAIL_GLOBAL_CAPACITY: i64 = 100_000;
const MAIL_HOURLY_CAPACITY: i64 = 20;
const MAIL_DAILY_CAPACITY: i64 = 50;
const MAIL_HOUR_SECONDS: i64 = 60 * 60;
const MAIL_DAY_SECONDS: i64 = 24 * MAIL_HOUR_SECONDS;
const MAIL_RETENTION_SECONDS: i64 = 30 * MAIL_DAY_SECONDS;
const MAIL_REPORT_EVIDENCE_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
const MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT: i64 = 8;
const MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY: i64 = 10_000;
const MAIL_PURGE_LIMIT: i64 = 256;
const MAIL_TRANSACTION_LOCK_ID: i64 = 0x5449_414d_4d41_494c;
const BOARD_BYTE_CAPACITY_PER_BOARD: i64 = 4 * 1_024 * 1_024;
const BOARD_GLOBAL_CAPACITY: i64 = 2_000;
const BOARD_GLOBAL_BYTE_CAPACITY: i64 = 8 * 1_024 * 1_024;
const BOARD_HOURLY_CAPACITY: i64 = 5;
const BOARD_DAILY_CAPACITY: i64 = 20;
const BOARD_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
const BOARD_REPORT_EVIDENCE_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
const BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT: i64 = 8;
const BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY: i64 = 10_000;
const BOARD_PURGE_LIMIT: i64 = 256;
const BOARD_TRANSACTION_LOCK_ID: i64 = 0x5449_414d_424f_4152;
const MAX_MODERATION_DURATION_SECONDS: i64 = 365 * 24 * 60 * 60;
const PERSISTENCE_REQUEST_QUEUE_CAPACITY: usize = 256;
const PERSISTENCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const POSTGRES_CHARACTER_BATCH_SIZE: usize =
    tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES / MAX_CHARACTER_JSON_BYTES;
const POSTGRES_CHARACTER_EXISTENCE_BATCH_SIZE: usize = 4_096;

#[derive(Clone)]
pub struct PostgresCharacterStore {
    tx: mpsc::Sender<Request>,
    passwords: PasswordService,
}

impl PostgresCharacterStore {
    pub async fn open(database_url: &str) -> Result<Self, StoreError> {
        Self::open_with_password_service(database_url, PasswordService::default()).await
    }

    async fn open_with_password_service(
        database_url: &str,
        passwords: PasswordService,
    ) -> Result<Self, StoreError> {
        let database_url = database_url.to_owned();
        let (tx, mut rx) = mpsc::channel(PERSISTENCE_REQUEST_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();

        thread::Builder::new()
            .name("postgres-writer".into())
            .spawn(move || {
                let mut connection = match (|| -> Result<Client, StoreError> {
                    let mut connection = tiamud_persistence_postgres::connect(&database_url)?;
                    tiamud_persistence_postgres::migrate(&mut connection)?;
                    Ok(connection)
                })() {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };

                while let Some(request) = rx.blocking_recv() {
                    match request {
                        Request::Shutdown { reply } => {
                            drop(connection);
                            let _ = reply.send(Ok(()));
                            return;
                        }
                        Request::Health { reply } => {
                            let result = connection
                                .query_one("SELECT 1::BIGINT", &[])
                                .map(|_| ())
                                .map_err(StoreError::from);
                            let _ = reply.send(result);
                        }
                        Request::FinishRegisterAccount {
                            name,
                            password_hash,
                            reply,
                        } => {
                            let _ = reply.send(finish_register_account(
                                &mut connection,
                                &name,
                                &password_hash,
                                unix_timestamp(),
                            ));
                        }
                        Request::PrepareLoginAccount { name, reply } => {
                            let _ = reply.send(prepare_login_account(&mut connection, &name));
                        }
                        Request::FinishLoginAccount { prepared, reply } => {
                            let _ = reply.send(finish_login_account(
                                &mut connection,
                                &prepared,
                                unix_timestamp(),
                            ));
                        }
                        Request::ResolveSession { token, reply } => {
                            let _ = reply.send(resolve_session(
                                &mut connection,
                                &token,
                                unix_timestamp(),
                            ));
                        }
                        Request::RevokeSession { token, reply } => {
                            let _ = reply.send(revoke_session(
                                &mut connection,
                                &token,
                                unix_timestamp(),
                            ));
                        }
                        #[cfg(test)]
                        Request::LoadOrCreateAccount { name, reply } => {
                            let _ = reply.send(load_or_create_account(&mut connection, &name));
                        }
                        Request::ListCharacters { account_id, reply } => {
                            let _ = reply.send(list_characters(&mut connection, account_id));
                        }
                        Request::LoadOrCreateCharacter {
                            account_id,
                            name,
                            start_room,
                            initial_gameplay,
                            reply,
                        } => {
                            let _ = reply.send(load_or_create_character_with_gameplay(
                                &mut connection,
                                account_id,
                                &name,
                                &start_room,
                                &initial_gameplay,
                            ));
                        }
                        Request::LoadCharacter {
                            account_id,
                            character_id,
                            reply,
                        } => {
                            let _ = reply.send(load_character(
                                &mut connection,
                                account_id,
                                character_id,
                            ));
                        }
                        Request::Save { snapshot, reply } => {
                            let _ = reply.send(save(&mut connection, &snapshot));
                        }
                        Request::LoadCharacterGameplay {
                            character_id,
                            reply,
                        } => {
                            let _ =
                                reply.send(load_character_gameplay(&mut connection, character_id));
                        }
                        Request::LoadCharacterGameplayBatch {
                            character_ids,
                            limits,
                            reply,
                        } => {
                            let _ = reply.send(load_character_gameplay_batch(
                                &mut connection,
                                character_ids,
                                limits,
                            ));
                        }
                        Request::EnsureCharacterIdsExistBatch {
                            character_ids,
                            reply,
                        } => {
                            let _ = reply.send(ensure_character_ids_exist_batch(
                                &mut connection,
                                character_ids,
                            ));
                        }
                        Request::LoadRuntimeSnapshot { reply } => {
                            let _ = reply.send(load_runtime_snapshot(&mut connection));
                        }
                        Request::LoadStartupWorldRecovery { reply } => {
                            let _ = reply.send(load_startup_world_recovery(&mut connection));
                        }
                        Request::HasDurableCharacterAuthority { reply } => {
                            let _ = reply.send(has_durable_character_authority(&mut connection));
                        }
                        Request::LoadAreaGeneration { reply } => {
                            let _ = reply.send(load_area_generation(&mut connection));
                        }
                        Request::BindAreaGenerationIfAbsent { generation, reply } => {
                            let _ = reply
                                .send(bind_area_generation_if_absent(&mut connection, &generation));
                        }
                        Request::CommitAreaGenerationActivation {
                            runtime,
                            expected_current,
                            next,
                            reply,
                        } => {
                            let _ = reply.send(commit_area_generation_activation(
                                &mut connection,
                                &runtime,
                                &expected_current,
                                &next,
                            ));
                        }
                        Request::LoadAreaCheckpoints { reply } => {
                            let _ = reply.send(load_area_checkpoints(&mut connection));
                        }
                        Request::CommitAreaRetirement {
                            runtime,
                            expected_generation,
                            checkpoint,
                            reply,
                        } => {
                            let _ = reply.send(commit_area_retirement(
                                &mut connection,
                                &runtime,
                                &expected_generation,
                                &checkpoint,
                            ));
                        }
                        Request::CommitAreaRehydration {
                            runtime,
                            expected_generation,
                            checkpoint,
                            reply,
                        } => {
                            let _ = reply.send(commit_area_rehydration(
                                &mut connection,
                                &runtime,
                                &expected_generation,
                                &checkpoint,
                            ));
                        }
                        Request::LoadDomainEvents { reply } => {
                            let _ = reply.send(load_domain_events(&mut connection));
                        }
                        Request::CommitAction {
                            characters,
                            runtime,
                            events,
                            reply,
                        } => {
                            let _ = reply.send(commit_action(
                                &mut connection,
                                &characters,
                                &runtime,
                                &events,
                            ));
                        }
                        Request::CommitStaffAction {
                            characters,
                            runtime,
                            events,
                            audit,
                            reply,
                        } => {
                            let _ = reply.send(commit_staff_action(
                                &mut connection,
                                &characters,
                                &runtime,
                                &events,
                                audit,
                                unix_timestamp(),
                            ));
                        }
                        Request::SocialProfile {
                            character_id,
                            reply,
                        } => {
                            let _ = reply.send(social_profile(
                                &mut connection,
                                character_id,
                                unix_timestamp(),
                            ));
                        }
                        Request::SocialTarget { name, reply } => {
                            let _ =
                                reply.send(social_target(&mut connection, &name, unix_timestamp()));
                        }
                        Request::SocialDeliveryAllowed {
                            sender,
                            recipient,
                            reply,
                        } => {
                            let _ = reply.send(social_delivery_allowed(
                                &mut connection,
                                sender,
                                recipient,
                            ));
                        }
                        Request::SetSocialRelation {
                            actor,
                            target,
                            relation,
                            enabled,
                            reply,
                        } => {
                            let _ = reply.send(set_social_relation(
                                &mut connection,
                                actor,
                                target,
                                relation,
                                enabled,
                                unix_timestamp(),
                            ));
                        }
                        Request::SocialChannelSubscriptions {
                            character_id,
                            reply,
                        } => {
                            let _ = reply
                                .send(social_channel_subscriptions(&mut connection, character_id));
                        }
                        Request::SocialChannelSubscribers { channel, reply } => {
                            let _ =
                                reply.send(social_channel_subscribers(&mut connection, channel));
                        }
                        Request::SetSocialChannelSubscription {
                            character_id,
                            channel,
                            subscribed,
                            reply,
                        } => {
                            let _ = reply.send(set_social_channel_subscription(
                                &mut connection,
                                character_id,
                                channel,
                                subscribed,
                                unix_timestamp(),
                            ));
                        }
                        Request::SendMail {
                            sender,
                            target_name,
                            public_id,
                            subject,
                            body,
                            reply,
                        } => {
                            let _ = reply.send(send_mail(
                                &mut connection,
                                sender,
                                &target_name,
                                &public_id,
                                &subject,
                                &body,
                                unix_timestamp(),
                            ));
                        }
                        Request::MailboxPage {
                            recipient,
                            page,
                            reply,
                        } => {
                            let _ = reply.send(mailbox_page(
                                &mut connection,
                                recipient,
                                page,
                                unix_timestamp(),
                            ));
                        }
                        Request::ReadMail {
                            recipient,
                            public_id,
                            reply,
                        } => {
                            let _ = reply.send(read_mail(
                                &mut connection,
                                recipient,
                                &public_id,
                                unix_timestamp(),
                            ));
                        }
                        Request::DeleteMail {
                            recipient,
                            public_id,
                            reply,
                        } => {
                            let _ = reply.send(delete_mail(
                                &mut connection,
                                recipient,
                                &public_id,
                                unix_timestamp(),
                            ));
                        }
                        Request::ReportMail {
                            recipient,
                            public_id,
                            reason,
                            reply,
                        } => {
                            let _ = reply.send(report_mail(
                                &mut connection,
                                recipient,
                                &public_id,
                                &reason,
                                unix_timestamp(),
                            ));
                        }
                        Request::PurgeExpiredMail { reply } => {
                            let _ =
                                reply.send(purge_expired_mail(&mut connection, unix_timestamp()));
                        }
                        Request::ListBoardPosts {
                            actor,
                            board,
                            page,
                            reply,
                        } => {
                            let _ = reply.send(list_board_posts(
                                &mut connection,
                                actor,
                                board,
                                page,
                                unix_timestamp(),
                            ));
                        }
                        Request::ReadBoardPost {
                            actor,
                            public_id,
                            reply,
                        } => {
                            let _ = reply.send(read_board_post(
                                &mut connection,
                                actor,
                                &public_id,
                                unix_timestamp(),
                            ));
                        }
                        Request::PublishBoardPost {
                            actor,
                            board,
                            public_id,
                            subject,
                            body,
                            reply,
                        } => {
                            let _ = reply.send(publish_board_post(
                                &mut connection,
                                actor,
                                board,
                                &public_id,
                                &subject,
                                &body,
                                unix_timestamp(),
                            ));
                        }
                        Request::DeleteBoardPost {
                            actor,
                            public_id,
                            reply,
                        } => {
                            let _ = reply.send(delete_board_post(
                                &mut connection,
                                actor,
                                &public_id,
                                unix_timestamp(),
                            ));
                        }
                        Request::ReportBoardPost {
                            actor,
                            public_id,
                            reason,
                            reply,
                        } => {
                            let _ = reply.send(report_board_post(
                                &mut connection,
                                actor,
                                &public_id,
                                &reason,
                                unix_timestamp(),
                            ));
                        }
                        Request::PurgeExpiredBoards { reply } => {
                            let _ =
                                reply.send(purge_expired_boards(&mut connection, unix_timestamp()));
                        }
                        Request::ReportCharacter {
                            actor,
                            target,
                            reason,
                            reply,
                        } => {
                            let _ = reply.send(report_character(
                                &mut connection,
                                actor,
                                target,
                                &reason,
                                unix_timestamp(),
                            ));
                        }
                        Request::ModerateCharacter {
                            actor,
                            target,
                            action,
                            reply,
                        } => {
                            let _ = reply.send(moderate_character(
                                &mut connection,
                                actor,
                                target,
                                action,
                                unix_timestamp(),
                            ));
                        }
                        Request::StaffAccess {
                            character_id,
                            reply,
                        } => {
                            let _ = reply.send(staff_access(&mut connection, character_id));
                        }
                        Request::StaffReports { audit, reply } => {
                            let _ =
                                reply.send(staff_reports(&mut connection, audit, unix_timestamp()));
                        }
                        Request::ResolveStaffReport {
                            report_id,
                            resolution,
                            resolution_reason,
                            audit,
                            reply,
                        } => {
                            let _ = reply.send(resolve_staff_report(
                                &mut connection,
                                report_id,
                                resolution,
                                &resolution_reason,
                                audit,
                                unix_timestamp(),
                            ));
                        }
                        Request::SetStaffProfile {
                            target_account,
                            profile,
                            lore_title,
                            audit,
                            reply,
                        } => {
                            let _ = reply.send(set_staff_profile(
                                &mut connection,
                                target_account,
                                profile,
                                lore_title.as_deref(),
                                audit,
                                unix_timestamp(),
                            ));
                        }
                        Request::SetStaffCapability {
                            target_account,
                            capability,
                            enabled,
                            audit,
                            reply,
                        } => {
                            let _ = reply.send(set_staff_capability(
                                &mut connection,
                                target_account,
                                capability,
                                enabled,
                                audit,
                                unix_timestamp(),
                            ));
                        }
                        Request::StaffAuditEntries { audit, reply } => {
                            let _ = reply.send(staff_audit_entries(
                                &mut connection,
                                audit,
                                unix_timestamp(),
                            ));
                        }
                        Request::AppendStaffAudit { audit, reply } => {
                            let _ = reply.send(append_staff_audit_transaction(
                                &mut connection,
                                audit,
                                unix_timestamp(),
                            ));
                        }
                        #[cfg(test)]
                        Request::AuditLog { reply } => {
                            let _ = reply.send(audit_log(&mut connection));
                        }
                    }
                }
            })
            .map_err(StoreError::ThreadSpawn)?;

        ready_rx.await.map_err(|_| StoreError::WorkerStopped)??;
        Ok(Self { tx, passwords })
    }

    async fn request<T>(
        &self,
        request: Request,
        receive: oneshot::Receiver<Result<T, StoreError>>,
    ) -> Result<T, StoreError> {
        self.tx
            .send_timeout(request, PERSISTENCE_REQUEST_TIMEOUT)
            .await
            .map_err(|error| match error {
                mpsc::error::SendTimeoutError::Timeout(_) => StoreError::RequestTimeout,
                mpsc::error::SendTimeoutError::Closed(_) => StoreError::WorkerStopped,
            })?;
        timeout(PERSISTENCE_REQUEST_TIMEOUT, receive)
            .await
            .map_err(|_| StoreError::RequestTimeout)?
            .map_err(|_| StoreError::WorkerStopped)?
    }

    /// Bound admission, then await the definitive result of an accepted
    /// mutation so a local deadline cannot race a later database commit.
    async fn request_commit<T>(
        &self,
        request: Request,
        receive: oneshot::Receiver<Result<T, StoreError>>,
    ) -> Result<T, StoreError> {
        self.tx
            .send_timeout(request, PERSISTENCE_REQUEST_TIMEOUT)
            .await
            .map_err(|error| match error {
                mpsc::error::SendTimeoutError::Timeout(_) => StoreError::RequestTimeout,
                mpsc::error::SendTimeoutError::Closed(_) => StoreError::WorkerStopped,
            })?;
        receive.await.map_err(|_| StoreError::WorkerStopped)?
    }
}

#[async_trait]
impl CharacterStore for PostgresCharacterStore {
    fn backend_name(&self) -> &'static str {
        "postgresql"
    }

    fn queue_depth(&self) -> usize {
        self.tx.max_capacity().saturating_sub(self.tx.capacity())
    }

    async fn health_check(&self) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::Health { reply }, receive).await
    }

    async fn shutdown(&self) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::Shutdown { reply }, receive)
            .await
    }

    async fn register_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError> {
        let name = normalise_name(name, "account")?;
        if !validate_password(password) {
            return Err(StoreError::InvalidPassword);
        }
        let password_hash = self
            .passwords
            .hash(password.to_owned())
            .await
            .map_err(map_password_work_error)?;
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::FinishRegisterAccount {
                name,
                password_hash,
                reply,
            },
            receive,
        )
        .await
    }

    async fn login_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError> {
        let name = normalise_name(name, "account")?;
        if !validate_password(password) {
            return Err(StoreError::InvalidPassword);
        }
        let (reply, receive) = oneshot::channel();
        let prepared = self
            .request(Request::PrepareLoginAccount { name, reply }, receive)
            .await?;
        let Some(prepared) = prepared else {
            self.passwords
                .dummy(password.to_owned())
                .await
                .map_err(map_password_work_error)?;
            return Err(StoreError::InvalidCredentials);
        };
        let valid = self
            .passwords
            .verify(password.to_owned(), prepared.encoded_hash.clone())
            .await
            .map_err(map_password_work_error)?;
        if !valid {
            return Err(StoreError::InvalidCredentials);
        }
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::FinishLoginAccount { prepared, reply }, receive)
            .await
    }

    async fn resolve_session(&self, token: &str) -> Result<AccountSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ResolveSession {
                token: token.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn revoke_session(&self, token: &str) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::RevokeSession {
                token: token.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    #[cfg(test)]
    async fn load_or_create_account(&self, name: &str) -> Result<AccountSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::LoadOrCreateAccount {
                name: name.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn list_characters(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::ListCharacters { account_id, reply }, receive)
            .await
    }

    #[cfg(test)]
    async fn load_or_create_character(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::LoadOrCreateCharacter {
                account_id,
                name: name.to_owned(),
                start_room: start_room.to_owned(),
                initial_gameplay: Box::new(CharacterGameplaySnapshot::default()),
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_or_create_character_with_gameplay(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
        initial_gameplay: CharacterGameplaySnapshot,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::LoadOrCreateCharacter {
                account_id,
                name: name.to_owned(),
                start_room: start_room.to_owned(),
                initial_gameplay: Box::new(initial_gameplay),
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_character(
        &self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadCharacter {
                account_id,
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::Save { snapshot, reply }, receive)
            .await
    }

    async fn load_character_gameplay(
        &self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadCharacterGameplay {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_character_gameplay_batch(
        &self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadCharacterGameplayBatch {
                character_ids,
                limits,
                reply,
            },
            receive,
        )
        .await
    }

    async fn ensure_character_ids_exist_batch(
        &self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::EnsureCharacterIdsExistBatch {
                character_ids,
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_runtime_snapshot(&self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadRuntimeSnapshot { reply }, receive)
            .await
    }

    async fn load_startup_world_recovery(&self) -> Result<StartupWorldRecovery, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadStartupWorldRecovery { reply }, receive)
            .await
    }

    async fn has_durable_character_authority(&self) -> Result<bool, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::HasDurableCharacterAuthority { reply }, receive)
            .await
    }

    async fn load_area_generation(&self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadAreaGeneration { reply }, receive)
            .await
    }

    async fn bind_area_generation_if_absent(
        &self,
        generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::BindAreaGenerationIfAbsent { generation, reply },
            receive,
        )
        .await
    }

    async fn commit_area_generation_activation(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAreaGenerationActivation {
                runtime,
                expected_current,
                next,
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_area_checkpoints(&self) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadAreaCheckpoints { reply }, receive)
            .await
    }

    async fn commit_area_retirement(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAreaRetirement {
                runtime,
                expected_generation,
                checkpoint,
                reply,
            },
            receive,
        )
        .await
    }

    async fn commit_area_rehydration(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAreaRehydration {
                runtime,
                expected_generation,
                checkpoint,
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_domain_events(&self) -> Result<Vec<DomainEvent>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadDomainEvents { reply }, receive)
            .await
    }

    async fn social_profile(&self, character_id: CharacterId) -> Result<SocialProfile, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialProfile {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_target(&self, name: &str) -> Result<SocialProfile, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialTarget {
                name: name.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_delivery_allowed(
        &self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialDeliveryAllowed {
                sender,
                recipient,
                reply,
            },
            receive,
        )
        .await
    }

    async fn set_social_relation(
        &self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetSocialRelation {
                actor,
                target,
                relation,
                enabled,
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_channel_subscriptions(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialChannelSubscriptions {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_channel_subscribers(
        &self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialChannelSubscribers { channel, reply },
            receive,
        )
        .await
    }

    async fn set_social_channel_subscription(
        &self,
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetSocialChannelSubscription {
                character_id,
                channel,
                subscribed,
                reply,
            },
            receive,
        )
        .await
    }

    async fn send_mail(
        &self,
        sender: CharacterId,
        target_name: &str,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<MailSendResult, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SendMail {
                sender,
                target_name: target_name.to_owned(),
                public_id: public_id.to_owned(),
                subject: subject.to_owned(),
                body: body.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn mailbox_page(
        &self,
        recipient: CharacterId,
        page: u32,
    ) -> Result<MailPage, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::MailboxPage {
                recipient,
                page,
                reply,
            },
            receive,
        )
        .await
    }

    async fn read_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
    ) -> Result<MailMessage, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReadMail {
                recipient,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn delete_mail(&self, recipient: CharacterId, public_id: &str) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::DeleteMail {
                recipient,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn report_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReportMail {
                recipient,
                public_id: public_id.to_owned(),
                reason: reason.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn purge_expired_mail(&self) -> Result<u64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::PurgeExpiredMail { reply }, receive)
            .await
    }

    async fn list_board_posts(
        &self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ListBoardPosts {
                actor,
                board,
                page,
                reply,
            },
            receive,
        )
        .await
    }

    async fn read_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<BoardEntry, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ReadBoardPost {
                actor,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn publish_board_post(
        &self,
        actor: CharacterId,
        board: BoardKey,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<BoardEntry, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::PublishBoardPost {
                actor,
                board,
                public_id: public_id.to_owned(),
                subject: subject.to_owned(),
                body: body.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn delete_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::DeleteBoardPost {
                actor,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn report_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReportBoardPost {
                actor,
                public_id: public_id.to_owned(),
                reason: reason.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn purge_expired_boards(&self) -> Result<u64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::PurgeExpiredBoards { reply }, receive)
            .await
    }

    async fn report_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        reason: &str,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReportCharacter {
                actor,
                target,
                reason: reason.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn moderate_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ModerateCharacter {
                actor,
                target,
                action,
                reply,
            },
            receive,
        )
        .await
    }

    async fn staff_access(&self, character_id: CharacterId) -> Result<StaffAccess, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::StaffAccess {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn staff_reports(&self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::StaffReports { audit, reply }, receive)
            .await
    }

    async fn resolve_staff_report(
        &self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: &str,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ResolveStaffReport {
                report_id,
                resolution,
                resolution_reason: resolution_reason.to_owned(),
                audit,
                reply,
            },
            receive,
        )
        .await
    }

    async fn set_staff_profile(
        &self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<&str>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetStaffProfile {
                target_account,
                profile,
                lore_title: lore_title.map(str::to_owned),
                audit,
                reply,
            },
            receive,
        )
        .await
    }

    async fn set_staff_capability(
        &self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetStaffCapability {
                target_account,
                capability,
                enabled,
                audit,
                reply,
            },
            receive,
        )
        .await
    }

    async fn staff_audit_entries(
        &self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::StaffAuditEntries { audit, reply }, receive)
            .await
    }

    async fn append_staff_audit(&self, audit: StaffAuditRecord) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::AppendStaffAudit { audit, reply }, receive)
            .await
    }

    #[cfg(test)]
    async fn audit_log(&self) -> Result<Vec<AuditEntry>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::AuditLog { reply }, receive).await
    }

    async fn commit_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAction {
                characters,
                runtime,
                events,
                reply,
            },
            receive,
        )
        .await
    }

    async fn commit_staff_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitStaffAction {
                characters,
                runtime,
                events,
                audit,
                reply,
            },
            receive,
        )
        .await
    }
}

fn normalise_name(name: &str, kind: &'static str) -> Result<String, StoreError> {
    let name = name.trim();
    if !valid_mail_visible_text(name, MAX_IDENTITY_NAME_BYTES) || name.contains('|') {
        return Err(StoreError::InvalidName { kind });
    }
    Ok(name.to_owned())
}

fn map_password_work_error(error: PasswordWorkError) -> StoreError {
    match error {
        PasswordWorkError::Overloaded | PasswordWorkError::Timeout => StoreError::RequestTimeout,
        PasswordWorkError::Execution => StoreError::PasswordHashing,
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn finish_register_account(
    connection: &mut Client,
    name: &str,
    password_hash: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let mut transaction = connection.transaction()?;
    // Serialises first-account bootstrap and cross-process duplicate checks.
    transaction.batch_execute("LOCK TABLE accounts IN SHARE ROW EXCLUSIVE MODE")?;
    if transaction
        .query_opt(
            "SELECT 1::BIGINT FROM accounts WHERE lower(name) = lower($1) LIMIT 1",
            &[&name],
        )?
        .is_some()
    {
        return Err(StoreError::AccountAlreadyExists);
    }
    let role = if transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM accounts WHERE auth_state = 'registered'",
            &[],
        )?
        .get::<_, i64>(0)
        == 0
    {
        AccountRole::Admin
    } else {
        AccountRole::Player
    };
    let id: i64 = transaction
        .query_one(
            "INSERT INTO accounts (name, password_hash, auth_state, role)
             VALUES ($1, $2, 'registered', $3) RETURNING id",
            &[&name, &password_hash, &role_as_database(role)],
        )?
        .get(0);
    let account = AccountSnapshot {
        id: AccountId(id),
        name: name.to_owned(),
    };
    if role == AccountRole::Admin {
        transaction.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES ($1, 'implementor', NULL, $1, $2, $2)",
            &[&account.id.0, &now],
        )?;
        for capability in StaffCapability::ALL {
            transaction.execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES ($1, $2, $1, $3)",
                &[&account.id.0, &capability.as_str(), &now],
            )?;
        }
        append_audit(
            &mut transaction,
            account.id,
            Some(account.id),
            "bootstrap_admin",
            serde_json::json!({"source": "first_registered_account"}),
            now,
        )?;
        transaction.execute(
            "INSERT INTO staff_audit
                 (actor_account_id, target_account_id, capability, action,
                  reason, details_json, created_at)
             VALUES ($1, $1, 'staff.assign', 'bootstrap.implementor',
                     'First registered account bootstrap',
                     '{\"source\":\"first_registered_account\"}', $2)",
            &[&account.id.0, &now],
        )?;
    }
    let session = issue_session(&mut transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

fn prepare_login_account<C: GenericClient>(
    connection: &mut C,
    name: &str,
) -> Result<Option<PreparedLogin>, StoreError> {
    let prepared = connection
        .query_opt(
            "SELECT id, password_hash FROM accounts
         WHERE lower(name) = lower($1) AND auth_state = 'registered'",
            &[&name],
        )?
        .and_then(|row| {
            row.get::<_, Option<String>>(1)
                .map(|encoded_hash| PreparedLogin {
                    account_id: AccountId(row.get(0)),
                    encoded_hash,
                })
        });
    Ok(prepared)
}

fn finish_login_account(
    connection: &mut Client,
    prepared: &PreparedLogin,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let mut transaction = connection.transaction()?;
    let current = transaction.query_opt(
        "SELECT name, password_hash FROM accounts
         WHERE id = $1 AND auth_state = 'registered'",
        &[&prepared.account_id.0],
    )?;
    let Some(row) = current else {
        return Err(StoreError::InvalidCredentials);
    };
    let Some(encoded_hash) = row.get::<_, Option<String>>(1) else {
        return Err(StoreError::InvalidCredentials);
    };
    if encoded_hash != prepared.encoded_hash {
        return Err(StoreError::InvalidCredentials);
    }
    let account = AccountSnapshot {
        id: prepared.account_id,
        name: row.get(0),
    };
    if account_is_banned(&mut transaction, account.id, now)? {
        return Err(StoreError::AccountBanned);
    }

    let session = issue_session(&mut transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

fn issue_session<C: GenericClient>(
    connection: &mut C,
    account: AccountSnapshot,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let ttl = i64::try_from(DEFAULT_SESSION_TTL.as_secs()).unwrap_or(i64::MAX);
    let expires_at = now.checked_add(ttl).ok_or(StoreError::InvalidSession)?;

    connection.execute(
        "DELETE FROM auth_sessions
         WHERE account_id = $1 AND (expires_at <= $2 OR revoked_at IS NOT NULL)",
        &[&account.id.0, &now],
    )?;
    let active: i64 = connection
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM auth_sessions WHERE account_id = $1",
            &[&account.id.0],
        )?
        .get(0);
    if active >= MAX_ACTIVE_SESSIONS {
        let remove_count = active - MAX_ACTIVE_SESSIONS + 1;
        connection.execute(
            "DELETE FROM auth_sessions WHERE token_hash IN (
                 SELECT token_hash FROM auth_sessions
                 WHERE account_id = $1
                 ORDER BY created_at, token_hash LIMIT $2
             )",
            &[&account.id.0, &remove_count],
        )?;
    }

    let token = SessionToken::generate();
    connection.execute(
        "INSERT INTO auth_sessions
             (token_hash, account_id, created_at, expires_at)
         VALUES ($1, $2, $3, $4)",
        &[&token.digest().as_slice(), &account.id.0, &now, &expires_at],
    )?;
    Ok(AuthenticatedSession {
        account,
        token,
        expires_at,
    })
}

fn resolve_session<C: GenericClient>(
    connection: &mut C,
    token: &str,
    now: i64,
) -> Result<AccountSnapshot, StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let row = connection
        .query_opt(
            "SELECT accounts.id, accounts.name, auth_sessions.expires_at,
                    auth_sessions.revoked_at
             FROM auth_sessions
             JOIN accounts ON accounts.id = auth_sessions.account_id
             WHERE auth_sessions.token_hash = $1",
            &[&token_hash.as_slice()],
        )?
        .ok_or(StoreError::InvalidSession)?;
    let account = AccountSnapshot {
        id: AccountId(row.get(0)),
        name: row.get(1),
    };
    let expires_at: i64 = row.get(2);
    let revoked_at: Option<i64> = row.get(3);
    if revoked_at.is_some() {
        return Err(StoreError::SessionRevoked);
    }
    if expires_at <= now {
        return Err(StoreError::SessionExpired);
    }
    if account_is_banned(connection, account.id, now)? {
        return Err(StoreError::AccountBanned);
    }
    Ok(account)
}

fn account_is_banned<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    now: i64,
) -> Result<bool, StoreError> {
    Ok(connection
        .query_one(
            "SELECT EXISTS(
                 SELECT 1 FROM account_bans
                 WHERE account_id = $1 AND revoked_at IS NULL
                   AND (expires_at IS NULL OR expires_at > $2)
             )",
            &[&account_id.0, &now],
        )?
        .get(0))
}

fn revoke_session<C: GenericClient>(
    connection: &mut C,
    token: &str,
    now: i64,
) -> Result<(), StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let updated = connection.execute(
        "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, $1)
         WHERE token_hash = $2",
        &[&now, &token_hash.as_slice()],
    )?;
    if updated == 0 {
        return Err(StoreError::InvalidSession);
    }
    Ok(())
}

fn ensure_account_exists<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
) -> Result<(), StoreError> {
    if connection
        .query_opt(
            "SELECT 1::BIGINT FROM accounts WHERE id = $1",
            &[&account_id.0],
        )?
        .is_some()
    {
        Ok(())
    } else {
        Err(StoreError::AccountNotFound(account_id))
    }
}

#[cfg(test)]
fn load_or_create_account(
    connection: &mut Client,
    name: &str,
) -> Result<AccountSnapshot, StoreError> {
    let name = normalise_name(name, "account")?;
    connection.execute(
        "INSERT INTO accounts (name, auth_state)
         VALUES ($1, 'guest') ON CONFLICT DO NOTHING",
        &[&name],
    )?;
    connection
        .query_opt(
            "SELECT id, name FROM accounts
             WHERE lower(name) = lower($1) AND auth_state = 'guest'",
            &[&name],
        )?
        .map(|row| AccountSnapshot {
            id: AccountId(row.get(0)),
            name: row.get(1),
        })
        .ok_or(StoreError::AccountAlreadyExists)
}

fn list_characters<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
) -> Result<Vec<CharacterSnapshot>, StoreError> {
    ensure_account_exists(connection, account_id)?;
    connection
        .query(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = $1 ORDER BY lower(name), id",
            &[&account_id.0],
        )?
        .iter()
        .map(character_from_row)
        .collect()
}

fn load_or_create_character_with_gameplay<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    name: &str,
    start_room: &str,
    initial_gameplay: &CharacterGameplaySnapshot,
) -> Result<CharacterSnapshot, StoreError> {
    ensure_account_exists(connection, account_id)?;
    let name = normalise_name(name, "character")?;
    let gameplay_json = bounded_json(
        initial_gameplay,
        "character gameplay",
        MAX_CHARACTER_JSON_BYTES,
    )?;
    connection.execute(
        "INSERT INTO characters (account_id, name, room_id, gameplay_json)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        &[&account_id.0, &name, &start_room, &gameplay_json],
    )?;
    connection
        .query_opt(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters
             WHERE account_id = $1 AND lower(name) = lower($2)",
            &[&account_id.0, &name],
        )?
        .as_ref()
        .map(character_from_row)
        .transpose()?
        .ok_or(StoreError::AccountAlreadyExists)
}

fn load_character<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    character_id: CharacterId,
) -> Result<CharacterSnapshot, StoreError> {
    ensure_account_exists(connection, account_id)?;
    connection
        .query_opt(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = $1 AND id = $2",
            &[&account_id.0, &character_id.0],
        )?
        .as_ref()
        .map(character_from_row)
        .transpose()?
        .ok_or(StoreError::CharacterNotFound {
            account_id,
            character_id,
        })
}

fn character_from_row(row: &postgres::Row) -> Result<CharacterSnapshot, StoreError> {
    let item_locations_json: String = row.get(4);
    Ok(CharacterSnapshot {
        id: CharacterId(row.get(0)),
        account_id: AccountId(row.get(1)),
        name: row.get(2),
        room_id: row.get(3),
        item_locations: serde_json::from_str(&item_locations_json)?,
    })
}

fn save<C: GenericClient>(
    connection: &mut C,
    snapshot: &CharacterSnapshot,
) -> Result<(), StoreError> {
    let name = normalise_name(&snapshot.name, "character")?;
    let item_locations_json = serde_json::to_string(&snapshot.item_locations)?;
    let updated = connection.execute(
        "UPDATE characters
         SET name = $1, room_id = $2, item_locations_json = $3,
             updated_at = floor(extract(epoch FROM now()))::BIGINT
         WHERE id = $4 AND account_id = $5",
        &[
            &name,
            &snapshot.room_id,
            &item_locations_json,
            &snapshot.id.0,
            &snapshot.account_id.0,
        ],
    )?;
    if updated == 0 {
        return Err(StoreError::CharacterNotFound {
            account_id: snapshot.account_id,
            character_id: snapshot.id,
        });
    }
    Ok(())
}

fn load_character_gameplay<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
) -> Result<CharacterGameplaySnapshot, StoreError> {
    let row = connection
        .query_opt(
            "SELECT gameplay_json FROM characters WHERE id = $1",
            &[&character_id.0],
        )?
        .ok_or(StoreError::CharacterGameplayNotFound(character_id))?;
    let json: String = row.get(0);
    serde_json::from_str(&json).map_err(StoreError::from)
}

fn load_character_gameplay_batch<C: GenericClient>(
    connection: &mut C,
    character_ids: Vec<CharacterId>,
    limits: StartupGameplayLoadLimits,
) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
    let character_ids = normalise_startup_character_ids(character_ids, limits)?;
    let mut total_json_bytes = 0_usize;
    let mut aggregates = Vec::new();
    for batch in character_ids.chunks(POSTGRES_CHARACTER_BATCH_SIZE) {
        let database_ids = batch.iter().map(|id| id.0).collect::<Vec<_>>();
        // `query` materializes one batch. At 16 rows times the 64 KiB document
        // cap, each allocation remains at most 1 MiB even when the caller's
        // explicit aggregate scope is the 8 MiB trade-participant budget.
        let rows = connection.query(
            "SELECT id, gameplay_json FROM characters WHERE id = ANY($1) ORDER BY id",
            &[&database_ids],
        )?;
        for row in rows {
            let character_id = CharacterId(row.get(0));
            let json: String = row.get(1);
            aggregates.push(decode_startup_gameplay(
                character_id,
                &json,
                &mut total_json_bytes,
                limits,
            )?);
        }
    }
    ensure_all_startup_characters_loaded(&character_ids, &aggregates)?;
    Ok(aggregates)
}

fn ensure_character_ids_exist_batch<C: GenericClient>(
    connection: &mut C,
    character_ids: Vec<CharacterId>,
) -> Result<(), StoreError> {
    let character_ids = normalise_character_identity_ids(character_ids)?;
    let mut found = Vec::with_capacity(character_ids.len());
    for batch in character_ids.chunks(POSTGRES_CHARACTER_EXISTENCE_BATCH_SIZE) {
        let database_ids = batch.iter().map(|id| id.0).collect::<Vec<_>>();
        let rows = connection.query(
            "SELECT id FROM characters WHERE id = ANY($1) ORDER BY id",
            &[&database_ids],
        )?;
        found.extend(
            rows.into_iter()
                .map(|row| CharacterId(row.get::<_, i64>(0))),
        );
    }
    ensure_all_character_ids_exist(&character_ids, &found)
}

fn load_runtime_snapshot<C: GenericClient>(
    connection: &mut C,
) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
    load_durable_runtime_snapshot(connection).map(|snapshot| snapshot.map(|(_, snapshot)| snapshot))
}

fn load_runtime_snapshot_for_update<C: GenericClient>(
    connection: &mut C,
) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
    connection
        .query_opt(
            "SELECT snapshot_version, snapshot_json
             FROM world_runtime WHERE singleton = 1 FOR UPDATE",
            &[],
        )?
        .map(|row| {
            let snapshot_version: i64 = row.get(0);
            let json: String = row.get(1);
            decode_runtime_snapshot(snapshot_version, &json)
        })
        .transpose()
}

fn load_durable_runtime_snapshot<C: GenericClient>(
    connection: &mut C,
) -> Result<Option<(u32, WorldRuntimeSnapshot)>, StoreError> {
    connection
        .query_opt(
            "SELECT snapshot_version, snapshot_json FROM world_runtime WHERE singleton = 1",
            &[],
        )?
        .map(|row| {
            let snapshot_version: i64 = row.get(0);
            let json: String = row.get(1);
            let durable_version =
                u32::try_from(snapshot_version).map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
            decode_runtime_snapshot(snapshot_version, &json)
                .map(|snapshot| (durable_version, snapshot))
        })
        .transpose()
}

fn has_durable_character_authority<C: GenericClient>(
    connection: &mut C,
) -> Result<bool, StoreError> {
    connection
        .query_one("SELECT EXISTS(SELECT 1 FROM characters LIMIT 1)", &[])
        .map(|row| row.get(0))
        .map_err(StoreError::from)
}

fn load_area_generation<C: GenericClient>(
    connection: &mut C,
) -> Result<Option<DurableAreaGeneration>, StoreError> {
    let generation = connection
        .query_opt(
            "SELECT world_id, active_catalog_sha256, previous_catalog_sha256,
                    activation_sequence
             FROM world_area_generation WHERE singleton = 1",
            &[],
        )?
        .map(|row| DurableAreaGeneration {
            world_id: row.get(0),
            active_catalog_sha256: row.get(1),
            previous_catalog_sha256: row.get(2),
            activation_sequence: u64::try_from(row.get::<_, i64>(3)).unwrap_or(0),
        });
    if let Some(generation) = &generation {
        validate_area_generation(generation)?;
    }
    Ok(generation)
}

fn load_area_checkpoints<C: GenericClient>(
    connection: &mut C,
) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
    let limit = i64::try_from(tiamud_core::MAX_RUNTIME_AREA_RESIDENCY_ENTRIES + 1)
        .map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    let rows = connection.query(
        "SELECT generation_sha256, unit_id, residency_epoch,
                checkpoint_version, checkpoint_sha256, captured_logical_tick,
                checkpoint_json
         FROM world_area_checkpoints
         ORDER BY generation_sha256, unit_id, residency_epoch
         LIMIT $1",
        &[&limit],
    )?;
    if rows.len() > tiamud_core::MAX_RUNTIME_AREA_RESIDENCY_ENTRIES {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let active_generation = load_area_generation(connection)?;
    rows.into_iter()
        .map(|row| {
            let durable = DurableAreaCheckpoint {
                generation_sha256: row.get(0),
                unit_id: row.get(1),
                residency_epoch: u64::try_from(row.get::<_, i64>(2)).unwrap_or(0),
                checkpoint_version: u32::try_from(row.get::<_, i64>(3)).unwrap_or(0),
                checkpoint_sha256: row.get(4),
                captured_logical_tick: u64::try_from(row.get::<_, i64>(5)).unwrap_or(u64::MAX),
                checkpoint_json: row.get(6),
            };
            if active_generation
                .as_ref()
                .is_none_or(|active| active.active_catalog_sha256 != durable.generation_sha256)
            {
                return Err(StoreError::InvalidAreaCheckpoint);
            }
            validate_area_checkpoint(&durable)?;
            Ok(durable)
        })
        .collect()
}

fn verify_exact_area_generation<C: GenericClient>(
    transaction: &mut C,
    expected: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    let row = transaction.query_opt(
        "SELECT world_id, active_catalog_sha256, previous_catalog_sha256,
                activation_sequence
         FROM world_area_generation WHERE singleton = 1 FOR UPDATE",
        &[],
    )?;
    let Some(row) = row else {
        return Err(StoreError::AreaGenerationConflict);
    };
    let actual = DurableAreaGeneration {
        world_id: row.get(0),
        active_catalog_sha256: row.get(1),
        previous_catalog_sha256: row.get(2),
        activation_sequence: u64::try_from(row.get::<_, i64>(3)).unwrap_or(0),
    };
    if actual != *expected {
        return Err(StoreError::AreaGenerationConflict);
    }
    Ok(())
}

fn persist_area_runtime<C: GenericClient>(
    transaction: &mut C,
    runtime: &WorldRuntimeSnapshot,
) -> Result<(), StoreError> {
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, $1, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        &[&adventure_json],
    )?;
    let last_event_sequence: i64 = transaction
        .query_one(
            "SELECT GREATEST(
                 COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
                 COALESCE((SELECT last_event_sequence FROM world_runtime
                           WHERE singleton = 1), 0))",
            &[],
        )?
        .get(0);
    let snapshot_version = i64::from(runtime.snapshot_version);
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, $1, $2, $3, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        &[&snapshot_version, &runtime_json, &last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ($1::BIGINT - $2::BIGINT) AND sequence <= $1::BIGINT",
        &[&last_event_sequence, &DOMAIN_EVENT_RETENTION],
    )?;
    Ok(())
}

fn commit_area_retirement(
    connection: &mut Client,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<(), StoreError> {
    let checkpoint = validate_area_retirement_commit(runtime, expected, durable)?;
    let epoch =
        i64::try_from(durable.residency_epoch).map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    let version = i64::from(durable.checkpoint_version);
    let tick = i64::try_from(durable.captured_logical_tick)
        .map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    let mut transaction = connection.transaction()?;
    verify_exact_area_generation(&mut transaction, expected)?;
    let current = load_runtime_snapshot_for_update(&mut transaction)?
        .ok_or(StoreError::AreaCheckpointConflict)?;
    validate_area_retirement_previous_runtime(&current, runtime, &checkpoint)?;
    let inserted = transaction.execute(
        "INSERT INTO world_area_checkpoints
             (generation_sha256, unit_id, residency_epoch,
              checkpoint_version, checkpoint_sha256, captured_logical_tick,
              checkpoint_json, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7,
                 floor(extract(epoch FROM now()))::BIGINT,
                 floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(generation_sha256, unit_id, residency_epoch) DO NOTHING",
        &[
            &durable.generation_sha256,
            &durable.unit_id,
            &epoch,
            &version,
            &durable.checkpoint_sha256,
            &tick,
            &durable.checkpoint_json,
        ],
    )?;
    if inserted != 1 {
        return Err(StoreError::AreaCheckpointConflict);
    }
    persist_area_runtime(&mut transaction, runtime)?;
    transaction.commit()?;
    Ok(())
}

fn commit_area_rehydration(
    connection: &mut Client,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<(), StoreError> {
    let checkpoint = validate_area_rehydration_commit(runtime, expected, durable)?;
    let epoch =
        i64::try_from(durable.residency_epoch).map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    let version = i64::from(durable.checkpoint_version);
    let tick = i64::try_from(durable.captured_logical_tick)
        .map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    let mut transaction = connection.transaction()?;
    verify_exact_area_generation(&mut transaction, expected)?;
    let current = load_runtime_snapshot_for_update(&mut transaction)?
        .ok_or(StoreError::AreaCheckpointConflict)?;
    validate_area_rehydration_previous_runtime(&current, runtime, durable, &checkpoint)?;
    let deleted = transaction.execute(
        "DELETE FROM world_area_checkpoints
         WHERE generation_sha256 = $1 AND unit_id = $2 AND residency_epoch = $3
           AND checkpoint_version = $4 AND checkpoint_sha256 = $5
           AND captured_logical_tick = $6 AND checkpoint_json = $7",
        &[
            &durable.generation_sha256,
            &durable.unit_id,
            &epoch,
            &version,
            &durable.checkpoint_sha256,
            &tick,
            &durable.checkpoint_json,
        ],
    )?;
    if deleted != 1 {
        return Err(StoreError::AreaCheckpointConflict);
    }
    persist_area_runtime(&mut transaction, runtime)?;
    transaction.commit()?;
    Ok(())
}

fn bind_area_generation_if_absent(
    connection: &mut Client,
    generation: &DurableAreaGeneration,
) -> Result<DurableAreaGeneration, StoreError> {
    validate_area_generation(generation)?;
    if generation.activation_sequence != 1 || generation.previous_catalog_sha256.is_some() {
        return Err(StoreError::InvalidAreaGeneration);
    }
    let mut transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO world_area_generation
             (singleton, world_id, active_catalog_sha256,
              previous_catalog_sha256, activation_sequence, updated_at)
         VALUES (1, $1, $2, NULL, 1,
                 floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO NOTHING",
        &[&generation.world_id, &generation.active_catalog_sha256],
    )?;
    let actual =
        load_area_generation(&mut transaction)?.ok_or(StoreError::InvalidAreaGeneration)?;
    transaction.commit()?;
    Ok(actual)
}

fn commit_area_generation_activation(
    connection: &mut Client,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    next: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    validate_area_generation_transition(expected, next)?;
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let world_adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    let expected_sequence = i64::try_from(expected.activation_sequence)
        .map_err(|_| StoreError::InvalidAreaGeneration)?;
    let next_sequence =
        i64::try_from(next.activation_sequence).map_err(|_| StoreError::InvalidAreaGeneration)?;
    let mut transaction = connection.transaction()?;
    let current = transaction.query_opt(
        "SELECT world_id, active_catalog_sha256, previous_catalog_sha256,
                activation_sequence
         FROM world_area_generation WHERE singleton = 1 FOR UPDATE",
        &[],
    )?;
    let Some(current) = current else {
        return Err(StoreError::AreaGenerationConflict);
    };
    let current = DurableAreaGeneration {
        world_id: current.get(0),
        active_catalog_sha256: current.get(1),
        previous_catalog_sha256: current.get(2),
        activation_sequence: u64::try_from(current.get::<_, i64>(3)).unwrap_or(0),
    };
    if current != *expected {
        return Err(StoreError::AreaGenerationConflict);
    }
    let changed = transaction.execute(
        "UPDATE world_area_generation
         SET active_catalog_sha256 = $1,
             previous_catalog_sha256 = $2,
             activation_sequence = $3,
             updated_at = floor(extract(epoch FROM now()))::BIGINT
         WHERE singleton = 1 AND activation_sequence = $4",
        &[
            &next.active_catalog_sha256,
            &next.previous_catalog_sha256,
            &next_sequence,
            &expected_sequence,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::AreaGenerationConflict);
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, $1, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        &[&world_adventure_json],
    )?;
    let last_event_sequence: i64 = transaction
        .query_one(
            "SELECT GREATEST(
                 COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
                 COALESCE((SELECT last_event_sequence FROM world_runtime
                           WHERE singleton = 1), 0)
             )",
            &[],
        )?
        .get(0);
    let snapshot_version = i64::from(runtime.snapshot_version);
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, $1, $2, $3, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        &[&snapshot_version, &runtime_json, &last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ($1::BIGINT - $2::BIGINT)
           AND sequence <= $1::BIGINT",
        &[&last_event_sequence, &DOMAIN_EVENT_RETENTION],
    )?;
    transaction.commit()?;
    Ok(())
}

fn load_startup_world_recovery(
    connection: &mut Client,
) -> Result<StartupWorldRecovery, StoreError> {
    let mut transaction = connection
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()?;
    let recovery = match load_durable_runtime_snapshot(&mut transaction)? {
        Some((durable_version, snapshot)) => StartupWorldRecovery::Snapshot {
            durable_version,
            snapshot,
        },
        None => match load_startup_domain_events(&mut transaction)? {
            events if events.is_empty() => StartupWorldRecovery::Empty,
            events => StartupWorldRecovery::Ledger(events),
        },
    };
    transaction.commit()?;
    Ok(recovery)
}

fn load_startup_domain_events<C: GenericClient>(
    connection: &mut C,
) -> Result<Vec<DomainEvent>, StoreError> {
    let rows = connection.query(
        "SELECT sequence, event_json
         FROM domain_events
         ORDER BY sequence
         LIMIT $1",
        &[&(DOMAIN_EVENT_RETENTION + 1)],
    )?;
    decode_startup_domain_events(
        rows.into_iter()
            .map(|row| (row.get::<_, i64>(0), row.get::<_, String>(1)))
            .collect(),
    )
}

fn load_domain_events<C: GenericClient>(
    connection: &mut C,
) -> Result<Vec<DomainEvent>, StoreError> {
    connection
        .query(
            "SELECT event_json FROM domain_events ORDER BY sequence",
            &[],
        )?
        .into_iter()
        .map(|row| {
            let json: String = row.get(0);
            serde_json::from_str(&json).map_err(StoreError::from)
        })
        .collect()
}

fn commit_action(
    connection: &mut Client,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
) -> Result<(), StoreError> {
    commit_action_with_retention(
        connection,
        characters,
        runtime,
        events,
        DOMAIN_EVENT_RETENTION,
        None,
    )
}

fn commit_staff_action(
    connection: &mut Client,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    commit_action_with_retention(
        connection,
        characters,
        runtime,
        events,
        DOMAIN_EVENT_RETENTION,
        Some((audit, now)),
    )
}

fn commit_action_with_retention(
    connection: &mut Client,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
    event_retention: i64,
    staff_audit: Option<(StaffAuditRecord, i64)>,
) -> Result<(), StoreError> {
    debug_assert!(event_retention > 0);
    let gameplay_json = characters
        .iter()
        .map(|snapshot| {
            bounded_json(
                &snapshot.gameplay,
                "character gameplay",
                MAX_CHARACTER_JSON_BYTES,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let adventure_json = characters
        .iter()
        .map(|snapshot| {
            bounded_json(
                &serde_json::json!({
                    "coins": snapshot.gameplay.coins,
                    "commodities": snapshot.gameplay.commodities,
                    "quests": snapshot.gameplay.quests,
                }),
                "character adventure",
                MAX_CHARACTER_JSON_BYTES,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let world_adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    let event_json = events
        .iter()
        .map(|event| bounded_json(event, "domain event", MAX_EVENT_JSON_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let staff_audit = staff_audit
        .map(|(audit, now)| prepare_staff_audit(audit).map(|audit| (audit, now)))
        .transpose()?;

    let mut transaction = connection.transaction()?;
    if let Some((audit, _)) = &staff_audit {
        validate_staff_authority(&mut transaction, audit, None, audit.target_account_id)?;
    }
    for ((snapshot, gameplay_json), adventure_json) in
        characters.iter().zip(gameplay_json).zip(adventure_json)
    {
        save(&mut transaction, &snapshot.character)?;
        let updated = transaction.execute(
            "UPDATE characters
             SET gameplay_json = $1,
                 updated_at = floor(extract(epoch FROM now()))::BIGINT
             WHERE id = $2 AND account_id = $3",
            &[
                &gameplay_json,
                &snapshot.character.id.0,
                &snapshot.character.account_id.0,
            ],
        )?;
        if updated == 0 {
            return Err(StoreError::CharacterNotFound {
                account_id: snapshot.character.account_id,
                character_id: snapshot.character.id,
            });
        }
        transaction.execute(
            "INSERT INTO character_adventure (character_id, state_json, updated_at)
             VALUES ($1, $2, floor(extract(epoch FROM now()))::BIGINT)
             ON CONFLICT(character_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at",
            &[&snapshot.character.id.0, &adventure_json],
        )?;
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, $1, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        &[&world_adventure_json],
    )?;
    for json in event_json {
        transaction.execute(
            "INSERT INTO domain_events (event_json) VALUES ($1)",
            &[&json],
        )?;
    }
    let last_event_sequence: i64 = transaction
        .query_one(
            "SELECT GREATEST(
                 COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
                 COALESCE((SELECT last_event_sequence FROM world_runtime
                           WHERE singleton = 1), 0)
             )",
            &[],
        )?
        .get(0);
    let snapshot_version = i64::from(runtime.snapshot_version);
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, $1, $2, $3, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        &[&snapshot_version, &runtime_json, &last_event_sequence],
    )?;
    transaction.execute(
        // Prepared parameters are initially `unknown` to PostgreSQL. Keep the
        // casts on both arithmetic operands so PostgreSQL 18 can resolve `-`.
        "DELETE FROM domain_events
         WHERE sequence <= ($1::BIGINT - $2::BIGINT)
           AND sequence <= $1::BIGINT",
        &[&last_event_sequence, &event_retention],
    )?;
    if let Some((audit, now)) = &staff_audit {
        insert_staff_audit(&mut transaction, audit, *now)?;
    }
    transaction.commit()?;
    Ok(())
}

fn role_from_database(value: &str) -> Result<AccountRole, StoreError> {
    match value {
        "player" => Ok(AccountRole::Player),
        "moderator" => Ok(AccountRole::Moderator),
        "admin" => Ok(AccountRole::Admin),
        _ => Err(StoreError::InvalidSocialInput),
    }
}

fn role_as_database(role: AccountRole) -> &'static str {
    match role {
        AccountRole::Player => "player",
        AccountRole::Moderator => "moderator",
        AccountRole::Admin => "admin",
    }
}

fn social_channel_from_database(value: &str) -> Result<SocialChannelName, StoreError> {
    match value {
        "general" => Ok(SocialChannelName::General),
        "newbie" => Ok(SocialChannelName::Newbie),
        "staff" => Ok(SocialChannelName::Staff),
        _ => Err(StoreError::InvalidSocialInput),
    }
}

fn ensure_social_character_exists<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
) -> Result<(), StoreError> {
    let exists: bool = connection
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM characters WHERE id = $1)",
            &[&character_id.0],
        )?
        .get(0);
    if exists {
        Ok(())
    } else {
        Err(StoreError::SocialTargetNotFound)
    }
}

fn social_profile<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let row = connection
        .query_opt(
            "SELECT characters.id, characters.account_id, characters.name, accounts.role,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = accounts.id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > $2)
                    )
             FROM characters
             JOIN accounts ON accounts.id = characters.account_id
             WHERE characters.id = $1",
            &[&character_id.0, &now],
        )?
        .ok_or(StoreError::SocialTargetNotFound)?;
    let role: String = row.get(3);
    Ok(SocialProfile {
        character_id: CharacterId(row.get(0)),
        account_id: AccountId(row.get(1)),
        character_name: row.get(2),
        role: role_from_database(&role)?,
        muted: row.get(4),
    })
}

fn social_target<C: GenericClient>(
    connection: &mut C,
    name: &str,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let name = normalise_name(name, "social target")?;
    let matches = connection.query(
        "SELECT id FROM characters WHERE lower(name) = lower($1) ORDER BY id LIMIT 2",
        &[&name],
    )?;
    match matches.as_slice() {
        [row] => social_profile(connection, CharacterId(row.get(0)), now),
        [] => Err(StoreError::SocialTargetNotFound),
        [_, ..] => Err(StoreError::AmbiguousSocialTarget),
    }
}

fn social_delivery_allowed<C: GenericClient>(
    connection: &mut C,
    sender: CharacterId,
    recipient: CharacterId,
) -> Result<bool, StoreError> {
    if sender == recipient {
        return Ok(true);
    }
    let both_exist: i64 = connection
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM characters WHERE id = ANY($1)",
            &[&[sender.0, recipient.0].as_slice()],
        )?
        .get(0);
    if both_exist != 2 {
        return Err(StoreError::SocialTargetNotFound);
    }
    let suppressed: bool = connection
        .query_one(
            "SELECT
                 EXISTS(SELECT 1 FROM social_ignores
                        WHERE owner_character_id = $2 AND target_character_id = $1)
                 OR EXISTS(SELECT 1 FROM social_blocks
                           WHERE owner_character_id = $2 AND target_character_id = $1)
                 OR EXISTS(SELECT 1 FROM social_blocks
                           WHERE owner_character_id = $1 AND target_character_id = $2)",
            &[&sender.0, &recipient.0],
        )?
        .get(0);
    Ok(!suppressed)
}

fn set_social_relation(
    connection: &mut Client,
    actor: CharacterId,
    target: CharacterId,
    relation: SocialRelationKind,
    enabled: bool,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let mut transaction = connection.transaction()?;
    let actor_profile = social_profile(&mut transaction, actor, now)?;
    let target_profile = social_profile(&mut transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let table = match relation {
        SocialRelationKind::Ignore => "social_ignores",
        SocialRelationKind::Block => "social_blocks",
    };
    if enabled {
        transaction.execute(
            &format!(
                "INSERT INTO {table}
                     (owner_character_id, target_character_id, created_at)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
            ),
            &[&actor.0, &target.0, &now],
        )?;
    } else {
        transaction.execute(
            &format!(
                "DELETE FROM {table}
                 WHERE owner_character_id = $1 AND target_character_id = $2"
            ),
            &[&actor.0, &target.0],
        )?;
    }
    let action = match (relation, enabled) {
        (SocialRelationKind::Ignore, true) => "ignore",
        (SocialRelationKind::Ignore, false) => "unignore",
        (SocialRelationKind::Block, true) => "block",
        (SocialRelationKind::Block, false) => "unblock",
    };
    append_audit(
        &mut transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action,
        serde_json::json!({"targetCharacterId": target.0}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn social_channel_subscriptions<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
) -> Result<Vec<SocialChannelName>, StoreError> {
    ensure_social_character_exists(connection, character_id)?;
    connection
        .query(
            "SELECT channel FROM social_channel_subscriptions
             WHERE character_id = $1
             ORDER BY CASE channel
                 WHEN 'general' THEN 0
                 WHEN 'newbie' THEN 1
                 WHEN 'staff' THEN 2
                 ELSE 3
             END",
            &[&character_id.0],
        )?
        .into_iter()
        .map(|row| {
            let channel: String = row.get(0);
            social_channel_from_database(&channel)
        })
        .collect()
}

fn social_channel_subscribers<C: GenericClient>(
    connection: &mut C,
    channel: SocialChannelName,
) -> Result<Vec<CharacterId>, StoreError> {
    Ok(connection
        .query(
            "SELECT character_id FROM social_channel_subscriptions
             WHERE channel = $1 ORDER BY character_id",
            &[&channel.as_str()],
        )?
        .into_iter()
        .map(|row| CharacterId(row.get(0)))
        .collect())
}

fn set_social_channel_subscription(
    connection: &mut Client,
    character_id: CharacterId,
    channel: SocialChannelName,
    subscribed: bool,
    now: i64,
) -> Result<(), StoreError> {
    let mut transaction = connection.transaction()?;
    ensure_social_character_exists(&mut transaction, character_id)?;
    if subscribed {
        transaction.execute(
            "INSERT INTO social_channel_subscriptions
                 (character_id, channel, created_at)
             VALUES ($1, $2, $3)
             ON CONFLICT(character_id, channel) DO NOTHING",
            &[&character_id.0, &channel.as_str(), &now],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM social_channel_subscriptions
             WHERE character_id = $1 AND channel = $2",
            &[&character_id.0, &channel.as_str()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn normalise_mail_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    if !valid_mail_visible_text(value, maximum) {
        Err(StoreError::InvalidMailInput)
    } else {
        Ok(value.to_owned())
    }
}

fn send_mail(
    connection: &mut Client,
    sender: CharacterId,
    target_name: &str,
    public_id: &str,
    subject: &str,
    body: &str,
    now: i64,
) -> Result<MailSendResult, StoreError> {
    let target_name = normalise_mail_text(target_name, tiamud_core::MAX_MAIL_TARGET_BYTES)?;
    let subject = normalise_mail_text(subject, MAX_MAIL_SUBJECT_BYTES)?;
    let body = normalise_mail_text(body, MAX_MAIL_BODY_BYTES)?;
    if !tiamud_core::valid_mail_target(&target_name) || !tiamud_core::valid_mail_subject(&subject) {
        return Err(StoreError::InvalidMailInput);
    }
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let expires_at = now
        .checked_add(MAIL_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidMailInput)?;
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&MAIL_TRANSACTION_LOCK_ID],
    )?;
    let sender_row = transaction
        .query_opt(
            "SELECT characters.account_id, characters.name,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = characters.account_id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > $2)
                    )
             FROM characters WHERE characters.id = $1",
            &[&sender.0, &now],
        )?
        .ok_or(StoreError::SocialTargetNotFound)?;
    let sender_account: i64 = sender_row.get(0);
    let sender_name: String = sender_row.get(1);
    let sender_muted: bool = sender_row.get(2);

    transaction.query(
        "SELECT id FROM accounts WHERE id = $1 FOR UPDATE",
        &[&sender_account],
    )?;

    let usage = transaction.query_opt(
        "SELECT hour_started_at, hour_count, day_started_at, day_count
         FROM social_mail_usage WHERE account_id = $1 FOR UPDATE",
        &[&sender_account],
    )?;
    let (hour_started_at, hour_count, day_started_at, day_count) =
        usage.map_or((now, 0_i64, now, 0_i64), |row| {
            let stored_hour: i64 = row.get(0);
            let stored_hour_count: i32 = row.get(1);
            let stored_day: i64 = row.get(2);
            let stored_day_count: i32 = row.get(3);
            let hour_expired =
                now < stored_hour || now.saturating_sub(stored_hour) >= MAIL_HOUR_SECONDS;
            let day_expired =
                now < stored_day || now.saturating_sub(stored_day) >= MAIL_DAY_SECONDS;
            (
                if hour_expired { now } else { stored_hour },
                if hour_expired {
                    0
                } else {
                    i64::from(stored_hour_count)
                },
                if day_expired { now } else { stored_day },
                if day_expired {
                    0
                } else {
                    i64::from(stored_day_count)
                },
            )
        });
    if hour_count >= MAIL_HOURLY_CAPACITY || day_count >= MAIL_DAILY_CAPACITY {
        return Err(StoreError::MailRateLimited);
    }
    let next_hour_count = i32::try_from(hour_count + 1).expect("mail hourly cap fits i32");
    let next_day_count = i32::try_from(day_count + 1).expect("mail daily cap fits i32");
    transaction.execute(
        "INSERT INTO social_mail_usage
             (account_id, hour_started_at, hour_count,
              day_started_at, day_count, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT(account_id) DO UPDATE SET
             hour_started_at = excluded.hour_started_at,
             hour_count = excluded.hour_count,
             day_started_at = excluded.day_started_at,
             day_count = excluded.day_count,
             updated_at = excluded.updated_at",
        &[
            &sender_account,
            &hour_started_at,
            &next_hour_count,
            &day_started_at,
            &next_day_count,
            &now,
        ],
    )?;

    if sender_muted {
        transaction.commit()?;
        return Err(StoreError::ChatMuted);
    }
    if !valid_mail_visible_text(&sender_name, MAX_IDENTITY_NAME_BYTES) {
        transaction.commit()?;
        return Ok(MailSendResult::Suppressed);
    }
    let duplicate_public_id: bool = transaction
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM social_mail_messages WHERE public_id = $1)",
            &[&public_id],
        )?
        .get(0);
    if duplicate_public_id {
        transaction.commit()?;
        return Ok(MailSendResult::Unavailable);
    }

    let target_rows = transaction.query(
        "SELECT characters.id, characters.account_id
         FROM characters
         WHERE characters.name = $1
         ORDER BY characters.id LIMIT 2",
        &[&target_name],
    )?;
    let target = match target_rows.as_slice() {
        [row] => Some((row.get::<_, i64>(0), row.get::<_, i64>(1))),
        [] | [_, ..] => None,
    };
    let Some((recipient, recipient_account)) = target else {
        transaction.commit()?;
        return Ok(MailSendResult::Unavailable);
    };
    if recipient == sender.0 {
        transaction.commit()?;
        return Ok(MailSendResult::Unavailable);
    }
    transaction.query(
        "SELECT id FROM accounts WHERE id = $1 FOR UPDATE",
        &[&recipient_account],
    )?;

    let recipient_banned: bool = transaction
        .query_one(
            "SELECT EXISTS(
                 SELECT 1 FROM account_bans
                 WHERE account_id = $1 AND revoked_at IS NULL
                   AND (expires_at IS NULL OR expires_at > $2)
             )",
            &[&recipient_account, &now],
        )?
        .get(0);
    let policy_suppressed: bool = transaction
        .query_one(
            "SELECT
                 EXISTS(SELECT 1 FROM social_ignores
                        WHERE owner_character_id = $2 AND target_character_id = $1)
                 OR EXISTS(SELECT 1 FROM social_blocks
                           WHERE owner_character_id = $2 AND target_character_id = $1)
                 OR EXISTS(SELECT 1 FROM social_blocks
                           WHERE owner_character_id = $1 AND target_character_id = $2)",
            &[&sender.0, &recipient],
        )?
        .get(0);
    let mailbox = transaction.query_one(
        "SELECT COUNT(*)::BIGINT,
                COALESCE(SUM(octet_length(subject) + octet_length(body)), 0)::BIGINT
         FROM social_mail_messages
         WHERE recipient_character_id = $1 AND expires_at > $2",
        &[&recipient, &now],
    )?;
    let mailbox_count: i64 = mailbox.get(0);
    let mailbox_bytes: i64 = mailbox.get(1);
    let added_bytes =
        i64::try_from(subject.len() + body.len()).map_err(|_| StoreError::InvalidMailInput)?;
    let global_count: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM social_mail_messages WHERE expires_at > $1",
            &[&now],
        )?
        .get(0);
    let suppressed = recipient_banned
        || policy_suppressed
        || mailbox_count >= MAILBOX_CAPACITY
        || mailbox_bytes.saturating_add(added_bytes) > MAILBOX_BYTE_CAPACITY
        || global_count >= MAIL_GLOBAL_CAPACITY;
    if suppressed {
        transaction.commit()?;
        return Ok(MailSendResult::Suppressed);
    }

    transaction.execute(
        "INSERT INTO social_mail_messages
             (public_id, sender_character_id, sender_name,
              recipient_character_id, subject, body, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[
            &public_id,
            &sender.0,
            &sender_name,
            &recipient,
            &subject,
            &body,
            &now,
            &expires_at,
        ],
    )?;
    transaction.commit()?;
    Ok(MailSendResult::Stored)
}

fn mailbox_page(
    connection: &mut Client,
    recipient: CharacterId,
    page: u32,
    now: i64,
) -> Result<MailPage, StoreError> {
    if !(1..=u32::try_from(MAIL_MAX_PAGES).expect("mail page cap fits u32")).contains(&page) {
        return Err(StoreError::InvalidMailInput);
    }
    ensure_social_character_exists(connection, recipient)?;
    let totals = connection.query_one(
        "SELECT COUNT(*)::BIGINT,
                COUNT(*) FILTER (WHERE read_at IS NULL)::BIGINT
         FROM social_mail_messages
         WHERE recipient_character_id = $1 AND expires_at > $2",
        &[&recipient.0, &now],
    )?;
    let total: i64 = totals.get(0);
    let unread: i64 = totals.get(1);
    let total_pages = ((total + MAIL_PAGE_SIZE - 1) / MAIL_PAGE_SIZE).max(1);
    if total_pages > MAIL_MAX_PAGES {
        return Err(StoreError::InvalidMailInput);
    }
    let offset = (i64::from(page) - 1) * MAIL_PAGE_SIZE;
    let messages = connection
        .query(
            "SELECT public_id, sender_name, subject, created_at, expires_at,
                    read_at IS NOT NULL
             FROM social_mail_messages
             WHERE recipient_character_id = $1 AND expires_at > $2
             ORDER BY created_at DESC, id DESC
             LIMIT $3 OFFSET $4",
            &[&recipient.0, &now, &MAIL_PAGE_SIZE, &offset],
        )?
        .into_iter()
        .map(|row| MailMessageSummary {
            public_id: row.get(0),
            sender_name: row.get(1),
            subject: row.get(2),
            created_at: row.get(3),
            expires_at: row.get(4),
            read: row.get(5),
        })
        .collect();
    Ok(MailPage {
        page,
        total_pages: u32::try_from(total_pages).map_err(|_| StoreError::InvalidMailInput)?,
        total_messages: u32::try_from(total).map_err(|_| StoreError::InvalidMailInput)?,
        unread_count: u32::try_from(unread).map_err(|_| StoreError::InvalidMailInput)?,
        capacity: u32::try_from(MAILBOX_CAPACITY).expect("mailbox cap fits u32"),
        messages,
    })
}

fn read_mail(
    connection: &mut Client,
    recipient: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<MailMessage, StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let mut transaction = connection.transaction()?;
    ensure_social_character_exists(&mut transaction, recipient)?;
    let row = transaction
        .query_opt(
            "SELECT public_id, sender_name, subject, body, created_at, expires_at
             FROM social_mail_messages
             WHERE recipient_character_id = $1 AND public_id = $2 AND expires_at > $3
             FOR UPDATE",
            &[&recipient.0, &public_id, &now],
        )?
        .ok_or(StoreError::MailNotFound)?;
    let message = MailMessage {
        summary: MailMessageSummary {
            public_id: row.get(0),
            sender_name: row.get(1),
            subject: row.get(2),
            created_at: row.get(4),
            expires_at: row.get(5),
            read: true,
        },
        body: row.get(3),
    };
    transaction.execute(
        "UPDATE social_mail_messages SET read_at = COALESCE(read_at, $1)
         WHERE recipient_character_id = $2 AND public_id = $3",
        &[&now, &recipient.0, &public_id],
    )?;
    transaction.commit()?;
    Ok(message)
}

fn delete_mail(
    connection: &mut Client,
    recipient: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let mut transaction = connection.transaction()?;
    ensure_social_character_exists(&mut transaction, recipient)?;
    let deleted = transaction.execute(
        "DELETE FROM social_mail_messages
         WHERE recipient_character_id = $1 AND public_id = $2 AND expires_at > $3",
        &[&recipient.0, &public_id, &now],
    )?;
    if deleted != 1 {
        return Err(StoreError::MailNotFound);
    }
    transaction.commit()?;
    Ok(())
}

fn report_mail(
    connection: &mut Client,
    recipient: CharacterId,
    public_id: &str,
    reason: &str,
    now: i64,
) -> Result<i64, StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let reason = normalise_mail_text(reason, MAX_MAIL_REPORT_REASON_BYTES)?;
    let evidence_expires_at = now
        .checked_add(MAIL_REPORT_EVIDENCE_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidMailInput)?;
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&MAIL_TRANSACTION_LOCK_ID],
    )?;
    let reporter = social_profile(&mut transaction, recipient, now)?;
    let recipient_account = reporter.account_id.0;
    let row = transaction
        .query_opt(
            "SELECT sender_character_id, sender_name, subject, body,
                    created_at, report_id
             FROM social_mail_messages
             WHERE recipient_character_id = $1 AND public_id = $2 AND expires_at > $3
             FOR UPDATE",
            &[&recipient.0, &public_id, &now],
        )?
        .ok_or(StoreError::MailNotFound)?;
    let sender: Option<i64> = row.get(0);
    let sender = sender.ok_or(StoreError::MailNotFound)?;
    let sender_profile = social_profile(&mut transaction, CharacterId(sender), now)?;
    let existing_report: Option<i64> = row.get(5);
    if existing_report.is_some() {
        return Err(StoreError::MailAlreadyReported);
    }
    let open_reports: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT
             FROM social_reports AS reports
             JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
             WHERE reporter.account_id = $1
               AND reports.report_kind = 'mail'
               AND reports.status = 'open'",
            &[&recipient_account],
        )?
        .get(0);
    if open_reports >= MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        return Err(StoreError::MailReportLimit);
    }
    let evidence_count: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM social_mail_report_evidence
             WHERE expires_at > $1",
            &[&now],
        )?
        .get(0);
    if evidence_count >= MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY {
        return Err(StoreError::MailEvidenceLimit);
    }

    let report_id: i64 = transaction
        .query_one(
            "INSERT INTO social_reports
                 (reporter_character_id, target_character_id, report_kind, reason, created_at)
             VALUES ($1, $2, 'mail', $3, $4) RETURNING id",
            &[&recipient.0, &sender, &reason, &now],
        )?
        .get(0);
    let sender_name: String = row.get(1);
    let subject: String = row.get(2);
    let body: String = row.get(3);
    let sent_at: i64 = row.get(4);
    transaction.execute(
        "INSERT INTO social_mail_report_evidence
             (report_id, sender_name, subject, body, sent_at, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &report_id,
            &sender_name,
            &subject,
            &body,
            &sent_at,
            &now,
            &evidence_expires_at,
        ],
    )?;
    let linked = transaction.execute(
        "UPDATE social_mail_messages SET report_id = $1
         WHERE recipient_character_id = $2 AND public_id = $3
           AND report_id IS NULL AND expires_at > $4",
        &[&report_id, &recipient.0, &public_id, &now],
    )?;
    if linked != 1 {
        return Err(StoreError::MailAlreadyReported);
    }
    append_audit(
        &mut transaction,
        reporter.account_id,
        Some(sender_profile.account_id),
        "mail_report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(report_id)
}

fn purge_expired_mail(connection: &mut Client, now: i64) -> Result<u64, StoreError> {
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&MAIL_TRANSACTION_LOCK_ID],
    )?;
    let deleted = purge_expired_mail_rows(&mut transaction, now, MAIL_PURGE_LIMIT)?;
    transaction.commit()?;
    u64::try_from(deleted).map_err(|_| StoreError::InvalidMailInput)
}

fn purge_expired_mail_rows<C: GenericClient>(
    connection: &mut C,
    now: i64,
    limit: i64,
) -> Result<i64, StoreError> {
    debug_assert!(limit > 0);
    let message_rows = connection.query(
        "WITH expired AS (
             SELECT id FROM social_mail_messages
             WHERE expires_at <= $1 ORDER BY expires_at, id LIMIT $2
         )
         DELETE FROM social_mail_messages AS messages
         USING expired WHERE messages.id = expired.id
         RETURNING messages.id",
        &[&now, &limit],
    )?;
    let message_count = i64::try_from(message_rows.len()).expect("purge batch fits i64");
    let remaining = limit - message_count;
    if remaining == 0 {
        return Ok(message_count);
    }
    let evidence_rows = connection.query(
        "WITH expired AS (
             SELECT report_id FROM social_mail_report_evidence
             WHERE expires_at <= $1 ORDER BY expires_at, report_id LIMIT $2
         )
         DELETE FROM social_mail_report_evidence AS evidence
         USING expired WHERE evidence.report_id = expired.report_id
         RETURNING evidence.report_id",
        &[&now, &remaining],
    )?;
    Ok(message_count + i64::try_from(evidence_rows.len()).expect("purge batch fits i64"))
}

fn normalise_board_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    if valid_board_visible_text(value, maximum) {
        Ok(value.to_owned())
    } else {
        Err(StoreError::InvalidBoardInput)
    }
}

fn check_board_capacity(
    account_count: i64,
    board_count: i64,
    board_bytes: i64,
    global_count: i64,
    global_bytes: i64,
    added_bytes: i64,
) -> Result<(), StoreError> {
    if account_count >= i64::from(BOARD_ACCOUNT_CAPACITY)
        || board_count >= i64::from(BOARD_CAPACITY_PER_BOARD)
        || board_bytes.saturating_add(added_bytes) > BOARD_BYTE_CAPACITY_PER_BOARD
        || global_count >= BOARD_GLOBAL_CAPACITY
        || global_bytes.saturating_add(added_bytes) > BOARD_GLOBAL_BYTE_CAPACITY
    {
        Err(StoreError::BoardCapacity)
    } else {
        Ok(())
    }
}

fn check_board_report_capacity(open_reports: i64, evidence_count: i64) -> Result<(), StoreError> {
    if open_reports >= BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        Err(StoreError::BoardReportLimit)
    } else if evidence_count >= BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY {
        Err(StoreError::BoardEvidenceLimit)
    } else {
        Ok(())
    }
}

fn consume_board_usage<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    let usage = connection.query_opt(
        "SELECT hour_started_at, hour_count, day_started_at, day_count
         FROM social_board_usage WHERE account_id = $1 FOR UPDATE",
        &[&account_id.0],
    )?;
    let (hour_started_at, hour_count, day_started_at, day_count) =
        usage.map_or((now, 0_i64, now, 0_i64), |row| {
            let stored_hour: i64 = row.get(0);
            let stored_hour_count: i32 = row.get(1);
            let stored_day: i64 = row.get(2);
            let stored_day_count: i32 = row.get(3);
            let hour_expired =
                now < stored_hour || now.saturating_sub(stored_hour) >= MAIL_HOUR_SECONDS;
            let day_expired =
                now < stored_day || now.saturating_sub(stored_day) >= MAIL_DAY_SECONDS;
            (
                if hour_expired { now } else { stored_hour },
                if hour_expired {
                    0
                } else {
                    i64::from(stored_hour_count)
                },
                if day_expired { now } else { stored_day },
                if day_expired {
                    0
                } else {
                    i64::from(stored_day_count)
                },
            )
        });
    if hour_count >= BOARD_HOURLY_CAPACITY || day_count >= BOARD_DAILY_CAPACITY {
        return Err(StoreError::BoardRateLimited);
    }
    let next_hour_count = i32::try_from(hour_count + 1).expect("board hourly cap fits i32");
    let next_day_count = i32::try_from(day_count + 1).expect("board daily cap fits i32");
    connection.execute(
        "INSERT INTO social_board_usage
             (account_id, hour_started_at, hour_count,
              day_started_at, day_count, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT(account_id) DO UPDATE SET
             hour_started_at = excluded.hour_started_at,
             hour_count = excluded.hour_count,
             day_started_at = excluded.day_started_at,
             day_count = excluded.day_count,
             updated_at = excluded.updated_at",
        &[
            &account_id.0,
            &hour_started_at,
            &next_hour_count,
            &day_started_at,
            &next_day_count,
            &now,
        ],
    )?;
    Ok(())
}

fn list_board_posts(
    connection: &mut Client,
    actor: CharacterId,
    board: BoardKey,
    page: u32,
    now: i64,
) -> Result<BoardPage, StoreError> {
    if !(1..=BOARD_MAX_PAGES).contains(&page) {
        return Err(StoreError::InvalidBoardInput);
    }
    let actor_profile = social_profile(connection, actor, now)?;
    let total: i64 = connection
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM social_board_posts
             WHERE board_key = $1 AND expires_at > $2",
            &[&board.as_str(), &now],
        )?
        .get(0);
    let total_posts = u32::try_from(total).map_err(|_| StoreError::InvalidBoardInput)?;
    let total_pages = total_posts.div_ceil(BOARD_PAGE_SIZE).max(1);
    if total_pages > BOARD_MAX_PAGES {
        return Err(StoreError::InvalidBoardInput);
    }
    let limit = i64::from(BOARD_PAGE_SIZE);
    let offset = i64::from((page - 1).saturating_mul(BOARD_PAGE_SIZE));
    let posts = connection
        .query(
            "SELECT posts.public_id, posts.author_character_id, authors.account_id,
                    posts.author_name, posts.subject, posts.created_at, posts.expires_at,
                    posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.board_key = $1 AND posts.expires_at > $2
             ORDER BY posts.created_at DESC, posts.id DESC
             LIMIT $3 OFFSET $4",
            &[&board.as_str(), &now, &limit, &offset],
        )?
        .into_iter()
        .map(|row| {
            let author_character = CharacterId(row.get(1));
            let author_account = AccountId(row.get(2));
            let report_id: Option<i64> = row.get(7);
            BoardPostSummary {
                public_id: row.get(0),
                board,
                author_name: row.get(3),
                subject: row.get(4),
                created_at: row.get(5),
                expires_at: row.get(6),
                can_delete: author_character == actor,
                can_report: author_account != actor_profile.account_id && report_id.is_none(),
            }
        })
        .collect();
    Ok(BoardPage {
        board,
        page,
        total_pages,
        total_posts,
        capacity: BOARD_CAPACITY_PER_BOARD,
        posts,
    })
}

fn read_board_post(
    connection: &mut Client,
    actor: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<BoardEntry, StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let actor_profile = social_profile(connection, actor, now)?;
    let row = connection
        .query_opt(
            "SELECT posts.board_key, posts.author_character_id, authors.account_id,
                    posts.author_name, posts.subject, posts.body,
                    posts.created_at, posts.expires_at, posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.public_id = $1 AND posts.expires_at > $2",
            &[&public_id, &now],
        )?
        .ok_or(StoreError::BoardPostUnavailable)?;
    let board_key: String = row.get(0);
    let board = BoardKey::parse(&board_key).ok_or(StoreError::InvalidBoardInput)?;
    let author_character = CharacterId(row.get(1));
    let author_account = AccountId(row.get(2));
    let report_id: Option<i64> = row.get(8);
    Ok(BoardEntry {
        summary: BoardPostSummary {
            public_id: public_id.to_owned(),
            board,
            author_name: row.get(3),
            subject: row.get(4),
            created_at: row.get(6),
            expires_at: row.get(7),
            can_delete: author_character == actor,
            can_report: author_account != actor_profile.account_id && report_id.is_none(),
        },
        body: row.get(5),
    })
}

fn publish_board_post(
    connection: &mut Client,
    actor: CharacterId,
    board: BoardKey,
    public_id: &str,
    subject: &str,
    body: &str,
    now: i64,
) -> Result<BoardEntry, StoreError> {
    if !valid_board_public_id(public_id) || !valid_board_subject(subject) {
        return Err(StoreError::InvalidBoardInput);
    }
    let subject = normalise_board_text(subject, MAX_BOARD_SUBJECT_BYTES)?;
    let body = normalise_board_text(body, MAX_BOARD_BODY_BYTES)?;
    let expires_at = now
        .checked_add(BOARD_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidBoardInput)?;
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&BOARD_TRANSACTION_LOCK_ID],
    )?;
    let profile = social_profile(&mut transaction, actor, now)?;
    if profile.muted {
        return Err(StoreError::ChatMuted);
    }
    if account_is_banned(&mut transaction, profile.account_id, now)? {
        return Err(StoreError::AccountBanned);
    }
    transaction.query(
        "SELECT id FROM accounts WHERE id = $1 FOR UPDATE",
        &[&profile.account_id.0],
    )?;
    consume_board_usage(&mut transaction, profile.account_id, now)?;
    let duplicate: bool = transaction
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM social_board_posts WHERE public_id = $1)",
            &[&public_id],
        )?
        .get(0);
    if duplicate {
        return Err(StoreError::InvalidBoardInput);
    }
    let account_count: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE authors.account_id = $1 AND posts.expires_at > $2",
            &[&profile.account_id.0, &now],
        )?
        .get(0);
    let board_totals = transaction.query_one(
        "SELECT COUNT(*)::BIGINT,
                COALESCE(SUM(octet_length(subject) + octet_length(body)), 0)::BIGINT
         FROM social_board_posts WHERE board_key = $1 AND expires_at > $2",
        &[&board.as_str(), &now],
    )?;
    let board_count: i64 = board_totals.get(0);
    let board_bytes: i64 = board_totals.get(1);
    let global_totals = transaction.query_one(
        "SELECT COUNT(*)::BIGINT,
                COALESCE(SUM(octet_length(subject) + octet_length(body)), 0)::BIGINT
         FROM social_board_posts WHERE expires_at > $1",
        &[&now],
    )?;
    let global_count: i64 = global_totals.get(0);
    let global_bytes: i64 = global_totals.get(1);
    let added_bytes =
        i64::try_from(subject.len() + body.len()).map_err(|_| StoreError::InvalidBoardInput)?;
    if let Err(error) = check_board_capacity(
        account_count,
        board_count,
        board_bytes,
        global_count,
        global_bytes,
        added_bytes,
    ) {
        transaction.commit()?;
        return Err(error);
    }
    transaction.execute(
        "INSERT INTO social_board_posts
             (public_id, board_key, author_character_id, author_name,
              subject, body, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[
            &public_id,
            &board.as_str(),
            &actor.0,
            &profile.character_name,
            &subject,
            &body,
            &now,
            &expires_at,
        ],
    )?;
    transaction.commit()?;
    Ok(BoardEntry {
        summary: BoardPostSummary {
            public_id: public_id.to_owned(),
            board,
            author_name: profile.character_name,
            subject,
            created_at: now,
            expires_at,
            can_delete: true,
            can_report: false,
        },
        body,
    })
}

fn delete_board_post(
    connection: &mut Client,
    actor: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&BOARD_TRANSACTION_LOCK_ID],
    )?;
    social_profile(&mut transaction, actor, now)?;
    let deleted = transaction.execute(
        "DELETE FROM social_board_posts
         WHERE public_id = $1 AND author_character_id = $2 AND expires_at > $3",
        &[&public_id, &actor.0, &now],
    )?;
    if deleted != 1 {
        return Err(StoreError::BoardPostUnavailable);
    }
    transaction.commit()?;
    Ok(())
}

fn report_board_post(
    connection: &mut Client,
    actor: CharacterId,
    public_id: &str,
    reason: &str,
    now: i64,
) -> Result<i64, StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let reason = normalise_board_text(reason, MAX_BOARD_REPORT_REASON_BYTES)?;
    let evidence_expires_at = now
        .checked_add(BOARD_REPORT_EVIDENCE_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidBoardInput)?;
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&BOARD_TRANSACTION_LOCK_ID],
    )?;
    let reporter = social_profile(&mut transaction, actor, now)?;
    let post = transaction
        .query_opt(
            "SELECT posts.author_character_id, authors.account_id, posts.board_key,
                    posts.author_name, posts.subject, posts.body,
                    posts.created_at, posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.public_id = $1 AND posts.expires_at > $2
             FOR UPDATE OF posts",
            &[&public_id, &now],
        )?
        .ok_or(StoreError::BoardPostUnavailable)?;
    let author_character = CharacterId(post.get(0));
    let author_account = AccountId(post.get(1));
    let report_id: Option<i64> = post.get(7);
    if report_id.is_some() {
        return Err(StoreError::BoardAlreadyReported);
    }
    if author_account == reporter.account_id {
        return Err(StoreError::BoardPostUnavailable);
    }
    let open_reports: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT
             FROM social_reports AS reports
             JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
             WHERE reporter.account_id = $1 AND reports.report_kind = 'board'
               AND reports.status = 'open'",
            &[&reporter.account_id.0],
        )?
        .get(0);
    check_board_report_capacity(open_reports, 0)?;
    let evidence_count: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM social_board_report_evidence
             WHERE expires_at > $1",
            &[&now],
        )?
        .get(0);
    check_board_report_capacity(open_reports, evidence_count)?;
    let report_id: i64 = transaction
        .query_one(
            "INSERT INTO social_reports
                 (reporter_character_id, target_character_id, report_kind, reason, created_at)
             VALUES ($1, $2, 'board', $3, $4) RETURNING id",
            &[&actor.0, &author_character.0, &reason, &now],
        )?
        .get(0);
    let board_key: String = post.get(2);
    let author_name: String = post.get(3);
    let subject: String = post.get(4);
    let body: String = post.get(5);
    let posted_at: i64 = post.get(6);
    transaction.execute(
        "INSERT INTO social_board_report_evidence
             (report_id, board_key, post_public_id, author_name,
              subject, body, posted_at, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &report_id,
            &board_key,
            &public_id,
            &author_name,
            &subject,
            &body,
            &posted_at,
            &now,
            &evidence_expires_at,
        ],
    )?;
    let linked = transaction.execute(
        "UPDATE social_board_posts SET report_id = $1
         WHERE public_id = $2 AND report_id IS NULL AND expires_at > $3",
        &[&report_id, &public_id, &now],
    )?;
    if linked != 1 {
        return Err(StoreError::BoardAlreadyReported);
    }
    append_audit(
        &mut transaction,
        reporter.account_id,
        Some(author_account),
        "board_report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(report_id)
}

fn purge_expired_boards(connection: &mut Client, now: i64) -> Result<u64, StoreError> {
    let mut transaction = connection.transaction()?;
    transaction.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&BOARD_TRANSACTION_LOCK_ID],
    )?;
    let post_rows = transaction.query(
        "WITH expired AS (
             SELECT id FROM social_board_posts
             WHERE expires_at <= $1 ORDER BY expires_at, id LIMIT $2
         )
         DELETE FROM social_board_posts AS posts
         USING expired WHERE posts.id = expired.id RETURNING posts.id",
        &[&now, &BOARD_PURGE_LIMIT],
    )?;
    let post_count = i64::try_from(post_rows.len()).expect("board purge batch fits i64");
    let remaining = BOARD_PURGE_LIMIT - post_count;
    let evidence_count = if remaining > 0 {
        let rows = transaction.query(
            "WITH expired AS (
                 SELECT report_id FROM social_board_report_evidence
                 WHERE expires_at <= $1 ORDER BY expires_at, report_id LIMIT $2
             )
             DELETE FROM social_board_report_evidence AS evidence
             USING expired WHERE evidence.report_id = expired.report_id
             RETURNING evidence.report_id",
            &[&now, &remaining],
        )?;
        i64::try_from(rows.len()).expect("board evidence purge batch fits i64")
    } else {
        0
    };
    transaction.commit()?;
    u64::try_from(post_count + evidence_count).map_err(|_| StoreError::InvalidBoardInput)
}

fn report_character(
    connection: &mut Client,
    actor: CharacterId,
    target: CharacterId,
    reason: &str,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let reason = normalise_social_text(reason, MAX_REPORT_REASON_BYTES)?;
    let mut transaction = connection.transaction()?;
    let actor_profile = social_profile(&mut transaction, actor, now)?;
    let target_profile = social_profile(&mut transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let report_id: i64 = transaction
        .query_one(
            "INSERT INTO social_reports
                 (reporter_character_id, target_character_id, reason, created_at)
             VALUES ($1, $2, $3, $4) RETURNING id",
            &[&actor.0, &target.0, &reason, &now],
        )?
        .get(0);
    append_audit(
        &mut transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        "report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

#[derive(Clone, Debug)]
struct PreparedStaffAudit {
    actor_account_id: AccountId,
    target_account_id: Option<AccountId>,
    capability: StaffCapability,
    action: String,
    reason: String,
    details_json: String,
}

fn staff_access<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
) -> Result<StaffAccess, StoreError> {
    let account_id = connection
        .query_opt(
            "SELECT account_id FROM characters WHERE id = $1",
            &[&character_id.0],
        )?
        .map(|row| AccountId(row.get(0)))
        .ok_or(StoreError::SocialTargetNotFound)?;
    staff_access_by_account(connection, account_id)
}

fn staff_access_by_account<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
) -> Result<StaffAccess, StoreError> {
    let row = connection
        .query_opt(
            "SELECT accounts.role, staff_profiles.profile_key, staff_profiles.lore_title
             FROM accounts
             LEFT JOIN staff_profiles ON staff_profiles.account_id = accounts.id
             WHERE accounts.id = $1",
            &[&account_id.0],
        )?
        .ok_or(StoreError::AccountNotFound(account_id))?;
    let role: String = row.get(0);
    let profile: Option<String> = row.get(1);
    let lore_title: Option<String> = row.get(2);
    let profile = profile
        .as_deref()
        .map(|value| {
            StaffProfileKey::parse(value)
                .ok_or_else(|| StoreError::InvalidStaffState(format!("profile {value:?}")))
        })
        .transpose()?;
    let capabilities = connection
        .query(
            "SELECT capability FROM staff_capability_assignments
             WHERE account_id = $1 ORDER BY capability",
            &[&account_id.0],
        )?
        .into_iter()
        .map(|row| {
            let value: String = row.get(0);
            StaffCapability::parse(&value)
                .ok_or_else(|| StoreError::InvalidStaffState(format!("capability {value:?}")))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if profile.is_none() && !capabilities.is_empty() {
        return Err(StoreError::InvalidStaffState(format!(
            "account {} has capabilities without a profile",
            account_id.0
        )));
    }
    Ok(StaffAccess {
        account_id,
        role: role_from_database(&role)?,
        profile,
        lore_title,
        capabilities,
    })
}

fn prepare_staff_audit(record: StaffAuditRecord) -> Result<PreparedStaffAudit, StoreError> {
    let action = normalise_staff_text(&record.action, MAX_STAFF_ACTION_BYTES)?;
    let reason = normalise_staff_text(&record.reason, MAX_SOCIAL_REASON_BYTES)?;
    let details_json = serde_json::to_string(&record.details)?;
    if details_json.len() > MAX_AUDIT_DETAILS_BYTES {
        return Err(StoreError::InvalidStaffInput);
    }
    if record.target_account_id == Some(record.actor_account_id) {
        return Err(StoreError::StaffHierarchy);
    }
    Ok(PreparedStaffAudit {
        actor_account_id: record.actor_account_id,
        target_account_id: record.target_account_id,
        capability: record.capability,
        action,
        reason,
        details_json,
    })
}

fn normalise_staff_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidStaffInput)
    } else {
        Ok(value.to_owned())
    }
}

fn normalise_lore_title(value: Option<&str>) -> Result<Option<String>, StoreError> {
    value
        .map(|value| normalise_staff_text(value, MAX_STAFF_LORE_TITLE_BYTES))
        .transpose()
}

fn validate_staff_authority<C: GenericClient>(
    connection: &mut C,
    audit: &PreparedStaffAudit,
    expected_capability: Option<StaffCapability>,
    target_account: Option<AccountId>,
) -> Result<StaffAccess, StoreError> {
    if expected_capability.is_some_and(|expected| expected != audit.capability)
        || audit.target_account_id != target_account
    {
        return Err(StoreError::StaffForbidden);
    }
    let actor = staff_access_by_account(connection, audit.actor_account_id)?;
    if !actor.capabilities.contains(&audit.capability) {
        return Err(StoreError::StaffForbidden);
    }
    let Some(actor_profile) = actor.profile else {
        return Err(StoreError::StaffForbidden);
    };
    if let Some(target_account) = target_account {
        if target_account == actor.account_id {
            return Err(StoreError::StaffHierarchy);
        }
        let target = staff_access_by_account(connection, target_account)?;
        if target
            .profile
            .is_some_and(|profile| profile.authority_rank() >= actor_profile.authority_rank())
        {
            return Err(StoreError::StaffHierarchy);
        }
    }
    Ok(actor)
}

fn insert_staff_audit<C: GenericClient>(
    connection: &mut C,
    audit: &PreparedStaffAudit,
    now: i64,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO staff_audit
             (actor_account_id, target_account_id, capability, action,
              reason, details_json, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &audit.actor_account_id.0,
            &audit.target_account_id.map(|account| account.0),
            &audit.capability.as_str(),
            &audit.action,
            &audit.reason,
            &audit.details_json,
            &now,
        ],
    )?;
    Ok(())
}

fn staff_reports(
    connection: &mut Client,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<Vec<StaffReport>, StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let mut transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &mut transaction,
        &audit,
        Some(StaffCapability::ReportsView),
        None,
    )?;
    let actor_rank = i32::from(
        actor
            .profile
            .ok_or(StoreError::StaffForbidden)?
            .authority_rank(),
    );
    insert_staff_audit(&mut transaction, &audit, now)?;
    let reports = transaction
        .query(
            "SELECT reports.id,
                    reporter.id, reporter.name,
                    target.id, target.account_id, target.name,
                    reports.reason, reports.created_at,
                    evidence.sender_name, evidence.subject, evidence.body,
                    evidence.sent_at, evidence.created_at, evidence.expires_at,
                    board_evidence.board_key, board_evidence.post_public_id,
                    board_evidence.author_name, board_evidence.subject,
                    board_evidence.body, board_evidence.posted_at,
                    board_evidence.created_at, board_evidence.expires_at
             FROM social_reports AS reports
             JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
             JOIN characters AS target ON target.id = reports.target_character_id
             LEFT JOIN staff_profiles AS target_staff
                    ON target_staff.account_id = target.account_id
             LEFT JOIN social_mail_report_evidence AS evidence
                    ON evidence.report_id = reports.id AND evidence.expires_at > $3
             LEFT JOIN social_board_report_evidence AS board_evidence
                    ON board_evidence.report_id = reports.id
                   AND board_evidence.expires_at > $3
             WHERE reports.status = 'open'
               AND CASE target_staff.profile_key
                       WHEN 'helper' THEN 10
                       WHEN 'builder' THEN 20
                       WHEN 'moderator' THEN 30
                       WHEN 'administrator' THEN 40
                       WHEN 'implementor' THEN 50
                       ELSE 0
                   END < $1
             ORDER BY reports.created_at, reports.id
             LIMIT $2",
            &[&actor_rank, &STAFF_LIST_LIMIT, &now],
        )?
        .into_iter()
        .map(|row| {
            let evidence_sender: Option<String> = row.get(8);
            let board_key: Option<String> = row.get(14);
            StaffReport {
                id: row.get(0),
                reporter_character_id: CharacterId(row.get(1)),
                reporter_name: row.get(2),
                target_character_id: CharacterId(row.get(3)),
                target_account_id: AccountId(row.get(4)),
                target_name: row.get(5),
                reason: row.get(6),
                created_at: row.get(7),
                mail_evidence: evidence_sender.map(|sender_name| MailReportEvidence {
                    sender_name,
                    subject: row.get(9),
                    body: row.get(10),
                    sent_at: row.get(11),
                    created_at: row.get(12),
                    expires_at: row.get(13),
                }),
                board_evidence: board_key.map(|board_key| BoardReportEvidence {
                    board: BoardKey::parse(&board_key).expect("schema limits board keys"),
                    post_public_id: row.get(15),
                    author_name: row.get(16),
                    subject: row.get(17),
                    body: row.get(18),
                    posted_at: row.get(19),
                    created_at: row.get(20),
                    expires_at: row.get(21),
                }),
            }
        })
        .collect();
    transaction.commit()?;
    Ok(reports)
}

fn resolve_staff_report(
    connection: &mut Client,
    report_id: i64,
    resolution: StaffReportResolution,
    resolution_reason: &str,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    if report_id <= 0 {
        return Err(StoreError::StaffReportUnavailable(report_id));
    }
    let resolution_reason = normalise_staff_text(resolution_reason, MAX_REPORT_REASON_BYTES)?;
    let mut audit = prepare_staff_audit(audit)?;
    let mut transaction = connection.transaction()?;
    let target_account = transaction
        .query_opt(
            "SELECT target.account_id
             FROM social_reports AS reports
             JOIN characters AS target ON target.id = reports.target_character_id
             WHERE reports.id = $1 AND reports.status = 'open'",
            &[&report_id],
        )?
        .map(|row| AccountId(row.get(0)))
        .ok_or(StoreError::StaffReportUnavailable(report_id))?;
    if audit
        .target_account_id
        .is_some_and(|account_id| account_id != target_account)
    {
        return Err(StoreError::StaffForbidden);
    }
    audit.target_account_id = Some(target_account);
    validate_staff_authority(
        &mut transaction,
        &audit,
        Some(StaffCapability::ReportsResolve),
        Some(target_account),
    )?;
    let updated = transaction.execute(
        "UPDATE social_reports
         SET status = 'resolved', resolution = $1, resolution_reason = $2,
             resolved_by_account_id = $3, resolved_at = $4
         WHERE id = $5 AND status = 'open'",
        &[
            &resolution.as_str(),
            &resolution_reason,
            &audit.actor_account_id.0,
            &now,
            &report_id,
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::StaffReportUnavailable(report_id));
    }
    if resolution == StaffReportResolution::ActionTaken {
        transaction.execute(
            "DELETE FROM social_mail_messages WHERE report_id = $1 AND expires_at > $2",
            &[&report_id, &now],
        )?;
        transaction.execute(
            "DELETE FROM social_board_posts WHERE report_id = $1 AND expires_at > $2",
            &[&report_id, &now],
        )?;
    }
    insert_staff_audit(&mut transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn set_staff_profile(
    connection: &mut Client,
    target_account: AccountId,
    profile: Option<StaffProfileKey>,
    lore_title: Option<&str>,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let lore_title = normalise_lore_title(lore_title)?;
    if profile.is_none() && lore_title.is_some() {
        return Err(StoreError::InvalidStaffInput);
    }
    let audit = prepare_staff_audit(audit)?;
    let mut transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &mut transaction,
        &audit,
        Some(StaffCapability::StaffAssign),
        Some(target_account),
    )?;
    let actor_profile = actor.profile.ok_or(StoreError::StaffForbidden)?;
    if profile.is_some_and(|profile| profile.authority_rank() >= actor_profile.authority_rank()) {
        return Err(StoreError::StaffHierarchy);
    }
    if profile.is_some_and(|profile| {
        profile
            .default_capabilities()
            .iter()
            .any(|capability| !actor.capabilities.contains(capability))
    }) {
        return Err(StoreError::StaffForbidden);
    }
    transaction.execute(
        "DELETE FROM staff_capability_assignments WHERE account_id = $1",
        &[&target_account.0],
    )?;
    if let Some(profile) = profile {
        transaction.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $5)
             ON CONFLICT(account_id) DO UPDATE SET
                 profile_key = EXCLUDED.profile_key,
                 lore_title = EXCLUDED.lore_title,
                 assigned_by_account_id = EXCLUDED.assigned_by_account_id,
                 updated_at = EXCLUDED.updated_at",
            &[
                &target_account.0,
                &profile.as_str(),
                &lore_title,
                &audit.actor_account_id.0,
                &now,
            ],
        )?;
        for capability in profile.default_capabilities() {
            transaction.execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &target_account.0,
                    &capability.as_str(),
                    &audit.actor_account_id.0,
                    &now,
                ],
            )?;
        }
    } else {
        transaction.execute(
            "DELETE FROM staff_profiles WHERE account_id = $1",
            &[&target_account.0],
        )?;
    }
    insert_staff_audit(&mut transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn set_staff_capability(
    connection: &mut Client,
    target_account: AccountId,
    capability: StaffCapability,
    enabled: bool,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let mut transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &mut transaction,
        &audit,
        Some(StaffCapability::StaffAssign),
        Some(target_account),
    )?;
    let target = staff_access_by_account(&mut transaction, target_account)?;
    if enabled && (target.profile.is_none() || !actor.capabilities.contains(&capability)) {
        return Err(StoreError::StaffForbidden);
    }
    if enabled {
        transaction.execute(
            "INSERT INTO staff_capability_assignments
                 (account_id, capability, assigned_by_account_id, created_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(account_id, capability) DO UPDATE SET
                 assigned_by_account_id = EXCLUDED.assigned_by_account_id,
                 created_at = EXCLUDED.created_at",
            &[
                &target_account.0,
                &capability.as_str(),
                &audit.actor_account_id.0,
                &now,
            ],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM staff_capability_assignments
             WHERE account_id = $1 AND capability = $2",
            &[&target_account.0, &capability.as_str()],
        )?;
    }
    insert_staff_audit(&mut transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn staff_audit_entries(
    connection: &mut Client,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<Vec<StaffAuditEntry>, StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let mut transaction = connection.transaction()?;
    validate_staff_authority(
        &mut transaction,
        &audit,
        Some(StaffCapability::StaffAudit),
        None,
    )?;
    insert_staff_audit(&mut transaction, &audit, now)?;
    let entries = transaction
        .query(
            "SELECT sequence, actor_account_id, target_account_id, capability,
                    action, reason, details_json, created_at
             FROM staff_audit ORDER BY sequence DESC LIMIT $1",
            &[&STAFF_LIST_LIMIT],
        )?
        .into_iter()
        .map(|row| {
            let capability: String = row.get(3);
            Ok(StaffAuditEntry {
                sequence: row.get(0),
                actor_account_id: AccountId(row.get(1)),
                target_account_id: row.get::<_, Option<i64>>(2).map(AccountId),
                capability: StaffCapability::parse(&capability).ok_or_else(|| {
                    StoreError::InvalidStaffState(format!("capability {capability:?}"))
                })?,
                action: row.get(4),
                reason: row.get(5),
                details_json: row.get(6),
                created_at: row.get(7),
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    transaction.commit()?;
    Ok(entries)
}

fn append_staff_audit_transaction(
    connection: &mut Client,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let mut transaction = connection.transaction()?;
    validate_staff_authority(&mut transaction, &audit, None, audit.target_account_id)?;
    insert_staff_audit(&mut transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn moderate_character(
    connection: &mut Client,
    actor: CharacterId,
    target: CharacterId,
    action: ModerationAction,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let (required_capability, action_name, reason, details) = match &action {
        ModerationAction::Mute { expires_at, reason } => {
            validate_expiry(*expires_at, now)?;
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationMute,
                "mute",
                reason.clone(),
                serde_json::json!({"expiresAt": expires_at, "reason": reason}),
            )
        }
        ModerationAction::Unmute { reason } => {
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationMute,
                "unmute",
                reason.clone(),
                serde_json::json!({"reason": reason}),
            )
        }
        ModerationAction::Ban { expires_at, reason } => {
            validate_expiry(*expires_at, now)?;
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationBan,
                "ban",
                reason.clone(),
                serde_json::json!({"expiresAt": expires_at, "reason": reason}),
            )
        }
        ModerationAction::Unban { reason } => {
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationBan,
                "unban",
                reason.clone(),
                serde_json::json!({"reason": reason}),
            )
        }
        ModerationAction::SetRole { role, reason } => {
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::StaffAssign,
                "set_role",
                reason.clone(),
                serde_json::json!({"role": role_as_database(*role), "reason": reason}),
            )
        }
    };
    let mut transaction = connection.transaction()?;
    let actor_profile = social_profile(&mut transaction, actor, now)?;
    let target_profile = social_profile(&mut transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let staff_audit = prepare_staff_audit(StaffAuditRecord {
        actor_account_id: actor_profile.account_id,
        target_account_id: Some(target_profile.account_id),
        capability: required_capability,
        action: action_name.to_owned(),
        reason: reason.clone(),
        details: details.clone(),
    })?;
    if let Err(error) = validate_staff_authority(
        &mut transaction,
        &staff_audit,
        Some(required_capability),
        Some(target_profile.account_id),
    ) {
        return Err(match error {
            StoreError::StaffForbidden | StoreError::StaffHierarchy => StoreError::SocialForbidden,
            error => error,
        });
    }

    match action {
        ModerationAction::Mute { expires_at, .. } => {
            revoke_active(
                &mut transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO moderation_mutes
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &target_profile.account_id.0,
                    &actor_profile.account_id.0,
                    &reason,
                    &now,
                    &expires_at,
                ],
            )?;
        }
        ModerationAction::Unmute { .. } => {
            revoke_active(
                &mut transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
        }
        ModerationAction::Ban { expires_at, .. } => {
            revoke_active(
                &mut transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO account_bans
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &target_profile.account_id.0,
                    &actor_profile.account_id.0,
                    &reason,
                    &now,
                    &expires_at,
                ],
            )?;
            transaction.execute(
                "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, $1)
                WHERE account_id = $2",
                &[&now, &target_profile.account_id.0],
            )?;
        }
        ModerationAction::Unban { .. } => {
            revoke_active(
                &mut transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
        }
        ModerationAction::SetRole { role, .. } => {
            transaction.execute(
                "UPDATE accounts SET role = $1 WHERE id = $2",
                &[&role_as_database(role), &target_profile.account_id.0],
            )?;
        }
    }
    append_audit(
        &mut transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action_name,
        details,
        now,
    )?;
    insert_staff_audit(&mut transaction, &staff_audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn validate_expiry(expires_at: Option<i64>, now: i64) -> Result<(), StoreError> {
    if expires_at.is_some_and(|expires_at| {
        expires_at <= now || expires_at.saturating_sub(now) > MAX_MODERATION_DURATION_SECONDS
    }) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(())
    }
}

fn revoke_active<C: GenericClient>(
    connection: &mut C,
    table: &str,
    target: AccountId,
    actor: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    debug_assert!(matches!(table, "moderation_mutes" | "account_bans"));
    connection.execute(
        &format!(
            "UPDATE {table}
             SET revoked_at = $1, revoked_by_account_id = $2
             WHERE account_id = $3 AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > $1)"
        ),
        &[&now, &actor.0, &target.0],
    )?;
    Ok(())
}

fn normalise_social_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(value.to_owned())
    }
}

fn append_audit<C: GenericClient>(
    connection: &mut C,
    actor: AccountId,
    target: Option<AccountId>,
    action: &str,
    details: serde_json::Value,
    now: i64,
) -> Result<(), StoreError> {
    let details_json = serde_json::to_string(&details)?;
    if action.is_empty() || action.len() > 64 || details_json.len() > MAX_AUDIT_DETAILS_BYTES {
        return Err(StoreError::InvalidSocialInput);
    }
    connection.execute(
        "INSERT INTO moderation_audit
             (actor_account_id, target_account_id, action, details_json, created_at)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            &actor.0,
            &target.map(|target| target.0),
            &action,
            &details_json,
            &now,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
fn audit_log(connection: &mut Client) -> Result<Vec<AuditEntry>, StoreError> {
    Ok(connection
        .query(
            "SELECT sequence, actor_account_id, target_account_id, action,
                    details_json, created_at
             FROM moderation_audit ORDER BY sequence",
            &[],
        )?
        .into_iter()
        .map(|row| AuditEntry {
            sequence: row.get(0),
            actor_account_id: AccountId(row.get(1)),
            target_account_id: row.get::<_, Option<i64>>(2).map(AccountId),
            action: row.get(3),
            details_json: row.get(4),
            created_at: row.get(5),
        })
        .collect())
}

fn bounded_json<T: serde::Serialize>(
    value: &T,
    kind: &'static str,
    maximum: usize,
) -> Result<String, StoreError> {
    let json = serde_json::to_string(value)?;
    if json.len() > maximum {
        return Err(StoreError::SnapshotTooLarge { kind, maximum });
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, env, process, time::Duration};
    use tiamud_core::{
        AdventureRuntimeSnapshot, AreaDynamicCheckpoint, AreaDynamicCheckpointIdentity,
        AreaRuntimeResidence, Direction, ItemBinding, ItemInstanceId, ItemLocationSnapshot,
        ItemProvenance, LootDropId, RuntimeItemInstanceSnapshot, RuntimeItemInstanceSnapshotV3,
        RuntimeItemLocation, WORLD_RUNTIME_SNAPSHOT_VERSION, World, WorldRuntimeSnapshot,
        WorldRuntimeSnapshotV3,
    };

    fn area_generation(
        world_id: &str,
        active: char,
        previous: Option<char>,
        activation_sequence: u64,
    ) -> DurableAreaGeneration {
        DurableAreaGeneration {
            world_id: world_id.into(),
            active_catalog_sha256: active.to_string().repeat(64),
            previous_catalog_sha256: previous.map(|digest| digest.to_string().repeat(64)),
            activation_sequence,
        }
    }

    fn area_checkpoint(runtime: &WorldRuntimeSnapshot) -> DurableAreaCheckpoint {
        let checkpoint = AreaDynamicCheckpoint {
            checkpoint_version: tiamud_core::AREA_DYNAMIC_CHECKPOINT_VERSION,
            generation_sha256: "a".repeat(64),
            areas: vec![AreaDynamicCheckpointIdentity {
                area_id: "demo:archive".into(),
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: 2,
            }],
            captured_world_revision: runtime.revision,
            captured_logical_tick: runtime.logical_tick,
            captured_rng_state: runtime.rng_state,
            captured_next_item_instance_sequence: runtime.next_item_instance_sequence,
            captured_next_loot_drop_id: runtime.next_loot_drop_id,
            captured_next_reward_open_id: runtime.next_reward_open_id,
            npcs: BTreeMap::new(),
            item_instances: BTreeMap::new(),
            item_provenance: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
        };
        tiamud_core::validate_area_dynamic_checkpoint(&checkpoint).unwrap();
        let checkpoint_json = serde_json::to_string(&checkpoint).unwrap();
        DurableAreaCheckpoint {
            generation_sha256: checkpoint.generation_sha256.clone(),
            unit_id: area_checkpoint_unit_id(&checkpoint),
            residency_epoch: checkpoint.areas[0].residency_epoch,
            checkpoint_version: checkpoint.checkpoint_version,
            checkpoint_sha256: hex::encode(Sha256::digest(checkpoint_json.as_bytes())),
            captured_logical_tick: checkpoint.captured_logical_tick,
            checkpoint_json,
        }
    }

    fn retired_runtime(
        mut runtime: WorldRuntimeSnapshot,
        durable: &DurableAreaCheckpoint,
    ) -> WorldRuntimeSnapshot {
        runtime.area_residency.insert(
            "demo:archive".into(),
            AreaRuntimeResidence::Retired {
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: durable.residency_epoch,
                checkpoint_version: durable.checkpoint_version,
                checkpoint_sha256: durable.checkpoint_sha256.clone(),
            },
        );
        runtime
    }

    fn initial_resident_runtime(mut runtime: WorldRuntimeSnapshot) -> WorldRuntimeSnapshot {
        runtime.area_residency.insert(
            "demo:archive".into(),
            AreaRuntimeResidence::Resident {
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: 1,
            },
        );
        runtime
    }

    fn rehydrated_runtime(mut runtime: WorldRuntimeSnapshot) -> WorldRuntimeSnapshot {
        runtime.area_residency.insert(
            "demo:archive".into(),
            AreaRuntimeResidence::Resident {
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: 3,
            },
        );
        runtime
    }

    #[test]
    fn postgres_mail_contract_keeps_exact_input_and_storage_bounds() {
        assert_eq!(MAILBOX_CAPACITY, 64);
        assert_eq!(MAILBOX_BYTE_CAPACITY, 128 * 1024);
        assert_eq!(MAIL_PAGE_SIZE, 10);
        assert_eq!(MAIL_MAX_PAGES, 7);
        assert_eq!(MAIL_HOURLY_CAPACITY, 20);
        assert_eq!(MAIL_DAILY_CAPACITY, 50);
        assert_eq!(MAIL_GLOBAL_CAPACITY, 100_000);
        assert_eq!(MAIL_PURGE_LIMIT, 256);
        assert_eq!(
            normalise_mail_text("subject", MAX_MAIL_SUBJECT_BYTES).unwrap(),
            "subject"
        );
        assert!(matches!(
            normalise_mail_text(" subject ", MAX_MAIL_SUBJECT_BYTES),
            Err(StoreError::InvalidMailInput)
        ));
        assert!(matches!(
            normalise_mail_text("bad\nbody", MAX_MAIL_BODY_BYTES),
            Err(StoreError::InvalidMailInput)
        ));
        assert!(matches!(
            normalise_name("A\u{202e}B", "character"),
            Err(StoreError::InvalidName { kind: "character" })
        ));
        assert!(matches!(
            normalise_name("A|B", "character"),
            Err(StoreError::InvalidName { kind: "character" })
        ));
        assert!(valid_mail_public_id("m0123456789abcdef0123456789abcdef"));
        assert!(!valid_mail_public_id("M0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn postgres_board_contract_keeps_exact_limits_and_validation() {
        assert_eq!(BOARD_ACCOUNT_CAPACITY, 64);
        assert_eq!(BOARD_CAPACITY_PER_BOARD, 1_000);
        assert_eq!(BOARD_BYTE_CAPACITY_PER_BOARD, 4 * 1_024 * 1_024);
        assert_eq!(BOARD_GLOBAL_CAPACITY, 2_000);
        assert_eq!(BOARD_GLOBAL_BYTE_CAPACITY, 8 * 1_024 * 1_024);
        assert_eq!(BOARD_PAGE_SIZE, 10);
        assert_eq!(BOARD_MAX_PAGES, 100);
        assert_eq!(BOARD_HOURLY_CAPACITY, 5);
        assert_eq!(BOARD_DAILY_CAPACITY, 20);
        assert_eq!(BOARD_PURGE_LIMIT, 256);
        assert_eq!(BOARD_RETENTION_SECONDS, 90 * MAIL_DAY_SECONDS);
        assert!(valid_board_public_id("b0123456789abcdef0123456789abcdef"));
        assert!(!valid_board_public_id("B0123456789abcdef0123456789abcdef"));
        assert!(valid_board_subject("Visible subject"));
        assert!(!valid_board_subject("Bad | subject"));
        assert!(matches!(
            normalise_board_text(" padded ", MAX_BOARD_SUBJECT_BYTES),
            Err(StoreError::InvalidBoardInput)
        ));
        assert!(
            check_board_capacity(
                i64::from(BOARD_ACCOUNT_CAPACITY) - 1,
                i64::from(BOARD_CAPACITY_PER_BOARD) - 1,
                BOARD_BYTE_CAPACITY_PER_BOARD - 2,
                BOARD_GLOBAL_CAPACITY - 1,
                BOARD_GLOBAL_BYTE_CAPACITY - 2,
                2,
            )
            .is_ok()
        );
        for result in [
            check_board_capacity(i64::from(BOARD_ACCOUNT_CAPACITY), 0, 0, 0, 0, 2),
            check_board_capacity(0, i64::from(BOARD_CAPACITY_PER_BOARD), 0, 0, 0, 2),
            check_board_capacity(0, 0, BOARD_BYTE_CAPACITY_PER_BOARD - 1, 0, 0, 2),
            check_board_capacity(0, 0, 0, BOARD_GLOBAL_CAPACITY, 0, 2),
            check_board_capacity(0, 0, 0, 0, BOARD_GLOBAL_BYTE_CAPACITY - 1, 2),
        ] {
            assert!(matches!(result, Err(StoreError::BoardCapacity)));
        }
        assert!(
            check_board_report_capacity(
                BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT - 1,
                BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY - 1,
            )
            .is_ok()
        );
        assert!(matches!(
            check_board_report_capacity(
                BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT,
                BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY - 1,
            ),
            Err(StoreError::BoardReportLimit)
        ));
        assert!(matches!(
            check_board_report_capacity(
                BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT - 1,
                BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY,
            ),
            Err(StoreError::BoardEvidenceLimit)
        ));
    }

    #[test]
    fn postgres_runtime_decoder_accepts_current_and_upgrades_strict_v3_v4_v6() {
        let mut current = World::demo().runtime_snapshot();
        let item_id = current.item_instances.keys().next().unwrap().clone();
        let item = current.item_instances.get_mut(&item_id).unwrap();
        item.location = RuntimeItemLocation::Carried {
            character_id: CharacterId(77),
        };
        item.binding = ItemBinding::Character {
            character_id: CharacterId(77),
        };
        let current_json = serde_json::to_value(&current).unwrap();
        assert_eq!(
            current_json["itemInstances"][item_id.0.as_str()]["binding"],
            serde_json::json!({"kind":"character", "characterId":77})
        );
        let decoded = decode_runtime_snapshot(
            i64::from(WORLD_RUNTIME_SNAPSHOT_VERSION),
            &current_json.to_string(),
        )
        .unwrap();
        assert_eq!(
            decoded.item_instances[&item_id].binding,
            ItemBinding::Character {
                character_id: CharacterId(77)
            }
        );

        let legacy = WorldRuntimeSnapshotV3 {
            snapshot_version: 3,
            revision: 5,
            logical_tick: 6,
            rng_state: 7,
            next_item_instance_sequence: 8,
            item_instances: BTreeMap::from([(
                ItemInstanceId("legacy:item".into()),
                RuntimeItemInstanceSnapshotV3 {
                    prototype_id: "legacy:item".into(),
                    location: RuntimeItemLocation::Room {
                        room_id: "legacy:room".into(),
                    },
                },
            )]),
            npcs: BTreeMap::new(),
            adventure: AdventureRuntimeSnapshot::default(),
            progression_flags: Default::default(),
        };
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        let upgraded = decode_runtime_snapshot(3, &legacy_json).unwrap();
        assert_eq!(
            upgraded.item_instances[&ItemInstanceId("legacy:item".into())].binding,
            ItemBinding::Unbound
        );
        assert!(decode_runtime_snapshot(4, &legacy_json).is_err());

        let mut smuggled = serde_json::to_value(legacy).unwrap();
        smuggled["itemInstances"]["legacy:item"]["binding"] =
            serde_json::json!({"kind":"character", "characterId":77});
        assert!(decode_runtime_snapshot(3, &smuggled.to_string()).is_err());

        let mut v4 = current_json;
        v4["snapshotVersion"] = serde_json::json!(4);
        for field in [
            "nextTradeId",
            "trades",
            "nextLootDropId",
            "itemProvenance",
            "lootEncounters",
            "groundLoot",
            "partyLootCursors",
            "nextRewardOpenId",
            "areaResidency",
        ] {
            v4.as_object_mut().unwrap().remove(field);
        }
        for item in v4["itemInstances"].as_object_mut().unwrap().values_mut() {
            item.as_object_mut().unwrap().remove("itemAffixes");
        }
        decode_runtime_snapshot(4, &v4.to_string()).unwrap();
        v4["trades"] = serde_json::json!({});
        assert!(decode_runtime_snapshot(4, &v4.to_string()).is_err());

        let mut v6 = serde_json::to_value(&current).unwrap();
        v6["snapshotVersion"] = serde_json::json!(6);
        v6.as_object_mut().unwrap().remove("nextRewardOpenId");
        v6.as_object_mut().unwrap().remove("areaResidency");
        for item in v6["itemInstances"].as_object_mut().unwrap().values_mut() {
            item.as_object_mut().unwrap().remove("itemAffixes");
        }
        let upgraded = decode_runtime_snapshot(6, &v6.to_string()).unwrap();
        assert_eq!(upgraded.next_reward_open_id, tiamud_core::RewardOpenId(1));
        v6["itemInstances"][item_id.0.as_str()]["itemAffixes"] = serde_json::json!([]);
        assert!(decode_runtime_snapshot(6, &v6.to_string()).is_err());
    }

    fn scoped_url(base: &str, schema: &str) -> String {
        let separator = if base.contains('?') { '&' } else { '?' };
        format!("{base}{separator}options=-csearch_path%3D{schema}")
    }

    async fn admin_batch(base_url: &str, sql: String) {
        let base_url = base_url.to_owned();
        tokio::task::spawn_blocking(move || {
            tiamud_persistence_postgres::connect(&base_url)
                .unwrap()
                .batch_execute(&sql)
                .unwrap();
        })
        .await
        .unwrap();
    }

    async fn admin_count(base_url: &str, sql: String) -> i64 {
        let base_url = base_url.to_owned();
        tokio::task::spawn_blocking(move || {
            tiamud_persistence_postgres::connect(&base_url)
                .unwrap()
                .query_one(&sql, &[])
                .unwrap()
                .get(0)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn postgres_startup_recovery_is_atomic_and_snapshot_authoritative() {
        let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
            return;
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("tiamud_recovery_test_{}_{}", process::id(), nonce);
        admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
        let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
            .await
            .unwrap();

        let empty = store.load_startup_world_recovery().await.unwrap();
        assert_eq!(empty, StartupWorldRecovery::Empty);
        assert!(!empty.requires_full_generation());

        let event = DomainEvent::CharacterMoved {
            actor_id: CharacterId(1),
            actor_name: "Aria".into(),
            from_room_id: "room:a".into(),
            to_room_id: "room:b".into(),
            direction: Direction::North,
        };
        let event_json = serde_json::to_string(&event).unwrap().replace('\'', "''");
        admin_batch(
            &base_url,
            format!("INSERT INTO {schema}.domain_events (event_json) VALUES ('{event_json}')"),
        )
        .await;
        let ledger = store.load_startup_world_recovery().await.unwrap();
        assert_eq!(ledger, StartupWorldRecovery::Ledger(vec![event]));
        assert!(ledger.requires_full_generation());

        let snapshot = World::demo().runtime_snapshot();
        store
            .commit_action(Vec::new(), snapshot.clone(), Vec::new())
            .await
            .unwrap();
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot: snapshot.clone(),
            }
        );

        let mut legacy = serde_json::to_value(snapshot).unwrap();
        let object = legacy.as_object_mut().unwrap();
        object.insert("snapshotVersion".into(), serde_json::json!(7));
        object.remove("areaResidency");
        let legacy_json = serde_json::to_string(&legacy).unwrap().replace('\'', "''");
        admin_batch(
            &base_url,
            format!(
                "UPDATE {schema}.world_runtime
                 SET snapshot_version = 7, snapshot_json = '{legacy_json}'
                 WHERE singleton = 1"
            ),
        )
        .await;
        let StartupWorldRecovery::Snapshot {
            durable_version,
            snapshot,
        } = store.load_startup_world_recovery().await.unwrap()
        else {
            panic!("expected durable snapshot authority");
        };
        assert_eq!(durable_version, 7);
        assert!(snapshot.area_residency.is_empty());

        // A present but invalid snapshot must never fall through to the valid
        // retained ledger observed by the same repeatable-read transaction.
        admin_batch(
            &base_url,
            format!("UPDATE {schema}.world_runtime SET snapshot_json = '{{}}' WHERE singleton = 1"),
        )
        .await;
        assert!(matches!(
            store.load_startup_world_recovery().await,
            Err(StoreError::InvalidRuntimeSnapshot)
        ));

        admin_batch(
            &base_url,
            format!(
                "DELETE FROM {schema}.world_runtime;
                 DELETE FROM {schema}.domain_events;
                 INSERT INTO {schema}.domain_events (sequence, event_json)
                 VALUES (1, '{event_json}'), (3, '{event_json}')"
            ),
        )
        .await;
        assert!(matches!(
            store.load_startup_world_recovery().await,
            Err(StoreError::InvalidStartupLedger)
        ));

        admin_batch(
            &base_url,
            format!(
                "DELETE FROM {schema}.domain_events;
                 INSERT INTO {schema}.domain_events (sequence, event_json)
                 SELECT sequence, '{{}}'
                 FROM generate_series(1, {}) AS generated(sequence)",
                DOMAIN_EVENT_RETENTION + 1
            ),
        )
        .await;
        assert!(matches!(
            store.load_startup_world_recovery().await,
            Err(StoreError::InvalidStartupLedger)
        ));

        drop(store);
        tokio::time::sleep(Duration::from_millis(25)).await;
        admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
    }

    #[tokio::test]
    async fn postgres_area_generation_bind_and_activation_are_restart_safe_and_cas_guarded() {
        let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
            return;
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("tiamud_generation_test_{}_{}", process::id(), nonce);
        admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
        let scoped_url = scoped_url(&base_url, &schema);
        let store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
        let competing_store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
        assert_eq!(store.load_area_generation().await.unwrap(), None);

        let initial = area_generation("Mundo de Ámbar", 'a', None, 1);
        assert_eq!(
            store
                .bind_area_generation_if_absent(initial.clone())
                .await
                .unwrap(),
            initial
        );
        assert_eq!(
            competing_store
                .bind_area_generation_if_absent(area_generation("Mundo de Ámbar", 'b', None, 1,))
                .await
                .unwrap(),
            initial
        );

        let mut activated_runtime = World::demo().runtime_snapshot();
        activated_runtime.logical_tick = 41;
        let activated = area_generation("Mundo de Ámbar", 'b', Some('a'), 2);
        store
            .commit_area_generation_activation(
                activated_runtime.clone(),
                initial.clone(),
                activated.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            competing_store.load_area_generation().await.unwrap(),
            Some(activated.clone())
        );
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot: activated_runtime.clone(),
            }
        );

        let mut stale_runtime = activated_runtime.clone();
        stale_runtime.logical_tick = 99;
        assert!(matches!(
            competing_store
                .commit_area_generation_activation(
                    stale_runtime,
                    initial,
                    area_generation("Mundo de Ámbar", 'c', Some('a'), 2),
                )
                .await,
            Err(StoreError::AreaGenerationConflict)
        ));
        assert_eq!(
            store.load_area_generation().await.unwrap(),
            Some(activated.clone())
        );
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot: activated_runtime.clone(),
            }
        );

        admin_batch(
            &base_url,
            format!(
                "CREATE FUNCTION {schema}.reject_generation_snapshot() RETURNS trigger
                 LANGUAGE plpgsql AS $$
                 BEGIN
                     RAISE EXCEPTION 'injected snapshot failure';
                 END;
                 $$;
                 CREATE TRIGGER reject_generation_snapshot
                 BEFORE UPDATE ON {schema}.world_runtime
                 FOR EACH ROW EXECUTE FUNCTION {schema}.reject_generation_snapshot()"
            ),
        )
        .await;
        let replacement = {
            let mut runtime = World::demo().runtime_snapshot();
            runtime.logical_tick = 73;
            runtime
        };
        assert!(
            store
                .commit_area_generation_activation(
                    replacement,
                    activated.clone(),
                    area_generation("Mundo de Ámbar", 'c', Some('b'), 3),
                )
                .await
                .is_err()
        );
        assert_eq!(
            store.load_area_generation().await.unwrap(),
            Some(activated.clone()),
            "the pointer update must roll back when snapshot persistence fails"
        );
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot: activated_runtime,
            }
        );

        drop(competing_store);
        drop(store);
        tokio::time::sleep(Duration::from_millis(25)).await;
        admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
    }

    #[tokio::test]
    async fn postgres_area_checkpoint_transitions_are_exact_restart_safe_and_atomic() {
        let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
            return;
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("tiamud_area_checkpoint_test_{}_{}", process::id(), nonce);
        admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
        let scoped_url = scoped_url(&base_url, &schema);
        let store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
        let generation = area_generation("Mundo de Ámbar", 'a', None, 1);
        store
            .bind_area_generation_if_absent(generation.clone())
            .await
            .unwrap();
        let baseline = initial_resident_runtime(World::demo().runtime_snapshot());
        store
            .commit_action(Vec::new(), baseline.clone(), Vec::new())
            .await
            .unwrap();
        let stale_checkpoint = area_checkpoint(&baseline);
        let stale_retired = retired_runtime(baseline.clone(), &stale_checkpoint);
        let mut current = baseline;
        current.logical_tick += 1;
        store
            .commit_action(Vec::new(), current.clone(), Vec::new())
            .await
            .unwrap();
        assert!(matches!(
            store
                .commit_area_retirement(stale_retired, generation.clone(), stale_checkpoint,)
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));
        let checkpoint = area_checkpoint(&current);
        let retired = retired_runtime(current.clone(), &checkpoint);

        admin_batch(
            &base_url,
            format!(
                "CREATE FUNCTION {schema}.reject_area_checkpoint_snapshot() RETURNS trigger
                 LANGUAGE plpgsql AS $$
                 BEGIN
                     RAISE EXCEPTION 'injected area checkpoint snapshot failure';
                 END;
                 $$;
                 CREATE TRIGGER reject_area_checkpoint_snapshot
                 BEFORE UPDATE ON {schema}.world_runtime
                 FOR EACH ROW EXECUTE FUNCTION {schema}.reject_area_checkpoint_snapshot()"
            ),
        )
        .await;
        assert!(
            store
                .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone(),)
                .await
                .is_err()
        );
        assert!(store.load_area_checkpoints().await.unwrap().is_empty());
        assert_eq!(store.load_runtime_snapshot().await.unwrap(), Some(current));

        admin_batch(
            &base_url,
            format!("DROP TRIGGER reject_area_checkpoint_snapshot ON {schema}.world_runtime"),
        )
        .await;
        store
            .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone())
            .await
            .unwrap();
        assert_eq!(
            store.load_area_checkpoints().await.unwrap(),
            vec![checkpoint.clone()]
        );
        assert_eq!(
            store.load_runtime_snapshot().await.unwrap(),
            Some(retired.clone())
        );
        assert!(matches!(
            store
                .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone())
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));

        let stale_resident = rehydrated_runtime(retired.clone());
        let mut current_retired = retired;
        current_retired.logical_tick += 1;
        store
            .commit_action(Vec::new(), current_retired.clone(), Vec::new())
            .await
            .unwrap();
        assert!(matches!(
            store
                .commit_area_rehydration(stale_resident, generation.clone(), checkpoint.clone(),)
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));
        assert_eq!(
            store.load_area_checkpoints().await.unwrap(),
            vec![checkpoint.clone()]
        );
        let resident = rehydrated_runtime(current_retired.clone());
        admin_batch(
            &base_url,
            format!(
                "CREATE TRIGGER reject_area_checkpoint_snapshot
                 BEFORE UPDATE ON {schema}.world_runtime
                 FOR EACH ROW EXECUTE FUNCTION {schema}.reject_area_checkpoint_snapshot()"
            ),
        )
        .await;
        assert!(
            store
                .commit_area_rehydration(resident.clone(), generation.clone(), checkpoint.clone(),)
                .await
                .is_err()
        );
        assert_eq!(
            store.load_area_checkpoints().await.unwrap(),
            vec![checkpoint.clone()]
        );
        assert_eq!(
            store.load_runtime_snapshot().await.unwrap(),
            Some(current_retired)
        );
        admin_batch(
            &base_url,
            format!("DROP TRIGGER reject_area_checkpoint_snapshot ON {schema}.world_runtime"),
        )
        .await;
        store
            .commit_area_rehydration(resident.clone(), generation.clone(), checkpoint.clone())
            .await
            .unwrap();
        assert!(store.load_area_checkpoints().await.unwrap().is_empty());
        assert_eq!(
            store.load_runtime_snapshot().await.unwrap(),
            Some(resident.clone())
        );
        assert!(matches!(
            store
                .commit_area_rehydration(resident, generation, checkpoint)
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));

        drop(store);
        tokio::time::sleep(Duration::from_millis(25)).await;
        admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
    }

    #[tokio::test]
    async fn postgres_board_advisory_lock_serializes_concurrent_quota() {
        let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
            return;
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("tiamud_board_lock_test_{}_{}", process::id(), nonce);
        admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
        let scoped_url = scoped_url(&base_url, &schema);
        let first_store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
        let account = first_store
            .register_account("Concurrent Board", "correct horse battery staple")
            .await
            .unwrap();
        let character = first_store
            .load_or_create_character(account.account.id, "Quill", "room:start")
            .await
            .unwrap();
        let second_store = PostgresCharacterStore::open(&scoped_url).await.unwrap();

        let (first, second, third, fourth, fifth, sixth) = tokio::join!(
            first_store.publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000001",
                "Concurrent one",
                "Body",
            ),
            second_store.publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000002",
                "Concurrent two",
                "Body",
            ),
            first_store.publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000003",
                "Concurrent three",
                "Body",
            ),
            second_store.publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000004",
                "Concurrent four",
                "Body",
            ),
            first_store.publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000005",
                "Concurrent five",
                "Body",
            ),
            second_store.publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000006",
                "Concurrent six",
                "Body",
            ),
        );
        let results = [first, second, third, fourth, fifth, sixth];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 5);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(StoreError::BoardRateLimited)))
                .count(),
            1
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_posts"),
            )
            .await,
            5
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT hour_count::BIGINT FROM {schema}.social_board_usage
                     WHERE account_id = {}",
                    account.account.id.0
                ),
            )
            .await,
            BOARD_HOURLY_CAPACITY
        );

        let now = unix_timestamp();
        admin_batch(
            &base_url,
            format!(
                "UPDATE {schema}.social_board_usage
                 SET hour_started_at = {now}, hour_count = 0,
                     day_started_at = {now}, day_count = 20, updated_at = {now}
                 WHERE account_id = {}",
                account.account.id.0
            ),
        )
        .await;
        assert!(matches!(
            first_store
                .publish_board_post(
                    character.id,
                    BoardKey::General,
                    "b00000000000000000000000000000007",
                    "Daily limit",
                    "Body",
                )
                .await,
            Err(StoreError::BoardRateLimited)
        ));
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT day_count::BIGINT FROM {schema}.social_board_usage
                     WHERE account_id = {}",
                    account.account.id.0
                ),
            )
            .await,
            BOARD_DAILY_CAPACITY
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_posts"),
            )
            .await,
            5,
            "the daily rejection must not create a sixth post"
        );

        first_store.shutdown().await.unwrap();
        second_store.shutdown().await.unwrap();
        drop(first_store);
        drop(second_store);
        tokio::time::sleep(Duration::from_millis(25)).await;
        admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
    }

    #[tokio::test]
    async fn postgres_store_round_trips_auth_characters_runtime_and_social_policy() {
        let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
            return;
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("tiamud_store_test_{}_{}", process::id(), nonce);
        admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;

        let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
            .await
            .unwrap();
        store.health_check().await.unwrap();
        assert!(!store.has_durable_character_authority().await.unwrap());

        let keeper = store
            .register_account(" Keeper ", "correct horse battery staple")
            .await
            .unwrap();
        let player = store
            .register_account("Player", "another correct horse battery staple")
            .await
            .unwrap();
        let login = store
            .login_account("KEEPER", "correct horse battery staple")
            .await
            .unwrap();
        assert_eq!(login.account, keeper.account);
        assert_eq!(
            store.resolve_session(login.token.as_str()).await.unwrap(),
            keeper.account
        );

        let keeper_character = store
            .load_or_create_character(keeper.account.id, "Aster", "room:start")
            .await
            .unwrap();
        assert!(store.has_durable_character_authority().await.unwrap());
        let keeper_staff = store.staff_access(keeper_character.id).await.unwrap();
        assert_eq!(keeper_staff.profile, Some(StaffProfileKey::Implementor));
        assert_eq!(
            keeper_staff.capabilities,
            StaffCapability::ALL.into_iter().collect()
        );
        let defaults = CharacterGameplaySnapshot::default();
        let mut selected_gameplay = CharacterGameplaySnapshot {
            lineage_id: "tiamud:mortal".into(),
            background_id: Some("tiamud:city-guard".into()),
            base_class_id: "tiamud:vanguard".into(),
            active_class_id: "tiamud:bastion".into(),
            class_history: vec!["tiamud:vanguard".into(), "tiamud:bastion".into()],
            attributes: tiamud_core::Attributes {
                defence: 9,
                ..defaults.attributes
            },
            resources: BTreeMap::from([(
                "tiamud:focus".into(),
                tiamud_core::ResourcePool {
                    current: 7,
                    max: 12,
                },
            )]),
            ability_cooldowns: BTreeMap::from([("tiamud:mind-whisper".into(), 23)]),
            ..defaults
        };
        selected_gameplay
            .progression_points
            .insert("talent".into(), 0);
        selected_gameplay.talent_node_ranks.insert(
            "tiamud:martial-talents".into(),
            BTreeMap::from([("tiamud:guarded-stance".into(), 1)]),
        );
        selected_gameplay.tags.insert("tiamud:founder".into());
        selected_gameplay
            .received_signals
            .insert("tiamud:heard-call".into());
        let player_character = store
            .load_or_create_character_with_gameplay(
                player.account.id,
                "Bryn",
                "room:start",
                selected_gameplay.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .load_character_gameplay(player_character.id)
                .await
                .unwrap(),
            selected_gameplay
        );
        let all_gameplay = store
            .load_character_gameplay_batch(
                vec![player_character.id, keeper_character.id],
                StartupGameplayLoadLimits::ITEM_OWNERS,
            )
            .await
            .unwrap();
        assert_eq!(all_gameplay.len(), 2);
        assert_eq!(all_gameplay[0].character_id, keeper_character.id);
        assert_eq!(all_gameplay[1].character_id, player_character.id);
        assert_eq!(all_gameplay[1].gameplay, selected_gameplay);
        assert_eq!(
            store.list_characters(keeper.account.id).await.unwrap(),
            vec![keeper_character.clone()]
        );
        assert_eq!(
            store
                .load_character(keeper.account.id, keeper_character.id)
                .await
                .unwrap(),
            keeper_character
        );

        let mut updated_character = keeper_character.clone();
        updated_character.room_id = "room:north".into();
        updated_character
            .item_locations
            .insert("item:lamp".into(), ItemLocationSnapshot::Carried);
        let gameplay = selected_gameplay;
        let runtime = WorldRuntimeSnapshot {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: 9,
            logical_tick: 3,
            rng_state: 42,
            next_item_instance_sequence: 1,
            item_instances: BTreeMap::from([(
                ItemInstanceId("item:lamp".into()),
                RuntimeItemInstanceSnapshot {
                    prototype_id: "item:lamp".into(),
                    location: RuntimeItemLocation::Carried {
                        character_id: keeper_character.id,
                    },
                    binding: ItemBinding::Character {
                        character_id: keeper_character.id,
                    },
                    item_affixes: Vec::new(),
                },
            )]),
            next_loot_drop_id: LootDropId(1),
            item_provenance: BTreeMap::from([(
                ItemInstanceId("item:lamp".into()),
                ItemProvenance::AuthoredInitial,
            )]),
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            npcs: BTreeMap::new(),
            adventure: AdventureRuntimeSnapshot::default(),
            progression_flags: Default::default(),
            next_trade_id: tiamud_core::TradeId(1),
            trades: BTreeMap::new(),
            next_reward_open_id: tiamud_core::RewardOpenId(1),
            area_residency: BTreeMap::new(),
        };
        let event = DomainEvent::CharacterMoved {
            actor_id: keeper_character.id,
            actor_name: keeper_character.name.clone(),
            from_room_id: "room:start".into(),
            to_room_id: "room:north".into(),
            direction: Direction::North,
        };
        store
            .commit_action(
                vec![DurableCharacterSnapshot {
                    character: updated_character.clone(),
                    gameplay: gameplay.clone(),
                }],
                runtime.clone(),
                vec![event.clone()],
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .load_character_gameplay(keeper_character.id)
                .await
                .unwrap(),
            gameplay
        );
        assert_eq!(
            store.load_runtime_snapshot().await.unwrap(),
            Some(runtime.clone())
        );
        assert_eq!(
            store.load_domain_events().await.unwrap(),
            vec![event.clone()]
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"
                ),
            )
            .await,
            1
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.character_adventure"),
            )
            .await,
            1
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.world_adventure"),
            )
            .await,
            1
        );
        assert_eq!(
            store
                .load_character(keeper.account.id, keeper_character.id)
                .await
                .unwrap(),
            updated_character
        );

        assert_eq!(
            store
                .social_profile(keeper_character.id)
                .await
                .unwrap()
                .role,
            AccountRole::Admin
        );
        assert!(
            store
                .social_channel_subscriptions(player_character.id)
                .await
                .unwrap()
                .is_empty()
        );
        for channel in [SocialChannelName::Newbie, SocialChannelName::General] {
            store
                .set_social_channel_subscription(player_character.id, channel, true)
                .await
                .unwrap();
        }
        store
            .set_social_channel_subscription(keeper_character.id, SocialChannelName::General, true)
            .await
            .unwrap();
        assert_eq!(
            store
                .social_channel_subscriptions(player_character.id)
                .await
                .unwrap(),
            vec![SocialChannelName::General, SocialChannelName::Newbie]
        );
        assert_eq!(
            store
                .social_channel_subscribers(SocialChannelName::General)
                .await
                .unwrap(),
            vec![keeper_character.id, player_character.id]
        );
        let public_id = "m0123456789abcdef0123456789abcdef";
        assert_eq!(
            store
                .send_mail(
                    keeper_character.id,
                    &player_character.name,
                    public_id,
                    "Lanterns",
                    "Meet beneath the arch.",
                )
                .await
                .unwrap(),
            MailSendResult::Stored
        );
        let page = store.mailbox_page(player_character.id, 1).await.unwrap();
        assert_eq!(page.total_messages, 1);
        assert_eq!(page.unread_count, 1);
        assert_eq!(page.capacity, 64);
        assert_eq!(page.messages[0].public_id, public_id);
        let message = store
            .read_mail(player_character.id, public_id)
            .await
            .unwrap();
        assert_eq!(message.body, "Meet beneath the arch.");
        assert!(message.summary.read);
        let mail_report = store
            .report_mail(player_character.id, public_id, "Unwanted offline contact")
            .await
            .unwrap();
        assert!(mail_report > 0);
        store
            .delete_mail(player_character.id, public_id)
            .await
            .unwrap();
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_mail_report_evidence"),
            )
            .await,
            1
        );
        let board_public_id = "b0123456789abcdef0123456789abcdef";
        let board_entry = store
            .publish_board_post(
                keeper_character.id,
                BoardKey::General,
                board_public_id,
                "Lantern board",
                "A durable public entry.",
            )
            .await
            .unwrap();
        assert!(board_entry.summary.can_delete);
        let board_page = store
            .list_board_posts(player_character.id, BoardKey::General, 1)
            .await
            .unwrap();
        assert_eq!(board_page.total_posts, 1);
        assert!(board_page.posts[0].can_report);
        assert_eq!(
            store
                .read_board_post(player_character.id, board_public_id)
                .await
                .unwrap()
                .body,
            "A durable public entry."
        );
        store
            .report_board_post(
                player_character.id,
                board_public_id,
                "Public board evidence",
            )
            .await
            .unwrap();
        store
            .delete_board_post(keeper_character.id, board_public_id)
            .await
            .unwrap();
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_report_evidence"),
            )
            .await,
            1
        );

        let actionable_id = "bfedcba9876543210fedcba9876543210";
        store
            .publish_board_post(
                player_character.id,
                BoardKey::Newbie,
                actionable_id,
                "Actionable board entry",
                "Remove this after staff review.",
            )
            .await
            .unwrap();
        let board_report_id = store
            .report_board_post(keeper_character.id, actionable_id, "Requires staff action")
            .await
            .unwrap();
        let reports = store
            .staff_reports(StaffAuditRecord {
                actor_account_id: keeper.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsView,
                action: "reports.list".into(),
                reason: "review board evidence".into(),
                details: serde_json::json!({}),
            })
            .await
            .unwrap();
        assert_eq!(
            reports
                .iter()
                .find(|report| report.id == board_report_id)
                .unwrap()
                .board_evidence
                .as_ref()
                .unwrap()
                .subject,
            "Actionable board entry"
        );
        store
            .resolve_staff_report(
                board_report_id,
                StaffReportResolution::ActionTaken,
                "board post removed",
                StaffAuditRecord {
                    actor_account_id: keeper.account.id,
                    target_account_id: None,
                    capability: StaffCapability::ReportsResolve,
                    action: "reports.resolve".into(),
                    reason: "review completed".into(),
                    details: serde_json::json!({"reportId": board_report_id}),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .read_board_post(keeper_character.id, actionable_id)
                .await,
            Err(StoreError::BoardPostUnavailable)
        ));
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_report_evidence"),
            )
            .await,
            2
        );
        store
            .set_social_relation(
                player_character.id,
                keeper_character.id,
                SocialRelationKind::Ignore,
                true,
            )
            .await
            .unwrap();
        assert!(
            !store
                .social_delivery_allowed(keeper_character.id, player_character.id)
                .await
                .unwrap()
        );
        assert_eq!(
            store
                .send_mail(
                    keeper_character.id,
                    &player_character.name,
                    "mfedcba9876543210fedcba9876543210",
                    "Suppressed",
                    "This body must not be stored.",
                )
                .await
                .unwrap(),
            MailSendResult::Suppressed
        );
        assert_eq!(
            store
                .mailbox_page(player_character.id, 1)
                .await
                .unwrap()
                .total_messages,
            0
        );
        store
            .moderate_character(
                keeper_character.id,
                player_character.id,
                ModerationAction::Mute {
                    expires_at: None,
                    reason: "integration test".into(),
                },
            )
            .await
            .unwrap();
        assert!(
            store
                .social_profile(player_character.id)
                .await
                .unwrap()
                .muted
        );
        store
            .report_character(player_character.id, keeper_character.id, "test report")
            .await
            .unwrap();
        assert!(store.audit_log().await.unwrap().len() >= 4);

        admin_batch(
            &base_url,
            format!(
                "INSERT INTO {schema}.domain_events (event_json)
                 SELECT '{{\"type\":\"retained-test\"}}'
                 FROM generate_series(1, {DOMAIN_EVENT_RETENTION})"
            ),
        )
        .await;
        store
            .commit_action(Vec::new(), runtime.clone(), vec![event.clone()])
            .await
            .unwrap();
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.domain_events"),
            )
            .await,
            DOMAIN_EVENT_RETENTION
        );
        let compacted_watermark = DOMAIN_EVENT_RETENTION + 2;
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"
                ),
            )
            .await,
            compacted_watermark
        );

        admin_batch(
            &base_url,
            format!(
                "CREATE FUNCTION {schema}.reject_domain_event_compaction()
                 RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                     RAISE EXCEPTION 'injected compaction failure';
                 END $$;
                 CREATE TRIGGER reject_domain_event_compaction
                 BEFORE DELETE ON {schema}.domain_events
                 FOR EACH STATEMENT
                 EXECUTE FUNCTION {schema}.reject_domain_event_compaction();"
            ),
        )
        .await;
        let mut rejected_runtime = runtime.clone();
        rejected_runtime.logical_tick += 1;
        assert!(
            store
                .commit_action(Vec::new(), rejected_runtime, vec![event])
                .await
                .is_err()
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.domain_events"),
            )
            .await,
            DOMAIN_EVENT_RETENTION
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"
                ),
            )
            .await,
            compacted_watermark
        );
        admin_batch(
            &base_url,
            format!(
                "DROP TRIGGER reject_domain_event_compaction ON {schema}.domain_events;
                 DROP FUNCTION {schema}.reject_domain_event_compaction();"
            ),
        )
        .await;

        store.revoke_session(login.token.as_str()).await.unwrap();
        assert!(matches!(
            store.resolve_session(login.token.as_str()).await,
            Err(StoreError::SessionRevoked)
        ));

        drop(store);
        tokio::time::sleep(Duration::from_millis(25)).await;
        admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
    }
}
