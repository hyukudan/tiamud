//! Backend-agnostic decoding and validation of durable startup state: the
//! domain-event ledger, runtime snapshots, character identity sets, startup
//! gameplay aggregates and area generations/checkpoints. Both stores read rows
//! with their own SQL and hand them to these functions, so admission is
//! identical across SQLite and PostgreSQL.

use super::*;

/// Snapshot v6 introduced the 2 MiB physical budget. Keep that admission
/// boundary stable when the current runtime format advances to v7 and beyond;
/// only the exact v1-v5 DTOs remain on the historical 1 MiB budget.
pub(crate) const LAST_ONE_MIB_RUNTIME_SNAPSHOT_VERSION: u32 = 5;

/// Maps a sectioned-runtime storage error to the store contract: database
/// errors stay database errors, every disagreement fails closed as an invalid
/// runtime snapshot and an oversized assembly reports its ceiling.
pub(crate) fn runtime_section_error(
    error: tiamud_persistence_sqlite::runtime_sections::RuntimeSectionError,
) -> StoreError {
    use tiamud_persistence_sqlite::runtime_sections::RuntimeSectionError;
    match error {
        RuntimeSectionError::Sqlite(error) => StoreError::Sqlite(error),
        RuntimeSectionError::TooLarge { maximum } => StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum,
        },
        RuntimeSectionError::InvalidManifest | RuntimeSectionError::Inconsistent(_) => {
            StoreError::InvalidRuntimeSnapshot
        }
    }
}

/// Verify, assemble and decode one stored sectioned runtime (schema v17,
/// ADR 0035). The manifest must agree exactly with the section rows, and the
/// assembled document is admitted under the same version-dependent ceiling as
/// the former single document before the version-directed decoder runs.
pub(crate) fn decode_stored_runtime(
    stored: &tiamud_persistence_sqlite::runtime_sections::StoredRuntime,
) -> Result<(u32, WorldRuntimeSnapshot), StoreError> {
    let durable_version =
        u32::try_from(stored.snapshot_version).map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
    let maximum = if durable_version <= LAST_ONE_MIB_RUNTIME_SNAPSHOT_VERSION {
        tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
    } else {
        MAX_RUNTIME_JSON_BYTES
    };
    let document =
        tiamud_persistence_sqlite::runtime_sections::assemble_stored_runtime(stored, maximum)
            .map_err(runtime_section_error)?;
    decode_runtime_snapshot(stored.snapshot_version, &document)
        .map(|snapshot| (durable_version, snapshot))
}

pub(crate) fn normalise_character_identity_ids(
    mut character_ids: Vec<CharacterId>,
) -> Result<Vec<CharacterId>, StoreError> {
    character_ids.sort_unstable();
    character_ids.dedup();
    if character_ids.len() > MAX_STARTUP_CHARACTER_EXISTENCE_IDS {
        return Err(StoreError::TooManyCharacterIdentities {
            maximum: MAX_STARTUP_CHARACTER_EXISTENCE_IDS,
        });
    }
    if let Some(invalid) = character_ids.iter().find(|id| id.0 <= 0) {
        return Err(StoreError::InvalidCharacterIdentity(*invalid));
    }
    Ok(character_ids)
}

pub(crate) fn ensure_all_character_ids_exist(
    requested: &[CharacterId],
    found: &[CharacterId],
) -> Result<(), StoreError> {
    let mut found_ids = BTreeSet::new();
    for character_id in found {
        if character_id.0 <= 0 || !found_ids.insert(*character_id) {
            return Err(StoreError::InvalidRuntimeSnapshot);
        }
    }
    if found_ids.len() > requested.len()
        || found_ids
            .iter()
            .any(|character_id| requested.binary_search(character_id).is_err())
    {
        return Err(StoreError::InvalidRuntimeSnapshot);
    }
    requested
        .iter()
        .find(|character_id| !found_ids.contains(character_id))
        .map_or(Ok(()), |missing| {
            Err(StoreError::CharacterIdentityNotFound(*missing))
        })
}

pub(crate) fn normalise_startup_character_ids(
    mut character_ids: Vec<CharacterId>,
    limits: StartupGameplayLoadLimits,
) -> Result<Vec<CharacterId>, StoreError> {
    character_ids.sort_unstable();
    character_ids.dedup();
    if character_ids.len() > limits.maximum_characters() {
        return Err(StoreError::TooManyCharacterAggregates {
            maximum: limits.maximum_characters(),
        });
    }
    if let Some(invalid) = character_ids.iter().find(|id| id.0 <= 0) {
        return Err(StoreError::CharacterGameplayNotFound(*invalid));
    }
    Ok(character_ids)
}

pub(crate) fn decode_startup_gameplay(
    character_id: CharacterId,
    json: &str,
    total_json_bytes: &mut usize,
    limits: StartupGameplayLoadLimits,
) -> Result<DurableCharacterGameplay, StoreError> {
    if json.len() > MAX_CHARACTER_JSON_BYTES {
        return Err(StoreError::SnapshotTooLarge {
            kind: "character gameplay",
            maximum: MAX_CHARACTER_JSON_BYTES,
        });
    }
    *total_json_bytes =
        total_json_bytes
            .checked_add(json.len())
            .ok_or(StoreError::SnapshotTooLarge {
                kind: limits.kind(),
                maximum: limits.maximum_bytes(),
            })?;
    if *total_json_bytes > limits.maximum_bytes() {
        return Err(StoreError::SnapshotTooLarge {
            kind: limits.kind(),
            maximum: limits.maximum_bytes(),
        });
    }
    Ok(DurableCharacterGameplay {
        character_id,
        gameplay: serde_json::from_str(json)?,
    })
}

pub(crate) fn ensure_all_startup_characters_loaded(
    requested: &[CharacterId],
    aggregates: &[DurableCharacterGameplay],
) -> Result<(), StoreError> {
    let mut loaded = BTreeSet::new();
    for aggregate in aggregates {
        if aggregate.character_id.0 <= 0 || !loaded.insert(aggregate.character_id) {
            return Err(StoreError::InvalidRuntimeSnapshot);
        }
    }
    if loaded.len() != requested.len() {
        return requested
            .iter()
            .find(|character_id| !loaded.contains(character_id))
            .map_or(Err(StoreError::InvalidRuntimeSnapshot), |missing| {
                Err(StoreError::CharacterGameplayNotFound(*missing))
            });
    }
    requested
        .iter()
        .find(|character_id| !loaded.contains(character_id))
        .map_or(Ok(()), |missing| {
            Err(StoreError::CharacterGameplayNotFound(*missing))
        })
}

pub(crate) fn validate_area_generation(
    generation: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    let valid_world_id = !generation.world_id.is_empty()
        && generation.world_id.len() <= 256
        && generation.world_id.trim() == generation.world_id
        && !generation.world_id.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{061c}'
                        | '\u{200b}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{feff}'
                )
        });
    let valid_digest = |digest: &str| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !valid_world_id
        || !valid_digest(&generation.active_catalog_sha256)
        || generation
            .previous_catalog_sha256
            .as_deref()
            .is_some_and(|digest| {
                !valid_digest(digest) || digest == generation.active_catalog_sha256
            })
        || generation.activation_sequence == 0
        || generation.activation_sequence > i64::MAX as u64
    {
        return Err(StoreError::InvalidAreaGeneration);
    }
    Ok(())
}

pub(crate) fn validate_area_generation_transition(
    expected: &DurableAreaGeneration,
    next: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    validate_area_generation(expected)?;
    validate_area_generation(next)?;
    if next.world_id != expected.world_id
        || next.previous_catalog_sha256.as_deref() != Some(expected.active_catalog_sha256.as_str())
        || next.active_catalog_sha256 == expected.active_catalog_sha256
        || next.activation_sequence != expected.activation_sequence.saturating_add(1)
    {
        return Err(StoreError::InvalidAreaGeneration);
    }
    Ok(())
}

pub(crate) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn area_checkpoint_unit_id(checkpoint: &AreaDynamicCheckpoint) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tiamud-area-retirement-unit-v1\0");
    digest.update(checkpoint.generation_sha256.as_bytes());
    for area in &checkpoint.areas {
        let revision = area.revision.to_string();
        for value in [
            area.area_id.as_str(),
            revision.as_str(),
            area.bundle_sha256.as_str(),
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    }
    hex::encode(digest.finalize())
}

pub(crate) fn validate_area_checkpoint(
    durable: &DurableAreaCheckpoint,
) -> Result<AreaDynamicCheckpoint, StoreError> {
    if !valid_sha256(&durable.generation_sha256)
        || !valid_sha256(&durable.unit_id)
        || !valid_sha256(&durable.checkpoint_sha256)
        || durable.residency_epoch == 0
        || durable.residency_epoch > i64::MAX as u64
        || durable.checkpoint_version != tiamud_core::AREA_DYNAMIC_CHECKPOINT_VERSION
        || durable.captured_logical_tick > i64::MAX as u64
        || durable.checkpoint_json.is_empty()
        || durable.checkpoint_json.len() > MAX_RUNTIME_JSON_BYTES
        || hex::encode(Sha256::digest(durable.checkpoint_json.as_bytes()))
            != durable.checkpoint_sha256
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let checkpoint: AreaDynamicCheckpoint = serde_json::from_str(&durable.checkpoint_json)
        .map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    if serde_json::to_string(&checkpoint).map_err(|_| StoreError::InvalidAreaCheckpoint)?
        != durable.checkpoint_json
        || tiamud_core::validate_area_dynamic_checkpoint(&checkpoint).is_err()
        || area_checkpoint_unit_id(&checkpoint) != durable.unit_id
        || checkpoint.generation_sha256 != durable.generation_sha256
        || checkpoint.checkpoint_version != durable.checkpoint_version
        || checkpoint.captured_logical_tick != durable.captured_logical_tick
        || checkpoint
            .areas
            .iter()
            .any(|area| area.residency_epoch != durable.residency_epoch)
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    Ok(checkpoint)
}

pub(crate) fn validate_area_retirement_commit(
    runtime: &WorldRuntimeSnapshot,
    generation: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<AreaDynamicCheckpoint, StoreError> {
    validate_area_generation(generation)?;
    let checkpoint = validate_area_checkpoint(durable)?;
    if generation.active_catalog_sha256 != durable.generation_sha256
        || runtime.snapshot_version != tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        || tiamud_core::validate_world_runtime_snapshot_semantics(runtime).is_err()
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let checkpoint_areas = checkpoint
        .areas
        .iter()
        .map(|area| area.area_id.as_str())
        .collect::<BTreeSet<_>>();
    for area in &checkpoint.areas {
        let Some(AreaRuntimeResidence::Retired {
            revision,
            bundle_sha256,
            residency_epoch,
            checkpoint_version,
            checkpoint_sha256,
        }) = runtime.area_residency.get(&area.area_id)
        else {
            return Err(StoreError::InvalidAreaCheckpoint);
        };
        if *revision != area.revision
            || bundle_sha256 != &area.bundle_sha256
            || *residency_epoch != area.residency_epoch
            || *checkpoint_version != durable.checkpoint_version
            || checkpoint_sha256 != &durable.checkpoint_sha256
        {
            return Err(StoreError::InvalidAreaCheckpoint);
        }
    }
    if runtime.area_residency.iter().any(|(area_id, state)| {
        matches!(state, AreaRuntimeResidence::Retired { checkpoint_sha256, .. }
            if checkpoint_sha256 == &durable.checkpoint_sha256)
            && !checkpoint_areas.contains(area_id.as_str())
    }) {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    Ok(checkpoint)
}

pub(crate) fn validate_area_rehydration_commit(
    runtime: &WorldRuntimeSnapshot,
    generation: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<AreaDynamicCheckpoint, StoreError> {
    validate_area_generation(generation)?;
    let checkpoint = validate_area_checkpoint(durable)?;
    if generation.active_catalog_sha256 != durable.generation_sha256
        || runtime.snapshot_version != tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        || tiamud_core::validate_world_runtime_snapshot_semantics(runtime).is_err()
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    for area in &checkpoint.areas {
        let Some(AreaRuntimeResidence::Resident {
            revision,
            bundle_sha256,
            residency_epoch,
        }) = runtime.area_residency.get(&area.area_id)
        else {
            return Err(StoreError::InvalidAreaCheckpoint);
        };
        if *revision != area.revision
            || bundle_sha256 != &area.bundle_sha256
            || area.residency_epoch.checked_add(1) != Some(*residency_epoch)
        {
            return Err(StoreError::InvalidAreaCheckpoint);
        }
    }
    if runtime.area_residency.values().any(|state| {
        matches!(state, AreaRuntimeResidence::Retired { checkpoint_sha256, .. }
            if checkpoint_sha256 == &durable.checkpoint_sha256)
    }) {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    Ok(checkpoint)
}

pub(crate) fn validate_area_retirement_previous_runtime(
    current: &WorldRuntimeSnapshot,
    next: &WorldRuntimeSnapshot,
    checkpoint: &AreaDynamicCheckpoint,
) -> Result<(), StoreError> {
    let mut expected = next.clone();
    for area in &checkpoint.areas {
        let previous_epoch = area
            .residency_epoch
            .checked_sub(1)
            .filter(|epoch| *epoch > 0)
            .ok_or(StoreError::AreaCheckpointConflict)?;
        expected.area_residency.insert(
            area.area_id.clone(),
            AreaRuntimeResidence::Resident {
                revision: area.revision,
                bundle_sha256: area.bundle_sha256.clone(),
                residency_epoch: previous_epoch,
            },
        );
    }
    for (npc_id, npc) in &checkpoint.npcs {
        if expected.npcs.insert(npc_id.clone(), npc.clone()).is_some() {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for (item_id, item) in &checkpoint.item_instances {
        if expected
            .item_instances
            .insert(item_id.clone(), item.clone())
            .is_some()
        {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for (item_id, provenance) in &checkpoint.item_provenance {
        if expected
            .item_provenance
            .insert(item_id.clone(), provenance.clone())
            .is_some()
        {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for (root_id, reservation) in &checkpoint.ground_loot {
        if expected
            .ground_loot
            .insert(root_id.clone(), reservation.clone())
            .is_some()
        {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    if current != &expected {
        return Err(StoreError::AreaCheckpointConflict);
    }
    Ok(())
}

pub(crate) fn validate_area_rehydration_previous_runtime(
    current: &WorldRuntimeSnapshot,
    next: &WorldRuntimeSnapshot,
    durable: &DurableAreaCheckpoint,
    checkpoint: &AreaDynamicCheckpoint,
) -> Result<(), StoreError> {
    let mut expected = next.clone();
    for area in &checkpoint.areas {
        expected.area_residency.insert(
            area.area_id.clone(),
            AreaRuntimeResidence::Retired {
                revision: area.revision,
                bundle_sha256: area.bundle_sha256.clone(),
                residency_epoch: area.residency_epoch,
                checkpoint_version: durable.checkpoint_version,
                checkpoint_sha256: durable.checkpoint_sha256.clone(),
            },
        );
    }
    for npc_id in checkpoint.npcs.keys() {
        if expected.npcs.remove(npc_id).is_none() {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for item_id in checkpoint.item_instances.keys() {
        expected.item_instances.remove(item_id);
        expected.item_provenance.remove(item_id);
        expected.ground_loot.remove(item_id);
    }
    for root_id in checkpoint.ground_loot.keys() {
        expected.ground_loot.remove(root_id);
    }
    if current != &expected {
        return Err(StoreError::AreaCheckpointConflict);
    }
    Ok(())
}

pub(crate) fn decode_startup_domain_events(
    rows: Vec<(i64, String)>,
) -> Result<Vec<DomainEvent>, StoreError> {
    if rows.len() > DOMAIN_EVENT_RETENTION as usize {
        return Err(StoreError::InvalidStartupLedger);
    }

    let mut expected_sequence = 1_i64;
    let mut events = Vec::with_capacity(rows.len());
    for (sequence, json) in rows {
        if sequence != expected_sequence {
            return Err(StoreError::InvalidStartupLedger);
        }
        if json.len() > MAX_EVENT_JSON_BYTES {
            return Err(StoreError::SnapshotTooLarge {
                kind: "domain event",
                maximum: MAX_EVENT_JSON_BYTES,
            });
        }
        events.push(serde_json::from_str(&json)?);
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or(StoreError::InvalidStartupLedger)?;
    }
    Ok(events)
}

pub(crate) fn decode_runtime_snapshot(
    durable_version: i64,
    json: &str,
) -> Result<WorldRuntimeSnapshot, StoreError> {
    let durable_version =
        u32::try_from(durable_version).map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
    let maximum = if durable_version <= LAST_ONE_MIB_RUNTIME_SNAPSHOT_VERSION {
        tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
    } else {
        MAX_RUNTIME_JSON_BYTES
    };
    // `str::len` is the UTF-8 byte length, unlike SQLite's text `length()`.
    if json.len() > maximum {
        return Err(StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum,
        });
    }
    let snapshot = tiamud_core::decode_world_runtime_snapshot(durable_version, json)
        .map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
    if snapshot.snapshot_version != tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION {
        return Err(StoreError::InvalidRuntimeSnapshot);
    }
    // Admission must cover the normalized current representation too. A valid
    // near-limit legacy payload can expand substantially when later versions
    // add explicit per-instance state. Reject it here, before the decoder hands
    // persistence an inadmissible current representation for restoration.
    bounded_json(&snapshot, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    Ok(snapshot)
}
