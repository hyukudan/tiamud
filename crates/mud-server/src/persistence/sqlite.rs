//! SQLite implementation of the transport-independent character store.
//!
//! One dedicated owner thread holds the `rusqlite::Connection` behind the
//! bounded request channel shared with PostgreSQL. Only SQL, statement
//! caching and transaction boundaries live here; backend-agnostic policy is
//! in [`super::common`] and [`super::recovery`].

use super::*;

pub(super) const MAX_DURABLE_AREA_CHECKPOINTS: usize =
    tiamud_core::MAX_RUNTIME_AREA_RESIDENCY_ENTRIES;
pub(super) const SQLITE_CHARACTER_BATCH_SIZE: usize = 900;

/// SQLite handler: each method reproduces the exact call the writer loop used
/// to inline, including the shared (`&`) vs mutable (`&mut`) connection borrow
/// per command and the SQLite-dialect `Health` probe.
pub(super) struct SqliteCommandHandler {
    connection: Connection,
}

impl PersistenceCommandHandler for SqliteCommandHandler {
    fn health(&mut self) -> Result<(), StoreError> {
        self.connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
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
        prepare_login_account(&self.connection, &name)
    }

    fn finish_login_account(
        &mut self,
        prepared: PreparedLogin,
    ) -> Result<AuthenticatedSession, StoreError> {
        finish_login_account(&mut self.connection, &prepared, unix_timestamp())
    }

    fn resolve_session(&mut self, token: String) -> Result<AccountSnapshot, StoreError> {
        resolve_session(&self.connection, &token, unix_timestamp())
    }

    fn revoke_session(&mut self, token: String) -> Result<(), StoreError> {
        revoke_session(&self.connection, &token, unix_timestamp())
    }

    #[cfg(test)]
    fn load_or_create_account(&mut self, name: String) -> Result<AccountSnapshot, StoreError> {
        load_or_create_account(&self.connection, &name)
    }

    fn list_characters(
        &mut self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError> {
        list_characters(&self.connection, account_id)
    }

    fn load_or_create_character(
        &mut self,
        account_id: AccountId,
        name: String,
        start_room: String,
        initial_gameplay: Box<CharacterGameplaySnapshot>,
    ) -> Result<CharacterSnapshot, StoreError> {
        load_or_create_character_with_gameplay(
            &self.connection,
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
        load_character(&self.connection, account_id, character_id)
    }

    fn save(&mut self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        save(&self.connection, &snapshot)
    }

    fn load_character_gameplay(
        &mut self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError> {
        load_character_gameplay(&self.connection, character_id)
    }

    fn load_character_gameplay_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
        load_character_gameplay_batch(&self.connection, character_ids, limits)
    }

    fn ensure_character_ids_exist_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError> {
        ensure_character_ids_exist_batch(&self.connection, character_ids)
    }

    fn load_runtime_snapshot(&mut self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
        load_runtime_snapshot(&self.connection)
    }

    fn load_startup_world_recovery(&mut self) -> Result<StartupWorldRecovery, StoreError> {
        load_startup_world_recovery(&mut self.connection)
    }

    fn has_durable_character_authority(&mut self) -> Result<bool, StoreError> {
        has_durable_character_authority(&self.connection)
    }

    fn load_area_generation(&mut self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        load_area_generation(&self.connection)
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
        load_area_checkpoints(&self.connection)
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
        load_domain_events(&self.connection)
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

    fn social_profile(&mut self, character_id: CharacterId) -> Result<SocialProfile, StoreError> {
        social_profile(&self.connection, character_id, unix_timestamp())
    }

    fn social_target(&mut self, name: String) -> Result<SocialProfile, StoreError> {
        social_target(&self.connection, &name, unix_timestamp())
    }

    fn social_delivery_allowed(
        &mut self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError> {
        social_delivery_allowed(&self.connection, sender, recipient)
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
        social_channel_subscriptions(&self.connection, character_id)
    }

    fn social_channel_subscribers(
        &mut self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        social_channel_subscribers(&self.connection, channel)
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
        mailbox_page(&self.connection, recipient, page, unix_timestamp())
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
        list_board_posts(&self.connection, actor, board, page, unix_timestamp())
    }

    fn read_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<BoardEntry, StoreError> {
        read_board_post(&self.connection, actor, &public_id, unix_timestamp())
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
        staff_access(&self.connection, character_id)
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
        audit_log(&self.connection)
    }
}

/// SQLite adapter with one dedicated owner thread.
///
/// `rusqlite::Connection` never crosses thread boundaries and gameplay tasks do
/// not block a Tokio executor while SQLite is writing.
#[derive(Clone)]
pub struct SqliteCharacterStore {
    tx: mpsc::Sender<Request>,
    passwords: PasswordService,
    #[cfg(test)]
    area_generation_fault: Arc<AtomicU8>,
    #[cfg(test)]
    area_generation_commit_gate: Arc<TestAreaGenerationCommitGate>,
}

#[cfg(test)]
pub(super) struct TestAreaGenerationCommitGate {
    armed: AtomicBool,
    started: AtomicBool,
    released: AtomicBool,
    changed: Notify,
}

#[cfg(test)]
impl TestAreaGenerationCommitGate {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            started: AtomicBool::new(false),
            released: AtomicBool::new(true),
            changed: Notify::new(),
        }
    }

    fn arm(&self) {
        self.started.store(false, Ordering::SeqCst);
        self.released.store(false, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }

    async fn block_if_armed(&self) {
        if self
            .armed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.started.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
        loop {
            let changed = self.changed.notified();
            if self.released.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }

    async fn wait_until_started(&self) {
        loop {
            let changed = self.changed.notified();
            if self.started.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
    }
}

impl SqliteCharacterStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_password_service(path, PasswordService::default()).await
    }

    pub(super) async fn open_with_password_service(
        path: impl AsRef<Path>,
        passwords: PasswordService,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_owned();
        // Backpressure here prevents an accidental caller from building an
        // unbounded heap queue while the database is busy.
        let (tx, mut rx) = mpsc::channel(PERSISTENCE_REQUEST_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();

        thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                let connection = match (|| -> Result<Connection, StoreError> {
                    let mut connection = Connection::open(path)?;
                    initialise(&mut connection)?;
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

                let mut handler = SqliteCommandHandler { connection };
                while let Some(request) = rx.blocking_recv() {
                    if dispatch_persistence_request(&mut handler, request).is_break() {
                        break;
                    }
                }
            })
            .map_err(StoreError::ThreadSpawn)?;

        ready_rx.await.map_err(|_| StoreError::WorkerStopped)??;
        Ok(Self {
            tx,
            passwords,
            #[cfg(test)]
            area_generation_fault: Arc::new(AtomicU8::new(0)),
            #[cfg(test)]
            area_generation_commit_gate: Arc::new(TestAreaGenerationCommitGate::new()),
        })
    }

    #[cfg(test)]
    pub(crate) fn simulate_next_area_generation_commit_ack_loss(&self) {
        self.area_generation_fault.store(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn simulate_ambiguous_area_generation_commit_failure(&self) {
        self.area_generation_fault.store(2, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn block_next_area_generation_commit(&self) {
        self.area_generation_commit_gate.arm();
    }

    #[cfg(test)]
    pub(crate) async fn wait_for_blocked_area_generation_commit(&self) {
        self.area_generation_commit_gate.wait_until_started().await;
    }

    #[cfg(test)]
    pub(crate) fn release_blocked_area_generation_commit(&self) {
        self.area_generation_commit_gate.release();
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

    /// Bound admission, but once a state-changing request is accepted, wait
    /// for its definitive result. A response deadline here could report a
    /// timeout while the owner thread subsequently commits the mutation.
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
impl CharacterStore for SqliteCharacterStore {
    fn backend_name(&self) -> &'static str {
        "sqlite"
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
        #[cfg(test)]
        if self
            .area_generation_fault
            .compare_exchange(3, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Err(StoreError::WorkerStopped);
        }
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
        #[cfg(test)]
        self.area_generation_commit_gate.block_if_armed().await;
        #[cfg(test)]
        let injected_fault = self.area_generation_fault.load(Ordering::SeqCst);
        #[cfg(test)]
        if injected_fault == 2 {
            self.area_generation_fault.store(3, Ordering::SeqCst);
            return Err(StoreError::WorkerStopped);
        }
        let (reply, receive) = oneshot::channel();
        let result = self
            .request_commit(
                Request::CommitAreaGenerationActivation {
                    runtime,
                    expected_current,
                    next,
                    reply,
                },
                receive,
            )
            .await;
        #[cfg(test)]
        if injected_fault == 1 && result.is_ok() {
            self.area_generation_fault.store(0, Ordering::SeqCst);
            return Err(StoreError::WorkerStopped);
        }
        result
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

pub(super) fn initialise(connection: &mut Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;

    match migrate(connection) {
        Ok(_) => Ok(()),
        Err(SchemaError::Sqlite(error)) => Err(StoreError::Sqlite(error)),
        Err(SchemaError::UnsupportedVersion { found, supported }) => {
            Err(StoreError::UnsupportedSchemaVersion { found, supported })
        }
        Err(SchemaError::ForeignKeyViolations) => Err(StoreError::SchemaForeignKeyViolations),
    }
}

#[cfg(test)]
pub(super) fn register_account(
    connection: &mut Connection,
    name: &str,
    password: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let name = normalise_name(name, "account")?;
    if !validate_password(password) {
        return Err(StoreError::InvalidPassword);
    }
    let password_hash = hash_password(password).map_err(|_| StoreError::PasswordHashing)?;
    finish_register_account(connection, &name, &password_hash, now)
}

#[cfg(test)]
pub(super) fn login_account(
    connection: &mut Connection,
    name: &str,
    password: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let name = normalise_name(name, "account")?;
    if !validate_password(password) {
        return Err(StoreError::InvalidPassword);
    }
    let Some(prepared) = prepare_login_account(connection, &name)? else {
        let _ = hash_password(password);
        return Err(StoreError::InvalidCredentials);
    };
    if !verify_password(password, &prepared.encoded_hash)
        .map_err(|_| StoreError::PasswordHashing)?
    {
        return Err(StoreError::InvalidCredentials);
    }
    finish_login_account(connection, &prepared, now)
}

pub(super) fn finish_register_account(
    connection: &mut Connection,
    name: &str,
    password_hash: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let transaction = connection.transaction()?;
    if transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE name = ?1)",
        [name],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(StoreError::AccountAlreadyExists);
    }

    let role = if transaction.query_row(
        "SELECT COUNT(*) FROM accounts WHERE auth_state = 'registered'",
        [],
        |row| row.get::<_, i64>(0),
    )? == 0
    {
        AccountRole::Admin
    } else {
        AccountRole::Player
    };
    transaction.execute(
        "INSERT INTO accounts (name, password_hash, auth_state, role)
         VALUES (?1, ?2, 'registered', ?3)",
        params![name, password_hash, role_as_database(role)],
    )?;
    let account = AccountSnapshot {
        id: AccountId(transaction.last_insert_rowid()),
        name: name.to_owned(),
    };
    if role == AccountRole::Admin {
        transaction.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (?1, 'implementor', NULL, ?1, ?2, ?2)",
            params![account.id.0, now],
        )?;
        for capability in StaffCapability::ALL {
            transaction.execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES (?1, ?2, ?1, ?3)",
                params![account.id.0, capability.as_str(), now],
            )?;
        }
        append_audit(
            &transaction,
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
             VALUES (?1, ?1, 'staff.assign', 'bootstrap.implementor',
                     'First registered account bootstrap',
                     '{\"source\":\"first_registered_account\"}', ?2)",
            params![account.id.0, now],
        )?;
    }
    let session = issue_session(&transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn prepare_login_account(
    connection: &Connection,
    name: &str,
) -> Result<Option<PreparedLogin>, StoreError> {
    connection
        .query_row(
            "SELECT id, password_hash FROM accounts
             WHERE name = ?1 AND auth_state = 'registered'",
            [name],
            |row| {
                let Some(encoded_hash) = row.get::<_, Option<String>>(1)? else {
                    return Ok(None);
                };
                Ok(Some(PreparedLogin {
                    account_id: AccountId(row.get(0)?),
                    encoded_hash,
                }))
            },
        )
        .optional()
        .map(|prepared| prepared.flatten())
        .map_err(StoreError::from)
}

pub(super) fn finish_login_account(
    connection: &mut Connection,
    prepared: &PreparedLogin,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let transaction = connection.transaction()?;
    let current = transaction
        .query_row(
            "SELECT name, password_hash FROM accounts
             WHERE id = ?1 AND auth_state = 'registered'",
            [prepared.account_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((name, Some(encoded_hash))) = current else {
        return Err(StoreError::InvalidCredentials);
    };
    if encoded_hash != prepared.encoded_hash {
        return Err(StoreError::InvalidCredentials);
    }
    let account = AccountSnapshot {
        id: prepared.account_id,
        name,
    };
    if account_is_banned(&transaction, account.id, now)? {
        return Err(StoreError::AccountBanned);
    }

    let session = issue_session(&transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

pub(super) fn issue_session(
    connection: &Connection,
    account: AccountSnapshot,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let ttl = i64::try_from(DEFAULT_SESSION_TTL.as_secs()).unwrap_or(i64::MAX);
    let expires_at = now.checked_add(ttl).ok_or(StoreError::InvalidSession)?;

    connection.execute(
        "DELETE FROM auth_sessions
         WHERE account_id = ?1 AND (expires_at <= ?2 OR revoked_at IS NOT NULL)",
        params![account.id.0, now],
    )?;
    let active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM auth_sessions WHERE account_id = ?1",
        [account.id.0],
        |row| row.get(0),
    )?;
    if active >= MAX_ACTIVE_SESSIONS {
        let remove_count = active - MAX_ACTIVE_SESSIONS + 1;
        connection.execute(
            "DELETE FROM auth_sessions WHERE token_hash IN (
                 SELECT token_hash FROM auth_sessions
                 WHERE account_id = ?1
                 ORDER BY created_at, rowid LIMIT ?2
             )",
            params![account.id.0, remove_count],
        )?;
    }

    let token = SessionToken::generate();
    connection.execute(
        "INSERT INTO auth_sessions
             (token_hash, account_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![token.digest().as_slice(), account.id.0, now, expires_at],
    )?;
    Ok(AuthenticatedSession {
        account,
        token,
        expires_at,
    })
}

pub(super) fn resolve_session(
    connection: &Connection,
    token: &str,
    now: i64,
) -> Result<AccountSnapshot, StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let session = connection
        .query_row(
            "SELECT accounts.id, accounts.name, auth_sessions.expires_at,
                    auth_sessions.revoked_at
             FROM auth_sessions
             JOIN accounts ON accounts.id = auth_sessions.account_id
             WHERE auth_sessions.token_hash = ?1",
            [token_hash.as_slice()],
            |row| {
                Ok((
                    AccountSnapshot {
                        id: AccountId(row.get(0)?),
                        name: row.get(1)?,
                    },
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::InvalidSession)?;

    if session.2.is_some() {
        return Err(StoreError::SessionRevoked);
    }
    if session.1 <= now {
        return Err(StoreError::SessionExpired);
    }
    if account_is_banned(connection, session.0.id, now)? {
        return Err(StoreError::AccountBanned);
    }
    Ok(session.0)
}

pub(super) fn account_is_banned(
    connection: &Connection,
    account_id: AccountId,
    now: i64,
) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM account_bans
                 WHERE account_id = ?1 AND revoked_at IS NULL
                   AND (expires_at IS NULL OR expires_at > ?2)
             )",
            params![account_id.0, now],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

pub(super) fn revoke_session(
    connection: &Connection,
    token: &str,
    now: i64,
) -> Result<(), StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let updated = connection.execute(
        "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, ?1)
         WHERE token_hash = ?2",
        params![now, token_hash.as_slice()],
    )?;
    if updated == 0 {
        return Err(StoreError::InvalidSession);
    }
    Ok(())
}

pub(super) fn ensure_account_exists(
    connection: &Connection,
    account_id: AccountId,
) -> Result<(), StoreError> {
    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
        [account_id.0],
        |row| row.get(0),
    )? {
        Ok(())
    } else {
        Err(StoreError::AccountNotFound(account_id))
    }
}

pub(super) fn ensure_social_character_exists(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<(), StoreError> {
    let exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM characters WHERE id = ?1)",
        [character_id.0],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::SocialTargetNotFound)
    }
}

pub(super) fn social_profile(
    connection: &Connection,
    character_id: CharacterId,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let raw = connection
        .query_row(
            "SELECT characters.id, characters.account_id, characters.name, accounts.role,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = accounts.id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > ?2)
                    )
             FROM characters
             JOIN accounts ON accounts.id = characters.account_id
             WHERE characters.id = ?1",
            params![character_id.0, now],
            |row| {
                Ok((
                    CharacterId(row.get(0)?),
                    AccountId(row.get(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::SocialTargetNotFound)?;
    Ok(SocialProfile {
        character_id: raw.0,
        account_id: raw.1,
        character_name: raw.2,
        role: role_from_database(&raw.3)?,
        muted: raw.4,
    })
}

pub(super) fn staff_access(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<StaffAccess, StoreError> {
    let account_id = connection
        .query_row(
            "SELECT account_id FROM characters WHERE id = ?1",
            [character_id.0],
            |row| row.get::<_, i64>(0).map(AccountId),
        )
        .optional()?
        .ok_or(StoreError::SocialTargetNotFound)?;
    staff_access_by_account(connection, account_id)
}

pub(super) fn staff_access_by_account(
    connection: &Connection,
    account_id: AccountId,
) -> Result<StaffAccess, StoreError> {
    let (role, profile, lore_title) = connection
        .query_row(
            "SELECT accounts.role, staff_profiles.profile_key, staff_profiles.lore_title
             FROM accounts
             LEFT JOIN staff_profiles ON staff_profiles.account_id = accounts.id
             WHERE accounts.id = ?1",
            [account_id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::AccountNotFound(account_id))?;
    let profile = parse_staff_profile(profile.as_deref())?;
    let mut statement = connection.prepare(
        "SELECT capability FROM staff_capability_assignments
         WHERE account_id = ?1 ORDER BY capability",
    )?;
    let raw = statement
        .query_map([account_id.0], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    staff_access_from_rows(account_id, &role, profile, lore_title, raw)
}

pub(super) fn validate_staff_authority(
    connection: &Connection,
    audit: &PreparedStaffAudit,
    expected_capability: Option<StaffCapability>,
    target_account: Option<AccountId>,
) -> Result<StaffAccess, StoreError> {
    validate_staff_authority_with(audit, expected_capability, target_account, |account| {
        staff_access_by_account(connection, account)
    })
}

pub(super) fn insert_staff_audit(
    connection: &Connection,
    audit: &PreparedStaffAudit,
    now: i64,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO staff_audit
             (actor_account_id, target_account_id, capability, action,
              reason, details_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            audit.actor_account_id.0,
            audit.target_account_id.map(|account| account.0),
            audit.capability.as_str(),
            audit.action,
            audit.reason,
            audit.details_json,
            now
        ],
    )?;
    Ok(())
}

pub(super) fn social_target(
    connection: &Connection,
    name: &str,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let name = normalise_name(name, "social target")?;
    let mut statement = connection
        .prepare("SELECT id FROM characters WHERE name = ?1 COLLATE NOCASE ORDER BY id LIMIT 2")?;
    let matches = statement
        .query_map([name], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match matches.as_slice() {
        [id] => social_profile(connection, CharacterId(*id), now),
        [] => Err(StoreError::SocialTargetNotFound),
        [_, ..] => Err(StoreError::AmbiguousSocialTarget),
    }
}

pub(super) fn social_delivery_allowed(
    connection: &Connection,
    sender: CharacterId,
    recipient: CharacterId,
) -> Result<bool, StoreError> {
    if sender == recipient {
        return Ok(true);
    }
    let both_exist: i64 = connection.query_row(
        "SELECT COUNT(*) FROM characters WHERE id IN (?1, ?2)",
        params![sender.0, recipient.0],
        |row| row.get(0),
    )?;
    if both_exist != 2 {
        return Err(StoreError::SocialTargetNotFound);
    }
    let suppressed = connection.query_row(
        "SELECT
             EXISTS(SELECT 1 FROM social_ignores
                    WHERE owner_character_id = ?2 AND target_character_id = ?1)
             OR EXISTS(SELECT 1 FROM social_blocks
                       WHERE owner_character_id = ?2 AND target_character_id = ?1)
             OR EXISTS(SELECT 1 FROM social_blocks
                       WHERE owner_character_id = ?1 AND target_character_id = ?2)",
        params![sender.0, recipient.0],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(!suppressed)
}

pub(super) fn set_social_relation(
    connection: &mut Connection,
    actor: CharacterId,
    target: CharacterId,
    relation: SocialRelationKind,
    enabled: bool,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let transaction = connection.transaction()?;
    let actor_profile = social_profile(&transaction, actor, now)?;
    let target_profile = social_profile(&transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let table = match relation {
        SocialRelationKind::Ignore => "social_ignores",
        SocialRelationKind::Block => "social_blocks",
    };
    let sql = if enabled {
        format!(
            "INSERT INTO {table} (owner_character_id, target_character_id, created_at)
             VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING"
        )
    } else {
        format!("DELETE FROM {table} WHERE owner_character_id = ?1 AND target_character_id = ?2")
    };
    if enabled {
        transaction.execute(&sql, params![actor.0, target.0, now])?;
    } else {
        transaction.execute(&sql, params![actor.0, target.0])?;
    }
    let action = match (relation, enabled) {
        (SocialRelationKind::Ignore, true) => "ignore",
        (SocialRelationKind::Ignore, false) => "unignore",
        (SocialRelationKind::Block, true) => "block",
        (SocialRelationKind::Block, false) => "unblock",
    };
    append_audit(
        &transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action,
        serde_json::json!({"targetCharacterId": target.0}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn social_channel_subscriptions(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<Vec<SocialChannelName>, StoreError> {
    ensure_social_character_exists(connection, character_id)?;
    let mut statement = connection.prepare(
        "SELECT channel FROM social_channel_subscriptions
         WHERE character_id = ?1
         ORDER BY CASE channel
             WHEN 'general' THEN 0
             WHEN 'newbie' THEN 1
             WHEN 'staff' THEN 2
             ELSE 3
         END",
    )?;
    let stored = statement
        .query_map([character_id.0], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    stored
        .iter()
        .map(|channel| social_channel_from_database(channel))
        .collect()
}

pub(super) fn social_channel_subscribers(
    connection: &Connection,
    channel: SocialChannelName,
) -> Result<Vec<CharacterId>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT character_id FROM social_channel_subscriptions
         WHERE channel = ?1 ORDER BY character_id",
    )?;
    statement
        .query_map([channel.as_str()], |row| {
            row.get::<_, i64>(0).map(CharacterId)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

pub(super) fn set_social_channel_subscription(
    connection: &mut Connection,
    character_id: CharacterId,
    channel: SocialChannelName,
    subscribed: bool,
    now: i64,
) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    ensure_social_character_exists(&transaction, character_id)?;
    if subscribed {
        transaction.execute(
            "INSERT INTO social_channel_subscriptions
                 (character_id, channel, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(character_id, channel) DO NOTHING",
            params![character_id.0, channel.as_str(), now],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM social_channel_subscriptions
             WHERE character_id = ?1 AND channel = ?2",
            params![character_id.0, channel.as_str()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn consume_mail_usage(
    connection: &Connection,
    account_id: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    let stored = connection
        .query_row(
            "SELECT hour_started_at, hour_count, day_started_at, day_count
             FROM social_mail_usage WHERE account_id = ?1",
            [account_id.0],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((stored_hour_start, stored_hour_count, stored_day_start, stored_day_count)) = stored
    else {
        connection.execute(
            "INSERT INTO social_mail_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (?1, ?2, 1, ?2, 1, ?2)",
            params![account_id.0, now],
        )?;
        return Ok(());
    };
    let window = QuotaWindow {
        hour_started_at: stored_hour_start,
        hour_count: stored_hour_count,
        day_started_at: stored_day_start,
        day_count: stored_day_count,
    }
    .rolled(now);
    if window.exhausted(MAIL_HOURLY_CAPACITY, MAIL_DAILY_CAPACITY) {
        return Err(StoreError::MailRateLimited);
    }
    connection.execute(
        "UPDATE social_mail_usage
         SET hour_started_at = ?1, hour_count = ?2,
             day_started_at = ?3, day_count = ?4, updated_at = ?5
         WHERE account_id = ?6",
        params![
            window.hour_started_at,
            window.hour_count.saturating_add(1),
            window.day_started_at,
            window.day_count.saturating_add(1),
            now,
            account_id.0
        ],
    )?;
    Ok(())
}

pub(super) fn send_mail(
    connection: &mut Connection,
    sender: CharacterId,
    target_name: &str,
    public_id: &str,
    subject: &str,
    body: &str,
    now: i64,
) -> Result<MailSendResult, StoreError> {
    let target_name = normalise_mail_text(target_name, MAX_IDENTITY_NAME_BYTES)?;
    let subject = normalise_mail_text(subject, MAX_MAIL_SUBJECT_BYTES)?;
    let body = normalise_mail_text(body, MAX_MAIL_BODY_BYTES)?;
    if !valid_mail_target(&target_name) || !valid_mail_subject(&subject) {
        return Err(StoreError::InvalidMailInput);
    }
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let transaction = connection.transaction()?;
    let (sender_account_id, sender_name, sender_muted) = transaction
        .query_row(
            "SELECT characters.account_id, characters.name,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = characters.account_id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > ?2)
                    )
             FROM characters WHERE characters.id = ?1",
            params![sender.0, now],
            |row| {
                Ok((
                    AccountId(row.get(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::SocialTargetNotFound)?;
    consume_mail_usage(&transaction, sender_account_id, now)?;

    if sender_muted {
        transaction.commit()?;
        return Err(StoreError::ChatMuted);
    }
    if !valid_mail_visible_text(&sender_name, MAX_IDENTITY_NAME_BYTES) {
        transaction.commit()?;
        return Ok(MailSendResult::Suppressed);
    }

    let duplicate_public_id = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_mail_messages WHERE public_id = ?1)",
        [public_id],
        |row| row.get::<_, bool>(0),
    )?;
    let result = if duplicate_public_id {
        MailSendResult::Unavailable
    } else {
        let mut statement = transaction.prepare(
            "SELECT id, account_id FROM characters
             WHERE name = ?1 COLLATE BINARY ORDER BY id LIMIT 2",
        )?;
        let targets = statement
            .query_map([target_name], |row| {
                Ok((CharacterId(row.get(0)?), AccountId(row.get(1)?)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        match targets.as_slice() {
            [(recipient, recipient_account)] if *recipient != sender => {
                let suppressed_by_policy = transaction.query_row(
                    "SELECT
                         EXISTS(SELECT 1 FROM account_bans
                                WHERE account_id = ?3 AND revoked_at IS NULL
                                  AND (expires_at IS NULL OR expires_at > ?4))
                         OR EXISTS(SELECT 1 FROM social_ignores
                                   WHERE owner_character_id = ?2
                                     AND target_character_id = ?1)
                         OR EXISTS(SELECT 1 FROM social_blocks
                                   WHERE owner_character_id = ?2
                                     AND target_character_id = ?1)
                         OR EXISTS(SELECT 1 FROM social_blocks
                                   WHERE owner_character_id = ?1
                                     AND target_character_id = ?2)",
                    params![sender.0, recipient.0, recipient_account.0, now],
                    |row| row.get::<_, bool>(0),
                )?;
                if suppressed_by_policy {
                    MailSendResult::Suppressed
                } else {
                    let (mailbox_count, mailbox_bytes) = transaction.query_row(
                        "SELECT COUNT(*),
                                COALESCE(SUM(length(CAST(subject AS BLOB))
                                           + length(CAST(body AS BLOB))), 0)
                         FROM social_mail_messages
                         WHERE recipient_character_id = ?1 AND expires_at > ?2",
                        params![recipient.0, now],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )?;
                    let global_count = transaction.query_row(
                        "SELECT COUNT(*) FROM social_mail_messages WHERE expires_at > ?1",
                        [now],
                        |row| row.get::<_, i64>(0),
                    )?;
                    let requested_bytes =
                        i64::try_from(subject.len().saturating_add(body.len())).unwrap_or(i64::MAX);
                    if mail_delivery_exceeds_capacity(
                        mailbox_count,
                        mailbox_bytes,
                        requested_bytes,
                        global_count,
                    ) {
                        MailSendResult::Suppressed
                    } else {
                        transaction.execute(
                            "INSERT INTO social_mail_messages
                                 (public_id, sender_character_id, sender_name,
                                  recipient_character_id, subject, body,
                                  created_at, expires_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            params![
                                public_id,
                                sender.0,
                                sender_name,
                                recipient.0,
                                subject,
                                body,
                                now,
                                now.saturating_add(MAIL_RETENTION_SECONDS)
                            ],
                        )?;
                        MailSendResult::Stored
                    }
                }
            }
            _ => MailSendResult::Unavailable,
        }
    };
    transaction.commit()?;
    Ok(result)
}

pub(super) fn mailbox_page(
    connection: &Connection,
    recipient: CharacterId,
    page: u32,
    now: i64,
) -> Result<MailPage, StoreError> {
    if !(1..=MAIL_MAX_PAGES).contains(&page) {
        return Err(StoreError::InvalidMailInput);
    }
    ensure_social_character_exists(connection, recipient)?;
    let (total, unread) = connection.query_row(
        "SELECT COUNT(*), SUM(CASE WHEN read_at IS NULL THEN 1 ELSE 0 END)
         FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND expires_at > ?2",
        params![recipient.0, now],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            ))
        },
    )?;
    let total_messages = u32::try_from(total).map_err(|_| StoreError::InvalidMailInput)?;
    let unread_count = u32::try_from(unread).map_err(|_| StoreError::InvalidMailInput)?;
    let total_pages = total_messages.div_ceil(MAIL_PAGE_SIZE).max(1);
    if total_pages > MAIL_MAX_PAGES {
        return Err(StoreError::InvalidMailInput);
    }
    let offset = i64::from((page - 1).saturating_mul(MAIL_PAGE_SIZE));
    let mut statement = connection.prepare(
        "SELECT public_id, sender_name, subject, created_at, expires_at,
                read_at IS NOT NULL
         FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND expires_at > ?2
         ORDER BY created_at DESC, id DESC
         LIMIT ?3 OFFSET ?4",
    )?;
    let messages = statement
        .query_map(
            params![recipient.0, now, i64::from(MAIL_PAGE_SIZE), offset],
            |row| {
                Ok(MailMessageSummary {
                    public_id: row.get(0)?,
                    sender_name: row.get(1)?,
                    subject: row.get(2)?,
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                    read: row.get(5)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(MailPage {
        page,
        total_pages,
        total_messages,
        unread_count,
        capacity: MAILBOX_CAPACITY,
        messages,
    })
}

pub(super) fn read_mail(
    connection: &mut Connection,
    recipient: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<MailMessage, StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let transaction = connection.transaction()?;
    ensure_social_character_exists(&transaction, recipient)?;
    let updated = transaction.execute(
        "UPDATE social_mail_messages SET read_at = COALESCE(read_at, ?1)
         WHERE recipient_character_id = ?2 AND public_id = ?3 AND expires_at > ?1",
        params![now, recipient.0, public_id],
    )?;
    if updated != 1 {
        return Err(StoreError::MailNotFound);
    }
    let message = transaction.query_row(
        "SELECT public_id, sender_name, subject, body, created_at, expires_at,
                read_at IS NOT NULL
         FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND public_id = ?2 AND expires_at > ?3",
        params![recipient.0, public_id, now],
        |row| {
            Ok(MailMessage {
                summary: MailMessageSummary {
                    public_id: row.get(0)?,
                    sender_name: row.get(1)?,
                    subject: row.get(2)?,
                    created_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    read: row.get(6)?,
                },
                body: row.get(3)?,
            })
        },
    )?;
    transaction.commit()?;
    Ok(message)
}

pub(super) fn delete_mail(
    connection: &mut Connection,
    recipient: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let transaction = connection.transaction()?;
    ensure_social_character_exists(&transaction, recipient)?;
    let deleted = transaction.execute(
        "DELETE FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND public_id = ?2 AND expires_at > ?3",
        params![recipient.0, public_id, now],
    )?;
    if deleted != 1 {
        return Err(StoreError::MailNotFound);
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn report_mail(
    connection: &mut Connection,
    recipient: CharacterId,
    public_id: &str,
    reason: &str,
    now: i64,
) -> Result<i64, StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let reason = normalise_mail_text(reason, MAX_REPORT_REASON_BYTES)?;
    let transaction = connection.transaction()?;
    let reporter = social_profile(&transaction, recipient, now)?;
    let message = transaction
        .query_row(
            "SELECT sender_character_id, sender_name, subject, body,
                    created_at, report_id
             FROM social_mail_messages
             WHERE recipient_character_id = ?1 AND public_id = ?2 AND expires_at > ?3",
            params![recipient.0, public_id, now],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?.map(CharacterId),
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::MailNotFound)?;
    if message.5.is_some() {
        return Err(StoreError::MailAlreadyReported);
    }
    let sender = message.0.ok_or(StoreError::MailNotFound)?;
    let sender_profile = social_profile(&transaction, sender, now)?;
    let open_reports = transaction.query_row(
        "SELECT COUNT(*)
         FROM social_reports AS reports
         JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
         WHERE reporter.account_id = ?1
           AND reports.report_kind = 'mail'
           AND reports.status = 'open'",
        [reporter.account_id.0],
        |row| row.get::<_, i64>(0),
    )?;
    check_mail_report_quota(open_reports)?;
    let evidence_count = transaction.query_row(
        "SELECT COUNT(*) FROM social_mail_report_evidence WHERE expires_at > ?1",
        [now],
        |row| row.get::<_, i64>(0),
    )?;
    check_mail_evidence_capacity(evidence_count)?;
    transaction.execute(
        "INSERT INTO social_reports
             (reporter_character_id, target_character_id, report_kind, reason, created_at)
         VALUES (?1, ?2, 'mail', ?3, ?4)",
        params![recipient.0, sender.0, reason, now],
    )?;
    let report_id = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO social_mail_report_evidence
             (report_id, sender_name, subject, body, sent_at, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            report_id,
            message.1,
            message.2,
            message.3,
            message.4,
            now,
            now.saturating_add(MAIL_REPORT_EVIDENCE_RETENTION_SECONDS)
        ],
    )?;
    let linked = transaction.execute(
        "UPDATE social_mail_messages SET report_id = ?1
         WHERE recipient_character_id = ?2 AND public_id = ?3
           AND report_id IS NULL AND expires_at > ?4",
        params![report_id, recipient.0, public_id, now],
    )?;
    if linked != 1 {
        return Err(StoreError::MailAlreadyReported);
    }
    append_audit(
        &transaction,
        reporter.account_id,
        Some(sender_profile.account_id),
        "mail_report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(report_id)
}

pub(super) fn purge_expired_mail(connection: &mut Connection, now: i64) -> Result<u64, StoreError> {
    let transaction = connection.transaction()?;
    let messages = transaction.execute(
        "DELETE FROM social_mail_messages
         WHERE id IN (
             SELECT id FROM social_mail_messages
             WHERE expires_at <= ?1 ORDER BY expires_at, id LIMIT ?2
         )",
        params![now, MAIL_PURGE_LIMIT],
    )?;
    let remaining = purge_remaining(MAIL_PURGE_LIMIT, messages);
    let evidence = if remaining > 0 {
        transaction.execute(
            "DELETE FROM social_mail_report_evidence
             WHERE report_id IN (
                 SELECT report_id FROM social_mail_report_evidence
                 WHERE expires_at <= ?1 ORDER BY expires_at, report_id LIMIT ?2
             )",
            params![now, remaining],
        )?
    } else {
        0
    };
    transaction.commit()?;
    u64::try_from(messages.saturating_add(evidence)).map_err(|_| StoreError::InvalidMailInput)
}

pub(super) fn consume_board_usage(
    connection: &Connection,
    account_id: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    let stored = connection
        .query_row(
            "SELECT hour_started_at, hour_count, day_started_at, day_count
             FROM social_board_usage WHERE account_id = ?1",
            [account_id.0],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((stored_hour_start, stored_hour_count, stored_day_start, stored_day_count)) = stored
    else {
        connection.execute(
            "INSERT INTO social_board_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (?1, ?2, 1, ?2, 1, ?2)",
            params![account_id.0, now],
        )?;
        return Ok(());
    };
    let window = QuotaWindow {
        hour_started_at: stored_hour_start,
        hour_count: stored_hour_count,
        day_started_at: stored_day_start,
        day_count: stored_day_count,
    }
    .rolled(now);
    if window.exhausted(BOARD_HOURLY_CAPACITY, BOARD_DAILY_CAPACITY) {
        return Err(StoreError::BoardRateLimited);
    }
    connection.execute(
        "UPDATE social_board_usage
         SET hour_started_at = ?1, hour_count = ?2,
             day_started_at = ?3, day_count = ?4, updated_at = ?5
         WHERE account_id = ?6",
        params![
            window.hour_started_at,
            window.hour_count.saturating_add(1),
            window.day_started_at,
            window.day_count.saturating_add(1),
            now,
            account_id.0
        ],
    )?;
    Ok(())
}

pub(super) fn list_board_posts(
    connection: &Connection,
    actor: CharacterId,
    board: BoardKey,
    page: u32,
    now: i64,
) -> Result<BoardPage, StoreError> {
    if !(1..=BOARD_MAX_PAGES).contains(&page) {
        return Err(StoreError::InvalidBoardInput);
    }
    let actor_profile = social_profile(connection, actor, now)?;
    let total = connection.query_row(
        "SELECT COUNT(*) FROM social_board_posts
         WHERE board_key = ?1 AND expires_at > ?2",
        params![board.as_str(), now],
        |row| row.get::<_, i64>(0),
    )?;
    let total_posts = u32::try_from(total).map_err(|_| StoreError::InvalidBoardInput)?;
    let total_pages = total_posts.div_ceil(BOARD_PAGE_SIZE).max(1);
    if total_pages > BOARD_MAX_PAGES {
        return Err(StoreError::InvalidBoardInput);
    }
    let offset = i64::from((page - 1).saturating_mul(BOARD_PAGE_SIZE));
    let mut statement = connection.prepare(
        "SELECT posts.public_id, posts.author_character_id, authors.account_id,
                posts.author_name, posts.subject, posts.created_at, posts.expires_at,
                posts.report_id
         FROM social_board_posts AS posts
         JOIN characters AS authors ON authors.id = posts.author_character_id
         WHERE posts.board_key = ?1 AND posts.expires_at > ?2
         ORDER BY posts.created_at DESC, posts.id DESC
         LIMIT ?3 OFFSET ?4",
    )?;
    let posts = statement
        .query_map(
            params![board.as_str(), now, i64::from(BOARD_PAGE_SIZE), offset],
            |row| {
                let author_character = CharacterId(row.get(1)?);
                let author_account = AccountId(row.get(2)?);
                let report_id = row.get::<_, Option<i64>>(7)?;
                Ok(BoardPostSummary {
                    public_id: row.get(0)?,
                    board,
                    author_name: row.get(3)?,
                    subject: row.get(4)?,
                    created_at: row.get(5)?,
                    expires_at: row.get(6)?,
                    can_delete: author_character == actor,
                    can_report: author_account != actor_profile.account_id && report_id.is_none(),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(BoardPage {
        board,
        page,
        total_pages,
        total_posts,
        capacity: BOARD_CAPACITY_PER_BOARD,
        posts,
    })
}

pub(super) fn read_board_post(
    connection: &Connection,
    actor: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<BoardEntry, StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let actor_profile = social_profile(connection, actor, now)?;
    connection
        .query_row(
            "SELECT posts.board_key, posts.author_character_id, authors.account_id,
                    posts.author_name, posts.subject, posts.body,
                    posts.created_at, posts.expires_at, posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.public_id = ?1 AND posts.expires_at > ?2",
            params![public_id, now],
            |row| {
                let board_key: String = row.get(0)?;
                let board = BoardKey::parse(&board_key).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        Type::Text,
                        Box::new(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid board key",
                        )),
                    )
                })?;
                let author_character = CharacterId(row.get(1)?);
                let author_account = AccountId(row.get(2)?);
                let report_id = row.get::<_, Option<i64>>(8)?;
                Ok(BoardEntry {
                    summary: BoardPostSummary {
                        public_id: public_id.to_owned(),
                        board,
                        author_name: row.get(3)?,
                        subject: row.get(4)?,
                        created_at: row.get(6)?,
                        expires_at: row.get(7)?,
                        can_delete: author_character == actor,
                        can_report: author_account != actor_profile.account_id
                            && report_id.is_none(),
                    },
                    body: row.get(5)?,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::BoardPostUnavailable)
}

pub(super) fn publish_board_post(
    connection: &mut Connection,
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
    let subject = normalise_board_text(subject, tiamud_core::MAX_BOARD_SUBJECT_BYTES)?;
    let body = normalise_board_text(body, tiamud_core::MAX_BOARD_BODY_BYTES)?;
    let expires_at = now
        .checked_add(BOARD_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidBoardInput)?;
    let transaction = connection.transaction()?;
    let profile = social_profile(&transaction, actor, now)?;
    if profile.muted {
        return Err(StoreError::ChatMuted);
    }
    if account_is_banned(&transaction, profile.account_id, now)? {
        return Err(StoreError::AccountBanned);
    }
    consume_board_usage(&transaction, profile.account_id, now)?;
    let duplicate = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_board_posts WHERE public_id = ?1)",
        [public_id],
        |row| row.get::<_, bool>(0),
    )?;
    if duplicate {
        return Err(StoreError::InvalidBoardInput);
    }
    let account_count = transaction.query_row(
        "SELECT COUNT(*)
         FROM social_board_posts AS posts
         JOIN characters AS authors ON authors.id = posts.author_character_id
         WHERE authors.account_id = ?1 AND posts.expires_at > ?2",
        params![profile.account_id.0, now],
        |row| row.get::<_, i64>(0),
    )?;
    let (board_count, board_bytes) = transaction.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(length(CAST(subject AS BLOB))
                             + length(CAST(body AS BLOB))), 0)
         FROM social_board_posts WHERE board_key = ?1 AND expires_at > ?2",
        params![board.as_str(), now],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let (global_count, global_bytes) = transaction.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(length(CAST(subject AS BLOB))
                             + length(CAST(body AS BLOB))), 0)
         FROM social_board_posts WHERE expires_at > ?1",
        [now],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let added_bytes = i64::try_from(subject.len().saturating_add(body.len()))
        .map_err(|_| StoreError::InvalidBoardInput)?;
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
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            public_id,
            board.as_str(),
            actor.0,
            profile.character_name,
            subject,
            body,
            now,
            expires_at
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

pub(super) fn delete_board_post(
    connection: &mut Connection,
    actor: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let transaction = connection.transaction()?;
    social_profile(&transaction, actor, now)?;
    let deleted = transaction.execute(
        "DELETE FROM social_board_posts
         WHERE public_id = ?1 AND author_character_id = ?2 AND expires_at > ?3",
        params![public_id, actor.0, now],
    )?;
    if deleted != 1 {
        return Err(StoreError::BoardPostUnavailable);
    }
    transaction.commit()?;
    Ok(())
}

pub(super) fn report_board_post(
    connection: &mut Connection,
    actor: CharacterId,
    public_id: &str,
    reason: &str,
    now: i64,
) -> Result<i64, StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let reason = normalise_board_text(reason, tiamud_core::MAX_BOARD_REPORT_REASON_BYTES)?;
    let evidence_expires_at = now
        .checked_add(BOARD_REPORT_EVIDENCE_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidBoardInput)?;
    let transaction = connection.transaction()?;
    let reporter = social_profile(&transaction, actor, now)?;
    let post = transaction
        .query_row(
            "SELECT posts.author_character_id, authors.account_id, posts.board_key,
                    posts.author_name, posts.subject, posts.body,
                    posts.created_at, posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.public_id = ?1 AND posts.expires_at > ?2",
            params![public_id, now],
            |row| {
                Ok((
                    CharacterId(row.get(0)?),
                    AccountId(row.get(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::BoardPostUnavailable)?;
    if post.7.is_some() {
        return Err(StoreError::BoardAlreadyReported);
    }
    if post.1 == reporter.account_id {
        return Err(StoreError::BoardPostUnavailable);
    }
    let open_reports = transaction.query_row(
        "SELECT COUNT(*)
         FROM social_reports AS reports
         JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
         WHERE reporter.account_id = ?1 AND reports.report_kind = 'board'
           AND reports.status = 'open'",
        [reporter.account_id.0],
        |row| row.get::<_, i64>(0),
    )?;
    check_board_report_capacity(open_reports, 0)?;
    let evidence_count = transaction.query_row(
        "SELECT COUNT(*) FROM social_board_report_evidence WHERE expires_at > ?1",
        [now],
        |row| row.get::<_, i64>(0),
    )?;
    check_board_report_capacity(open_reports, evidence_count)?;
    transaction.execute(
        "INSERT INTO social_reports
             (reporter_character_id, target_character_id, report_kind, reason, created_at)
         VALUES (?1, ?2, 'board', ?3, ?4)",
        params![actor.0, post.0.0, reason, now],
    )?;
    let report_id = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO social_board_report_evidence
             (report_id, board_key, post_public_id, author_name,
              subject, body, posted_at, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            report_id,
            post.2,
            public_id,
            post.3,
            post.4,
            post.5,
            post.6,
            now,
            evidence_expires_at
        ],
    )?;
    let linked = transaction.execute(
        "UPDATE social_board_posts SET report_id = ?1
         WHERE public_id = ?2 AND report_id IS NULL AND expires_at > ?3",
        params![report_id, public_id, now],
    )?;
    if linked != 1 {
        return Err(StoreError::BoardAlreadyReported);
    }
    append_audit(
        &transaction,
        reporter.account_id,
        Some(post.1),
        "board_report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(report_id)
}

pub(super) fn purge_expired_boards(
    connection: &mut Connection,
    now: i64,
) -> Result<u64, StoreError> {
    let transaction = connection.transaction()?;
    let posts = transaction.execute(
        "DELETE FROM social_board_posts
         WHERE id IN (
             SELECT id FROM social_board_posts
             WHERE expires_at <= ?1 ORDER BY expires_at, id LIMIT ?2
         )",
        params![now, BOARD_PURGE_LIMIT],
    )?;
    let remaining = purge_remaining(BOARD_PURGE_LIMIT, posts);
    let evidence = if remaining > 0 {
        transaction.execute(
            "DELETE FROM social_board_report_evidence
             WHERE report_id IN (
                 SELECT report_id FROM social_board_report_evidence
                 WHERE expires_at <= ?1 ORDER BY expires_at, report_id LIMIT ?2
             )",
            params![now, remaining],
        )?
    } else {
        0
    };
    transaction.commit()?;
    u64::try_from(posts.saturating_add(evidence)).map_err(|_| StoreError::InvalidBoardInput)
}

pub(super) fn report_character(
    connection: &mut Connection,
    actor: CharacterId,
    target: CharacterId,
    reason: &str,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let reason = normalise_social_text(reason, MAX_REPORT_REASON_BYTES)?;
    let transaction = connection.transaction()?;
    let actor_profile = social_profile(&transaction, actor, now)?;
    let target_profile = social_profile(&transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    transaction.execute(
        "INSERT INTO social_reports
             (reporter_character_id, target_character_id, reason, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![actor.0, target.0, reason, now],
    )?;
    append_audit(
        &transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        "report",
        serde_json::json!({"reportId": transaction.last_insert_rowid()}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn staff_reports(
    connection: &mut Connection,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<Vec<StaffReport>, StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::ReportsView),
        None,
    )?;
    let actor_rank = actor
        .profile
        .ok_or(StoreError::StaffForbidden)?
        .authority_rank();
    insert_staff_audit(&transaction, &audit, now)?;
    let reports = {
        let mut statement = transaction.prepare(
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
             LEFT JOIN social_mail_report_evidence AS evidence
                    ON evidence.report_id = reports.id AND evidence.expires_at > ?3
             LEFT JOIN social_board_report_evidence AS board_evidence
                    ON board_evidence.report_id = reports.id
                   AND board_evidence.expires_at > ?3
             LEFT JOIN staff_profiles AS target_staff
                    ON target_staff.account_id = target.account_id
             WHERE reports.status = 'open'
               AND CASE target_staff.profile_key
                       WHEN 'helper' THEN 10
                       WHEN 'builder' THEN 20
                       WHEN 'moderator' THEN 30
                       WHEN 'administrator' THEN 40
                       WHEN 'implementor' THEN 50
                       ELSE 0
                   END < ?1
             ORDER BY reports.created_at, reports.id
             LIMIT ?2",
        )?;
        statement
            .query_map(
                params![i64::from(actor_rank), STAFF_LIST_LIMIT, now],
                |row| {
                    let evidence_sender = row.get::<_, Option<String>>(8)?;
                    let board_key = row.get::<_, Option<String>>(14)?;
                    Ok(StaffReport {
                        id: row.get(0)?,
                        reporter_character_id: CharacterId(row.get(1)?),
                        reporter_name: row.get(2)?,
                        target_character_id: CharacterId(row.get(3)?),
                        target_account_id: AccountId(row.get(4)?),
                        target_name: row.get(5)?,
                        reason: row.get(6)?,
                        created_at: row.get(7)?,
                        mail_evidence: match evidence_sender {
                            Some(sender_name) => Some(MailReportEvidence {
                                sender_name,
                                subject: row.get(9)?,
                                body: row.get(10)?,
                                sent_at: row.get(11)?,
                                created_at: row.get(12)?,
                                expires_at: row.get(13)?,
                            }),
                            None => None,
                        },
                        board_evidence: match board_key {
                            Some(board_key) => Some(BoardReportEvidence {
                                board: BoardKey::parse(&board_key).ok_or_else(|| {
                                    rusqlite::Error::FromSqlConversionFailure(
                                        14,
                                        Type::Text,
                                        Box::new(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            "invalid board key",
                                        )),
                                    )
                                })?,
                                post_public_id: row.get(15)?,
                                author_name: row.get(16)?,
                                subject: row.get(17)?,
                                body: row.get(18)?,
                                posted_at: row.get(19)?,
                                created_at: row.get(20)?,
                                expires_at: row.get(21)?,
                            }),
                            None => None,
                        },
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    transaction.commit()?;
    Ok(reports)
}

pub(super) fn resolve_staff_report(
    connection: &mut Connection,
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
    let transaction = connection.transaction()?;
    let target_account = transaction
        .query_row(
            "SELECT target.account_id
             FROM social_reports AS reports
             JOIN characters AS target ON target.id = reports.target_character_id
             WHERE reports.id = ?1 AND reports.status = 'open'",
            [report_id],
            |row| row.get::<_, i64>(0).map(AccountId),
        )
        .optional()?
        .ok_or(StoreError::StaffReportUnavailable(report_id))?;
    bind_report_audit_target(&mut audit, target_account)?;
    validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::ReportsResolve),
        Some(target_account),
    )?;
    let updated = transaction.execute(
        "UPDATE social_reports
         SET status = 'resolved', resolution = ?1, resolution_reason = ?2,
             resolved_by_account_id = ?3, resolved_at = ?4
         WHERE id = ?5 AND status = 'open'",
        params![
            resolution.as_str(),
            resolution_reason,
            audit.actor_account_id.0,
            now,
            report_id
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::StaffReportUnavailable(report_id));
    }
    if resolution == StaffReportResolution::ActionTaken {
        transaction.execute(
            "DELETE FROM social_mail_messages
             WHERE report_id = ?1 AND expires_at > ?2",
            params![report_id, now],
        )?;
        transaction.execute(
            "DELETE FROM social_board_posts
             WHERE report_id = ?1 AND expires_at > ?2",
            params![report_id, now],
        )?;
    }
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn set_staff_profile(
    connection: &mut Connection,
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
    let transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::StaffAssign),
        Some(target_account),
    )?;
    authorize_profile_assignment(&actor, profile)?;
    transaction.execute(
        "DELETE FROM staff_capability_assignments WHERE account_id = ?1",
        [target_account.0],
    )?;
    if let Some(profile) = profile {
        transaction.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(account_id) DO UPDATE SET
                 profile_key = excluded.profile_key,
                 lore_title = excluded.lore_title,
                 assigned_by_account_id = excluded.assigned_by_account_id,
                 updated_at = excluded.updated_at",
            params![
                target_account.0,
                profile.as_str(),
                lore_title,
                audit.actor_account_id.0,
                now
            ],
        )?;
        for capability in profile.default_capabilities() {
            transaction.execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    target_account.0,
                    capability.as_str(),
                    audit.actor_account_id.0,
                    now
                ],
            )?;
        }
    } else {
        transaction.execute(
            "DELETE FROM staff_profiles WHERE account_id = ?1",
            [target_account.0],
        )?;
    }
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn set_staff_capability(
    connection: &mut Connection,
    target_account: AccountId,
    capability: StaffCapability,
    enabled: bool,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::StaffAssign),
        Some(target_account),
    )?;
    let target = staff_access_by_account(&transaction, target_account)?;
    authorize_capability_change(&actor, &target, capability, enabled)?;
    if enabled {
        transaction.execute(
            "INSERT INTO staff_capability_assignments
                 (account_id, capability, assigned_by_account_id, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, capability) DO UPDATE SET
                 assigned_by_account_id = excluded.assigned_by_account_id,
                 created_at = excluded.created_at",
            params![
                target_account.0,
                capability.as_str(),
                audit.actor_account_id.0,
                now
            ],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM staff_capability_assignments
             WHERE account_id = ?1 AND capability = ?2",
            params![target_account.0, capability.as_str()],
        )?;
    }
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn staff_audit_entries(
    connection: &mut Connection,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<Vec<StaffAuditEntry>, StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::StaffAudit),
        None,
    )?;
    insert_staff_audit(&transaction, &audit, now)?;
    let entries = {
        let mut statement = transaction.prepare(
            "SELECT sequence, actor_account_id, target_account_id, capability,
                    action, reason, details_json, created_at
             FROM staff_audit ORDER BY sequence DESC LIMIT ?1",
        )?;
        let raw = statement
            .query_map([STAFF_LIST_LIMIT], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raw.into_iter()
            .map(|row| {
                Ok(StaffAuditEntry {
                    sequence: row.0,
                    actor_account_id: AccountId(row.1),
                    target_account_id: row.2.map(AccountId),
                    capability: StaffCapability::parse(&row.3).ok_or_else(|| {
                        StoreError::InvalidStaffState(format!("capability {:?}", row.3))
                    })?,
                    action: row.4,
                    reason: row.5,
                    details_json: row.6,
                    created_at: row.7,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?
    };
    transaction.commit()?;
    Ok(entries)
}

pub(super) fn append_staff_audit_transaction(
    connection: &mut Connection,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    validate_staff_authority(&transaction, &audit, None, audit.target_account_id)?;
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn moderate_character(
    connection: &mut Connection,
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
    let transaction = connection.transaction()?;
    let actor_profile = social_profile(&transaction, actor, now)?;
    let target_profile = social_profile(&transaction, target, now)?;
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
        &transaction,
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
                &transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO moderation_mutes
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    target_profile.account_id.0,
                    actor_profile.account_id.0,
                    &reason,
                    now,
                    expires_at
                ],
            )?;
        }
        ModerationAction::Unmute { .. } => {
            revoke_active(
                &transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
        }
        ModerationAction::Ban { expires_at, .. } => {
            revoke_active(
                &transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO account_bans
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    target_profile.account_id.0,
                    actor_profile.account_id.0,
                    &reason,
                    now,
                    expires_at
                ],
            )?;
            transaction.execute(
                "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, ?1)
                 WHERE account_id = ?2",
                params![now, target_profile.account_id.0],
            )?;
        }
        ModerationAction::Unban { .. } => {
            revoke_active(
                &transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
        }
        ModerationAction::SetRole { role, .. } => {
            transaction.execute(
                "UPDATE accounts SET role = ?1 WHERE id = ?2",
                params![role_as_database(role), target_profile.account_id.0],
            )?;
        }
    }
    append_audit(
        &transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action_name,
        details,
        now,
    )?;
    insert_staff_audit(&transaction, &staff_audit, now)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn revoke_active(
    connection: &Connection,
    table: &str,
    target: AccountId,
    actor: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    debug_assert!(matches!(table, "moderation_mutes" | "account_bans"));
    connection.execute(
        &format!(
            "UPDATE {table}
             SET revoked_at = ?1, revoked_by_account_id = ?2
             WHERE account_id = ?3 AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > ?1)"
        ),
        params![now, actor.0, target.0],
    )?;
    Ok(())
}

pub(super) fn append_audit(
    connection: &Connection,
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
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            actor.0,
            target.map(|target| target.0),
            action,
            details_json,
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
pub(super) fn audit_log(connection: &Connection) -> Result<Vec<AuditEntry>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT sequence, actor_account_id, target_account_id, action, details_json, created_at
         FROM moderation_audit ORDER BY sequence",
    )?;
    statement
        .query_map([], |row| {
            Ok(AuditEntry {
                sequence: row.get(0)?,
                actor_account_id: AccountId(row.get(1)?),
                target_account_id: row.get::<_, Option<i64>>(2)?.map(AccountId),
                action: row.get(3)?,
                details_json: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

#[cfg(test)]
pub(super) fn load_or_create_account(
    connection: &Connection,
    name: &str,
) -> Result<AccountSnapshot, StoreError> {
    let name = normalise_name(name, "account")?;
    connection.execute(
        "INSERT INTO accounts (name, auth_state)
         VALUES (?1, 'guest') ON CONFLICT(name) DO NOTHING",
        [&name],
    )?;
    connection
        .query_row(
            "SELECT id, name FROM accounts
             WHERE name = ?1 AND auth_state = 'guest'",
            [&name],
            |row| {
                Ok(AccountSnapshot {
                    id: AccountId(row.get(0)?),
                    name: row.get(1)?,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::AccountAlreadyExists)
}

pub(super) fn list_characters(
    connection: &Connection,
    account_id: AccountId,
) -> Result<Vec<CharacterSnapshot>, StoreError> {
    ensure_account_exists(connection, account_id)?;
    let mut statement = connection.prepare(
        "SELECT id, account_id, name, room_id, item_locations_json
         FROM characters WHERE account_id = ?1 ORDER BY name COLLATE NOCASE, id",
    )?;
    let snapshots = statement
        .query_map([account_id.0], character_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(snapshots)
}

#[cfg(test)]
pub(super) fn load_or_create_character(
    connection: &Connection,
    account_id: AccountId,
    name: &str,
    start_room: &str,
) -> Result<CharacterSnapshot, StoreError> {
    load_or_create_character_with_gameplay(
        connection,
        account_id,
        name,
        start_room,
        &CharacterGameplaySnapshot::default(),
    )
}

pub(super) fn load_or_create_character_with_gameplay(
    connection: &Connection,
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
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, name) DO NOTHING",
        params![account_id.0, name, start_room, gameplay_json],
    )?;
    connection
        .query_row(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = ?1 AND name = ?2",
            params![account_id.0, name],
            character_from_row,
        )
        .map_err(StoreError::from)
}

pub(super) fn load_character(
    connection: &Connection,
    account_id: AccountId,
    character_id: CharacterId,
) -> Result<CharacterSnapshot, StoreError> {
    ensure_account_exists(connection, account_id)?;
    connection
        .query_row(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = ?1 AND id = ?2",
            params![account_id.0, character_id.0],
            character_from_row,
        )
        .optional()?
        .ok_or(StoreError::CharacterNotFound {
            account_id,
            character_id,
        })
}

pub(super) fn character_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CharacterSnapshot> {
    let item_locations_json = row.get::<_, String>(4)?;
    let item_locations = serde_json::from_str(&item_locations_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
    })?;
    Ok(CharacterSnapshot {
        id: CharacterId(row.get(0)?),
        account_id: AccountId(row.get(1)?),
        name: row.get(2)?,
        room_id: row.get(3)?,
        item_locations,
    })
}

pub(super) fn save(
    connection: &Connection,
    snapshot: &CharacterSnapshot,
) -> Result<(), StoreError> {
    let (name, item_locations_json) = encode_character_row(snapshot)?;
    let updated = connection
        .prepare_cached(
            "UPDATE characters
             SET name = ?1, room_id = ?2, item_locations_json = ?3, updated_at = unixepoch()
             WHERE id = ?4 AND account_id = ?5",
        )?
        .execute(params![
            name,
            snapshot.room_id,
            item_locations_json,
            snapshot.id.0,
            snapshot.account_id.0
        ])?;
    if updated == 0 {
        return Err(StoreError::CharacterNotFound {
            account_id: snapshot.account_id,
            character_id: snapshot.id,
        });
    }
    Ok(())
}

pub(super) fn load_character_gameplay(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<CharacterGameplaySnapshot, StoreError> {
    let json = connection
        .query_row(
            "SELECT gameplay_json FROM characters WHERE id = ?1",
            [character_id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(StoreError::CharacterGameplayNotFound(character_id))?;
    serde_json::from_str(&json).map_err(StoreError::from)
}

pub(super) fn load_character_gameplay_batch(
    connection: &Connection,
    character_ids: Vec<CharacterId>,
    limits: StartupGameplayLoadLimits,
) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
    let character_ids = normalise_startup_character_ids(character_ids, limits)?;
    let mut aggregates = Vec::new();
    let mut total_json_bytes = 0_usize;
    for batch in character_ids.chunks(SQLITE_CHARACTER_BATCH_SIZE) {
        let placeholders = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, gameplay_json FROM characters WHERE id IN ({placeholders}) ORDER BY id"
        );
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(batch.iter().map(|id| id.0)))?;
        while let Some(row) = rows.next()? {
            let character_id = CharacterId(row.get(0)?);
            let json: String = row.get(1)?;
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

pub(super) fn ensure_character_ids_exist_batch(
    connection: &Connection,
    character_ids: Vec<CharacterId>,
) -> Result<(), StoreError> {
    let character_ids = normalise_character_identity_ids(character_ids)?;
    let mut found = Vec::with_capacity(character_ids.len());
    for batch in character_ids.chunks(SQLITE_CHARACTER_BATCH_SIZE) {
        let placeholders = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT id FROM characters WHERE id IN ({placeholders}) ORDER BY id");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(batch.iter().map(|id| id.0)), |row| {
            row.get::<_, i64>(0).map(CharacterId)
        })?;
        for row in rows {
            found.push(row?);
        }
    }
    ensure_all_character_ids_exist(&character_ids, &found)
}

pub(super) fn load_runtime_snapshot(
    connection: &Connection,
) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
    load_durable_runtime_snapshot(connection).map(|snapshot| snapshot.map(|(_, snapshot)| snapshot))
}

pub(super) fn load_durable_runtime_snapshot(
    connection: &Connection,
) -> Result<Option<(u32, WorldRuntimeSnapshot)>, StoreError> {
    connection
        .query_row(
            "SELECT snapshot_version, snapshot_json FROM world_runtime WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .map(|(snapshot_version, json)| {
            let durable_version =
                u32::try_from(snapshot_version).map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
            decode_runtime_snapshot(snapshot_version, &json)
                .map(|snapshot| (durable_version, snapshot))
        })
        .transpose()
}

pub(super) fn has_durable_character_authority(connection: &Connection) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM characters LIMIT 1)",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

pub(super) fn load_area_generation(
    connection: &Connection,
) -> Result<Option<DurableAreaGeneration>, StoreError> {
    let generation = connection
        .query_row(
            "SELECT world_id, active_catalog_sha256, previous_catalog_sha256,
                    activation_sequence
             FROM world_area_generation WHERE singleton = 1",
            [],
            |row| {
                let sequence = row.get::<_, i64>(3)?;
                Ok(DurableAreaGeneration {
                    world_id: row.get(0)?,
                    active_catalog_sha256: row.get(1)?,
                    previous_catalog_sha256: row.get(2)?,
                    activation_sequence: u64::try_from(sequence).unwrap_or(0),
                })
            },
        )
        .optional()?;
    if let Some(generation) = &generation {
        validate_area_generation(generation)?;
    }
    Ok(generation)
}

pub(super) fn load_area_checkpoints(
    connection: &Connection,
) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT generation_sha256, unit_id, residency_epoch,
                checkpoint_version, checkpoint_sha256, captured_logical_tick,
                checkpoint_json
         FROM world_area_checkpoints
         ORDER BY generation_sha256, unit_id, residency_epoch
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_DURABLE_AREA_CHECKPOINTS as i64 + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() > MAX_DURABLE_AREA_CHECKPOINTS {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let active_generation = load_area_generation(connection)?;
    rows.into_iter()
        .map(
            |(generation, unit_id, epoch, version, sha256, tick, json)| {
                let durable = DurableAreaCheckpoint {
                    generation_sha256: generation,
                    unit_id,
                    residency_epoch: u64::try_from(epoch).unwrap_or(0),
                    checkpoint_version: u32::try_from(version).unwrap_or(0),
                    checkpoint_sha256: sha256,
                    captured_logical_tick: u64::try_from(tick).unwrap_or(u64::MAX),
                    checkpoint_json: json,
                };
                if active_generation
                    .as_ref()
                    .is_none_or(|active| active.active_catalog_sha256 != durable.generation_sha256)
                {
                    return Err(StoreError::InvalidAreaCheckpoint);
                }
                validate_area_checkpoint(&durable)?;
                Ok(durable)
            },
        )
        .collect()
}

pub(super) fn persist_area_runtime(
    transaction: &rusqlite::Transaction<'_>,
    runtime: &WorldRuntimeSnapshot,
) -> Result<(), StoreError> {
    let (runtime_json, adventure_json) = encode_runtime_documents(runtime)?;
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, ?1, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        [adventure_json],
    )?;
    let last_event_sequence = transaction.query_row(
        "SELECT max(
             COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
             COALESCE((SELECT last_event_sequence FROM world_runtime
                       WHERE singleton = 1), 0)
         )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        params![runtime.snapshot_version, runtime_json, last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ?1 - ?2 AND sequence <= ?1",
        params![last_event_sequence, DOMAIN_EVENT_RETENTION],
    )?;
    Ok(())
}

pub(super) fn verify_exact_area_generation(
    transaction: &rusqlite::Transaction<'_>,
    expected: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    let sequence = i64::try_from(expected.activation_sequence)
        .map_err(|_| StoreError::InvalidAreaGeneration)?;
    let changed = transaction.execute(
        "UPDATE world_area_generation SET updated_at = updated_at
         WHERE singleton = 1 AND world_id = ?1
           AND active_catalog_sha256 = ?2
           AND previous_catalog_sha256 IS ?3
           AND activation_sequence = ?4",
        params![
            expected.world_id,
            expected.active_catalog_sha256,
            expected.previous_catalog_sha256,
            sequence,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::AreaGenerationConflict);
    }
    Ok(())
}

pub(super) fn commit_area_retirement(
    connection: &mut Connection,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<(), StoreError> {
    let checkpoint = validate_area_retirement_commit(runtime, expected, durable)?;
    let transaction = connection.transaction()?;
    verify_exact_area_generation(&transaction, expected)?;
    let current = load_runtime_snapshot(&transaction)?.ok_or(StoreError::AreaCheckpointConflict)?;
    validate_area_retirement_previous_runtime(&current, runtime, &checkpoint)?;
    let inserted = transaction.execute(
        "INSERT INTO world_area_checkpoints
             (generation_sha256, unit_id, residency_epoch,
              checkpoint_version, checkpoint_sha256, captured_logical_tick,
              checkpoint_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch(), unixepoch())
         ON CONFLICT(generation_sha256, unit_id, residency_epoch) DO NOTHING",
        params![
            durable.generation_sha256,
            durable.unit_id,
            i64::try_from(durable.residency_epoch).unwrap_or(0),
            i64::from(durable.checkpoint_version),
            durable.checkpoint_sha256,
            i64::try_from(durable.captured_logical_tick).unwrap_or(-1),
            durable.checkpoint_json,
        ],
    )?;
    if inserted != 1 {
        return Err(StoreError::AreaCheckpointConflict);
    }
    persist_area_runtime(&transaction, runtime)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn commit_area_rehydration(
    connection: &mut Connection,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<(), StoreError> {
    let checkpoint = validate_area_rehydration_commit(runtime, expected, durable)?;
    let transaction = connection.transaction()?;
    verify_exact_area_generation(&transaction, expected)?;
    let current = load_runtime_snapshot(&transaction)?.ok_or(StoreError::AreaCheckpointConflict)?;
    validate_area_rehydration_previous_runtime(&current, runtime, durable, &checkpoint)?;
    let deleted = transaction.execute(
        "DELETE FROM world_area_checkpoints
         WHERE generation_sha256 = ?1 AND unit_id = ?2 AND residency_epoch = ?3
           AND checkpoint_version = ?4 AND checkpoint_sha256 = ?5
           AND captured_logical_tick = ?6 AND checkpoint_json = ?7",
        params![
            durable.generation_sha256,
            durable.unit_id,
            i64::try_from(durable.residency_epoch).unwrap_or(0),
            i64::from(durable.checkpoint_version),
            durable.checkpoint_sha256,
            i64::try_from(durable.captured_logical_tick).unwrap_or(-1),
            durable.checkpoint_json,
        ],
    )?;
    if deleted != 1 {
        return Err(StoreError::AreaCheckpointConflict);
    }
    persist_area_runtime(&transaction, runtime)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn bind_area_generation_if_absent(
    connection: &mut Connection,
    generation: &DurableAreaGeneration,
) -> Result<DurableAreaGeneration, StoreError> {
    validate_area_generation(generation)?;
    if generation.activation_sequence != 1 || generation.previous_catalog_sha256.is_some() {
        return Err(StoreError::InvalidAreaGeneration);
    }
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO world_area_generation
             (singleton, world_id, active_catalog_sha256,
              previous_catalog_sha256, activation_sequence, updated_at)
         VALUES (1, ?1, ?2, NULL, 1, unixepoch())
         ON CONFLICT(singleton) DO NOTHING",
        params![generation.world_id, generation.active_catalog_sha256],
    )?;
    let actual = load_area_generation(&transaction)?.ok_or(StoreError::InvalidAreaGeneration)?;
    transaction.commit()?;
    Ok(actual)
}

pub(super) fn commit_area_generation_activation(
    connection: &mut Connection,
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
    let transaction = connection.transaction()?;
    let changed = transaction.execute(
        "UPDATE world_area_generation
         SET active_catalog_sha256 = ?1,
             previous_catalog_sha256 = ?2,
             activation_sequence = ?3,
             updated_at = unixepoch()
         WHERE singleton = 1
           AND world_id = ?4
           AND active_catalog_sha256 = ?5
           AND previous_catalog_sha256 IS ?6
           AND activation_sequence = ?7",
        params![
            next.active_catalog_sha256,
            next.previous_catalog_sha256,
            next_sequence,
            expected.world_id,
            expected.active_catalog_sha256,
            expected.previous_catalog_sha256,
            expected_sequence,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::AreaGenerationConflict);
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, ?1, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        [world_adventure_json],
    )?;
    let last_event_sequence = transaction.query_row(
        "SELECT max(
             COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
             COALESCE((SELECT last_event_sequence FROM world_runtime
                       WHERE singleton = 1), 0)
         )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        params![runtime.snapshot_version, runtime_json, last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ?1 - ?2
           AND sequence <= ?1",
        params![last_event_sequence, DOMAIN_EVENT_RETENTION],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn load_startup_world_recovery(
    connection: &mut Connection,
) -> Result<StartupWorldRecovery, StoreError> {
    let transaction = connection.transaction()?;
    let recovery = match load_durable_runtime_snapshot(&transaction)? {
        Some((durable_version, snapshot)) => StartupWorldRecovery::Snapshot {
            durable_version,
            snapshot,
        },
        None => match load_startup_domain_events(&transaction)? {
            events if events.is_empty() => StartupWorldRecovery::Empty,
            events => StartupWorldRecovery::Ledger(events),
        },
    };
    transaction.commit()?;
    Ok(recovery)
}

pub(super) fn load_startup_domain_events(
    connection: &Connection,
) -> Result<Vec<DomainEvent>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_json
         FROM domain_events
         ORDER BY sequence
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![DOMAIN_EVENT_RETENTION + 1], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    decode_startup_domain_events(rows)
}

pub(super) fn load_domain_events(connection: &Connection) -> Result<Vec<DomainEvent>, StoreError> {
    let mut statement =
        connection.prepare("SELECT event_json FROM domain_events ORDER BY sequence")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|json| {
            let json = json?;
            serde_json::from_str(&json).map_err(StoreError::from)
        })
        .collect()
}

pub(super) fn commit_action(
    connection: &mut Connection,
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

pub(super) fn commit_staff_action(
    connection: &mut Connection,
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

pub(super) fn commit_action_with_retention(
    connection: &mut Connection,
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

    let transaction = connection.transaction()?;
    if let Some((audit, _)) = &staff_audit {
        validate_staff_authority(&transaction, audit, None, audit.target_account_id)?;
    }
    for ((snapshot, gameplay_json), adventure_json) in
        characters.iter().zip(gameplay_json).zip(adventure_json)
    {
        save(&transaction, &snapshot.character)?;
        let updated = transaction
            .prepare_cached(
                "UPDATE characters SET gameplay_json = ?1, updated_at = unixepoch()
             WHERE id = ?2 AND account_id = ?3",
            )?
            .execute(params![
                gameplay_json,
                snapshot.character.id.0,
                snapshot.character.account_id.0
            ])?;
        if updated == 0 {
            return Err(StoreError::CharacterNotFound {
                account_id: snapshot.character.account_id,
                character_id: snapshot.character.id,
            });
        }
        transaction
            .prepare_cached(
                "INSERT INTO character_adventure (character_id, state_json, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(character_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at",
            )?
            .execute(params![snapshot.character.id.0, adventure_json])?;
    }
    transaction
        .prepare_cached(
            "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, ?1, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        )?
        .execute([world_adventure_json])?;
    {
        let mut insert_event =
            transaction.prepare_cached("INSERT INTO domain_events (event_json) VALUES (?1)")?;
        for event_json in event_json {
            insert_event.execute([event_json])?;
        }
    }
    let last_event_sequence = transaction
        .prepare_cached(
            "SELECT max(
             COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
             COALESCE((SELECT last_event_sequence FROM world_runtime
                       WHERE singleton = 1), 0)
         )",
        )?
        .query_row([], |row| row.get::<_, i64>(0))?;
    transaction
        .prepare_cached(
            "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        )?
        .execute(params![
            runtime.snapshot_version,
            runtime_json,
            last_event_sequence
        ])?;
    transaction
        .prepare_cached(
            "DELETE FROM domain_events
             WHERE sequence <= ?1 - ?2
               AND sequence <= ?1",
        )?
        .execute(params![last_event_sequence, event_retention])?;
    if let Some((audit, now)) = &staff_audit {
        insert_staff_audit(&transaction, audit, *now)?;
    }
    transaction.commit()?;
    Ok(())
}
