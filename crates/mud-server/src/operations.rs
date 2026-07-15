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

/// Bounded per-IP token bucket used for password-bearing endpoints.
#[derive(Debug)]
pub struct AuthRateLimiter {
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
    refill_per_second: f64,
    burst: f64,
}

impl AuthRateLimiter {
    pub fn per_minute(attempts: u32, burst: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            refill_per_second: f64::from(attempts.max(1)) / 60.0,
            burst: f64::from(burst.max(1)),
        }
    }

    pub fn check(&self, address: IpAddr, now: Instant) -> bool {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !buckets.contains_key(&address)
            && buckets.len() >= MAX_TRACKED_AUTH_ADDRESSES
            && let Some(oldest) = buckets
                .iter()
                .min_by_key(|(_, bucket)| bucket.last_seen)
                .map(|(address, _)| *address)
        {
            buckets.remove(&oldest);
        }

        let bucket = buckets.entry(address).or_insert(Bucket {
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
        metrics.database_transaction_finished(Duration::from_millis(10), false);
        metrics.set_database_queue_depth(3);
        metrics.tick_finished(Duration::from_millis(5), true);
        metrics.script_finished(Duration::from_millis(2), ScriptMetricOutcome::Timeout);
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
        assert!(output.contains("tiamud_database_transactions_total 1"));
        assert!(output.contains("tiamud_database_errors_total 1"));
        assert!(output.contains("tiamud_database_latency_seconds_sum 0.010000"));
        assert!(output.contains("tiamud_database_queue_depth 3"));
        assert!(output.contains("tiamud_ticks_total 1"));
        assert!(output.contains("tiamud_tick_latency_seconds_sum 0.005000"));
        assert!(output.contains("tiamud_scripts_total 1"));
        assert!(output.contains("tiamud_script_timeouts_total 1"));
        assert!(output.contains("tiamud_script_latency_seconds_sum 0.002000"));
        assert!(output.contains("process_resident_memory_bytes "));
    }
}
