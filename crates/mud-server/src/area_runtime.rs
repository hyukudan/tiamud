use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(test)]
use std::sync::{
    Condvar,
    atomic::{AtomicBool, Ordering},
};

use thiserror::Error;
use tiamud_core::{
    AreaGenerationId, AreaLoadRequest, PreparedCoreArea, PreparedCoreAreaBatch, World,
};
use tokio::sync::{Notify, Semaphore, oneshot};
use world_pack::{AreaPortalDirection, LocalAreaGeneration};

use crate::{
    operations::{AreaLoadMetricResult, RuntimeMetrics},
    runtime_content::RuntimeContentDelta,
    world_loader::{PinnedWorldContent, convert_compiled_area, map_direction},
};

const MAX_ACTIVE_PREPARATIONS: usize = 2;
const MAX_PENDING_PREPARATIONS: usize = 8;
const AREA_PREPARATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(crate) struct PreparedAreaBatch {
    pub(crate) core: PreparedCoreAreaBatch,
    pub(crate) runtime_by_area: BTreeMap<String, RuntimeContentDelta>,
    pub(crate) bundle_bytes: u64,
    pub(crate) closure_areas: usize,
}

pub(crate) struct CatalogWorldBootstrap {
    pub(crate) world: World,
    pub(crate) runtime_content: crate::runtime_content::RuntimeContent,
    pub(crate) created_dynamic_state: bool,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum AreaPreparationError {
    #[error("unknown area")]
    UnknownArea,
    #[error("area preparation queue is full")]
    QueueFull,
    #[error("area preparation timed out")]
    Timeout,
    #[error("area preparation was cancelled")]
    Cancelled,
    #[error("area preparation failed: {0}")]
    Invalid(String),
}

#[derive(Clone)]
pub(crate) struct AreaLoadCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    generation: Arc<LocalAreaGeneration>,
    pinned: Arc<PinnedWorldContent>,
    active_permits: Arc<Semaphore>,
    preparation_timeout: Duration,
    state: Mutex<CoordinatorState>,
    metrics: Option<Arc<RuntimeMetrics>>,
    #[cfg(test)]
    test_worker_gate: Mutex<Option<Arc<TestWorkerGate>>>,
}

#[cfg(test)]
struct TestWorkerGate {
    started: AtomicBool,
    released: Mutex<bool>,
    released_changed: Condvar,
}

#[cfg(test)]
impl TestWorkerGate {
    fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            released: Mutex::new(false),
            released_changed: Condvar::new(),
        }
    }

    fn wait(&self) {
        self.started.store(true, Ordering::Release);
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while !*released {
            released = self
                .released_changed
                .wait(released)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        self.released_changed.notify_all();
    }
}

struct ActivePreparationGuard {
    coordinator: AreaLoadCoordinator,
}

impl Drop for ActivePreparationGuard {
    fn drop(&mut self) {
        self.coordinator.set_active_delta(-1);
    }
}

#[derive(Default)]
struct CoordinatorState {
    active: usize,
    next_job_id: u64,
    next_waiter_id: u64,
    resident: BTreeSet<String>,
    jobs: HashMap<String, AreaJob>,
}

struct AreaJob {
    id: u64,
    waiters: Vec<AreaWaiter>,
    status: AreaJobStatus,
    cancel_queued: Arc<Notify>,
}

struct AreaWaiter {
    id: u64,
    send: oneshot::Sender<AreaPreparationResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AreaJobStatus {
    Queued,
    Running,
    TimedOut,
}

struct AreaWaiterGuard {
    coordinator: AreaLoadCoordinator,
    area_id: String,
    job_id: u64,
    waiter_id: u64,
    armed: bool,
}

impl Drop for AreaWaiterGuard {
    fn drop(&mut self) {
        if self.armed {
            self.coordinator
                .cancel_waiter(&self.area_id, self.job_id, self.waiter_id);
        }
    }
}

type AreaPreparationResult = Result<Arc<PreparedAreaBatch>, AreaPreparationError>;

impl AreaLoadCoordinator {
    pub(crate) fn new(
        generation: LocalAreaGeneration,
        pinned: PinnedWorldContent,
        metrics: Option<Arc<RuntimeMetrics>>,
    ) -> Self {
        Self {
            inner: Arc::new(CoordinatorInner {
                generation: Arc::new(generation),
                pinned: Arc::new(pinned),
                active_permits: Arc::new(Semaphore::new(MAX_ACTIVE_PREPARATIONS)),
                preparation_timeout: AREA_PREPARATION_TIMEOUT,
                state: Mutex::new(CoordinatorState::default()),
                metrics,
                #[cfg(test)]
                test_worker_gate: Mutex::new(None),
            }),
        }
    }

    pub(crate) fn generation(&self) -> &LocalAreaGeneration {
        &self.inner.generation
    }

    #[cfg(test)]
    fn configure_test_worker_gate(&mut self, timeout: Duration, gate: Arc<TestWorkerGate>) {
        let inner = Arc::get_mut(&mut self.inner).expect("test coordinator is not shared yet");
        inner.preparation_timeout = timeout;
        *inner
            .test_worker_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(gate);
    }

    pub(crate) async fn bootstrap_world(
        &self,
        full_generation: bool,
    ) -> Result<CatalogWorldBootstrap, AreaPreparationError> {
        let start_area = self
            .inner
            .generation
            .room_owner(&self.inner.pinned.start_room_id)
            .ok_or_else(|| AreaPreparationError::Invalid("start room has no catalog owner".into()))?
            .to_owned();
        let first = self.prepare(start_area).await?;
        let seed = self.inner.pinned.rng_seed.unwrap_or(0x5449_414d_5544);
        let (mut world, first_outcome) = World::from_initial_prepared_areas_with_configuration(
            &self.inner.pinned.start_room_id,
            first.core.clone(),
            |world| {
                world.set_rng_seed(seed);
                world.configure_progression(
                    self.inner.pinned.character_progression.clone(),
                    self.inner.pinned.abilities.clone(),
                )?;
                world.configure_social_emotes(self.inner.pinned.social_emotes.clone())?;
                Ok(())
            },
        )
        .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
        let mut runtime_content =
            crate::runtime_content::RuntimeContent::new(Vec::new(), Vec::new(), seed);
        if let Some(metrics) = &self.inner.metrics {
            runtime_content = runtime_content.with_operational_metrics(Arc::clone(metrics));
        }
        install_runtime_slices(
            &mut runtime_content,
            &first.runtime_by_area,
            &first_outcome.installed_area_ids,
        )?;
        self.mark_resident(&first_outcome.installed_area_ids);
        let mut created_dynamic_state = first_outcome.created_dynamic_state;

        if full_generation {
            let area_ids = self
                .inner
                .generation
                .catalog()
                .entries
                .iter()
                .map(|entry| entry.area_id.clone())
                .collect::<Vec<_>>();
            for area_id in area_ids {
                if self.is_resident(&area_id) {
                    continue;
                }
                let prepared = self.prepare(area_id).await?;
                let checkpoint = world.lifecycle_checkpoint();
                let outcome = match world.install_prepared_areas(prepared.core.clone()) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        world.restore_lifecycle_checkpoint(checkpoint);
                        return Err(AreaPreparationError::Invalid(error.to_string()));
                    }
                };
                if let Err(error) = install_runtime_slices(
                    &mut runtime_content,
                    &prepared.runtime_by_area,
                    &outcome.installed_area_ids,
                ) {
                    world.restore_lifecycle_checkpoint(checkpoint);
                    return Err(error);
                }
                self.mark_resident(&outcome.installed_area_ids);
                created_dynamic_state |= outcome.created_dynamic_state;
            }
        }
        if let Some(metrics) = &self.inner.metrics {
            metrics.set_resident_areas(world.active_area_count());
            metrics.set_deferred_area_exits(world.deferred_area_exit_count());
        }
        Ok(CatalogWorldBootstrap {
            world,
            runtime_content,
            created_dynamic_state,
        })
    }

    pub(crate) async fn prepare(&self, area_id: impl Into<String>) -> AreaPreparationResult {
        let area_id = area_id.into();
        if let Some(metrics) = &self.inner.metrics {
            metrics.record_area_load_demand();
        }
        if self.inner.generation.entry(&area_id).is_none() {
            if let Some(metrics) = &self.inner.metrics {
                metrics.record_area_load_rejection();
            }
            return Err(AreaPreparationError::UnknownArea);
        }
        if self.is_resident(&area_id) {
            return self.resident_noop_batch();
        }
        let (send, receive) = oneshot::channel();
        let (spawn_job_id, timed_out, waiter_id, job_id) = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let waiter_id = state.next_waiter_id;
            state.next_waiter_id = state.next_waiter_id.wrapping_add(1);
            if let Some(job) = state.jobs.get_mut(&area_id) {
                let job_id = job.id;
                if job.status == AreaJobStatus::TimedOut {
                    (None, true, waiter_id, job_id)
                } else {
                    job.waiters.push(AreaWaiter {
                        id: waiter_id,
                        send,
                    });
                    if let Some(metrics) = &self.inner.metrics {
                        metrics.record_area_load_coalesced();
                    }
                    (None, false, waiter_id, job_id)
                }
            } else {
                if state.jobs.len() >= MAX_ACTIVE_PREPARATIONS + MAX_PENDING_PREPARATIONS {
                    if let Some(metrics) = &self.inner.metrics {
                        metrics.record_area_load_rejection();
                        metrics.area_load_finished(
                            Duration::ZERO,
                            0,
                            AreaLoadMetricResult::QueueFull,
                        );
                    }
                    return Err(AreaPreparationError::QueueFull);
                }
                let job_id = state.next_job_id;
                state.next_job_id = state.next_job_id.wrapping_add(1);
                state.jobs.insert(
                    area_id.clone(),
                    AreaJob {
                        id: job_id,
                        waiters: vec![AreaWaiter {
                            id: waiter_id,
                            send,
                        }],
                        status: AreaJobStatus::Queued,
                        cancel_queued: Arc::new(Notify::new()),
                    },
                );
                (Some(job_id), false, waiter_id, job_id)
            }
        };
        self.update_queue_metrics();
        if timed_out {
            if let Some(metrics) = &self.inner.metrics {
                metrics.record_area_load_rejection();
            }
            return Err(AreaPreparationError::Timeout);
        }
        if let Some(spawn_job_id) = spawn_job_id {
            let coordinator = self.clone();
            let job_area_id = area_id.clone();
            tokio::spawn(async move {
                coordinator.run_job(job_area_id, spawn_job_id).await;
            });
        }
        let mut guard = AreaWaiterGuard {
            coordinator: self.clone(),
            area_id,
            job_id,
            waiter_id,
            armed: true,
        };
        let result = receive
            .await
            .unwrap_or(Err(AreaPreparationError::Cancelled));
        guard.armed = false;
        result
    }

    async fn run_job(&self, area_id: String, job_id: u64) {
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + self.inner.preparation_timeout;
        let prepared_area_id = area_id.clone();
        let Some(cancel_queued) = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .jobs
            .get(&area_id)
            .filter(|job| job.id == job_id)
            .map(|job| Arc::clone(&job.cancel_queued))
        else {
            return;
        };
        let permit = tokio::select! {
            permit = Arc::clone(&self.inner.active_permits).acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => {
                    self.finish_job(
                        &area_id,
                        job_id,
                        Err(AreaPreparationError::Cancelled),
                        started,
                    );
                    return;
                }
            },
            _ = cancel_queued.notified() => return,
            _ = tokio::time::sleep_until(deadline) => {
                self.finish_job(
                    &area_id,
                    job_id,
                    Err(AreaPreparationError::Timeout),
                    started,
                );
                return;
            }
        };
        let should_run = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(job) = state.jobs.get_mut(&area_id).filter(|job| job.id == job_id) else {
                return;
            };
            if job.waiters.is_empty() || job.status != AreaJobStatus::Queued {
                false
            } else {
                job.status = AreaJobStatus::Running;
                true
            }
        };
        if !should_run || !self.has_live_waiter(&prepared_area_id, job_id) {
            self.remove_finished_tombstone(&area_id, job_id);
            return;
        }
        self.set_active_delta(1);
        let active = ActivePreparationGuard {
            coordinator: self.clone(),
        };
        let generation = Arc::clone(&self.inner.generation);
        let pinned = Arc::clone(&self.inner.pinned);
        let resident = Arc::new(self.resident_snapshot());
        #[cfg(test)]
        let test_worker_gate = self
            .inner
            .test_worker_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _active = active;
            #[cfg(test)]
            if let Some(gate) = test_worker_gate {
                gate.wait();
            }
            prepare_area_closure(&generation, &pinned, &prepared_area_id, &resident)
        });
        let mut result = match tokio::time::timeout_at(deadline, &mut worker).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(AreaPreparationError::Invalid(error.to_string())),
            Err(_) => {
                self.timeout_running_job(&area_id, job_id, started);
                let _ = worker.await;
                self.remove_finished_tombstone(&area_id, job_id);
                return;
            }
        };
        if result.is_ok() && !self.has_live_waiter(&area_id, job_id) {
            result = Err(AreaPreparationError::Cancelled);
        }
        self.finish_job(&area_id, job_id, result, started);
    }

    fn finish_job(
        &self,
        area_id: &str,
        job_id: u64,
        result: AreaPreparationResult,
        started: Instant,
    ) {
        let waiters = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.jobs.get(area_id).is_none_or(|job| job.id != job_id) {
                return;
            }
            state
                .jobs
                .remove(area_id)
                .map(|job| {
                    job.waiters
                        .into_iter()
                        .map(|waiter| waiter.send)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let (metric_result, bytes) = match &result {
            Ok(batch) => (AreaLoadMetricResult::Prepared, batch.bundle_bytes),
            Err(AreaPreparationError::Timeout) => (AreaLoadMetricResult::Timeout, 0),
            Err(AreaPreparationError::Cancelled) => (AreaLoadMetricResult::Cancelled, 0),
            Err(_) => (AreaLoadMetricResult::PreparationFailed, 0),
        };
        if let Some(metrics) = &self.inner.metrics {
            if let Ok(batch) = &result {
                metrics.record_area_closure(batch.closure_areas);
            }
            metrics.area_load_finished(started.elapsed(), bytes, metric_result);
        }
        self.update_queue_metrics();
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    fn timeout_running_job(&self, area_id: &str, job_id: u64, started: Instant) {
        let waiters = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(job) = state.jobs.get_mut(area_id).filter(|job| job.id == job_id) else {
                return;
            };
            job.status = AreaJobStatus::TimedOut;
            std::mem::take(&mut job.waiters)
                .into_iter()
                .map(|waiter| waiter.send)
                .collect::<Vec<_>>()
        };
        if let Some(metrics) = &self.inner.metrics {
            metrics.area_load_finished(started.elapsed(), 0, AreaLoadMetricResult::Timeout);
        }
        for waiter in waiters {
            let _ = waiter.send(Err(AreaPreparationError::Timeout));
        }
        self.update_queue_metrics();
    }

    fn remove_finished_tombstone(&self, area_id: &str, job_id: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.jobs.get(area_id).is_some_and(|job| job.id == job_id) {
            state.jobs.remove(area_id);
        }
        drop(state);
        self.update_queue_metrics();
    }

    fn set_active_delta(&self, delta: isize) {
        let active = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if delta > 0 {
                state.active = state.active.saturating_add(delta as usize);
            } else {
                state.active = state.active.saturating_sub(delta.unsigned_abs());
            }
            state.active
        };
        if let Some(metrics) = &self.inner.metrics {
            metrics.set_preparing_areas(active);
        }
        self.update_queue_metrics();
    }

    fn has_live_waiter(&self, area_id: &str, job_id: u64) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .jobs
            .get(area_id)
            .filter(|job| job.id == job_id)
            .is_some_and(|job| job.waiters.iter().any(|waiter| !waiter.send.is_closed()))
    }

    fn cancel_waiter(&self, area_id: &str, job_id: u64, waiter_id: u64) {
        let cancel = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(job) = state.jobs.get_mut(area_id).filter(|job| job.id == job_id) else {
                return;
            };
            job.waiters.retain(|waiter| waiter.id != waiter_id);
            if job.waiters.is_empty() && job.status == AreaJobStatus::Queued {
                let cancel = Arc::clone(&job.cancel_queued);
                state.jobs.remove(area_id);
                Some(cancel)
            } else {
                None
            }
        };
        if let Some(cancel) = cancel {
            cancel.notify_one();
        }
        self.update_queue_metrics();
    }

    pub(crate) fn is_resident(&self, area_id: &str) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .resident
            .contains(area_id)
    }

    pub(crate) fn mark_resident(&self, area_ids: &[String]) {
        let count = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.resident.extend(area_ids.iter().cloned());
            state.resident.len()
        };
        if let Some(metrics) = &self.inner.metrics {
            metrics.set_resident_areas(count);
        }
    }

    fn resident_snapshot(&self) -> BTreeSet<String> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .resident
            .clone()
    }

    fn resident_noop_batch(&self) -> AreaPreparationResult {
        let generation_id =
            AreaGenerationId::from_catalog_sha256(self.inner.generation.generation_id().to_owned())
                .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
        Ok(Arc::new(PreparedAreaBatch {
            core: PreparedCoreAreaBatch {
                generation_id,
                areas: Vec::new(),
                deferred_exits: Vec::new(),
            },
            runtime_by_area: BTreeMap::new(),
            bundle_bytes: 0,
            closure_areas: 0,
        }))
    }

    fn update_queue_metrics(&self) {
        let pending = {
            let state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.jobs.len().saturating_sub(state.active)
        };
        if let Some(metrics) = &self.inner.metrics {
            metrics.set_pending_areas(pending);
        }
    }
}

fn install_runtime_slices(
    runtime_content: &mut crate::runtime_content::RuntimeContent,
    runtime_by_area: &BTreeMap<String, RuntimeContentDelta>,
    installed_area_ids: &[String],
) -> Result<(), AreaPreparationError> {
    for area_id in installed_area_ids {
        let delta = runtime_by_area.get(area_id).cloned().ok_or_else(|| {
            AreaPreparationError::Invalid("prepared area has no runtime slice".into())
        })?;
        runtime_content
            .install_delta(delta)
            .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
    }
    Ok(())
}

fn prepare_area_closure(
    generation: &LocalAreaGeneration,
    pinned: &PinnedWorldContent,
    area_id: &str,
    resident: &BTreeSet<String>,
) -> AreaPreparationResult {
    let closure = generation
        .hard_dependency_closure(area_id)
        .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
    let closure_ids = closure
        .iter()
        .map(|entry| entry.area_id.as_str())
        .collect::<BTreeSet<_>>();
    let closure_areas = closure.len();
    let entries = closure
        .into_iter()
        .filter(|entry| !resident.contains(&entry.area_id))
        .collect::<Vec<_>>();
    let generation_id = AreaGenerationId::from_catalog_sha256(generation.generation_id())
        .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
    let mut areas = Vec::with_capacity(entries.len());
    let mut runtime_by_area = BTreeMap::new();
    let mut bundle_bytes = 0_u64;
    let mut deferred_exits = Vec::new();

    for entry in entries {
        bundle_bytes = bundle_bytes
            .checked_add(entry.stored_bytes)
            .ok_or_else(|| AreaPreparationError::Invalid("bundle byte sum overflow".into()))?;
        let bundle = generation
            .read_bundle_exact(&entry.area_id)
            .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
        let loaded = convert_compiled_area(bundle.compiled_area, pinned)
            .map_err(|error| AreaPreparationError::Invalid(error.to_string()))?;
        areas.push(PreparedCoreArea {
            area_id: entry.area_id.clone(),
            revision: entry.revision,
            bundle_sha256: entry.bundle_sha256.clone(),
            rooms: loaded.definition.rooms,
            room_presentations: loaded.room_presentations,
            item_prototypes: loaded.definition.item_prototypes,
            initial_item_instances: loaded.definition.initial_item_instances,
            npcs: loaded.npcs,
            npc_ability_sets: loaded.npc_ability_sets,
            loot_tables: loaded.loot_tables,
            loot_sources: loaded.loot_sources,
            affixes: loaded.affixes,
            affix_pools: loaded.affix_pools,
            room_light: loaded.room_light,
            hidden_items: loaded.hidden_items,
            hidden_npcs: loaded.hidden_npcs,
            secret_exits: loaded.secret_exits,
        });
        runtime_by_area.insert(entry.area_id.clone(), loaded.runtime_content);

        for portal in &entry.portals {
            if portal.direction == AreaPortalDirection::Outgoing
                && !closure_ids.contains(portal.peer_area_id.as_str())
            {
                deferred_exits.push(AreaLoadRequest {
                    generation_id: generation_id.clone(),
                    source_room_id: portal.source_room_id.clone(),
                    direction: map_direction(portal.source_exit),
                    target_room_id: portal.target_room_id.clone(),
                    target_area_id: portal.peer_area_id.clone(),
                    minimum_target_revision: portal.minimum_peer_revision,
                });
            }
        }
    }
    deferred_exits.sort_by(|left, right| {
        left.source_room_id
            .cmp(&right.source_room_id)
            .then_with(|| left.direction.cmp(&right.direction))
            .then_with(|| left.target_room_id.cmp(&right.target_room_id))
    });
    deferred_exits.dedup_by(|left, right| {
        left.source_room_id == right.source_room_id && left.direction == right.direction
    });
    let batch = Arc::new(PreparedAreaBatch {
        core: PreparedCoreAreaBatch {
            generation_id,
            areas,
            deferred_exits,
        },
        runtime_by_area,
        bundle_bytes,
        closure_areas,
    });
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;
    use tokio::sync::oneshot;
    use world_pack::{
        AreaPortalDirection, VerifiedAreaRelease, build_area_release, compose_world_project,
        local_area_bundle_path, local_area_catalog_path, local_area_world_path,
        parse_area_release_manifest,
    };

    use super::*;
    use crate::{
        persistence::{CharacterStore, SqliteCharacterStore},
        world_actor::{WorldHandle, WorldSchedulerConfig},
        world_loader::parse_pinned_world_content,
    };

    struct PublishedRegistry {
        _temp: TempDir,
        registry: PathBuf,
        catalog_path: PathBuf,
        release: VerifiedAreaRelease,
    }

    impl PublishedRegistry {
        fn real_lantern_square() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let composed =
                compose_world_project(root.join("worlds/lantern-square.project.json")).unwrap();
            let manifest_bytes =
                fs::read(root.join("worlds/lantern-square.area-release.json")).unwrap();
            let manifest = parse_area_release_manifest(&manifest_bytes).unwrap();
            let release = build_area_release(&composed, &manifest, None).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let registry = temp.path().to_owned();

            let world_path = local_area_world_path(
                &registry,
                &release.catalog.world_id,
                &release.catalog.world_artifact_sha256,
            );
            fs::create_dir_all(world_path.parent().unwrap()).unwrap();
            fs::write(world_path, &release.world_artifact).unwrap();
            for (area_id, bytes) in &release.bundles {
                let entry = release
                    .catalog
                    .entries
                    .iter()
                    .find(|entry| &entry.area_id == area_id)
                    .unwrap();
                let path = local_area_bundle_path(
                    &registry,
                    &release.catalog.world_id,
                    area_id,
                    &entry.bundle_sha256,
                );
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, bytes).unwrap();
            }
            let catalog_path = local_area_catalog_path(
                &registry,
                &release.catalog.world_id,
                &release.catalog_sha256,
            );
            fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
            fs::write(&catalog_path, &release.catalog_bytes).unwrap();

            Self {
                _temp: temp,
                registry,
                catalog_path,
                release,
            }
        }

        fn coordinator(&self) -> AreaLoadCoordinator {
            let generation =
                LocalAreaGeneration::open_exact(&self.registry, &self.catalog_path).unwrap();
            let pinned = parse_pinned_world_content(generation.world_artifact()).unwrap();
            AreaLoadCoordinator::new(generation, pinned, None)
        }

        fn bundle_path(&self, area_id: &str) -> PathBuf {
            let entry = self
                .release
                .catalog
                .entries
                .iter()
                .find(|entry| entry.area_id == area_id)
                .unwrap();
            local_area_bundle_path(
                &self.registry,
                &self.release.catalog.world_id,
                area_id,
                &entry.bundle_sha256,
            )
        }
    }

    #[tokio::test]
    async fn prepares_real_area_with_exact_bundle_bytes_and_deferred_portals() {
        let published = PublishedRegistry::real_lantern_square();
        let area_id = "archive";
        let entry = published
            .release
            .catalog
            .entries
            .iter()
            .find(|entry| entry.area_id == area_id)
            .unwrap();
        assert!(entry.hard_dependencies.is_empty());
        let outgoing = entry
            .portals
            .iter()
            .filter(|portal| portal.direction == AreaPortalDirection::Outgoing)
            .collect::<Vec<_>>();
        assert_eq!(outgoing.len(), 1);
        let on_disk_bytes = fs::metadata(published.bundle_path(area_id)).unwrap().len();
        let coordinator = published.coordinator();

        let batch = coordinator.prepare(area_id).await.unwrap();

        assert_eq!(batch.core.areas.len(), 1);
        assert_eq!(batch.bundle_bytes, entry.stored_bytes);
        assert_eq!(batch.bundle_bytes, on_disk_bytes);
        assert_eq!(
            batch.core.generation_id.as_str(),
            published.release.catalog_sha256
        );
        let area = &batch.core.areas[0];
        assert_eq!(area.area_id, entry.area_id);
        assert_eq!(area.revision, entry.revision);
        assert_eq!(area.bundle_sha256, entry.bundle_sha256);
        assert_eq!(batch.core.deferred_exits.len(), outgoing.len());
        for portal in outgoing {
            assert!(batch.core.deferred_exits.iter().any(|request| {
                request.generation_id == batch.core.generation_id
                    && request.source_room_id == portal.source_room_id
                    && request.direction == map_direction(portal.source_exit)
                    && request.target_room_id == portal.target_room_id
                    && request.target_area_id == portal.peer_area_id
                    && request.minimum_target_revision == portal.minimum_peer_revision
            }));
        }
    }

    #[tokio::test]
    async fn coalesces_concurrent_requests_into_the_same_prepared_batch() {
        let published = PublishedRegistry::real_lantern_square();
        let coordinator = published.coordinator();

        let (first, second) =
            tokio::join!(coordinator.prepare("square"), coordinator.prepare("square"));
        let first = first.unwrap();
        let second = second.unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.core, second.core);
    }

    #[tokio::test]
    async fn timed_out_worker_keeps_its_key_and_physical_slot_until_it_really_finishes() {
        let published = PublishedRegistry::real_lantern_square();
        let mut coordinator = published.coordinator();
        let gate = Arc::new(TestWorkerGate::new());
        coordinator.configure_test_worker_gate(Duration::from_millis(50), Arc::clone(&gate));
        let first_coordinator = coordinator.clone();
        let first = tokio::spawn(async move { first_coordinator.prepare("square").await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !gate.started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        assert!(matches!(
            first.await.unwrap(),
            Err(AreaPreparationError::Timeout)
        ));
        {
            let state = coordinator
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(state.active, 1);
            assert_eq!(state.jobs["square"].status, AreaJobStatus::TimedOut);
        }
        assert!(matches!(
            coordinator.prepare("square").await,
            Err(AreaPreparationError::Timeout)
        ));
        assert_eq!(coordinator.inner.active_permits.available_permits(), 1);

        gate.release();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let done = {
                    let state = coordinator
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    state.active == 0 && !state.jobs.contains_key("square")
                };
                if done {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            coordinator.inner.active_permits.available_permits(),
            MAX_ACTIVE_PREPARATIONS
        );
    }

    #[tokio::test]
    async fn rejects_an_eleventh_distinct_job_when_active_and_pending_capacity_is_full() {
        let published = PublishedRegistry::real_lantern_square();
        let coordinator = published.coordinator();
        {
            let mut state = coordinator
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for index in 0..(MAX_ACTIVE_PREPARATIONS + MAX_PENDING_PREPARATIONS) {
                let (send, _receive) = oneshot::channel();
                state.jobs.insert(
                    format!("occupied-{index}"),
                    AreaJob {
                        id: index as u64,
                        waiters: vec![AreaWaiter {
                            id: index as u64,
                            send,
                        }],
                        status: AreaJobStatus::Queued,
                        cancel_queued: Arc::new(Notify::new()),
                    },
                );
            }
            assert_eq!(
                state.jobs.len(),
                MAX_ACTIVE_PREPARATIONS + MAX_PENDING_PREPARATIONS
            );
        }

        let error = coordinator.prepare("square").await.unwrap_err();

        assert_eq!(error, AreaPreparationError::QueueFull);
    }

    #[test]
    fn stale_job_identity_cannot_adopt_or_remove_a_requeued_key() {
        let published = PublishedRegistry::real_lantern_square();
        let coordinator = published.coordinator();
        let (send, mut receive) = oneshot::channel();
        coordinator
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .jobs
            .insert(
                "square".into(),
                AreaJob {
                    id: 22,
                    waiters: vec![AreaWaiter { id: 7, send }],
                    status: AreaJobStatus::Queued,
                    cancel_queued: Arc::new(Notify::new()),
                },
            );

        assert!(!coordinator.has_live_waiter("square", 11));
        coordinator.cancel_waiter("square", 11, 7);
        coordinator.timeout_running_job("square", 11, Instant::now());
        coordinator.finish_job(
            "square",
            11,
            Err(AreaPreparationError::Cancelled),
            Instant::now(),
        );
        coordinator.remove_finished_tombstone("square", 11);

        let state = coordinator
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(state.jobs["square"].id, 22);
        assert_eq!(state.jobs["square"].status, AreaJobStatus::Queued);
        drop(state);
        assert!(matches!(
            receive.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        coordinator.remove_finished_tombstone("square", 22);
    }

    #[tokio::test]
    async fn cancelling_the_last_waiter_discards_the_queued_job() {
        let published = PublishedRegistry::real_lantern_square();
        let coordinator = published.coordinator();
        let first = coordinator.inner.active_permits.acquire().await.unwrap();
        let second = coordinator.inner.active_permits.acquire().await.unwrap();
        let request_coordinator = coordinator.clone();
        let request = tokio::spawn(async move { request_coordinator.prepare("square").await });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if coordinator
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .jobs
                    .contains_key("square")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        request.abort();
        let _ = request.await;
        drop(first);
        drop(second);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !coordinator
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .jobs
                    .contains_key("square")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn keeps_the_opened_generation_fixed_when_catalog_bytes_change_on_disk() {
        let published = PublishedRegistry::real_lantern_square();
        let expected_generation = published.release.catalog_sha256.clone();
        let coordinator = published.coordinator();
        fs::write(&published.catalog_path, b"changed after generation open").unwrap();

        let batch = coordinator.prepare("square").await.unwrap();

        assert_eq!(batch.core.generation_id.as_str(), expected_generation);
    }

    #[tokio::test]
    async fn confirmed_resident_root_is_not_reread_from_its_bundle() {
        let published = PublishedRegistry::real_lantern_square();
        let coordinator = published.coordinator();
        let start_owner = coordinator
            .generation()
            .room_owner(&published.release.catalog.start_room_id)
            .unwrap()
            .to_owned();
        let bootstrap = coordinator.bootstrap_world(false).await.unwrap();
        assert!(bootstrap.world.area_is_active(&start_owner));
        let path = published.bundle_path(&start_owner);
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(path, bytes).unwrap();

        let batch = coordinator.prepare(start_owner).await.unwrap();

        assert!(batch.core.areas.is_empty());
        assert_eq!(batch.bundle_bytes, 0);
    }

    #[tokio::test]
    async fn reports_tampered_bundle_as_internal_preparation_failure() {
        let published = PublishedRegistry::real_lantern_square();
        let coordinator = published.coordinator();
        let bundle_path = published.bundle_path("square");
        let mut bytes = fs::read(&bundle_path).unwrap();
        bytes.push(b' ');
        fs::write(bundle_path, bytes).unwrap();

        let error = coordinator.prepare("square").await.unwrap_err();

        assert!(matches!(error, AreaPreparationError::Invalid(_)));
    }

    #[tokio::test]
    async fn deferred_move_loads_installs_and_retries_once_end_to_end() {
        let published = PublishedRegistry::real_lantern_square();
        let generation =
            LocalAreaGeneration::open_exact(&published.registry, &published.catalog_path).unwrap();
        let pinned = parse_pinned_world_content(generation.world_artifact()).unwrap();
        let metrics = Arc::new(RuntimeMetrics::default());
        let coordinator = AreaLoadCoordinator::new(generation, pinned, Some(metrics.clone()));
        let database = tempfile::tempdir().unwrap();
        let store: Arc<dyn CharacterStore> = Arc::new(
            SqliteCharacterStore::open(database.path().join("area-e2e.sqlite3"))
                .await
                .unwrap(),
        );
        let recovery = store.load_startup_world_recovery().await.unwrap();
        let bootstrap = coordinator.bootstrap_world(false).await.unwrap();
        let confirm_initial_dynamic = bootstrap.created_dynamic_state;
        let world = WorldHandle::spawn_with_scheduler_content_areas_and_recovery(
            bootstrap.world,
            store.clone(),
            WorldSchedulerConfig::default(),
            bootstrap.runtime_content,
            Some(coordinator.clone()),
            recovery,
            confirm_initial_dynamic,
        )
        .await
        .unwrap();
        let entered = world.enter("Area Walker".into()).await.unwrap();
        let portal = published
            .release
            .catalog
            .entries
            .iter()
            .find(|entry| entry.area_id == "square")
            .unwrap()
            .portals
            .iter()
            .find(|portal| {
                portal.direction == AreaPortalDirection::Outgoing
                    && portal.source_room_id == published.release.catalog.start_room_id
            })
            .unwrap();
        let target_title = coordinator
            .generation()
            .fallback()
            .world
            .rooms
            .iter()
            .find(|room| room.id == portal.target_room_id)
            .unwrap()
            .title
            .clone();

        let moved = world
            .apply(
                entered.session_id,
                tiamud_core::PlayerAction::Move(map_direction(portal.source_exit)),
            )
            .await
            .unwrap();

        assert_eq!(moved.scene.title, target_title);
        let output = metrics.prometheus(world.queue_depth());
        assert!(output.contains("tiamud_areas{state=\"resident\"} 3"));
        assert!(output.contains("tiamud_area_loads_total{result=\"prepared\"} 2"));
        assert!(output.contains("tiamud_area_loads_total{result=\"installed\"} 1"));
        assert!(output.contains("tiamud_area_loads_total{result=\"already_resident\"} 0"));
        assert!(output.contains("tiamud_area_load_demands_total 2"));
        assert!(output.contains("tiamud_areas{state=\"pending\"} 0"));
        world.shutdown().await.unwrap();

        let recovery = store.load_startup_world_recovery().await.unwrap();
        assert!(recovery.requires_full_generation());
        let generation =
            LocalAreaGeneration::open_exact(&published.registry, &published.catalog_path).unwrap();
        let pinned = parse_pinned_world_content(generation.world_artifact()).unwrap();
        let reboot_coordinator = AreaLoadCoordinator::new(generation, pinned, None);
        let reboot = reboot_coordinator.bootstrap_world(true).await.unwrap();
        assert_eq!(
            reboot.world.active_area_count(),
            published.release.catalog.entries.len()
        );
        let rebooted = WorldHandle::spawn_with_scheduler_content_areas_and_recovery(
            reboot.world,
            store.clone(),
            WorldSchedulerConfig::default(),
            reboot.runtime_content,
            Some(reboot_coordinator),
            recovery,
            false,
        )
        .await
        .unwrap();
        rebooted.shutdown().await.unwrap();
        store.shutdown().await.unwrap();
    }

    #[test]
    fn bounded_public_errors_do_not_render_content_identifiers() {
        let secret_area = "private:area-id";
        let secret_room = "private:room-id";
        let secret_digest = "a".repeat(64);
        for error in [
            AreaPreparationError::QueueFull,
            AreaPreparationError::Timeout,
            AreaPreparationError::UnknownArea,
        ] {
            let rendered = error.to_string();
            assert!(!rendered.contains(secret_area));
            assert!(!rendered.contains(secret_room));
            assert!(!rendered.contains(&secret_digest));
        }
    }
}
