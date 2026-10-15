//! Loot tables, ground loot reservations, group loot decisions, sealed
//! rewards and affix rolls.

use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootRightsPolicy {
    CreditedCharacter,
    PartyRoundRobin,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootEntryKind {
    Item,
    Nothing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootSourceKind {
    NpcSpawn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootGuaranteedDefinition {
    pub id: String,
    pub item_prototype_id: String,
    pub quantity: u16,
    pub affix_pool_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootEntryDefinition {
    pub kind: LootEntryKind,
    pub id: String,
    pub weight: u32,
    pub item_prototype_id: Option<String>,
    pub min_quantity: Option<u16>,
    pub max_quantity: Option<u16>,
    pub affix_pool_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootPoolDefinition {
    pub id: String,
    pub rolls: u8,
    pub entries: Vec<LootEntryDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootTableDefinition {
    pub id: String,
    pub digest_sha256: String,
    pub guaranteed: Vec<LootGuaranteedDefinition>,
    pub pools: Vec<LootPoolDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootSourceDefinition {
    pub kind: LootSourceKind,
    pub id: String,
    pub npc_spawn_id: String,
    pub loot_table_id: String,
    pub rights: LootRightsPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LootDropId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RewardOpenId(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemProvenance {
    AuthoredInitial,
    LegacySnapshot {
        source_version: u32,
    },
    RuntimeCreated {
        created_tick: u64,
    },
    NpcLoot {
        drop_id: LootDropId,
        npc_spawn_id: String,
        loot_table_id: String,
        table_digest_sha256: String,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sealed_loot_table_id: Option<String>,
        roll_index: u8,
        quantity_index: u16,
        generated_tick: u64,
    },
    SealedReward {
        open_id: RewardOpenId,
        source_item_id: ItemInstanceId,
        source_npc_loot_digest_sha256: String,
        sealed_prototype_id: String,
        loot_table_id: String,
        table_digest_sha256: String,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool_id: Option<String>,
        roll_index: u8,
        quantity_index: u16,
        generated_tick: u64,
    },
}

/// Exact provenance vocabulary accepted from snapshot v6.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemProvenanceV6 {
    AuthoredInitial,
    LegacySnapshot {
        source_version: u32,
    },
    RuntimeCreated {
        created_tick: u64,
    },
    NpcLoot {
        drop_id: LootDropId,
        npc_spawn_id: String,
        loot_table_id: String,
        table_digest_sha256: String,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool_id: Option<String>,
        roll_index: u8,
        quantity_index: u16,
        generated_tick: u64,
    },
}

impl From<ItemProvenanceV6> for ItemProvenance {
    fn from(value: ItemProvenanceV6) -> Self {
        match value {
            ItemProvenanceV6::AuthoredInitial => Self::AuthoredInitial,
            ItemProvenanceV6::LegacySnapshot { source_version } => {
                Self::LegacySnapshot { source_version }
            }
            ItemProvenanceV6::RuntimeCreated { created_tick } => {
                Self::RuntimeCreated { created_tick }
            }
            ItemProvenanceV6::NpcLoot {
                drop_id,
                npc_spawn_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                roll_index,
                quantity_index,
                generated_tick,
            } => Self::NpcLoot {
                drop_id,
                npc_spawn_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                sealed_loot_table_id: None,
                roll_index,
                quantity_index,
                generated_tick,
            },
        }
    }
}

/// Returns `true` when a distribution mode is the fail-safe `RoundRobin`
/// default, so snapshot v9 omits the field for encounters that never opted into
/// an advanced mode. This keeps a defeat that uses the unchanged phase 4A path
/// byte-identical to its v8 encoding.
pub(crate) fn is_round_robin_distribution(mode: &loot_distribution::LootDistributionMode) -> bool {
    matches!(mode, loot_distribution::LootDistributionMode::RoundRobin)
}

/// Human-facing label for a group-loot distribution mode, shared verbatim by
/// Web and Telnet projections.
pub(crate) fn loot_distribution_label(
    mode: loot_distribution::LootDistributionMode,
) -> &'static str {
    match mode {
        loot_distribution::LootDistributionMode::RoundRobin => "round-robin",
        loot_distribution::LootDistributionMode::NeedGreed => "need/greed",
        loot_distribution::LootDistributionMode::MasterLoot => "master-loot",
    }
}

/// Human-facing label for a single `need`/`greed`/`pass` declaration.
pub(crate) fn loot_decision_label(decision: loot_distribution::LootDecision) -> &'static str {
    match decision {
        loot_distribution::LootDecision::Need => "need",
        loot_distribution::LootDecision::Greed => "greed",
        loot_distribution::LootDecision::Pass => "pass",
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootEncounter {
    pub npc_spawn_id: String,
    pub initiator: CharacterId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_id: Option<u64>,
    pub cohort: Vec<CharacterId>,
    pub room_id: String,
    pub started_tick: u64,
    /// Snapshot v9. The group loot distribution mode that governs how each
    /// resolved root selects its beneficiary. `RoundRobin` (the default) is the
    /// unchanged phase 4A behaviour; `NeedGreed` and `MasterLoot` consult the
    /// per-root declarations/assignments below and fall back to round-robin when
    /// no advanced winner exists.
    #[serde(default, skip_serializing_if = "is_round_robin_distribution")]
    pub distribution: loot_distribution::LootDistributionMode,
    /// Snapshot v9. Per-root `need`/`greed`/`pass` declarations keyed by the
    /// resolved root instance, then by the declaring cohort member. Empty is the
    /// fail-safe default and always resolves through the round-robin path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub loot_declarations:
        BTreeMap<ItemInstanceId, BTreeMap<CharacterId, loot_distribution::LootDecision>>,
    /// Snapshot v9. Per-root master-loot assignments made by the party leader,
    /// keyed by the resolved root instance. Empty is the fail-safe default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub master_assignments: BTreeMap<ItemInstanceId, CharacterId>,
}

/// Exact loot-encounter shape produced through snapshot v8. It predates the v9
/// group-distribution fields and, via `deny_unknown_fields`, rejects any v8
/// payload that smuggles a v9 loot field. The v8 -> v9 upgrade normalizes it to
/// the `RoundRobin` default with empty declarations and assignments.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootEncounterV8 {
    pub npc_spawn_id: String,
    pub initiator: CharacterId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_id: Option<u64>,
    pub cohort: Vec<CharacterId>,
    pub room_id: String,
    pub started_tick: u64,
}

impl From<LootEncounterV8> for LootEncounter {
    fn from(value: LootEncounterV8) -> Self {
        Self {
            npc_spawn_id: value.npc_spawn_id,
            initiator: value.initiator,
            party_id: value.party_id,
            cohort: value.cohort,
            room_id: value.room_id,
            started_tick: value.started_tick,
            distribution: loot_distribution::LootDistributionMode::RoundRobin,
            loot_declarations: BTreeMap::new(),
            master_assignments: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroundLootReservation {
    pub drop_id: LootDropId,
    pub root_item_id: ItemInstanceId,
    pub room_id: String,
    pub cohort: Vec<CharacterId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beneficiary: Option<CharacterId>,
    #[serde(default)]
    pub passed: BTreeSet<CharacterId>,
    pub created_tick: u64,
    pub claim_until_tick: u64,
    pub despawn_at_tick: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdventureRuntimeSnapshot {
    pub next_party_id: u64,
    pub parties: BTreeMap<u64, PartySnapshot>,
    pub shop_stock: BTreeMap<String, u32>,
    pub weather: WeatherKind,
    pub next_weather_tick: u64,
}

impl Default for AdventureRuntimeSnapshot {
    fn default() -> Self {
        Self {
            next_party_id: 1,
            parties: BTreeMap::new(),
            shop_stock: BTreeMap::from([("tonic".into(), 20), ("trail-ration".into(), 20)]),
            weather: WeatherKind::Rain,
            next_weather_tick: 5,
        }
    }
}

/// Portable, durable state for one character. Identity/location and gameplay
/// are kept together so persistence cannot publish only half of an action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableCharacterSnapshot {
    pub character: CharacterSnapshot,
    pub gameplay: CharacterGameplaySnapshot,
}

impl World {
    pub(crate) fn prune_consumed_runtime_items(&mut self) -> Result<(), WorldError> {
        let reserved = self.reserved_item_ids()?;
        let parents = self
            .item_instances
            .values()
            .filter_map(|instance| match &instance.location {
                SharedItemLocation::Contained(parent) => Some(parent.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let removable = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                (runtime_item_sequence(item_id).is_some()
                    && matches!(instance.location, SharedItemLocation::Consumed)
                    && !self.initial_item_prototypes.contains_key(item_id)
                    && !self.ground_loot.contains_key(item_id)
                    && !reserved.contains(item_id)
                    && !parents.contains(item_id))
                .then_some(item_id.clone())
            })
            .collect::<Vec<_>>();
        for item_id in removable {
            self.item_instances.remove(&item_id);
            self.item_provenance.remove(&item_id);
        }
        Ok(())
    }

    pub(crate) fn roll_loot_table(
        &mut self,
        table_id: &str,
    ) -> Result<(LootTable, Vec<RolledLootUnit>), WorldError> {
        let table = self
            .loot_tables
            .get(table_id)
            .cloned()
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        let mut units = Vec::with_capacity(table.worst_case_units);
        for guaranteed in &table.guaranteed {
            for quantity_index in 0..guaranteed.quantity {
                units.push(RolledLootUnit {
                    prototype_id: guaranteed.item_prototype_id.clone(),
                    entry_id: guaranteed.id.clone(),
                    pool_id: None,
                    roll_index: 0,
                    quantity_index,
                    affix_pool_id: guaranteed.affix_pool_id.clone(),
                });
            }
        }
        for pool in &table.pools {
            let total_weight = pool.entries.iter().try_fold(0_u64, |total, entry| {
                total
                    .checked_add(u64::from(entry.weight))
                    .ok_or(WorldError::InvalidGameplayDefinition)
            })?;
            for roll_index in 0..pool.rolls {
                let selected = self.rng.uniform_below(total_weight)?;
                let mut cumulative = 0_u64;
                let entry = pool
                    .entries
                    .iter()
                    .find(|entry| {
                        cumulative = cumulative.saturating_add(u64::from(entry.weight));
                        selected < cumulative
                    })
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if entry.kind == LootEntryKind::Nothing {
                    continue;
                }
                let minimum = entry
                    .min_quantity
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let maximum = entry
                    .max_quantity
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let width = u64::from(maximum - minimum) + 1;
                let quantity = minimum
                    + u16::try_from(self.rng.uniform_below(width)?)
                        .map_err(|_| WorldError::InvalidGameplayDefinition)?;
                for quantity_index in 0..quantity {
                    units.push(RolledLootUnit {
                        prototype_id: entry
                            .item_prototype_id
                            .clone()
                            .ok_or(WorldError::InvalidGameplayDefinition)?,
                        entry_id: entry.id.clone(),
                        pool_id: Some(pool.id.clone()),
                        roll_index,
                        quantity_index,
                        affix_pool_id: entry.affix_pool_id.clone(),
                    });
                }
            }
        }
        if units.len() > MAX_LOOT_UNITS_PER_RESOLUTION {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        Ok((table, units))
    }

    pub(crate) fn final_loot_cohort(
        &self,
        encounter: &LootEncounter,
        room_id: &str,
    ) -> Vec<CharacterId> {
        let mut cohort = vec![encounter.initiator];
        if let Some(party_id) = encounter.party_id
            && let Some(party) = self.adventure.parties.get(&party_id)
            && party.members.contains(&encounter.initiator)
        {
            for member in encounter
                .cohort
                .iter()
                .copied()
                .filter(|member| *member != encounter.initiator)
            {
                let eligible = party.members.contains(&member)
                    && self
                        .session_for_character(member)
                        .and_then(|session| self.character(session).ok())
                        .and_then(|character| {
                            room_checked(&self.entities, character.location)
                                .ok()
                                .map(|room| room.stable_id == room_id)
                        })
                        .unwrap_or(false)
                    && self.character_gameplay.get(&member).is_some_and(|state| {
                        state.vitals.health > 0 && state.recovery_at_tick.is_none()
                    });
                if eligible {
                    cohort.push(member);
                }
            }
        }
        cohort.sort_unstable();
        cohort.dedup();
        cohort
    }

    pub(crate) fn reserve_ground_loot(
        &mut self,
        drop_id: LootDropId,
        roots: &[ItemInstanceId],
        encounter: &LootEncounter,
        rights: LootRightsPolicy,
        room_id: &str,
        tick: u64,
    ) -> Result<(), WorldError> {
        let cohort = self.final_loot_cohort(encounter, room_id);
        let round_robin_party = encounter.party_id.filter(|party_id| {
            rights == LootRightsPolicy::PartyRoundRobin
                && cohort.len() > 1
                && self.adventure.parties.contains_key(party_id)
        });
        let mut cursor = round_robin_party
            .and_then(|party_id| self.party_loot_cursors.get(&party_id).copied())
            .unwrap_or(0);
        // Authored credited-character rights are never overridden by a saved
        // advanced decision. Revalidate leadership/cohort at resolution too.
        let distribution = if round_robin_party.is_some_and(|party_id| {
            self.adventure
                .parties
                .get(&party_id)
                .is_some_and(|party| party.leader == encounter.initiator)
        }) {
            encounter.distribution
        } else {
            loot_distribution::LootDistributionMode::RoundRobin
        };
        for root_item_id in roots {
            if self.ground_loot.len() >= MAX_GROUND_LOOT_RESERVATIONS {
                return Err(WorldError::InvalidDomainTransition);
            }
            // The distribution mode governs beneficiary selection. `RoundRobin`
            // is the unchanged phase 4A path. `NeedGreed`/`MasterLoot` consult
            // the durable per-root declarations/assignments and resolve
            // deterministically; a mode that produces no advanced winner (no
            // declaration, all `pass`, or no assignment) falls back to the
            // exact round-robin path below, so a root can never be stuck. Empty
            // declarations never draw the RNG, so an encounter that never opted
            // into an advanced mode is byte-identical to its 4A behaviour.
            let advanced_winner = match distribution {
                loot_distribution::LootDistributionMode::RoundRobin => None,
                loot_distribution::LootDistributionMode::NeedGreed => {
                    match encounter.loot_declarations.get(root_item_id) {
                        Some(declarations) => {
                            // The final cohort is recomputed at defeat and may
                            // omit a declarer who left the room or fell. Drop
                            // those stale declarations so a genuine claimant is
                            // never rejected; if none survive, fall back.
                            let live = declarations
                                .iter()
                                .filter(|(member, _)| cohort.contains(member))
                                .map(|(member, decision)| (*member, *decision))
                                .collect::<BTreeMap<_, _>>();
                            match loot_distribution::resolve_need_greed(
                                &cohort,
                                &live,
                                &mut self.rng,
                            )? {
                                loot_distribution::LootResolution::Winner(id) => Some(id),
                                loot_distribution::LootResolution::FallbackRoundRobin => None,
                            }
                        }
                        None => None,
                    }
                }
                loot_distribution::LootDistributionMode::MasterLoot => {
                    // Only honour an assignment whose recipient survived into
                    // the final cohort; otherwise fall back rather than erroring
                    // the whole defeat resolution.
                    let assigned = encounter
                        .master_assignments
                        .get(root_item_id)
                        .copied()
                        .filter(|member| cohort.contains(member));
                    match loot_distribution::resolve_master_loot(&cohort, assigned)? {
                        loot_distribution::LootResolution::Winner(id) => Some(id),
                        loot_distribution::LootResolution::FallbackRoundRobin => None,
                    }
                }
            };
            let beneficiary = if let Some(winner) = advanced_winner {
                Some(winner)
            } else if round_robin_party.is_some() {
                let index = usize::try_from(cursor % u64::try_from(cohort.len()).unwrap_or(1))
                    .map_err(|_| WorldError::InvalidDomainTransition)?;
                cursor = cursor
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Some(cohort[index])
            } else {
                Some(encounter.initiator)
            };
            self.ground_loot.insert(
                root_item_id.clone(),
                GroundLootReservation {
                    drop_id,
                    root_item_id: root_item_id.clone(),
                    room_id: room_id.to_owned(),
                    cohort: cohort.clone(),
                    beneficiary,
                    passed: BTreeSet::new(),
                    created_tick: tick,
                    claim_until_tick: tick
                        .checked_add(LOOT_EXCLUSIVE_TICKS)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                    despawn_at_tick: tick
                        .checked_add(LOOT_DESPAWN_TICKS)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                },
            );
        }
        if let Some(party_id) = round_robin_party
            && cursor > 0
        {
            self.party_loot_cursors.insert(party_id, cursor);
        }
        Ok(())
    }

    pub(crate) fn resolve_npc_defeat(
        &mut self,
        tick: u64,
        npc_id: &str,
        character_id: CharacterId,
        character_name: &str,
        room_id: &str,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let (npc_name, reward, respawn_at_tick, loot) = {
            let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
            npc.engaged_character = None;
            npc.statuses.clear();
            npc.ability_cooldowns.clear();
            let respawn_at_tick = tick.saturating_add(npc.respawn_ticks);
            npc.respawn_at_tick = Some(respawn_at_tick);
            (
                npc.name.clone(),
                npc.experience_reward,
                respawn_at_tick,
                npc.loot_item_ids.clone(),
            )
        };
        events.push(DomainEvent::Gameplay {
            event: GameplayEvent::NpcDefeated {
                character_id,
                character_name: character_name.to_owned(),
                npc_id: npc_id.to_owned(),
                npc_name: npc_name.clone(),
                respawn_at_tick,
                room_id: room_id.to_owned(),
            },
        });
        let encounter = self
            .loot_encounters
            .remove(npc_id)
            .unwrap_or_else(|| LootEncounter {
                npc_spawn_id: npc_id.to_owned(),
                initiator: character_id,
                party_id: None,
                cohort: vec![character_id],
                room_id: room_id.to_owned(),
                started_tick: tick,
                distribution: loot_distribution::LootDistributionMode::RoundRobin,
                loot_declarations: BTreeMap::new(),
                master_assignments: BTreeMap::new(),
            });
        let mut loot = loot;
        loot.sort();
        let fixed_candidates = loot
            .into_iter()
            .filter(|item_id| {
                self.item_location(item_id)
                    == Some(&SharedItemLocation::HeldByNpc(npc_id.to_owned()))
            })
            .collect::<Vec<_>>();
        let source = self.loot_sources.get(npc_id).cloned();
        if source.is_some() || !fixed_candidates.is_empty() {
            self.prune_consumed_runtime_items()?;
            let drop_id = self.next_loot_drop_id;
            self.next_loot_drop_id = LootDropId(
                drop_id
                    .0
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?,
            );
            // Roll before any capacity decision. A rejected drop therefore
            // advances the deterministic RNG and durable drop identity exactly
            // as an admitted drop would.
            let rolled = source
                .as_ref()
                .map(|source| self.roll_loot_table(&source.loot_table_id))
                .transpose()?;
            let rolled = rolled
                .map(|(table, units)| {
                    let mut prepared = Vec::with_capacity(units.len());
                    for unit in units {
                        let item_affixes = unit
                            .affix_pool_id
                            .as_deref()
                            .map(|pool_id| self.roll_item_affix(pool_id))
                            .transpose()?
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>();
                        prepared.push((unit, item_affixes));
                    }
                    Ok::<_, WorldError>((table, prepared))
                })
                .transpose()?;
            let rights = source
                .as_ref()
                .map_or(LootRightsPolicy::CreditedCharacter, |source| source.rights);
            let mut fixed_suppressed = 0_usize;
            for item_id in fixed_candidates {
                let checkpoint = self.checkpoint();
                let admitted = if self.ground_loot.len() < MAX_GROUND_LOOT_RESERVATIONS {
                    self.set_item_location(&item_id, SharedItemLocation::Room(room_id.to_owned()))?;
                    self.reserve_ground_loot(
                        drop_id,
                        std::slice::from_ref(&item_id),
                        &encounter,
                        rights,
                        room_id,
                        tick,
                    )?;
                    let fits = serde_json::to_vec(&self.runtime_snapshot())
                        .map_err(|_| WorldError::InvalidDomainTransition)?
                        .len()
                        <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
                    if !fits {
                        self.restore_checkpoint(checkpoint);
                    }
                    fits
                } else {
                    false
                };
                if admitted {
                    let item_name = self.item_effective_name(&item_id)?;
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::LootDropped {
                            npc_id: npc_id.to_owned(),
                            npc_name: npc_name.clone(),
                            item_id,
                            item_name,
                            room_id: room_id.to_owned(),
                        },
                    });
                } else {
                    self.set_item_location(&item_id, SharedItemLocation::Staged)?;
                    fixed_suppressed = fixed_suppressed.saturating_add(1);
                }
            }
            if fixed_suppressed > 0 {
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootGenerationSuppressed {
                        drop_id,
                        npc_id: npc_id.to_owned(),
                        npc_name: npc_name.clone(),
                        reason: format!(
                            "capacity: {fixed_suppressed} authored fixed root(s) tombstoned"
                        ),
                        room_id: room_id.to_owned(),
                    },
                });
            }
            if let Some((table, units)) = rolled {
                let capacity_suppressed = !units.is_empty()
                    && (self
                        .item_instances
                        .len()
                        .checked_add(table.worst_case_units)
                        .is_none_or(|total| total > MAX_ITEM_INSTANCES)
                        || self
                            .ground_loot
                            .len()
                            .checked_add(units.len())
                            .is_none_or(|total| total > MAX_GROUND_LOOT_RESERVATIONS));
                if capacity_suppressed {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::LootGenerationSuppressed {
                            drop_id,
                            npc_id: npc_id.to_owned(),
                            npc_name: npc_name.clone(),
                            reason: "capacity".into(),
                            room_id: room_id.to_owned(),
                        },
                    });
                } else if !units.is_empty() {
                    // This checkpoint is intentionally after RNG/drop-id and
                    // after all admitted fixed roots. Restoring it suppresses
                    // the complete random batch without rewinding either.
                    let checkpoint = self.checkpoint();
                    let mut generated_events = Vec::new();
                    let mut generated_roots = Vec::with_capacity(units.len());
                    for (unit, item_affixes) in units {
                        let sealed_loot_table_id = self
                            .item_prototypes
                            .get(&unit.prototype_id)
                            .and_then(|prototype| prototype.sealed_loot_table_id.clone());
                        let item_id = self.create_staged_item_instance(&unit.prototype_id)?;
                        self.item_instances
                            .get_mut(&item_id)
                            .ok_or(WorldError::InvalidDomainTransition)?
                            .item_affixes = item_affixes;
                        self.set_item_location(
                            &item_id,
                            SharedItemLocation::Room(room_id.to_owned()),
                        )?;
                        self.item_provenance.insert(
                            item_id.clone(),
                            ItemProvenance::NpcLoot {
                                drop_id,
                                npc_spawn_id: npc_id.to_owned(),
                                loot_table_id: table.id.clone(),
                                table_digest_sha256: table.digest_sha256.clone(),
                                entry_id: unit.entry_id,
                                pool_id: unit.pool_id,
                                sealed_loot_table_id,
                                roll_index: unit.roll_index,
                                quantity_index: unit.quantity_index,
                                generated_tick: tick,
                            },
                        );
                        let item_name = self.item_effective_name(&item_id)?;
                        generated_roots.push(item_id.clone());
                        generated_events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootDropped {
                                npc_id: npc_id.to_owned(),
                                npc_name: npc_name.clone(),
                                item_id,
                                item_name,
                                room_id: room_id.to_owned(),
                            },
                        });
                    }
                    self.reserve_ground_loot(
                        drop_id,
                        &generated_roots,
                        &encounter,
                        rights,
                        room_id,
                        tick,
                    )?;
                    let fits = serde_json::to_vec(&self.runtime_snapshot())
                        .map_err(|_| WorldError::InvalidDomainTransition)?
                        .len()
                        <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
                    if fits {
                        generated_events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootGenerated {
                                drop_id,
                                npc_id: npc_id.to_owned(),
                                npc_name: npc_name.clone(),
                                item_ids: generated_roots,
                                room_id: room_id.to_owned(),
                            },
                        });
                        events.extend(generated_events);
                    } else {
                        self.restore_checkpoint(checkpoint);
                        events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootGenerationSuppressed {
                                drop_id,
                                npc_id: npc_id.to_owned(),
                                npc_name: npc_name.clone(),
                                reason: "snapshot_bytes".into(),
                                room_id: room_id.to_owned(),
                            },
                        });
                    }
                }
            }
        }
        let total = {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            state.experience = state.experience.saturating_add(reward);
            state.experience
        };
        events.push(DomainEvent::Gameplay {
            event: GameplayEvent::ExperienceGained {
                character_id,
                character_name: character_name.to_owned(),
                amount: reward,
                total,
                room_id: room_id.to_owned(),
            },
        });
        self.advance_character_progression(character_id, character_name, room_id, events)?;
        Ok(())
    }
}

impl World {
    pub(crate) fn apply_loot(
        &mut self,
        session: SessionId,
        action: LootAction,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let actor_id = character.id;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        match action {
            LootAction::Status => {
                const MAX_STATUS_ROWS: usize = 20;
                let observer = self.observer(session)?;
                let mut claims = self
                    .ground_loot
                    .iter()
                    .filter(|(_, reservation)| {
                        reservation.room_id == room_id
                            && reservation.claim_until_tick > self.logical_tick
                            && reservation.beneficiary == Some(actor_id)
                    })
                    .collect::<Vec<_>>();
                claims.sort_by(|(left, _), (right, _)| left.cmp(right));
                let omitted = claims.len().saturating_sub(MAX_STATUS_ROWS);
                let mut output = String::from("Your reserved loot in this room:\r\n");
                if claims.is_empty() {
                    output.push_str("- none\r\n");
                } else {
                    for (item_id, reservation) in claims.into_iter().take(MAX_STATUS_ROWS) {
                        let item = self.item_prototype_for(item_id)?;
                        let selector = opaque_id(observer.salt, "item", item_id.as_str());
                        let remaining = reservation
                            .claim_until_tick
                            .saturating_sub(self.logical_tick);
                        output.push_str(&format!(
                            "- {} ({} ticks; ref {})\r\n",
                            item.name, remaining, selector
                        ));
                    }
                    if omitted > 0 {
                        output.push_str(&format!("- and {omitted} more\r\n"));
                    }
                }
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(output),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            LootAction::Pass(query) => {
                let candidates = self
                    .ground_loot
                    .iter()
                    .filter_map(|(item_id, reservation)| {
                        (reservation.room_id == room_id
                            && reservation.claim_until_tick > self.logical_tick
                            && (reservation.beneficiary == Some(actor_id)
                                || reservation.passed.contains(&actor_id)))
                        .then_some(item_id)
                    });
                let item_id = resolve_item_query(
                    &self.item_instances,
                    &self.item_prototypes,
                    candidates,
                    &query,
                    self.observer(session)?.salt,
                )
                .map_err(|_| WorldError::LootUnavailable)?;
                let reservation = self
                    .ground_loot
                    .get(&item_id)
                    .cloned()
                    .ok_or(WorldError::LootUnavailable)?;
                let item_name = self.item_effective_name(&item_id)?;
                if reservation.passed.contains(&actor_id) {
                    return Ok(ActionOutcome {
                        scene: self.scene_for(session)?,
                        container_contents: None,
                        trade_contents: None,
                        reward_odds: None,
                        output: Some(format!("You have already passed on {item_name}.\r\n")),
                        changed_character: None,
                        domain_events: Vec::new(),
                    });
                }
                if reservation.beneficiary != Some(actor_id) {
                    return Err(WorldError::LootUnavailable);
                }
                let actor_index = reservation
                    .cohort
                    .iter()
                    .position(|member| *member == actor_id)
                    .ok_or(WorldError::LootUnavailable)?;
                let mut passed = reservation.passed.clone();
                passed.insert(actor_id);
                let beneficiary = (1..=reservation.cohort.len())
                    .map(|offset| {
                        reservation.cohort[(actor_index + offset) % reservation.cohort.len()]
                    })
                    .find(|candidate| !passed.contains(candidate));
                let claim = self
                    .ground_loot
                    .get_mut(&item_id)
                    .ok_or(WorldError::LootUnavailable)?;
                claim.passed = passed;
                claim.beneficiary = beneficiary;
                self.revision = self.revision.wrapping_add(1).max(1);
                let summary = beneficiary.map_or_else(
                    || format!("{item_name} is now public loot."),
                    |_| format!("The reservation for {item_name} passes to the next member."),
                );
                let event = DomainEvent::Gameplay {
                    event: GameplayEvent::LootRightsChanged {
                        item_id,
                        room_id,
                        summary: summary.clone(),
                    },
                };
                self.bump_view_revisions(&event)?;
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!("{summary}\r\n")),
                    changed_character: None,
                    domain_events: vec![event],
                })
            }
            LootAction::Roll => {
                let npc_id = self.active_decision_encounter(actor_id, &room_id, false)?;
                let output = self.render_loot_decisions(session, &npc_id)?;
                self.loot_decision_outcome(session, output)
            }
            LootAction::Mode(mode) => {
                let npc_id = self.active_decision_encounter(actor_id, &room_id, true)?;
                if mode != loot_distribution::LootDistributionMode::RoundRobin
                    && (!self
                        .loot_sources
                        .get(&npc_id)
                        .is_some_and(|source| source.rights == LootRightsPolicy::PartyRoundRobin)
                        || !self.loot_encounters.get(&npc_id).is_some_and(|encounter| {
                            encounter.party_id.is_some() && encounter.cohort.len() > 1
                        }))
                {
                    return Err(WorldError::LootUnavailable);
                }
                let encounter = self
                    .loot_encounters
                    .get_mut(&npc_id)
                    .ok_or(WorldError::LootUnavailable)?;
                encounter.distribution = mode;
                // Switching away from an advanced mode drops its now-irrelevant
                // per-root state so the frozen encounter always validates and a
                // later defeat never resolves against stale declarations.
                if mode != loot_distribution::LootDistributionMode::NeedGreed {
                    encounter.loot_declarations.clear();
                }
                if mode != loot_distribution::LootDistributionMode::MasterLoot {
                    encounter.master_assignments.clear();
                }
                self.revision = self.revision.wrapping_add(1).max(1);
                let output = format!(
                    "Group loot mode set to {}.\r\n",
                    loot_distribution_label(mode)
                );
                self.loot_decision_outcome(session, output)
            }
            LootAction::Declare { decision, query } => {
                let npc_id = self.active_decision_encounter(actor_id, &room_id, false)?;
                if self.loot_encounters.get(&npc_id).map(|e| e.distribution)
                    != Some(loot_distribution::LootDistributionMode::NeedGreed)
                {
                    return Err(WorldError::LootUnavailable);
                }
                let roots = self.declarable_loot_roots(&npc_id);
                let salt = self.observer(session)?.salt;
                let item_id = resolve_item_query(
                    &self.item_instances,
                    &self.item_prototypes,
                    roots.iter(),
                    &query,
                    salt,
                )
                .map_err(|_| WorldError::LootUnavailable)?;
                let item_name = self.item_effective_name(&item_id)?;
                let encounter = self
                    .loot_encounters
                    .get_mut(&npc_id)
                    .ok_or(WorldError::LootUnavailable)?;
                let per_root = encounter.loot_declarations.entry(item_id).or_default();
                // Exactly one declaration per (root, member); a repeat is refused
                // rather than silently overwriting a prior choice.
                if per_root.contains_key(&actor_id) {
                    return Err(WorldError::LootUnavailable);
                }
                per_root.insert(actor_id, decision);
                self.revision = self.revision.wrapping_add(1).max(1);
                let output = format!(
                    "You declare {} on {item_name}.\r\n",
                    loot_decision_label(decision)
                );
                self.loot_decision_outcome(session, output)
            }
            LootAction::Master { query, member } => {
                let npc_id = self.active_decision_encounter(actor_id, &room_id, true)?;
                if self.loot_encounters.get(&npc_id).map(|e| e.distribution)
                    != Some(loot_distribution::LootDistributionMode::MasterLoot)
                {
                    return Err(WorldError::LootUnavailable);
                }
                let recipient = self.resolve_connected_character(&member)?;
                if !self
                    .loot_encounters
                    .get(&npc_id)
                    .is_some_and(|encounter| encounter.cohort.contains(&recipient))
                {
                    return Err(WorldError::LootUnavailable);
                }
                let roots = self.declarable_loot_roots(&npc_id);
                let salt = self.observer(session)?.salt;
                let item_id = resolve_item_query(
                    &self.item_instances,
                    &self.item_prototypes,
                    roots.iter(),
                    &query,
                    salt,
                )
                .map_err(|_| WorldError::LootUnavailable)?;
                let item_name = self.item_effective_name(&item_id)?;
                let recipient_name = self
                    .connected_character_name(recipient)
                    .unwrap_or_else(|| "the member".to_owned());
                let encounter = self
                    .loot_encounters
                    .get_mut(&npc_id)
                    .ok_or(WorldError::LootUnavailable)?;
                encounter.master_assignments.insert(item_id, recipient);
                self.revision = self.revision.wrapping_add(1).max(1);
                let output = format!("You assign {item_name} to {recipient_name}.\r\n");
                self.loot_decision_outcome(session, output)
            }
        }
    }

    /// The carried loot roots of a frozen encounter's NPC that are still held by
    /// that NPC and therefore addressable for a group-loot declaration or
    /// assignment. These exact instance ids become the ground-loot roots at
    /// defeat, so a declaration keyed here resolves the same physical drop.
    pub(crate) fn declarable_loot_roots(&self, npc_id: &str) -> Vec<ItemInstanceId> {
        self.npcs
            .get(npc_id)
            .map(|npc| {
                npc.loot_item_ids
                    .iter()
                    .filter(|item_id| {
                        self.item_location(item_id)
                            == Some(&SharedItemLocation::HeldByNpc(npc_id.to_owned()))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolve the single live frozen encounter the actor may act on from their
    /// current room. `require_leader` restricts the match to encounters the
    /// actor initiated while still leading its party; otherwise any cohort
    /// member qualifies. The encounter must still be live (its NPC engaged by the
    /// initiator) and the match must be unambiguous, so an actor juggling two
    /// concurrent encounters fails closed rather than guessing.
    pub(crate) fn active_decision_encounter(
        &self,
        actor_id: CharacterId,
        room_id: &str,
        require_leader: bool,
    ) -> Result<String, WorldError> {
        let mut found: Option<String> = None;
        for (npc_id, encounter) in &self.loot_encounters {
            if encounter.room_id != room_id {
                continue;
            }
            if self
                .npcs
                .get(npc_id)
                .is_none_or(|npc| npc.engaged_character != Some(encounter.initiator))
            {
                continue;
            }
            let eligible = if require_leader {
                encounter.cohort.contains(&actor_id)
                    && encounter.initiator == actor_id
                    && encounter.party_id.is_none_or(|party_id| {
                        self.adventure.parties.get(&party_id).is_some_and(|party| {
                            party.leader == actor_id && party.members.contains(&actor_id)
                        })
                    })
            } else {
                encounter.cohort.contains(&actor_id)
            };
            if !eligible {
                continue;
            }
            if found.is_some() {
                return Err(WorldError::LootUnavailable);
            }
            found = Some(npc_id.clone());
        }
        found.ok_or(WorldError::LootUnavailable)
    }

    /// Render the cohort-private view of an in-combat group-loot encounter: its
    /// active mode, the declarable roots (with per-observer opaque refs) and the
    /// current declarations or master assignments. Text-only and bounded by the
    /// encounter's carried-loot cap, so Web and Telnet share it verbatim.
    pub(crate) fn render_loot_decisions(
        &self,
        session: SessionId,
        npc_id: &str,
    ) -> Result<String, WorldError> {
        let salt = self.observer(session)?.salt;
        let encounter = self
            .loot_encounters
            .get(npc_id)
            .ok_or(WorldError::LootUnavailable)?;
        let npc_name = self
            .npcs
            .get(npc_id)
            .map(|npc| npc.name.clone())
            .unwrap_or_default();
        let roots = self.declarable_loot_roots(npc_id);
        let mut output = format!(
            "Group loot for {npc_name} (mode: {}):\r\n",
            loot_distribution_label(encounter.distribution)
        );
        if roots.is_empty() {
            output.push_str("- (no carried loot to divide)\r\n");
        }
        for item_id in &roots {
            let prototype = self.item_prototype_for(item_id)?;
            let selector = opaque_id(salt, "item", item_id.as_str());
            match encounter.distribution {
                loot_distribution::LootDistributionMode::NeedGreed => {
                    let declarations = encounter.loot_declarations.get(item_id);
                    let count = |wanted: loot_distribution::LootDecision| {
                        declarations
                            .map(|members| {
                                members.values().filter(|value| **value == wanted).count()
                            })
                            .unwrap_or(0)
                    };
                    let declared = declarations.map(BTreeMap::len).unwrap_or(0);
                    let pending = encounter.cohort.len().saturating_sub(declared);
                    output.push_str(&format!(
                        "- {} (ref {}): {} need, {} greed, {} pass, {} pending\r\n",
                        prototype.name,
                        selector,
                        count(loot_distribution::LootDecision::Need),
                        count(loot_distribution::LootDecision::Greed),
                        count(loot_distribution::LootDecision::Pass),
                        pending,
                    ));
                }
                loot_distribution::LootDistributionMode::MasterLoot => {
                    match encounter
                        .master_assignments
                        .get(item_id)
                        .and_then(|member| self.connected_character_name(*member))
                    {
                        Some(name) => output.push_str(&format!(
                            "- {} (ref {}): assigned to {name}\r\n",
                            prototype.name, selector
                        )),
                        None => output.push_str(&format!(
                            "- {} (ref {}): unassigned\r\n",
                            prototype.name, selector
                        )),
                    }
                }
                loot_distribution::LootDistributionMode::RoundRobin => {
                    output.push_str(&format!("- {} (ref {})\r\n", prototype.name, selector));
                }
            }
        }
        Ok(output)
    }

    pub(crate) fn loot_decision_outcome(
        &self,
        session: SessionId,
        output: String,
    ) -> Result<ActionOutcome, WorldError> {
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    pub(crate) fn open_reward(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.open_reward_inner(session, query) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn open_reward_inner(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let source_item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        self.ensure_items_unreserved(std::iter::once(&source_item_id))?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let source_instance = self
            .item_instances
            .get(&source_item_id)
            .cloned()
            .ok_or(WorldError::RewardUnavailable)?;
        if source_instance.binding != (ItemBinding::Character { character_id })
            || !source_instance.item_affixes.is_empty()
        {
            return Err(WorldError::RewardUnavailable);
        }
        let source_prototype = self
            .item_prototypes
            .get(&source_instance.prototype_id)
            .cloned()
            .ok_or(WorldError::RewardUnavailable)?;
        let table_id = source_prototype
            .sealed_loot_table_id
            .clone()
            .ok_or(WorldError::RewardUnavailable)?;
        if source_prototype.transfer_policy != ItemTransferPolicy::CharacterBound
            || source_prototype.stack_policy != ItemStackPolicy::Unique
            || source_prototype.max_stack_units != 1
            || !source_prototype.consumable
            || source_prototype.container.is_some()
            || source_prototype.equipment_slot.is_some()
            || source_prototype.attack_bonus != 0
            || source_prototype.heal_amount != 0
            || source_prototype.effect.is_some()
        {
            return Err(WorldError::RewardUnavailable);
        }
        let source_provenance = self
            .item_provenance
            .get(&source_item_id)
            .cloned()
            .ok_or(WorldError::RewardUnavailable)?;
        if !matches!(
            &source_provenance,
            ItemProvenance::NpcLoot {
                sealed_loot_table_id: Some(authored_table_id),
                ..
            } if authored_table_id == &table_id
        ) {
            return Err(WorldError::RewardUnavailable);
        }
        let source_npc_loot_digest_sha256 = canonical_sha256(&source_provenance)?;
        let source_item_name = self.item_effective_name(&source_item_id)?;
        let open_id = self.next_reward_open_id;
        self.next_reward_open_id = RewardOpenId(
            open_id
                .0
                .checked_add(1)
                .ok_or(WorldError::RewardUnavailable)?,
        );
        let (table, units) = self.roll_loot_table(&table_id)?;
        if units.is_empty() || units.len() > MAX_SEALED_REWARD_UNITS {
            return Err(WorldError::RewardUnavailable);
        }

        let mut generated = Vec::with_capacity(units.len());
        for unit in units {
            let item_id = self.create_staged_item_instance(&unit.prototype_id)?;
            let item_affixes = unit
                .affix_pool_id
                .as_deref()
                .map(|pool_id| self.roll_item_affix(pool_id))
                .transpose()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            self.item_instances
                .get_mut(&item_id)
                .ok_or(WorldError::RewardUnavailable)?
                .item_affixes = item_affixes;
            generated.push((item_id, unit));
        }

        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&source_item_id)
            .ok_or(WorldError::RewardUnavailable)?
            .location = SharedItemLocation::Consumed;
        for (item_id, _) in &generated {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::RewardUnavailable)?
                .location = SharedItemLocation::Carried(character_id);
        }
        let mut roots = Vec::with_capacity(generated.len() + 1);
        roots.push(source_item_id.clone());
        roots.extend(generated.iter().map(|(item_id, _)| item_id.clone()));
        self.apply_item_ownership_transitions(&mut prospective, &roots)?;
        self.validate_prospective_item_locations(&prospective, [character_id])?;
        self.item_instances = prospective;

        let mut item_ids = Vec::with_capacity(generated.len());
        let mut item_names = Vec::with_capacity(generated.len());
        for (item_id, unit) in generated {
            self.item_provenance.insert(
                item_id.clone(),
                ItemProvenance::SealedReward {
                    open_id,
                    source_item_id: source_item_id.clone(),
                    source_npc_loot_digest_sha256: source_npc_loot_digest_sha256.clone(),
                    sealed_prototype_id: source_instance.prototype_id.clone(),
                    loot_table_id: table.id.clone(),
                    table_digest_sha256: table.digest_sha256.clone(),
                    entry_id: unit.entry_id,
                    pool_id: unit.pool_id,
                    roll_index: unit.roll_index,
                    quantity_index: unit.quantity_index,
                    generated_tick: self.logical_tick,
                },
            );
            item_names.push(self.item_effective_name(&item_id)?);
            item_ids.push(item_id);
        }
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::RewardOpened {
                open_id,
                character_id,
                character_name,
                source_item_id,
                source_item_name,
                item_ids,
                item_names: item_names.clone(),
                room_id,
            },
        };
        let outcome = self.finish_gameplay_action(
            session,
            format!(
                "Abres la recompensa sellada y obtienes: {}.\r\n",
                item_names.join(", ")
            ),
            vec![event],
        )?;
        if serde_json::to_vec(&self.runtime_snapshot())
            .map_err(|_| WorldError::RewardUnavailable)?
            .len()
            > MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        {
            return Err(WorldError::InventoryCapacityExceeded);
        }
        Ok(outcome)
    }

    pub(crate) fn roll_item_affix(
        &mut self,
        pool_id: &str,
    ) -> Result<Option<AppliedItemAffix>, WorldError> {
        let pool = self
            .affix_pools
            .get(pool_id)
            .cloned()
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        let selected = self.rng.uniform_below(pool.total_weight)?;
        let mut cumulative = 0_u64;
        let entry = pool
            .entries
            .iter()
            .find(|entry| {
                cumulative = cumulative.saturating_add(u64::from(entry.weight));
                selected < cumulative
            })
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        if entry.kind == AffixPoolEntryKind::Nothing {
            return Ok(None);
        }
        let definition = self
            .affixes
            .get(
                entry
                    .affix_id
                    .as_deref()
                    .ok_or(WorldError::InvalidGameplayDefinition)?,
            )
            .cloned()
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        let mut applied = AppliedItemAffix {
            affix_id: definition.id.clone(),
            definition_digest_sha256: canonical_sha256(&definition)?,
            pool_id: pool.id,
            pool_digest_sha256: pool.digest_sha256,
            record_digest_sha256: String::new(),
            label: definition.label,
            position: definition.position,
            attack_bonus_delta: definition.attack_bonus_delta,
            heal_amount_delta: definition.heal_amount_delta,
        };
        applied.record_digest_sha256 = applied_affix_record_sha256(&applied)?;
        Ok(Some(applied))
    }

    pub(crate) fn reward_odds(
        &self,
        session: SessionId,
        target: &str,
        page: u16,
    ) -> Result<ActionOutcome, WorldError> {
        if page == 0 {
            return Err(WorldError::RewardUnavailable);
        }
        let source_item_id = self.resolve_accessible_item(session, target, ItemScope::Carried)?;
        let character_id = self.character(session)?.id;
        if !self.item_is_openable_reward(&source_item_id, character_id) {
            return Err(WorldError::RewardUnavailable);
        }
        let source = self
            .item_instances
            .get(&source_item_id)
            .ok_or(WorldError::RewardUnavailable)?;
        let table_id = self
            .item_prototypes
            .get(&source.prototype_id)
            .and_then(|prototype| prototype.sealed_loot_table_id.as_deref())
            .ok_or(WorldError::RewardUnavailable)?;
        let table = self
            .loot_tables
            .get(table_id)
            .ok_or(WorldError::RewardUnavailable)?;

        enum OddsRow {
            Guaranteed(RewardOddsGuaranteedRow),
            Weighted(RewardOddsWeightedRow),
            Affix(RewardOddsAffixRow),
        }
        let mut rows = Vec::new();
        let append_affixes =
            |outcome_label: &str, pool_id: Option<&str>| -> Result<Vec<OddsRow>, WorldError> {
                let Some(pool_id) = pool_id else {
                    return Ok(Vec::new());
                };
                let pool = self
                    .affix_pools
                    .get(pool_id)
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let mut affix_rows = Vec::with_capacity(pool.entries.len());
                for entry in &pool.entries {
                    let (affix_label, position, attack_bonus_delta, heal_amount_delta) =
                        if entry.kind == AffixPoolEntryKind::Nothing {
                            ("sin afijo".to_owned(), None, 0, 0)
                        } else {
                            let definition = self
                                .affixes
                                .get(
                                    entry
                                        .affix_id
                                        .as_deref()
                                        .ok_or(WorldError::InvalidGameplayDefinition)?,
                                )
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            (
                                definition.label.clone(),
                                Some(definition.position),
                                definition.attack_bonus_delta,
                                definition.heal_amount_delta,
                            )
                        };
                    affix_rows.push(OddsRow::Affix(RewardOddsAffixRow {
                        parent_outcome_label: outcome_label.to_owned(),
                        affix_label,
                        position,
                        attack_bonus_delta,
                        heal_amount_delta,
                        conditional_probability_numerator: u64::from(entry.weight),
                        conditional_probability_denominator: pool.total_weight,
                    }));
                }
                Ok(affix_rows)
            };
        for guaranteed in &table.guaranteed {
            let label = self
                .item_prototypes
                .get(&guaranteed.item_prototype_id)
                .ok_or(WorldError::InvalidGameplayDefinition)?
                .name
                .clone();
            rows.push(OddsRow::Guaranteed(RewardOddsGuaranteedRow {
                label: label.clone(),
                quantity: guaranteed.quantity,
                affix_pool_label: guaranteed
                    .affix_pool_id
                    .as_ref()
                    .map(|_| "afijo aleatorio".to_owned()),
            }));
            rows.extend(append_affixes(&label, guaranteed.affix_pool_id.as_deref())?);
        }
        for (pool_index, pool) in table.pools.iter().enumerate() {
            let denominator = pool.entries.iter().try_fold(0_u64, |total, entry| {
                total
                    .checked_add(u64::from(entry.weight))
                    .ok_or(WorldError::InvalidGameplayDefinition)
            })?;
            for entry in &pool.entries {
                let (label, minimum, maximum) = if entry.kind == LootEntryKind::Nothing {
                    ("sin recompensa".to_owned(), 0, 0)
                } else {
                    (
                        self.item_prototypes
                            .get(
                                entry
                                    .item_prototype_id
                                    .as_deref()
                                    .ok_or(WorldError::InvalidGameplayDefinition)?,
                            )
                            .ok_or(WorldError::InvalidGameplayDefinition)?
                            .name
                            .clone(),
                        entry
                            .min_quantity
                            .ok_or(WorldError::InvalidGameplayDefinition)?,
                        entry
                            .max_quantity
                            .ok_or(WorldError::InvalidGameplayDefinition)?,
                    )
                };
                rows.push(OddsRow::Weighted(RewardOddsWeightedRow {
                    pool_label: format!("Tirada {}", pool_index + 1),
                    rolls: pool.rolls,
                    outcome_label: label.clone(),
                    min_quantity: minimum,
                    max_quantity: maximum,
                    probability_numerator: u64::from(entry.weight),
                    probability_denominator: denominator,
                    affix_pool_label: entry
                        .affix_pool_id
                        .as_ref()
                        .map(|_| "afijo aleatorio".to_owned()),
                }));
                rows.extend(append_affixes(&label, entry.affix_pool_id.as_deref())?);
            }
        }
        if rows.len() > MAX_REWARD_ODDS_ROWS {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let total_pages = rows.len().max(1).div_ceil(REWARD_ODDS_PAGE_SIZE);
        let page_index = page_index(page);
        if page_index >= total_pages {
            return Err(WorldError::RewardUnavailable);
        }
        let mut guaranteed = Vec::new();
        let mut weighted = Vec::new();
        let mut affixes = Vec::new();
        for row in rows
            .into_iter()
            .skip(page_index * REWARD_ODDS_PAGE_SIZE)
            .take(REWARD_ODDS_PAGE_SIZE)
        {
            match row {
                OddsRow::Guaranteed(row) => guaranteed.push(row),
                OddsRow::Weighted(row) => weighted.push(row),
                OddsRow::Affix(row) => affixes.push(row),
            }
        }
        let total_pages = u16::try_from(total_pages).map_err(|_| WorldError::RewardUnavailable)?;
        let selector = opaque_id(
            self.observer(session)?.salt,
            "item",
            source_item_id.as_str(),
        );
        let mut output = format!("Probabilidades de recompensa, página {page}/{total_pages}:\r\n");
        for row in &guaranteed {
            let affix = if row.affix_pool_label.is_some() {
                "; afijo aleatorio"
            } else {
                ""
            };
            output.push_str(&format!(
                "- {} × {}: garantizado{affix}.\r\n",
                row.quantity, row.label,
            ));
        }
        for row in &weighted {
            let affix = if row.affix_pool_label.is_some() {
                "; afijo aleatorio"
            } else {
                ""
            };
            output.push_str(&format!(
                "- {}: {} tirada(s); {}; cantidad {}..={}; probabilidad {}/{} por tirada{affix}.\r\n",
                row.pool_label,
                row.rolls,
                row.outcome_label,
                row.min_quantity,
                row.max_quantity,
                row.probability_numerator,
                row.probability_denominator,
            ));
        }
        for row in &affixes {
            let position = match row.position {
                Some(AffixPosition::Prefix) => "prefix",
                Some(AffixPosition::Suffix) => "suffix",
                None => "none",
            };
            output.push_str(&format!(
                "- {} → {}: posición {position}; ataque {:+}; curación {:+}; probabilidad {}/{} condicional.\r\n",
                row.parent_outcome_label,
                row.affix_label,
                row.attack_bonus_delta,
                row.heal_amount_delta,
                row.conditional_probability_numerator,
                row.conditional_probability_denominator
            ));
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: Some(RewardOddsView {
                selector,
                page,
                total_pages,
                guaranteed,
                weighted,
                affixes,
            }),
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }
}

impl World {
    pub(crate) fn freeze_loot_encounter(
        &mut self,
        npc_id: &str,
        character_id: CharacterId,
        room_id: &str,
    ) -> Result<(), WorldError> {
        if self.loot_encounters.contains_key(npc_id) {
            return Ok(());
        }
        if self.loot_encounters.len() >= MAX_LOOT_ENCOUNTERS {
            return Err(WorldError::LootUnavailable);
        }
        let party_id = self.party_for_member(character_id);
        let mut peers = Vec::new();
        if let Some(party_id) = party_id {
            let party = self
                .adventure
                .parties
                .get(&party_id)
                .ok_or(WorldError::PartyUnavailable)?;
            for connected in self.connected_characters()? {
                if connected.character_id != character_id
                    && connected.room_id == room_id
                    && party.members.contains(&connected.character_id)
                    && self
                        .character_gameplay
                        .get(&connected.character_id)
                        .is_some_and(|state| {
                            state.vitals.health > 0 && state.recovery_at_tick.is_none()
                        })
                {
                    peers.push(connected.character_id);
                }
            }
        }
        peers.sort_unstable();
        peers.dedup();
        peers.truncate(5);
        let mut cohort = peers;
        cohort.push(character_id);
        cohort.sort_unstable();
        self.loot_encounters.insert(
            npc_id.to_owned(),
            LootEncounter {
                npc_spawn_id: npc_id.to_owned(),
                initiator: character_id,
                party_id,
                cohort,
                room_id: room_id.to_owned(),
                started_tick: self.logical_tick,
                distribution: loot_distribution::LootDistributionMode::RoundRobin,
                loot_declarations: BTreeMap::new(),
                master_assignments: BTreeMap::new(),
            },
        );
        Ok(())
    }
}

impl World {
    pub fn loot_identity_ids(&self) -> BTreeSet<CharacterId> {
        self.loot_encounters
            .values()
            .flat_map(|encounter| encounter.cohort.iter().copied())
            .chain(
                self.ground_loot
                    .values()
                    .flat_map(|reservation| reservation.cohort.iter().copied()),
            )
            .collect()
    }

    pub fn cancel_all_loot_encounters_for_restart(
        &mut self,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let encounters = std::mem::take(&mut *self.loot_encounters);
        let mut events = Vec::with_capacity(encounters.len());
        for (npc_id, _) in encounters {
            let npc = self
                .npcs
                .get_mut(&npc_id)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            npc.engaged_character = None;
            npc.statuses.clear();
            npc.ability_cooldowns.clear();
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::LootEncounterCancelled {
                    npc_id,
                    npc_name: npc.name.clone(),
                    room_id: npc.spawn_room_id.clone(),
                },
            });
        }
        if !events.is_empty() {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(WorldError::InvalidDomainTransition)?;
        }
        Ok(events)
    }

    pub fn expire_ground_loot(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        let tick = self.logical_tick;
        let mut events = Vec::new();
        let roots = self.ground_loot.keys().cloned().collect::<Vec<_>>();
        for root in roots {
            let reservation = self
                .ground_loot
                .get(&root)
                .cloned()
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            if tick >= reservation.despawn_at_tick {
                let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
                let subtree = graph.subtree_ids(&root)?;
                let mut preserved = BTreeSet::new();
                for item_id in &subtree {
                    // Legacy v1-v5 snapshots deliberately label every
                    // restored unit as LegacySnapshot because those DTOs did
                    // not carry provenance. Identity still tells us whether a
                    // unit belongs to the current WorldPack. Preserve/stage
                    // those authored identities as tombstones too, otherwise
                    // omitting them from the next snapshot would make restore
                    // recreate fixed NPC loot at its authored location.
                    if self.initial_item_prototypes.contains_key(item_id)
                        || matches!(
                            self.item_provenance.get(item_id),
                            Some(ItemProvenance::AuthoredInitial)
                        )
                    {
                        preserved.extend(graph.subtree_ids(item_id)?);
                    }
                }
                let removed = subtree
                    .iter()
                    .filter(|item_id| !preserved.contains(*item_id))
                    .cloned()
                    .collect::<Vec<_>>();
                let reserved = self.reserved_item_ids()?;
                if removed.iter().any(|item_id| reserved.contains(item_id)) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                let detached = preserved
                    .iter()
                    .filter(|item_id| {
                        *item_id == &root
                            || self.item_instances.get(*item_id).is_some_and(|instance| {
                                matches!(
                                    &instance.location,
                                    SharedItemLocation::Contained(parent) if !preserved.contains(parent)
                                )
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                for item_id in detached {
                    self.set_item_location(&item_id, SharedItemLocation::Staged)?;
                }
                for item_id in &removed {
                    self.item_instances.remove(item_id);
                    self.item_provenance.remove(item_id);
                }
                self.ground_loot.remove(&root);
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootDespawned {
                        item_ids: subtree,
                        room_id: reservation.room_id,
                    },
                });
            } else if tick >= reservation.claim_until_tick && reservation.beneficiary.is_some() {
                self.ground_loot
                    .get_mut(&root)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?
                    .beneficiary = None;
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootRightsChanged {
                        item_id: root,
                        room_id: reservation.room_id,
                        summary: "The loot is now public.".into(),
                    },
                });
            }
        }
        self.party_loot_cursors
            .retain(|party_id, _| self.adventure.parties.contains_key(party_id));
        Ok(events)
    }
}
