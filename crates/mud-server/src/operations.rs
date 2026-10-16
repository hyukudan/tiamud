//! Small, dependency-free operational controls for the single-host server.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tiamud_core::AccountId;

const MAX_TRACKED_AUTH_ADDRESSES: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveSessionLimit {
    Account,
    Address,
}

#[derive(Debug, Default)]
struct ActiveSessionCounts {
    accounts: HashMap<AccountId, usize>,
    addresses: HashMap<IpAddr, usize>,
}

/// Process-wide concurrent game-session cap shared by WebSocket and Telnet.
/// Entries exist only while a permit is alive and therefore cannot grow beyond
/// the number of active connections.
#[derive(Debug)]
pub struct ActiveSessionLimiter {
    counts: Mutex<ActiveSessionCounts>,
    max_per_account: usize,
    max_per_address: usize,
}

impl ActiveSessionLimiter {
    pub fn new(max_per_account: u32, max_per_address: u32) -> Self {
        Self {
            counts: Mutex::new(ActiveSessionCounts::default()),
            max_per_account: max_per_account.max(1) as usize,
            max_per_address: max_per_address.max(1) as usize,
        }
    }

    pub fn acquire(
        self: &Arc<Self>,
        account: AccountId,
        address: IpAddr,
    ) -> Result<ActiveSessionPermit, ActiveSessionLimit> {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if counts.accounts.get(&account).copied().unwrap_or(0) >= self.max_per_account {
            return Err(ActiveSessionLimit::Account);
        }
        if counts.addresses.get(&address).copied().unwrap_or(0) >= self.max_per_address {
            return Err(ActiveSessionLimit::Address);
        }
        *counts.accounts.entry(account).or_default() += 1;
        *counts.addresses.entry(address).or_default() += 1;
        Ok(ActiveSessionPermit {
            limiter: self.clone(),
            account,
            address,
        })
    }

    fn release(&self, account: AccountId, address: IpAddr) {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        decrement_or_remove(&mut counts.accounts, account);
        decrement_or_remove(&mut counts.addresses, address);
    }
}

fn decrement_or_remove<K: Eq + std::hash::Hash + Copy>(counts: &mut HashMap<K, usize>, key: K) {
    if let Some(count) = counts.get_mut(&key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            counts.remove(&key);
        }
    }
}

#[derive(Debug)]
pub struct ActiveSessionPermit {
    limiter: Arc<ActiveSessionLimiter>,
    account: AccountId,
    address: IpAddr,
}

impl Drop for ActiveSessionPermit {
    fn drop(&mut self) {
        self.limiter.release(self.account, self.address);
    }
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    updated_at: Instant,
    last_seen: Instant,
}

/// Bounded per-key token bucket. Tracking at most `max_tracked` keys keeps
/// memory bounded; the least recently seen key is forgotten first.
#[derive(Debug)]
pub struct KeyedRateLimiter<K> {
    buckets: Mutex<HashMap<K, Bucket>>,
    refill_per_second: f64,
    burst: f64,
    max_tracked: usize,
}

/// Per-IP limiter used for password-bearing endpoints.
pub type AuthRateLimiter = KeyedRateLimiter<IpAddr>;

impl AuthRateLimiter {
    pub fn per_minute(attempts: u32, burst: u32) -> Self {
        Self::per_minute_tracking(attempts, burst, MAX_TRACKED_AUTH_ADDRESSES)
    }
}

impl<K: Copy + Eq + std::hash::Hash> KeyedRateLimiter<K> {
    pub fn per_minute_tracking(attempts: u32, burst: u32, max_tracked: usize) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            refill_per_second: f64::from(attempts.max(1)) / 60.0,
            burst: f64::from(burst.max(1)),
            max_tracked: max_tracked.max(1),
        }
    }

    pub fn check(&self, key: K, now: Instant) -> bool {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !buckets.contains_key(&key)
            && buckets.len() >= self.max_tracked
            && let Some(oldest) = buckets
                .iter()
                .min_by_key(|(_, bucket)| bucket.last_seen)
                .map(|(key, _)| *key)
        {
            buckets.remove(&oldest);
        }

        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: self.burst,
            updated_at: now,
            last_seen: now,
        });
        let elapsed = now
            .saturating_duration_since(bucket.updated_at)
            .as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_second).min(self.burst);
        bucket.updated_at = now;
        bucket.last_seen = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    #[cfg(test)]
    fn tracked_addresses(&self) -> usize {
        self.buckets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }
}

/// Per-connection limiter for commands. It is intentionally local to a
/// connection, so disconnected clients cannot leave unbounded server state.
#[derive(Debug)]
pub struct CommandRateLimiter {
    tokens: f64,
    updated_at: Instant,
    refill_per_second: f64,
    burst: f64,
}

impl CommandRateLimiter {
    pub fn per_second(commands: u32, burst: u32, now: Instant) -> Self {
        Self {
            tokens: f64::from(burst.max(1)),
            updated_at: now,
            refill_per_second: f64::from(commands.max(1)),
            burst: f64::from(burst.max(1)),
        }
    }

    pub fn check(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.updated_at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_second).min(self.burst);
        self.updated_at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    websocket_connections: AtomicUsize,
    telnet_connections: AtomicUsize,
    authenticated_sessions: AtomicUsize,
    actions_total: AtomicU64,
    actions_failed: AtomicU64,
    action_latency_microseconds: AtomicU64,
    auth_rate_limited_total: AtomicU64,
    websocket_output_coalesced_total: AtomicU64,
    telnet_output_coalesced_total: AtomicU64,
    websocket_output_dropped_total: AtomicU64,
    telnet_output_dropped_total: AtomicU64,
    websocket_output_queue_depth: AtomicUsize,
    telnet_output_queue_depth: AtomicUsize,
    websocket_slow_disconnects_total: AtomicU64,
    telnet_slow_disconnects_total: AtomicU64,
    websocket_outbound_budget_exceeded_total: AtomicU64,
    gmcp_scene_overflow_total: AtomicU64,
    database_transactions_total: AtomicU64,
    database_errors_total: AtomicU64,
    database_latency_microseconds: AtomicU64,
    database_queue_depth: AtomicUsize,
    ticks_total: AtomicU64,
    tick_errors_total: AtomicU64,
    tick_latency_microseconds: AtomicU64,
    scripts_total: AtomicU64,
    script_errors_total: AtomicU64,
    script_timeouts_total: AtomicU64,
    script_latency_microseconds: AtomicU64,
    delivery_mailboxes: AtomicUsize,
    world_delivery_enqueued_total: AtomicU64,
    social_delivery_enqueued_total: AtomicU64,
    world_delivery_dropped_total: AtomicU64,
    social_delivery_dropped_total: AtomicU64,
    resident_areas: AtomicUsize,
    preparing_areas: AtomicUsize,
    pending_areas: AtomicUsize,
    deferred_area_exits: AtomicUsize,
    area_load_demands_total: AtomicU64,
    area_load_coalesced_total: AtomicU64,
    area_load_rejected_total: AtomicU64,
    area_closure_areas_total: AtomicU64,
    area_load_installed_total: AtomicU64,
    area_load_prepared_total: AtomicU64,
    area_load_already_resident_total: AtomicU64,
    area_load_queue_full_total: AtomicU64,
    area_load_timeout_total: AtomicU64,
    area_load_cancelled_total: AtomicU64,
    area_load_preparation_failed_total: AtomicU64,
    area_load_installation_failed_total: AtomicU64,
    area_load_latency_microseconds: AtomicU64,
    area_bundle_bytes_total: AtomicU64,
    area_generation_phase: AtomicUsize,
    area_generation_noop_total: AtomicU64,
    area_generation_activated_total: AtomicU64,
    area_generation_rolled_back_total: AtomicU64,
    area_generation_busy_total: AtomicU64,
    area_generation_incompatible_total: AtomicU64,
    area_generation_verification_failed_total: AtomicU64,
    area_generation_persistence_failed_total: AtomicU64,
    area_generation_conflict_total: AtomicU64,
    area_generation_internal_failed_total: AtomicU64,
    area_generation_latency_microseconds: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputTransport {
    Websocket,
    Telnet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptMetricOutcome {
    Success,
    Error,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutedDeliveryKind {
    World,
    Social,
}

/// Closed outcomes for an admitted area-load demand.
///
/// Keeping this vocabulary in code prevents catalog-controlled identifiers or
/// failure details from becoming Prometheus labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AreaLoadMetricResult {
    Prepared,
    Installed,
    AlreadyResident,
    QueueFull,
    Timeout,
    Cancelled,
    PreparationFailed,
    InstallationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AreaGenerationMetricPhase {
    Active,
    Staging,
    Draining,
    Committing,
}

/// Closed outcomes for a local operator reconciliation. Content-controlled
/// identifiers and digests never become metric labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AreaGenerationMetricResult {
    Noop,
    Activated,
    RolledBack,
    Busy,
    Incompatible,
    VerificationFailed,
    PersistenceFailed,
    Conflict,
    InternalFailed,
}

impl RuntimeMetrics {
    pub fn websocket_opened(&self) {
        self.websocket_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn websocket_closed(&self) {
        self.websocket_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn telnet_opened(&self) {
        self.telnet_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn telnet_closed(&self) {
        self.telnet_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn session_opened(&self) {
        self.authenticated_sessions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn session_closed(&self) {
        self.authenticated_sessions.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn action_finished(&self, elapsed: Duration, succeeded: bool) {
        self.actions_total.fetch_add(1, Ordering::Relaxed);
        self.action_latency_microseconds.fetch_add(
            elapsed.as_micros().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        if !succeeded {
            self.actions_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn auth_rate_limited(&self) {
        self.auth_rate_limited_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn output_coalesced(&self, transport: OutputTransport) {
        let counter = match transport {
            OutputTransport::Websocket => &self.websocket_output_coalesced_total,
            OutputTransport::Telnet => &self.telnet_output_coalesced_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn output_dropped(&self, transport: OutputTransport) {
        let counter = match transport {
            OutputTransport::Websocket => &self.websocket_output_dropped_total,
            OutputTransport::Telnet => &self.telnet_output_dropped_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn output_frame_enqueued(&self, transport: OutputTransport) {
        self.output_queue_depth(transport)
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn output_frame_removed(&self, transport: OutputTransport) {
        let depth = self.output_queue_depth(transport);
        let _ = depth.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_sub(1))
        });
    }

    fn output_queue_depth(&self, transport: OutputTransport) -> &AtomicUsize {
        match transport {
            OutputTransport::Websocket => &self.websocket_output_queue_depth,
            OutputTransport::Telnet => &self.telnet_output_queue_depth,
        }
    }

    pub fn slow_client_disconnected(&self, transport: OutputTransport) {
        let counter = match transport {
            OutputTransport::Websocket => &self.websocket_slow_disconnects_total,
            OutputTransport::Telnet => &self.telnet_slow_disconnects_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Records output which was rejected before transport because its encoded
    /// representation exceeded the protocol's per-message byte budget.
    pub fn websocket_outbound_budget_exceeded(&self) {
        self.websocket_outbound_budget_exceeded_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a full `Tiamud.Scene` projection which had to degrade to the
    /// bounded GMCP fallback.
    pub fn gmcp_scene_overflow(&self) {
        self.gmcp_scene_overflow_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn database_transaction_finished(&self, elapsed: Duration, succeeded: bool) {
        self.database_transactions_total
            .fetch_add(1, Ordering::Relaxed);
        self.database_latency_microseconds.fetch_add(
            elapsed.as_micros().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        if !succeeded {
            self.database_errors_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_database_queue_depth(&self, depth: usize) {
        self.database_queue_depth.store(depth, Ordering::Relaxed);
    }

    pub fn tick_finished(&self, elapsed: Duration, succeeded: bool) {
        self.ticks_total.fetch_add(1, Ordering::Relaxed);
        self.tick_latency_microseconds.fetch_add(
            elapsed.as_micros().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        if !succeeded {
            self.tick_errors_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn script_finished(&self, elapsed: Duration, outcome: ScriptMetricOutcome) {
        self.scripts_total.fetch_add(1, Ordering::Relaxed);
        self.script_latency_microseconds.fetch_add(
            elapsed.as_micros().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        match outcome {
            ScriptMetricOutcome::Success => {}
            ScriptMetricOutcome::Error => {
                self.script_errors_total.fetch_add(1, Ordering::Relaxed);
            }
            ScriptMetricOutcome::Timeout => {
                self.script_timeouts_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn set_delivery_mailboxes(&self, count: usize) {
        self.delivery_mailboxes.store(count, Ordering::Relaxed);
    }

    pub fn delivery_enqueued(&self, kind: RoutedDeliveryKind) {
        let counter = match kind {
            RoutedDeliveryKind::World => &self.world_delivery_enqueued_total,
            RoutedDeliveryKind::Social => &self.social_delivery_enqueued_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn delivery_dropped(&self, kind: RoutedDeliveryKind) {
        let counter = match kind {
            RoutedDeliveryKind::World => &self.world_delivery_dropped_total,
            RoutedDeliveryKind::Social => &self.social_delivery_dropped_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Sets the number of areas which the actor has published as resident.
    pub fn set_resident_areas(&self, count: usize) {
        self.resident_areas.store(count, Ordering::Relaxed);
    }

    /// Sets the number of areas currently held in private preparation arenas.
    pub fn set_preparing_areas(&self, count: usize) {
        self.preparing_areas.store(count, Ordering::Relaxed);
    }

    pub fn set_pending_areas(&self, count: usize) {
        self.pending_areas.store(count, Ordering::Relaxed);
    }

    pub fn set_deferred_area_exits(&self, count: usize) {
        self.deferred_area_exits.store(count, Ordering::Relaxed);
    }

    pub fn record_area_load_demand(&self) {
        self.area_load_demands_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_area_load_coalesced(&self) {
        self.area_load_coalesced_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_area_load_rejection(&self) {
        self.area_load_rejected_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_area_closure(&self, area_count: usize) {
        self.area_closure_areas_total
            .fetch_add(area_count.try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Records one completed or rejected area-load demand.
    ///
    /// `bundle_bytes` is the number of immutable bundle bytes read for this
    /// demand and should be zero when no bundle was admitted. The outcome is a
    /// closed enum so callers cannot add content-controlled labels.
    pub fn area_load_finished(
        &self,
        elapsed: Duration,
        bundle_bytes: u64,
        result: AreaLoadMetricResult,
    ) {
        let counter = match result {
            AreaLoadMetricResult::Prepared => &self.area_load_prepared_total,
            AreaLoadMetricResult::Installed => &self.area_load_installed_total,
            AreaLoadMetricResult::AlreadyResident => &self.area_load_already_resident_total,
            AreaLoadMetricResult::QueueFull => &self.area_load_queue_full_total,
            AreaLoadMetricResult::Timeout => &self.area_load_timeout_total,
            AreaLoadMetricResult::Cancelled => &self.area_load_cancelled_total,
            AreaLoadMetricResult::PreparationFailed => &self.area_load_preparation_failed_total,
            AreaLoadMetricResult::InstallationFailed => &self.area_load_installation_failed_total,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        self.area_load_latency_microseconds.fetch_add(
            elapsed.as_micros().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.area_bundle_bytes_total
            .fetch_add(bundle_bytes, Ordering::Relaxed);
    }

    pub fn set_area_generation_phase(&self, phase: AreaGenerationMetricPhase) {
        let value = match phase {
            AreaGenerationMetricPhase::Active => 0,
            AreaGenerationMetricPhase::Staging => 1,
            AreaGenerationMetricPhase::Draining => 2,
            AreaGenerationMetricPhase::Committing => 3,
        };
        self.area_generation_phase.store(value, Ordering::Relaxed);
    }

    pub fn area_generation_finished(&self, elapsed: Duration, result: AreaGenerationMetricResult) {
        let counter = match result {
            AreaGenerationMetricResult::Noop => &self.area_generation_noop_total,
            AreaGenerationMetricResult::Activated => &self.area_generation_activated_total,
            AreaGenerationMetricResult::RolledBack => &self.area_generation_rolled_back_total,
            AreaGenerationMetricResult::Busy => &self.area_generation_busy_total,
            AreaGenerationMetricResult::Incompatible => &self.area_generation_incompatible_total,
            AreaGenerationMetricResult::VerificationFailed => {
                &self.area_generation_verification_failed_total
            }
            AreaGenerationMetricResult::PersistenceFailed => {
                &self.area_generation_persistence_failed_total
            }
            AreaGenerationMetricResult::Conflict => &self.area_generation_conflict_total,
            AreaGenerationMetricResult::InternalFailed => {
                &self.area_generation_internal_failed_total
            }
        };
        counter.fetch_add(1, Ordering::Relaxed);
        self.area_generation_latency_microseconds.fetch_add(
            elapsed.as_micros().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.set_area_generation_phase(AreaGenerationMetricPhase::Active);
    }

    pub fn prometheus(&self, world_queue_depth: usize) -> String {
        format!(
            concat!(
                "# TYPE tiamud_connections gauge\n",
                "tiamud_connections{{transport=\"websocket\"}} {}\n",
                "tiamud_connections{{transport=\"telnet\"}} {}\n",
                "# TYPE tiamud_authenticated_sessions gauge\n",
                "tiamud_authenticated_sessions {}\n",
                "# TYPE tiamud_actions_total counter\n",
                "tiamud_actions_total {}\n",
                "# TYPE tiamud_action_errors_total counter\n",
                "tiamud_action_errors_total {}\n",
                "# TYPE tiamud_action_latency_seconds_sum counter\n",
                "tiamud_action_latency_seconds_sum {:.6}\n",
                "# TYPE tiamud_world_queue_depth gauge\n",
                "tiamud_world_queue_depth {}\n",
                "# TYPE tiamud_auth_rate_limited_total counter\n",
                "tiamud_auth_rate_limited_total {}\n",
                "# TYPE tiamud_output_coalesced_total counter\n",
                "tiamud_output_coalesced_total{{transport=\"websocket\"}} {}\n",
                "tiamud_output_coalesced_total{{transport=\"telnet\"}} {}\n",
                "# TYPE tiamud_output_dropped_total counter\n",
                "tiamud_output_dropped_total{{transport=\"websocket\"}} {}\n",
                "tiamud_output_dropped_total{{transport=\"telnet\"}} {}\n",
                "# TYPE tiamud_output_queue_depth gauge\n",
                "tiamud_output_queue_depth{{transport=\"websocket\"}} {}\n",
                "tiamud_output_queue_depth{{transport=\"telnet\"}} {}\n",
                "# TYPE tiamud_slow_client_disconnects_total counter\n",
                "tiamud_slow_client_disconnects_total{{transport=\"websocket\"}} {}\n",
                "tiamud_slow_client_disconnects_total{{transport=\"telnet\"}} {}\n",
                "# TYPE tiamud_outbound_budget_exceeded_total counter\n",
                "tiamud_outbound_budget_exceeded_total{{transport=\"websocket\",message=\"frame\"}} {}\n",
                "tiamud_outbound_budget_exceeded_total{{transport=\"gmcp\",message=\"scene\"}} {}\n",
                "# TYPE tiamud_database_transactions_total counter\n",
                "tiamud_database_transactions_total {}\n",
                "# TYPE tiamud_database_errors_total counter\n",
                "tiamud_database_errors_total {}\n",
                "# TYPE tiamud_database_latency_seconds_sum counter\n",
                "tiamud_database_latency_seconds_sum {:.6}\n",
                "# TYPE tiamud_database_queue_depth gauge\n",
                "tiamud_database_queue_depth {}\n",
                "# TYPE tiamud_ticks_total counter\n",
                "tiamud_ticks_total {}\n",
                "# TYPE tiamud_tick_errors_total counter\n",
                "tiamud_tick_errors_total {}\n",
                "# TYPE tiamud_tick_latency_seconds_sum counter\n",
                "tiamud_tick_latency_seconds_sum {:.6}\n",
                "# TYPE tiamud_scripts_total counter\n",
                "tiamud_scripts_total {}\n",
                "# TYPE tiamud_script_errors_total counter\n",
                "tiamud_script_errors_total {}\n",
                "# TYPE tiamud_script_timeouts_total counter\n",
                "tiamud_script_timeouts_total {}\n",
                "# TYPE tiamud_script_latency_seconds_sum counter\n",
                "tiamud_script_latency_seconds_sum {:.6}\n",
                "# TYPE tiamud_delivery_mailboxes gauge\n",
                "tiamud_delivery_mailboxes {}\n",
                "# TYPE tiamud_delivery_enqueued_total counter\n",
                "tiamud_delivery_enqueued_total{{kind=\"world\"}} {}\n",
                "tiamud_delivery_enqueued_total{{kind=\"social\"}} {}\n",
                "# TYPE tiamud_delivery_dropped_total counter\n",
                "tiamud_delivery_dropped_total{{kind=\"world\"}} {}\n",
                "tiamud_delivery_dropped_total{{kind=\"social\"}} {}\n",
                "# TYPE tiamud_areas gauge\n",
                "tiamud_areas{{state=\"resident\"}} {}\n",
                "tiamud_areas{{state=\"preparing\"}} {}\n",
                "tiamud_areas{{state=\"pending\"}} {}\n",
                "# TYPE tiamud_deferred_area_exits gauge\n",
                "tiamud_deferred_area_exits {}\n",
                "# TYPE tiamud_area_load_demands_total counter\n",
                "tiamud_area_load_demands_total {}\n",
                "# TYPE tiamud_area_load_coalesced_total counter\n",
                "tiamud_area_load_coalesced_total {}\n",
                "# TYPE tiamud_area_load_rejected_total counter\n",
                "tiamud_area_load_rejected_total {}\n",
                "# TYPE tiamud_area_closure_areas_total counter\n",
                "tiamud_area_closure_areas_total {}\n",
                "# TYPE tiamud_area_loads_total counter\n",
                "tiamud_area_loads_total{{result=\"prepared\"}} {}\n",
                "tiamud_area_loads_total{{result=\"installed\"}} {}\n",
                "tiamud_area_loads_total{{result=\"already_resident\"}} {}\n",
                "tiamud_area_loads_total{{result=\"queue_full\"}} {}\n",
                "tiamud_area_loads_total{{result=\"timeout\"}} {}\n",
                "tiamud_area_loads_total{{result=\"cancelled\"}} {}\n",
                "tiamud_area_loads_total{{result=\"preparation_failed\"}} {}\n",
                "tiamud_area_loads_total{{result=\"installation_failed\"}} {}\n",
                "# TYPE tiamud_area_load_latency_seconds_sum counter\n",
                "tiamud_area_load_latency_seconds_sum {:.6}\n",
                "# TYPE tiamud_area_bundle_bytes_total counter\n",
                "tiamud_area_bundle_bytes_total {}\n",
                "# TYPE tiamud_area_generation_phase gauge\n",
                "tiamud_area_generation_phase{{state=\"active\"}} {}\n",
                "tiamud_area_generation_phase{{state=\"staging\"}} {}\n",
                "tiamud_area_generation_phase{{state=\"draining\"}} {}\n",
                "tiamud_area_generation_phase{{state=\"committing\"}} {}\n",
                "# TYPE tiamud_area_generation_transitions_total counter\n",
                "tiamud_area_generation_transitions_total{{result=\"noop\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"activated\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"rolled_back\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"busy\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"incompatible\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"verification_failed\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"persistence_failed\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"conflict\"}} {}\n",
                "tiamud_area_generation_transitions_total{{result=\"internal_failed\"}} {}\n",
                "# TYPE tiamud_area_generation_latency_seconds_sum counter\n",
                "tiamud_area_generation_latency_seconds_sum {:.6}\n",
                "# TYPE process_resident_memory_bytes gauge\n",
                "process_resident_memory_bytes {}\n",
            ),
            self.websocket_connections.load(Ordering::Relaxed),
            self.telnet_connections.load(Ordering::Relaxed),
            self.authenticated_sessions.load(Ordering::Relaxed),
            self.actions_total.load(Ordering::Relaxed),
            self.actions_failed.load(Ordering::Relaxed),
            self.action_latency_microseconds.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            world_queue_depth,
            self.auth_rate_limited_total.load(Ordering::Relaxed),
            self.websocket_output_coalesced_total
                .load(Ordering::Relaxed),
            self.telnet_output_coalesced_total.load(Ordering::Relaxed),
            self.websocket_output_dropped_total.load(Ordering::Relaxed),
            self.telnet_output_dropped_total.load(Ordering::Relaxed),
            self.websocket_output_queue_depth.load(Ordering::Relaxed),
            self.telnet_output_queue_depth.load(Ordering::Relaxed),
            self.websocket_slow_disconnects_total
                .load(Ordering::Relaxed),
            self.telnet_slow_disconnects_total.load(Ordering::Relaxed),
            self.websocket_outbound_budget_exceeded_total
                .load(Ordering::Relaxed),
            self.gmcp_scene_overflow_total.load(Ordering::Relaxed),
            self.database_transactions_total.load(Ordering::Relaxed),
            self.database_errors_total.load(Ordering::Relaxed),
            self.database_latency_microseconds.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            self.database_queue_depth.load(Ordering::Relaxed),
            self.ticks_total.load(Ordering::Relaxed),
            self.tick_errors_total.load(Ordering::Relaxed),
            self.tick_latency_microseconds.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            self.scripts_total.load(Ordering::Relaxed),
            self.script_errors_total.load(Ordering::Relaxed),
            self.script_timeouts_total.load(Ordering::Relaxed),
            self.script_latency_microseconds.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            self.delivery_mailboxes.load(Ordering::Relaxed),
            self.world_delivery_enqueued_total.load(Ordering::Relaxed),
            self.social_delivery_enqueued_total.load(Ordering::Relaxed),
            self.world_delivery_dropped_total.load(Ordering::Relaxed),
            self.social_delivery_dropped_total.load(Ordering::Relaxed),
            self.resident_areas.load(Ordering::Relaxed),
            self.preparing_areas.load(Ordering::Relaxed),
            self.pending_areas.load(Ordering::Relaxed),
            self.deferred_area_exits.load(Ordering::Relaxed),
            self.area_load_demands_total.load(Ordering::Relaxed),
            self.area_load_coalesced_total.load(Ordering::Relaxed),
            self.area_load_rejected_total.load(Ordering::Relaxed),
            self.area_closure_areas_total.load(Ordering::Relaxed),
            self.area_load_prepared_total.load(Ordering::Relaxed),
            self.area_load_installed_total.load(Ordering::Relaxed),
            self.area_load_already_resident_total
                .load(Ordering::Relaxed),
            self.area_load_queue_full_total.load(Ordering::Relaxed),
            self.area_load_timeout_total.load(Ordering::Relaxed),
            self.area_load_cancelled_total.load(Ordering::Relaxed),
            self.area_load_preparation_failed_total
                .load(Ordering::Relaxed),
            self.area_load_installation_failed_total
                .load(Ordering::Relaxed),
            self.area_load_latency_microseconds.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            self.area_bundle_bytes_total.load(Ordering::Relaxed),
            (self.area_generation_phase.load(Ordering::Relaxed) == 0) as u8,
            (self.area_generation_phase.load(Ordering::Relaxed) == 1) as u8,
            (self.area_generation_phase.load(Ordering::Relaxed) == 2) as u8,
            (self.area_generation_phase.load(Ordering::Relaxed) == 3) as u8,
            self.area_generation_noop_total.load(Ordering::Relaxed),
            self.area_generation_activated_total.load(Ordering::Relaxed),
            self.area_generation_rolled_back_total
                .load(Ordering::Relaxed),
            self.area_generation_busy_total.load(Ordering::Relaxed),
            self.area_generation_incompatible_total
                .load(Ordering::Relaxed),
            self.area_generation_verification_failed_total
                .load(Ordering::Relaxed),
            self.area_generation_persistence_failed_total
                .load(Ordering::Relaxed),
            self.area_generation_conflict_total.load(Ordering::Relaxed),
            self.area_generation_internal_failed_total
                .load(Ordering::Relaxed),
            self.area_generation_latency_microseconds
                .load(Ordering::Relaxed) as f64
                / 1_000_000.0,
            process_resident_memory_bytes(),
        )
    }
}

#[cfg(target_os = "linux")]
fn process_resident_memory_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix("VmRSS:")?.trim();
            let kibibytes = value.split_ascii_whitespace().next()?.parse::<u64>().ok()?;
            Some(kibibytes.saturating_mul(1024))
        })
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
const fn process_resident_memory_bytes() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn auth_bucket_refills_and_rejects_a_burst() {
        let limiter = AuthRateLimiter::per_minute(60, 2);
        let address = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let start = Instant::now();
        assert!(limiter.check(address, start));
        assert!(limiter.check(address, start));
        assert!(!limiter.check(address, start));
        assert!(limiter.check(address, start + Duration::from_secs(1)));
    }

    #[test]
    fn command_bucket_is_connection_local_and_refills() {
        let start = Instant::now();
        let mut limiter = CommandRateLimiter::per_second(2, 1, start);
        assert!(limiter.check(start));
        assert!(!limiter.check(start));
        assert!(limiter.check(start + Duration::from_millis(500)));
    }

    #[test]
    fn active_session_caps_are_shared_and_permits_release_both_dimensions() {
        let limiter = Arc::new(ActiveSessionLimiter::new(1, 2));
        let first_address = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let second_address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
        let first = limiter.acquire(AccountId(1), first_address).unwrap();
        assert!(matches!(
            limiter.acquire(AccountId(1), second_address),
            Err(ActiveSessionLimit::Account)
        ));
        let second = limiter.acquire(AccountId(2), first_address).unwrap();
        assert!(matches!(
            limiter.acquire(AccountId(3), first_address),
            Err(ActiveSessionLimit::Address)
        ));

        drop(first);
        let third = limiter.acquire(AccountId(3), first_address).unwrap();
        drop((second, third));
        assert!(limiter.acquire(AccountId(1), second_address).is_ok());
    }

    #[test]
    fn auth_address_tracking_is_strictly_bounded() {
        let limiter = AuthRateLimiter::per_minute(1, 1);
        let start = Instant::now();
        for suffix in 0..=MAX_TRACKED_AUTH_ADDRESSES {
            let address = IpAddr::V6(Ipv6Addr::new(
                0x2001,
                0xdb8,
                0,
                0,
                0,
                0,
                (suffix >> 16) as u16,
                suffix as u16,
            ));
            assert!(limiter.check(address, start + Duration::from_micros(suffix as u64)));
        }
        assert_eq!(limiter.tracked_addresses(), MAX_TRACKED_AUTH_ADDRESSES);
    }

    #[test]
    fn metrics_use_prometheus_text_format() {
        let metrics = RuntimeMetrics::default();
        metrics.websocket_opened();
        metrics.telnet_opened();
        metrics.session_opened();
        metrics.action_finished(Duration::from_millis(25), false);
        metrics.auth_rate_limited();
        metrics.output_coalesced(OutputTransport::Websocket);
        metrics.output_coalesced(OutputTransport::Telnet);
        metrics.output_dropped(OutputTransport::Websocket);
        metrics.output_frame_enqueued(OutputTransport::Websocket);
        metrics.output_frame_enqueued(OutputTransport::Telnet);
        metrics.output_frame_removed(OutputTransport::Telnet);
        metrics.slow_client_disconnected(OutputTransport::Telnet);
        metrics.websocket_outbound_budget_exceeded();
        metrics.gmcp_scene_overflow();
        metrics.database_transaction_finished(Duration::from_millis(10), false);
        metrics.set_database_queue_depth(3);
        metrics.tick_finished(Duration::from_millis(5), true);
        metrics.script_finished(Duration::from_millis(2), ScriptMetricOutcome::Timeout);
        metrics.set_resident_areas(3);
        metrics.set_preparing_areas(2);
        metrics.set_pending_areas(1);
        metrics.set_deferred_area_exits(4);
        metrics.record_area_load_demand();
        metrics.record_area_load_coalesced();
        metrics.record_area_load_rejection();
        metrics.record_area_closure(3);
        metrics.area_load_finished(
            Duration::from_millis(4),
            4_096,
            AreaLoadMetricResult::Installed,
        );
        let output = metrics.prometheus(7);
        assert!(output.contains("tiamud_connections{transport=\"websocket\"} 1"));
        assert!(output.contains("tiamud_connections{transport=\"telnet\"} 1"));
        assert!(output.contains("tiamud_action_errors_total 1"));
        assert!(output.contains("tiamud_action_latency_seconds_sum 0.025000"));
        assert!(output.contains("tiamud_world_queue_depth 7"));
        assert!(output.contains("tiamud_output_coalesced_total{transport=\"websocket\"} 1"));
        assert!(output.contains("tiamud_output_coalesced_total{transport=\"telnet\"} 1"));
        assert!(output.contains("tiamud_output_dropped_total{transport=\"websocket\"} 1"));
        assert!(output.contains("tiamud_output_queue_depth{transport=\"websocket\"} 1"));
        assert!(output.contains("tiamud_output_queue_depth{transport=\"telnet\"} 0"));
        assert!(output.contains("tiamud_slow_client_disconnects_total{transport=\"telnet\"} 1"));
        assert!(output.contains(
            "tiamud_outbound_budget_exceeded_total{transport=\"websocket\",message=\"frame\"} 1"
        ));
        assert!(output.contains(
            "tiamud_outbound_budget_exceeded_total{transport=\"gmcp\",message=\"scene\"} 1"
        ));
        assert!(output.contains("tiamud_database_transactions_total 1"));
        assert!(output.contains("tiamud_database_errors_total 1"));
        assert!(output.contains("tiamud_database_latency_seconds_sum 0.010000"));
        assert!(output.contains("tiamud_database_queue_depth 3"));
        assert!(output.contains("tiamud_ticks_total 1"));
        assert!(output.contains("tiamud_tick_latency_seconds_sum 0.005000"));
        assert!(output.contains("tiamud_scripts_total 1"));
        assert!(output.contains("tiamud_script_timeouts_total 1"));
        assert!(output.contains("tiamud_script_latency_seconds_sum 0.002000"));
        assert!(output.contains("tiamud_areas{state=\"resident\"} 3"));
        assert!(output.contains("tiamud_areas{state=\"preparing\"} 2"));
        assert!(output.contains("tiamud_areas{state=\"pending\"} 1"));
        assert!(output.contains("tiamud_deferred_area_exits 4"));
        assert!(output.contains("tiamud_area_load_demands_total 1"));
        assert!(output.contains("tiamud_area_load_coalesced_total 1"));
        assert!(output.contains("tiamud_area_load_rejected_total 1"));
        assert!(output.contains("tiamud_area_closure_areas_total 3"));
        assert!(output.contains("tiamud_area_loads_total{result=\"installed\"} 1"));
        assert!(output.contains("tiamud_area_load_latency_seconds_sum 0.004000"));
        assert!(output.contains("tiamud_area_bundle_bytes_total 4096"));
        assert!(output.contains("process_resident_memory_bytes "));
    }

    #[test]
    fn area_load_metrics_expose_only_the_closed_result_vocabulary() {
        let metrics = RuntimeMetrics::default();
        let results = [
            AreaLoadMetricResult::Prepared,
            AreaLoadMetricResult::Installed,
            AreaLoadMetricResult::AlreadyResident,
            AreaLoadMetricResult::QueueFull,
            AreaLoadMetricResult::Timeout,
            AreaLoadMetricResult::Cancelled,
            AreaLoadMetricResult::PreparationFailed,
            AreaLoadMetricResult::InstallationFailed,
        ];
        for (index, result) in results.into_iter().enumerate() {
            metrics.area_load_finished(
                Duration::from_micros((index + 1) as u64),
                (index + 1) as u64,
                result,
            );
        }

        let output = metrics.prometheus(0);
        let result_series = output
            .lines()
            .filter(|line| line.starts_with("tiamud_area_loads_total{"))
            .collect::<Vec<_>>();
        assert_eq!(
            result_series,
            vec![
                "tiamud_area_loads_total{result=\"prepared\"} 1",
                "tiamud_area_loads_total{result=\"installed\"} 1",
                "tiamud_area_loads_total{result=\"already_resident\"} 1",
                "tiamud_area_loads_total{result=\"queue_full\"} 1",
                "tiamud_area_loads_total{result=\"timeout\"} 1",
                "tiamud_area_loads_total{result=\"cancelled\"} 1",
                "tiamud_area_loads_total{result=\"preparation_failed\"} 1",
                "tiamud_area_loads_total{result=\"installation_failed\"} 1",
            ]
        );
        assert!(output.contains("tiamud_area_load_latency_seconds_sum 0.000036"));
        assert!(output.contains("tiamud_area_bundle_bytes_total 36"));
        assert!(!output.contains("areaId"));
        assert!(!output.contains("roomId"));
        assert!(!output.contains("sha256"));
    }

    #[test]
    fn area_load_metrics_are_safe_for_concurrent_recording() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let workers = (0..4)
            .map(|_| {
                let metrics = metrics.clone();
                std::thread::spawn(move || {
                    for _ in 0..250 {
                        metrics.area_load_finished(
                            Duration::from_micros(10),
                            4,
                            AreaLoadMetricResult::Installed,
                        );
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }

        let output = metrics.prometheus(0);
        assert!(output.contains("tiamud_area_loads_total{result=\"installed\"} 1000"));
        assert!(output.contains("tiamud_area_load_latency_seconds_sum 0.010000"));
        assert!(output.contains("tiamud_area_bundle_bytes_total 4000"));
    }

    #[test]
    fn area_generation_metrics_use_only_closed_phase_and_result_labels() {
        let metrics = RuntimeMetrics::default();
        metrics.set_area_generation_phase(AreaGenerationMetricPhase::Committing);
        for result in [
            AreaGenerationMetricResult::Noop,
            AreaGenerationMetricResult::Activated,
            AreaGenerationMetricResult::RolledBack,
            AreaGenerationMetricResult::Busy,
            AreaGenerationMetricResult::Incompatible,
            AreaGenerationMetricResult::VerificationFailed,
            AreaGenerationMetricResult::PersistenceFailed,
            AreaGenerationMetricResult::Conflict,
            AreaGenerationMetricResult::InternalFailed,
        ] {
            metrics.area_generation_finished(Duration::from_micros(2), result);
        }

        let output = metrics.prometheus(0);
        assert!(output.contains("tiamud_area_generation_phase{state=\"active\"} 1"));
        assert!(output.contains("tiamud_area_generation_phase{state=\"committing\"} 0"));
        let results = output
            .lines()
            .filter(|line| line.starts_with("tiamud_area_generation_transitions_total{"))
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 9);
        assert!(results.iter().all(|line| line.ends_with(" 1")));
        assert!(output.contains("tiamud_area_generation_latency_seconds_sum 0.000018"));
        assert!(!output.contains("catalogSha256"));
        assert!(!output.contains("areaId"));
        assert!(!output.contains("generationId"));
    }
}
