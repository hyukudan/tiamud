//! Lua StoryPack scripts in server sessions (ADR 0034, phase S6 wiring).
//!
//! Story scripts are pure functions run by `tiamud-story-script` in the
//! quota-limited Lua sandbox. A Lua call is synchronous CPU work, so an
//! instance of a scripted story never runs story-core on a Tokio worker:
//! every operation that may evaluate a script is moved, together with the
//! instance's single-writer state, onto a bounded blocking pool and handed
//! back when it finishes. The pool caps concurrent jobs globally and bounds
//! the number of admitted (running plus waiting) jobs; past that bound an
//! input is refused with `busy` before anything runs.
//!
//! Each job carries an aggregate wall-clock budget per input on top of the
//! sandbox quotas of every call: a call never gets more wall time than what
//! remains of the budget, and a call that starts with nothing left fails.
//! Failing only ever fails the step; story-core leaves the state untouched.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use tiamud_story_core::{Effect, ScriptCall, ScriptFailure, ScriptHost};
use tiamud_story_script::{LuaStoryHost, story_script_limits};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Default aggregate wall-clock budget of the scripts of one input.
pub const DEFAULT_SCRIPT_BUDGET: Duration = Duration::from_millis(100);
/// Accepted range of `TIAMUD_STORY_SCRIPT_BUDGET_MS`: the upper end is the
/// worst case of 16 calls × 25 ms that story-core and the sandbox allow.
pub const MIN_SCRIPT_BUDGET_MS: u64 = 10;
pub const MAX_SCRIPT_BUDGET_MS: u64 = 400;
/// Concurrent script jobs (`TIAMUD_STORY_SCRIPT_WORKERS`).
pub const MAX_SCRIPT_WORKERS: usize = 64;
/// Default concurrency when unset: the host's parallelism, at most this.
const DEFAULT_WORKERS_CAP: usize = 16;
/// Admitted jobs waiting for a worker (`TIAMUD_STORY_SCRIPT_QUEUE`).
pub const MAX_SCRIPT_QUEUE: usize = 1_024;
/// Consecutive script failures after which an instance closes (its last
/// good state is autosaved).
pub const SCRIPT_FAILURES_BEFORE_CLOSE: u32 = 3;

/// Settings of the S6 script runtime. Disabled by default: a catalog pack
/// that declares scripts is then refused at startup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptSettings {
    pub enabled: bool,
    /// Aggregate wall-clock budget of every script call of one input.
    pub budget: Duration,
    /// Script jobs that may run at the same time (blocking threads).
    pub workers: usize,
    /// Further admitted jobs that may wait for a worker; beyond this an
    /// input is refused with `busy`.
    pub queue: usize,
    /// Consecutive failures that close an instance.
    pub max_failures: u32,
}

impl Default for ScriptSettings {
    fn default() -> Self {
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .clamp(1, DEFAULT_WORKERS_CAP);
        Self {
            enabled: false,
            budget: DEFAULT_SCRIPT_BUDGET,
            workers,
            queue: workers.saturating_mul(4),
            max_failures: SCRIPT_FAILURES_BEFORE_CLOSE,
        }
    }
}

/// Content-free script counters (never labelled by story or script id).
#[derive(Debug, Default)]
pub struct ScriptCounters {
    pub invocations: AtomicU64,
    pub failures: AtomicU64,
    pub budget_exceeded: AtomicU64,
    pub busy: AtomicU64,
}

/// [`ScriptHost`] of one instance: the Lua host under the per-input budget.
/// [`BudgetHost::begin`] starts the budget of a new input.
#[derive(Debug)]
pub struct BudgetHost {
    budget: Duration,
    /// Deadline of the current input. A mutex (never contended: one
    /// instance runs one job at a time) keeps the instance `Sync`.
    deadline: Mutex<Option<Instant>>,
    counters: Arc<ScriptCounters>,
}

impl BudgetHost {
    pub fn new(budget: Duration, counters: Arc<ScriptCounters>) -> Self {
        Self {
            budget,
            deadline: Mutex::new(None),
            counters,
        }
    }

    /// Starts the aggregate budget of one input (one pool job).
    pub fn begin(&self) -> Instant {
        let now = Instant::now();
        let deadline = now.checked_add(self.budget).unwrap_or(now);
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(deadline);
        deadline
    }

    fn limited<T>(
        &self,
        call: impl FnOnce(&LuaStoryHost) -> Result<T, ScriptFailure>,
    ) -> Result<T, ScriptFailure> {
        let current = *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = current.unwrap_or_else(|| self.begin());
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.counters
                .budget_exceeded
                .fetch_add(1, Ordering::Relaxed);
            self.counters.failures.fetch_add(1, Ordering::Relaxed);
            return Err(ScriptFailure::new(format!(
                "aggregate script budget of {} ms per input exceeded",
                self.budget.as_millis()
            )));
        }
        let mut limits = story_script_limits();
        limits.max_wall_time = limits.max_wall_time.min(remaining);
        self.counters.invocations.fetch_add(1, Ordering::Relaxed);
        let result = call(&LuaStoryHost::with_limits(limits));
        if result.is_err() {
            self.counters.failures.fetch_add(1, Ordering::Relaxed);
            if Instant::now() >= deadline {
                self.counters
                    .budget_exceeded
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }
}

impl ScriptHost for BudgetHost {
    fn check(&self, call: &ScriptCall<'_>) -> Result<bool, ScriptFailure> {
        self.limited(|host| host.check(call))
    }

    fn run(&self, call: &ScriptCall<'_>) -> Result<Vec<Effect>, ScriptFailure> {
        self.limited(|host| host.run(call))
    }
}

/// The pool is saturated: nothing ran.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PoolBusy;

/// Global bounded pool for script jobs.
#[derive(Debug)]
pub struct ScriptPool {
    permits: Arc<Semaphore>,
    admitted: Arc<AtomicUsize>,
    capacity: usize,
    counters: Arc<ScriptCounters>,
}

/// One admitted job: counts against the pool bound until dropped.
#[derive(Debug)]
pub struct Admission {
    admitted: Arc<AtomicUsize>,
    permits: Arc<Semaphore>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.admitted.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ScriptPool {
    pub fn new(settings: &ScriptSettings, counters: Arc<ScriptCounters>) -> Self {
        let workers = settings.workers.clamp(1, MAX_SCRIPT_WORKERS);
        Self {
            permits: Arc::new(Semaphore::new(workers)),
            admitted: Arc::new(AtomicUsize::new(0)),
            capacity: workers.saturating_add(settings.queue.min(MAX_SCRIPT_QUEUE)),
            counters,
        }
    }

    /// Admits one job or refuses it (counted as busy) when the pool already
    /// holds its bound of running plus waiting jobs.
    pub fn admit(&self) -> Result<Admission, PoolBusy> {
        let admitted = self
            .admitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < self.capacity).then_some(count + 1)
            });
        match admitted {
            Ok(_) => Ok(Admission {
                admitted: self.admitted.clone(),
                permits: self.permits.clone(),
            }),
            Err(_) => {
                self.counters.busy.fetch_add(1, Ordering::Relaxed);
                Err(PoolBusy)
            }
        }
    }
}

impl Admission {
    /// Waits for a worker and runs `job` on a blocking thread. `None` only
    /// when the blocking task was lost (runtime shutdown or a panic that
    /// escaped the job).
    pub async fn run<R: Send + 'static>(
        self,
        job: impl FnOnce() -> R + Send + 'static,
    ) -> Option<R> {
        let permit: OwnedSemaphorePermit = self.permits.clone().acquire_owned().await.ok()?;
        tokio::task::spawn_blocking(move || {
            let _admission = self;
            let _permit = permit;
            job()
        })
        .await
        .ok()
    }
}
