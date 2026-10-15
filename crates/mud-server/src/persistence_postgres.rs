//! PostgreSQL implementation of the transport-independent character store.
//!
//! `postgres::Client` is synchronous, so one dedicated owner thread performs
//! database work behind the same bounded Tokio channel used by SQLite. Argon2
//! runs in a separately bounded blocking service, and no PostgreSQL call runs
//! on an async executor thread.

use std::thread;

use async_trait::async_trait;
use postgres::{Client, GenericClient, IsolationLevel, ToStatement};
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, BoardKey, CharacterGameplaySnapshot, CharacterId,
    CharacterSnapshot, DomainEvent, DurableCharacterSnapshot, MAX_BOARD_BODY_BYTES,
    MAX_BOARD_REPORT_REASON_BYTES, MAX_BOARD_SUBJECT_BYTES, MAX_MAIL_BODY_BYTES,
    MAX_MAIL_REPORT_REASON_BYTES, MAX_MAIL_SUBJECT_BYTES, SocialChannelName, StaffCapability,
    StaffProfileKey, StaffReportResolution, WorldRuntimeSnapshot, valid_board_public_id,
    valid_board_subject, valid_mail_public_id, valid_mail_visible_text,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};

use crate::{
    auth::{
        AuthenticatedSession, DEFAULT_SESSION_TTL, PasswordService, SessionToken, digest_token,
        validate_password,
    },
    persistence::{
        self, BOARD_CAPACITY_PER_BOARD, BOARD_MAX_PAGES, BOARD_PAGE_SIZE, BoardEntry, BoardPage,
        BoardPostSummary, BoardReportEvidence, CharacterStore, DOMAIN_EVENT_RETENTION,
        DurableAreaCheckpoint, DurableAreaGeneration, DurableCharacterGameplay, MAILBOX_CAPACITY,
        MAX_REPORT_REASON_BYTES, MailMessage, MailMessageSummary, MailPage, MailReportEvidence,
        MailSendResult, ModerationAction, PersistenceCommandHandler, PreparedLogin, Request,
        SocialProfile, SocialRelationKind, StaffAccess, StaffAuditEntry, StaffAuditRecord,
        StaffReport, StartupGameplayLoadLimits, StartupWorldRecovery, StoreError,
        StoryPackCompatibility, StorySaveRecord, StorySaveSummary, StorySaveWrite, StorySlot,
        decode_runtime_snapshot, decode_startup_domain_events, decode_startup_gameplay,
        dispatch_persistence_request, ensure_all_character_ids_exist,
        ensure_all_startup_characters_loaded, normalise_character_identity_ids,
        normalise_startup_character_ids, validate_area_checkpoint, validate_area_generation,
        validate_area_generation_transition, validate_area_rehydration_commit,
        validate_area_rehydration_previous_runtime, validate_area_retirement_commit,
        validate_area_retirement_previous_runtime,
    },
};
// Backend-agnostic bounds, JSON shaping, quota/capacity, retention and staff
// authority policy shared with the SQLite store.
use crate::persistence::common::*;

#[cfg(test)]
use crate::persistence::{AuditEntry, area_checkpoint_unit_id};

// PostgreSQL binds `BIGINT` page arithmetic; the values are the shared `u32`
// public bounds widened losslessly.
const MAIL_PAGE_SIZE: i64 = persistence::MAIL_PAGE_SIZE as i64;
const MAIL_MAX_PAGES: i64 = persistence::MAIL_MAX_PAGES as i64;
const MAIL_TRANSACTION_LOCK_ID: i64 = 0x5449_414d_4d41_494c;
const BOARD_TRANSACTION_LOCK_ID: i64 = 0x5449_414d_424f_4152;
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
                let connection = match (|| -> Result<Client, StoreError> {
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

                let mut handler = PostgresCommandHandler { connection };
                while let Some(request) = rx.blocking_recv() {
                    if dispatch_persistence_request(&mut handler, request).is_break() {
                        break;
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

/// PostgreSQL handler: `postgres::Client` requires `&mut self`, so every
/// method borrows the connection mutably. Bodies otherwise mirror the SQLite
/// handler command-for-command; only `Health` differs by SQL dialect.
struct PostgresCommandHandler {
    connection: Client,
}

impl PersistenceCommandHandler for PostgresCommandHandler {
    fn health(&mut self) -> Result<(), StoreError> {
        self.connection
            .query_one("SELECT 1::BIGINT", &[])
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn finish_register_account(
        &mut self,
        name: String,
        password_hash: String,
    ) -> Result<AuthenticatedSession, StoreError> {
        finish_register_account(
            &mut self.connection,
            &name,
            &password_hash,
            unix_timestamp(),
        )
    }

    fn prepare_login_account(&mut self, name: String) -> Result<Option<PreparedLogin>, StoreError> {
        prepare_login_account(&mut self.connection, &name)
    }

    fn finish_login_account(
        &mut self,
        prepared: PreparedLogin,
    ) -> Result<AuthenticatedSession, StoreError> {
        finish_login_account(&mut self.connection, &prepared, unix_timestamp())
    }

    fn resolve_session(&mut self, token: String) -> Result<AccountSnapshot, StoreError> {
        resolve_session(&mut self.connection, &token, unix_timestamp())
    }

    fn revoke_session(&mut self, token: String) -> Result<(), StoreError> {
        revoke_session(&mut self.connection, &token, unix_timestamp())
    }

    #[cfg(test)]
    fn load_or_create_account(&mut self, name: String) -> Result<AccountSnapshot, StoreError> {
        load_or_create_account(&mut self.connection, &name)
    }

    fn list_characters(
        &mut self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError> {
        list_characters(&mut self.connection, account_id)
    }

    fn load_or_create_character(
        &mut self,
        account_id: AccountId,
        name: String,
        start_room: String,
        initial_gameplay: Box<CharacterGameplaySnapshot>,
    ) -> Result<CharacterSnapshot, StoreError> {
        load_or_create_character_with_gameplay(
            &mut self.connection,
            account_id,
            &name,
            &start_room,
            &initial_gameplay,
        )
    }

    fn load_character(
        &mut self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError> {
        load_character(&mut self.connection, account_id, character_id)
    }

    fn save(&mut self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        save(&mut self.connection, &snapshot)
    }

    fn load_character_gameplay(
        &mut self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError> {
        load_character_gameplay(&mut self.connection, character_id)
    }

    fn load_character_gameplay_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
        load_character_gameplay_batch(&mut self.connection, character_ids, limits)
    }

    fn ensure_character_ids_exist_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError> {
        ensure_character_ids_exist_batch(&mut self.connection, character_ids)
    }

    fn load_runtime_snapshot(&mut self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
        load_runtime_snapshot(&mut self.connection)
    }

    fn load_startup_world_recovery(&mut self) -> Result<StartupWorldRecovery, StoreError> {
        load_startup_world_recovery(&mut self.connection)
    }

    fn has_durable_character_authority(&mut self) -> Result<bool, StoreError> {
        has_durable_character_authority(&mut self.connection)
    }

    fn load_area_generation(&mut self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        load_area_generation(&mut self.connection)
    }

    fn bind_area_generation_if_absent(
        &mut self,
        generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError> {
        bind_area_generation_if_absent(&mut self.connection, &generation)
    }

    fn commit_area_generation_activation(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), StoreError> {
        commit_area_generation_activation(&mut self.connection, &runtime, &expected_current, &next)
    }

    fn load_area_checkpoints(&mut self) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
        load_area_checkpoints(&mut self.connection)
    }

    fn commit_area_retirement(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        commit_area_retirement(
            &mut self.connection,
            &runtime,
            &expected_generation,
            &checkpoint,
        )
    }

    fn commit_area_rehydration(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        commit_area_rehydration(
            &mut self.connection,
            &runtime,
            &expected_generation,
            &checkpoint,
        )
    }

    fn load_domain_events(&mut self) -> Result<Vec<DomainEvent>, StoreError> {
        load_domain_events(&mut self.connection)
    }

    fn save_story_slot(&mut self, write: StorySaveWrite) -> Result<StorySaveSummary, StoreError> {
        save_story_slot(&mut self.connection, write, unix_timestamp())
    }

    fn load_story_slot(
        &mut self,
        account_id: AccountId,
        story_id: String,
        slot: StorySlot,
        pack: StoryPackCompatibility,
    ) -> Result<Option<StorySaveRecord>, StoreError> {
        load_story_slot(&mut self.connection, account_id, &story_id, slot, &pack)
    }

    fn list_story_slots(
        &mut self,
        account_id: AccountId,
        story_id: String,
    ) -> Result<Vec<StorySaveSummary>, StoreError> {
        list_story_slots(&mut self.connection, account_id, &story_id)
    }

    fn delete_story_slot(
        &mut self,
        account_id: AccountId,
        story_id: String,
        slot: StorySlot,
    ) -> Result<bool, StoreError> {
        delete_story_slot(&mut self.connection, account_id, &story_id, slot)
    }

    fn commit_action(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError> {
        commit_action(&mut self.connection, &characters, &runtime, &events)
    }

    fn commit_staff_action(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        commit_staff_action(
            &mut self.connection,
            &characters,
            &runtime,
            &events,
            audit,
            unix_timestamp(),
        )
    }

    fn commit_characters(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
    ) -> Result<(), StoreError> {
        commit_characters_only(&mut self.connection, &characters)
    }

    fn social_profile(&mut self, character_id: CharacterId) -> Result<SocialProfile, StoreError> {
        social_profile(&mut self.connection, character_id, unix_timestamp())
    }

    fn social_target(&mut self, name: String) -> Result<SocialProfile, StoreError> {
        social_target(&mut self.connection, &name, unix_timestamp())
    }

    fn social_delivery_allowed(
        &mut self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError> {
        social_delivery_allowed(&mut self.connection, sender, recipient)
    }

    fn set_social_relation(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError> {
        set_social_relation(
            &mut self.connection,
            actor,
            target,
            relation,
            enabled,
            unix_timestamp(),
        )
    }

    fn social_channel_subscriptions(
        &mut self,
        character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError> {
        social_channel_subscriptions(&mut self.connection, character_id)
    }

    fn social_channel_subscribers(
        &mut self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        social_channel_subscribers(&mut self.connection, channel)
    }

    fn set_social_channel_subscription(
        &mut self,
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
    ) -> Result<(), StoreError> {
        set_social_channel_subscription(
            &mut self.connection,
            character_id,
            channel,
            subscribed,
            unix_timestamp(),
        )
    }

    fn send_mail(
        &mut self,
        sender: CharacterId,
        target_name: String,
        public_id: String,
        subject: String,
        body: String,
    ) -> Result<MailSendResult, StoreError> {
        send_mail(
            &mut self.connection,
            sender,
            &target_name,
            &public_id,
            &subject,
            &body,
            unix_timestamp(),
        )
    }

    fn mailbox_page(&mut self, recipient: CharacterId, page: u32) -> Result<MailPage, StoreError> {
        mailbox_page(&mut self.connection, recipient, page, unix_timestamp())
    }

    fn read_mail(
        &mut self,
        recipient: CharacterId,
        public_id: String,
    ) -> Result<MailMessage, StoreError> {
        read_mail(
            &mut self.connection,
            recipient,
            &public_id,
            unix_timestamp(),
        )
    }

    fn delete_mail(&mut self, recipient: CharacterId, public_id: String) -> Result<(), StoreError> {
        delete_mail(
            &mut self.connection,
            recipient,
            &public_id,
            unix_timestamp(),
        )
    }

    fn report_mail(
        &mut self,
        recipient: CharacterId,
        public_id: String,
        reason: String,
    ) -> Result<i64, StoreError> {
        report_mail(
            &mut self.connection,
            recipient,
            &public_id,
            &reason,
            unix_timestamp(),
        )
    }

    fn purge_expired_mail(&mut self) -> Result<u64, StoreError> {
        purge_expired_mail(&mut self.connection, unix_timestamp())
    }

    fn list_board_posts(
        &mut self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError> {
        list_board_posts(&mut self.connection, actor, board, page, unix_timestamp())
    }

    fn read_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<BoardEntry, StoreError> {
        read_board_post(&mut self.connection, actor, &public_id, unix_timestamp())
    }

    fn publish_board_post(
        &mut self,
        actor: CharacterId,
        board: BoardKey,
        public_id: String,
        subject: String,
        body: String,
    ) -> Result<BoardEntry, StoreError> {
        publish_board_post(
            &mut self.connection,
            actor,
            board,
            &public_id,
            &subject,
            &body,
            unix_timestamp(),
        )
    }

    fn delete_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<(), StoreError> {
        delete_board_post(&mut self.connection, actor, &public_id, unix_timestamp())
    }

    fn report_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
        reason: String,
    ) -> Result<i64, StoreError> {
        report_board_post(
            &mut self.connection,
            actor,
            &public_id,
            &reason,
            unix_timestamp(),
        )
    }

    fn purge_expired_boards(&mut self) -> Result<u64, StoreError> {
        purge_expired_boards(&mut self.connection, unix_timestamp())
    }

    fn report_character(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        reason: String,
    ) -> Result<(), StoreError> {
        report_character(
            &mut self.connection,
            actor,
            target,
            &reason,
            unix_timestamp(),
        )
    }

    fn moderate_character(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError> {
        moderate_character(
            &mut self.connection,
            actor,
            target,
            action,
            unix_timestamp(),
        )
    }

    fn staff_access(&mut self, character_id: CharacterId) -> Result<StaffAccess, StoreError> {
        staff_access(&mut self.connection, character_id)
    }

    fn staff_reports(&mut self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError> {
        staff_reports(&mut self.connection, audit, unix_timestamp())
    }

    fn resolve_staff_report(
        &mut self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: String,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        resolve_staff_report(
            &mut self.connection,
            report_id,
            resolution,
            &resolution_reason,
            audit,
            unix_timestamp(),
        )
    }

    fn set_staff_profile(
        &mut self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<String>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        set_staff_profile(
            &mut self.connection,
            target_account,
            profile,
            lore_title.as_deref(),
            audit,
            unix_timestamp(),
        )
    }

    fn set_staff_capability(
        &mut self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        set_staff_capability(
            &mut self.connection,
            target_account,
            capability,
            enabled,
            audit,
            unix_timestamp(),
        )
    }

    fn staff_audit_entries(
        &mut self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError> {
        staff_audit_entries(&mut self.connection, audit, unix_timestamp())
    }

    fn append_staff_audit(&mut self, audit: StaffAuditRecord) -> Result<(), StoreError> {
        append_staff_audit_transaction(&mut self.connection, audit, unix_timestamp())
    }

    #[cfg(test)]
    fn audit_log(&mut self) -> Result<Vec<AuditEntry>, StoreError> {
        audit_log(&mut self.connection)
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

    async fn save_story_slot(&self, write: StorySaveWrite) -> Result<StorySaveSummary, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::SaveStorySlot { write, reply }, receive)
            .await
    }

    async fn load_story_slot(
        &self,
        account_id: AccountId,
        story_id: &str,
        slot: StorySlot,
        pack: StoryPackCompatibility,
    ) -> Result<Option<StorySaveRecord>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadStorySlot {
                account_id,
                story_id: story_id.to_owned(),
                slot,
                pack,
                reply,
            },
            receive,
        )
        .await
    }

    async fn list_story_slots(
        &self,
        account_id: AccountId,
        story_id: &str,
    ) -> Result<Vec<StorySaveSummary>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ListStorySlots {
                account_id,
                story_id: story_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn delete_story_slot(
        &self,
        account_id: AccountId,
        story_id: &str,
        slot: StorySlot,
    ) -> Result<bool, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::DeleteStorySlot {
                account_id,
                story_id: story_id.to_owned(),
                slot,
                reply,
            },
            receive,
        )
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

    async fn commit_characters(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::CommitCharacters { characters, reply }, receive)
            .await
    }
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
    let gameplay_json = encode_character_gameplay(initial_gameplay)?;
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

const SAVE_CHARACTER_SQL: &str = "UPDATE characters
     SET name = $1, room_id = $2, item_locations_json = $3,
         updated_at = floor(extract(epoch FROM now()))::BIGINT
     WHERE id = $4 AND account_id = $5";

fn save<C: GenericClient>(
    connection: &mut C,
    snapshot: &CharacterSnapshot,
) -> Result<(), StoreError> {
    save_with(connection, SAVE_CHARACTER_SQL, snapshot)
}

/// Saves one character through `statement`, which is either
/// [`SAVE_CHARACTER_SQL`] or a statement prepared from it once per transaction.
fn save_with<C: GenericClient, T: ?Sized + ToStatement>(
    connection: &mut C,
    statement: &T,
    snapshot: &CharacterSnapshot,
) -> Result<(), StoreError> {
    let (name, item_locations_json) = encode_character_row(snapshot)?;
    let updated = connection.execute(
        statement,
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
    let (runtime_json, adventure_json) = encode_runtime_documents(runtime)?;
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

// ---------------------------------------------------------------------------
// Story save slots (schema v16, ADR 0030)
// ---------------------------------------------------------------------------

/// Atomic upsert of one slot. `SELECT .. FOR UPDATE` on the account row
/// serializes concurrent writers of the same account (also across processes)
/// between the cap check and the write; the whole operation is one
/// transaction.
fn save_story_slot(
    connection: &mut Client,
    write: StorySaveWrite,
    now: i64,
) -> Result<StorySaveSummary, StoreError> {
    let prepared = prepare_story_save(write)?;
    // The shared contract bounds slot to 0..=9 and the version to 1..=255.
    let slot = i16::try_from(prepared.slot).map_err(|_| StoreError::InvalidStorySave)?;
    let state_format_version =
        i32::try_from(prepared.state_format_version).map_err(|_| StoreError::InvalidStorySave)?;
    let mut transaction = connection.transaction()?;
    if transaction
        .query_opt(
            "SELECT id FROM accounts WHERE id = $1 FOR UPDATE",
            &[&prepared.account_id.0],
        )?
        .is_none()
    {
        return Err(StoreError::AccountNotFound(prepared.account_id));
    }
    let totals = transaction.query_one(
        "SELECT COUNT(DISTINCT story_id)::BIGINT,
                COALESCE(SUM(octet_length(state_json)), 0)::BIGINT
         FROM story_saves WHERE account_id = $1",
        &[&prepared.account_id.0],
    )?;
    let stories_with_saves: i64 = totals.get(0);
    let account_bytes: i64 = totals.get(1);
    let story_has_saves: bool = transaction
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM story_saves WHERE account_id = $1 AND story_id = $2)",
            &[&prepared.account_id.0, &prepared.story_id],
        )?
        .get(0);
    let replaced_bytes: i64 = transaction
        .query_opt(
            "SELECT octet_length(state_json)::BIGINT FROM story_saves
             WHERE account_id = $1 AND story_id = $2 AND slot = $3",
            &[&prepared.account_id.0, &prepared.story_id, &slot],
        )?
        .map_or(0, |row| row.get(0));
    check_story_save_account_capacity(
        stories_with_saves,
        story_has_saves,
        account_bytes,
        replaced_bytes,
        prepared.state_bytes,
    )?;
    let now = now.max(0);
    transaction.execute(
        "INSERT INTO story_saves
             (account_id, story_id, slot, pack_id, pack_sha256,
              state_format_version, state_json, state_sha256,
              created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
         ON CONFLICT (account_id, story_id, slot) DO UPDATE SET
             pack_id = EXCLUDED.pack_id,
             pack_sha256 = EXCLUDED.pack_sha256,
             state_format_version = EXCLUDED.state_format_version,
             state_json = EXCLUDED.state_json,
             state_sha256 = EXCLUDED.state_sha256,
             updated_at = GREATEST(story_saves.created_at, EXCLUDED.updated_at)",
        &[
            &prepared.account_id.0,
            &prepared.story_id,
            &slot,
            &prepared.pack_id,
            &prepared.pack_sha256,
            &state_format_version,
            &prepared.state_json,
            &prepared.state_sha256,
            &now,
        ],
    )?;
    let row = read_story_save_row(
        &mut transaction,
        prepared.account_id,
        &prepared.story_id,
        slot,
    )?
    .ok_or(StoreError::StorySaveCorrupt)?;
    let record = decode_story_save_row(row)?;
    transaction.commit()?;
    Ok(record.summary)
}

fn read_story_save_row<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    story_id: &str,
    slot: i16,
) -> Result<Option<StorySaveRow>, StoreError> {
    Ok(connection
        .query_opt(
            "SELECT story_id, slot, pack_id, pack_sha256, state_format_version,
                    state_json, state_sha256, created_at, updated_at
             FROM story_saves
             WHERE account_id = $1 AND story_id = $2 AND slot = $3",
            &[&account_id.0, &story_id, &slot],
        )?
        .map(|row| StorySaveRow {
            story_id: row.get(0),
            slot: i64::from(row.get::<_, i16>(1)),
            pack_id: row.get(2),
            pack_sha256: row.get(3),
            state_format_version: i64::from(row.get::<_, i32>(4)),
            state_json: row.get(5),
            state_sha256: row.get(6),
            created_at: row.get(7),
            updated_at: row.get(8),
        }))
}

fn load_story_slot<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    story_id: &str,
    slot: StorySlot,
    pack: &StoryPackCompatibility,
) -> Result<Option<StorySaveRecord>, StoreError> {
    validate_story_lookup(account_id, story_id)?;
    validate_story_pack_compatibility(pack)?;
    let Some(row) = read_story_save_row(connection, account_id, story_id, i16::from(slot.get()))?
    else {
        return Ok(None);
    };
    let record = decode_story_save_row(row)?;
    check_story_pack_compatibility(&record.summary, pack)?;
    Ok(Some(record))
}

fn list_story_slots<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    story_id: &str,
) -> Result<Vec<StorySaveSummary>, StoreError> {
    validate_story_lookup(account_id, story_id)?;
    let limit = row_count(tiamud_persistence_sqlite::story_saves::STORY_SLOTS_PER_STORY + 1);
    let rows = connection.query(
        "SELECT story_id, slot, pack_id, pack_sha256, state_format_version,
                octet_length(state_json)::BIGINT, state_sha256, created_at, updated_at
         FROM story_saves
         WHERE account_id = $1 AND story_id = $2
         ORDER BY slot
         LIMIT $3",
        &[&account_id.0, &story_id, &limit],
    )?;
    if rows.len() > tiamud_persistence_sqlite::story_saves::STORY_SLOTS_PER_STORY {
        return Err(StoreError::StorySaveCorrupt);
    }
    rows.into_iter()
        .map(|row| {
            decode_story_save_summary(
                row.get(0),
                i64::from(row.get::<_, i16>(1)),
                row.get(2),
                row.get(3),
                i64::from(row.get::<_, i32>(4)),
                row.get(5),
                row.get(6),
                row.get(7),
                row.get(8),
            )
        })
        .collect()
}

fn delete_story_slot<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    story_id: &str,
    slot: StorySlot,
) -> Result<bool, StoreError> {
    validate_story_lookup(account_id, story_id)?;
    let deleted = connection.execute(
        "DELETE FROM story_saves WHERE account_id = $1 AND story_id = $2 AND slot = $3",
        &[&account_id.0, &story_id, &i16::from(slot.get())],
    )?;
    Ok(deleted == 1)
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
    let (runtime_json, world_adventure_json) = encode_runtime_documents(runtime)?;
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

/// Characters-only commit (ADR 0027): the same character rows that
/// `commit_action_with_retention` writes, in one transaction, without
/// touching the runtime snapshot, world adventure row or event ledger.
fn commit_characters_only(
    connection: &mut Client,
    characters: &[DurableCharacterSnapshot],
) -> Result<(), StoreError> {
    let rows = characters
        .iter()
        .map(|snapshot| {
            Ok::<_, StoreError>((
                snapshot,
                encode_character_gameplay(&snapshot.gameplay)?,
                encode_character_adventure(&snapshot.gameplay)?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut transaction = connection.transaction()?;
    if !rows.is_empty() {
        let save_character = transaction.prepare(SAVE_CHARACTER_SQL)?;
        let update_gameplay = transaction.prepare(
            "UPDATE characters
             SET gameplay_json = $1,
                 updated_at = floor(extract(epoch FROM now()))::BIGINT
             WHERE id = $2 AND account_id = $3",
        )?;
        let upsert_adventure = transaction.prepare(
            "INSERT INTO character_adventure (character_id, state_json, updated_at)
             VALUES ($1, $2, floor(extract(epoch FROM now()))::BIGINT)
             ON CONFLICT(character_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at",
        )?;
        for (snapshot, gameplay_json, adventure_json) in rows {
            save_with(&mut transaction, &save_character, &snapshot.character)?;
            let updated = transaction.execute(
                &update_gameplay,
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
                &upsert_adventure,
                &[&snapshot.character.id.0, &adventure_json],
            )?;
        }
    }
    transaction.commit()?;
    Ok(())
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
    let EncodedActionCommit {
        gameplay_json,
        adventure_json,
        runtime_json,
        world_adventure_json,
        event_json,
        staff_audit,
    } = encode_action_commit(characters, runtime, events, staff_audit)?;

    let mut transaction = connection.transaction()?;
    if let Some((audit, _)) = &staff_audit {
        validate_staff_authority(&mut transaction, audit, None, audit.target_account_id)?;
    }
    // Statements repeated per character or per event are prepared once per
    // transaction instead of once per execution. Order and atomicity are
    // unchanged: every row is still written by the same transaction.
    if !characters.is_empty() {
        let save_character = transaction.prepare(SAVE_CHARACTER_SQL)?;
        let update_gameplay = transaction.prepare(
            "UPDATE characters
             SET gameplay_json = $1,
                 updated_at = floor(extract(epoch FROM now()))::BIGINT
             WHERE id = $2 AND account_id = $3",
        )?;
        let upsert_adventure = transaction.prepare(
            "INSERT INTO character_adventure (character_id, state_json, updated_at)
             VALUES ($1, $2, floor(extract(epoch FROM now()))::BIGINT)
             ON CONFLICT(character_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at",
        )?;
        for ((snapshot, gameplay_json), adventure_json) in
            characters.iter().zip(gameplay_json).zip(adventure_json)
        {
            save_with(&mut transaction, &save_character, &snapshot.character)?;
            let updated = transaction.execute(
                &update_gameplay,
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
                &upsert_adventure,
                &[&snapshot.character.id.0, &adventure_json],
            )?;
        }
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, $1, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        &[&world_adventure_json],
    )?;
    if !event_json.is_empty() {
        let insert_event =
            transaction.prepare("INSERT INTO domain_events (event_json) VALUES ($1)")?;
        for json in event_json {
            transaction.execute(&insert_event, &[&json])?;
        }
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
    let window = usage.map_or(QuotaWindow::empty(now), |row| {
        QuotaWindow {
            hour_started_at: row.get(0),
            hour_count: i64::from(row.get::<_, i32>(1)),
            day_started_at: row.get(2),
            day_count: i64::from(row.get::<_, i32>(3)),
        }
        .rolled(now)
    });
    if window.exhausted(MAIL_HOURLY_CAPACITY, MAIL_DAILY_CAPACITY) {
        return Err(StoreError::MailRateLimited);
    }
    let next_hour_count = next_quota_count(window.hour_count).ok_or(StoreError::MailRateLimited)?;
    let next_day_count = next_quota_count(window.day_count).ok_or(StoreError::MailRateLimited)?;
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
            &window.hour_started_at,
            &next_hour_count,
            &window.day_started_at,
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
        || mail_delivery_exceeds_capacity(mailbox_count, mailbox_bytes, added_bytes, global_count);
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
    if !(1..=MAIL_MAX_PAGES).contains(&i64::from(page)) {
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
    let total_pages = (total.saturating_add(MAIL_PAGE_SIZE - 1) / MAIL_PAGE_SIZE).max(1);
    if total_pages > MAIL_MAX_PAGES {
        return Err(StoreError::InvalidMailInput);
    }
    let offset = (i64::from(page) - 1).saturating_mul(MAIL_PAGE_SIZE);
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
        capacity: MAILBOX_CAPACITY,
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
    check_mail_report_quota(open_reports)?;
    let evidence_count: i64 = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM social_mail_report_evidence
             WHERE expires_at > $1",
            &[&now],
        )?
        .get(0);
    check_mail_evidence_capacity(evidence_count)?;

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
    let message_count = row_count(message_rows.len());
    let remaining = purge_remaining(limit, message_rows.len());
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
    Ok(message_count.saturating_add(row_count(evidence_rows.len())))
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
    let window = usage.map_or(QuotaWindow::empty(now), |row| {
        QuotaWindow {
            hour_started_at: row.get(0),
            hour_count: i64::from(row.get::<_, i32>(1)),
            day_started_at: row.get(2),
            day_count: i64::from(row.get::<_, i32>(3)),
        }
        .rolled(now)
    });
    if window.exhausted(BOARD_HOURLY_CAPACITY, BOARD_DAILY_CAPACITY) {
        return Err(StoreError::BoardRateLimited);
    }
    let next_hour_count =
        next_quota_count(window.hour_count).ok_or(StoreError::BoardRateLimited)?;
    let next_day_count = next_quota_count(window.day_count).ok_or(StoreError::BoardRateLimited)?;
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
            &window.hour_started_at,
            &next_hour_count,
            &window.day_started_at,
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
    let post_count = row_count(post_rows.len());
    let remaining = purge_remaining(BOARD_PURGE_LIMIT, post_rows.len());
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
        row_count(rows.len())
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
    let profile = parse_staff_profile(profile.as_deref())?;
    let capabilities = connection.query(
        "SELECT capability FROM staff_capability_assignments
         WHERE account_id = $1 ORDER BY capability",
        &[&account_id.0],
    )?;
    staff_access_from_rows(
        account_id,
        &role,
        profile,
        lore_title,
        capabilities.into_iter().map(|row| row.get::<_, String>(0)),
    )
}

fn validate_staff_authority<C: GenericClient>(
    connection: &mut C,
    audit: &PreparedStaffAudit,
    expected_capability: Option<StaffCapability>,
    target_account: Option<AccountId>,
) -> Result<StaffAccess, StoreError> {
    validate_staff_authority_with(audit, expected_capability, target_account, |account| {
        staff_access_by_account(connection, account)
    })
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
            let board_evidence = match board_key {
                Some(board_key) => Some(BoardReportEvidence {
                    board: BoardKey::parse(&board_key).ok_or_else(|| {
                        StoreError::InvalidStaffState(format!(
                            "board report evidence key {board_key:?}"
                        ))
                    })?,
                    post_public_id: row.get(15),
                    author_name: row.get(16),
                    subject: row.get(17),
                    body: row.get(18),
                    posted_at: row.get(19),
                    created_at: row.get(20),
                    expires_at: row.get(21),
                }),
                None => None,
            };
            Ok(StaffReport {
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
                board_evidence,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
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
    bind_report_audit_target(&mut audit, target_account)?;
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
    authorize_profile_assignment(&actor, profile)?;
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
    authorize_capability_change(&actor, &target, capability, enabled)?;
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
    let PreparedModeration {
        required_capability,
        action_name,
        reason,
        details,
    } = prepare_moderation(&action, now)?;
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

fn append_audit<C: GenericClient>(
    connection: &mut C,
    actor: AccountId,
    target: Option<AccountId>,
    action: &str,
    details: serde_json::Value,
    now: i64,
) -> Result<(), StoreError> {
    let details_json = encode_moderation_audit_details(action, &details)?;
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

#[cfg(test)]
mod tests;
