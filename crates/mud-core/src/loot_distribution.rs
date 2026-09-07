//! Pure, deterministic core logic for advanced group loot distribution.
//!
//! This module implements the *domain resolution* slice of
//! [ADR 0023](../../../docs/decisions/0023-need-greed-and-master-loot-distribution.md):
//! the `need_greed` and `master_loot` distribution modes that extend, and never
//! replace, the `party_round_robin` default already delivered in phase 4A.
//!
//! Scope of this slice is deliberately narrow. It is transport- and
//! persistence-free: it owns only the deterministic, exactly-once *decision* of
//! which cohort member wins a single loot root given a frozen cohort, the per
//! member declarations (or the leader's assignment) and an unbiased RNG source.
//! The world wires this resolver into `LootEncounter`, snapshot v9 and the
//! carried-root decisions resolved at NPC defeat. The actor persists those
//! mutations before acknowledgement. Post-defeat decision windows remain
//! deferred; this pure module does not own SQL or client protocols.
//!
//! The resolution mirrors the ADR exactly:
//!
//! 1. every `need` beats every `greed`;
//! 2. inside the winning set (all `need`, or all `greed` when nobody needed) a
//!    tie is broken with the same unbiased integer rejection sampling that 4A
//!    uses, over candidates in a stable ascending `CharacterId` order;
//! 3. if a numeric tie were still unresolved, the final stable tiebreak is the
//!    smallest `CharacterId`; and
//! 4. if nobody declared `need` or `greed` (all `pass` or silence), the root
//!    falls back to the round-robin / public path.
//!
//! Per the ADR non-goals, this module has no pity, no boxes, no keys, no premium
//! currency and no paid rolls. It is pure arithmetic over identities.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{CharacterId, DeterministicRng, WorldError};

/// Maximum cohort size (initiator included) frozen at encounter time, matching
/// the bound already enforced for `LootEncounter` cohorts in phase 4A.
pub const MAX_COHORT: usize = 6;

/// The closed vocabulary of group loot distribution modes.
///
/// `RoundRobin` is the default: it is what applies when there is no party, when
/// the cohort collapses to a single person, or when the party leader makes no
/// explicit choice. The advanced modes are a runtime choice of the leader and
/// never derived from prose, XP or class.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum LootDistributionMode {
    /// Phase 4A behaviour, unchanged, and the fail-safe default.
    #[default]
    RoundRobin,
    /// Each eligible cohort member declares `need`, `greed` or `pass` and the
    /// resolution is deterministic.
    NeedGreed,
    /// The party leader assigns each root to a concrete cohort member.
    MasterLoot,
}

/// A single cohort member's per-root declaration under `need_greed`.
///
/// `Pass` is equivalent to not declaring; the resolver treats an absent entry
/// and an explicit `Pass` identically.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootDecision {
    /// The member needs the root to use it; needs beat greeds.
    Need,
    /// The member only covets the root; greeds only win when nobody needs.
    Greed,
    /// The member declines; equivalent to silence.
    Pass,
}

/// The deterministic outcome of resolving one loot root.
///
/// `FallbackRoundRobin` means the advanced mode produced no winner (nobody
/// needed or greeded, or the leader made no valid assignment) and the caller
/// must resolve the root through the unchanged 4A round-robin / public path.
/// A root can therefore never be stuck under an advanced mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LootResolution {
    /// The root is reserved exclusively for this cohort member.
    Winner(CharacterId),
    /// No advanced winner; resolve the root via the round-robin fallback.
    FallbackRoundRobin,
}

/// An unbiased tiebreak RNG source.
///
/// The trait keeps the resolver decoupled from any concrete generator while
/// guaranteeing the rejection-sampled, bias-free integer draw the ADR requires.
/// Production wiring uses the crate's [`DeterministicRng`], so the advanced
/// resolution shares the exact same unbiased sampler as 4A loot.
pub trait TieBreakRng {
    /// Return a uniformly distributed index in `0..count` without modulo bias.
    ///
    /// `count` must be non-zero; a zero or out-of-range count fails closed.
    fn tiebreak_index(&mut self, count: usize) -> Result<usize, WorldError>;
}

impl TieBreakRng for DeterministicRng {
    fn tiebreak_index(&mut self, count: usize) -> Result<usize, WorldError> {
        let upper = u64::try_from(count).map_err(|_| WorldError::InvalidDomainTransition)?;
        // `uniform_below` is the same rejection sampler phase 4A uses for loot,
        // and it fails closed on a zero upper bound.
        let drawn = self.uniform_below(upper)?;
        usize::try_from(drawn).map_err(|_| WorldError::InvalidDomainTransition)
    }
}

/// Validate a frozen cohort: non-empty, at most [`MAX_COHORT`], positive
/// identities, strictly ascending (stable order by `CharacterId`, no
/// duplicates). This mirrors the invariants 4A already enforces on
/// `LootEncounter` cohorts, so an ill-formed cohort fails closed rather than
/// resolving.
fn validate_cohort(cohort: &[CharacterId]) -> Result<(), WorldError> {
    let ordered = cohort.windows(2).all(|pair| pair[0] < pair[1]);
    if cohort.is_empty()
        || cohort.len() > MAX_COHORT
        || cohort.iter().any(|member| member.0 <= 0)
        || !ordered
    {
        return Err(WorldError::LootUnavailable);
    }
    Ok(())
}

/// Deterministically resolve one loot root under `need_greed`.
///
/// Given the frozen cohort (stable ascending order by `CharacterId`), each
/// member's declaration and an unbiased RNG, produce the winning member or the
/// round-robin fallback. The resolution is exactly-once in the conceptual sense:
/// the same cohort, declarations and RNG seed always produce the same winner.
///
/// Foreign declaration keys, an out-of-order cohort or an over-large cohort are
/// rejected closed. `Pass` and absent declarations are treated identically.
pub fn resolve_need_greed<R: TieBreakRng>(
    cohort: &[CharacterId],
    declarations: &BTreeMap<CharacterId, LootDecision>,
    rng: &mut R,
) -> Result<LootResolution, WorldError> {
    validate_cohort(cohort)?;
    // Every declaration must belong to the frozen cohort; a foreign identity is
    // schema-smuggling and fails closed.
    if declarations.keys().any(|member| !cohort.contains(member)) {
        return Err(WorldError::LootUnavailable);
    }

    // Needs beat greeds: the winning set is all needers, or all greeders only
    // when nobody needed. Iterating the cohort preserves the stable ascending
    // `CharacterId` order, so `candidates[0]` is always the smallest identity.
    let winning_set = |wanted: LootDecision| -> Vec<CharacterId> {
        cohort
            .iter()
            .copied()
            .filter(|member| declarations.get(member) == Some(&wanted))
            .collect()
    };

    let mut candidates = winning_set(LootDecision::Need);
    if candidates.is_empty() {
        candidates = winning_set(LootDecision::Greed);
    }

    match candidates.len() {
        // Nobody needed or greeded: fall back to round-robin. Never stuck.
        0 => Ok(LootResolution::FallbackRoundRobin),
        // A single claimant wins outright; no roll perturbs the RNG sequence.
        1 => Ok(LootResolution::Winner(candidates[0])),
        // A genuine tie: break it with an unbiased integer draw over the stably
        // ordered candidates. If the draw were ever out of range, the final
        // stable tiebreak is the smallest `CharacterId` (`candidates[0]`).
        count => {
            let index = rng.tiebreak_index(count)?;
            let winner = candidates.get(index).copied().unwrap_or(candidates[0]);
            Ok(LootResolution::Winner(winner))
        }
    }
}

/// Deterministically resolve one loot root under `master_loot`.
///
/// The leader's assignment is itself the reservation. A recipient outside the
/// frozen cohort is rejected closed (never silently reassigned). No assignment
/// (`None`) means the leader did not assign in the window, so the root falls
/// back to round-robin. There is no RNG: the outcome is the leader's choice.
pub fn resolve_master_loot(
    cohort: &[CharacterId],
    assigned_to: Option<CharacterId>,
) -> Result<LootResolution, WorldError> {
    validate_cohort(cohort)?;
    match assigned_to {
        Some(recipient) if cohort.contains(&recipient) => Ok(LootResolution::Winner(recipient)),
        // Assignment to a non-cohort identity is rejected, not silently dropped.
        Some(_) => Err(WorldError::LootUnavailable),
        // No assignment in the window: fall back to round-robin.
        None => Ok(LootResolution::FallbackRoundRobin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cohort(ids: &[i64]) -> Vec<CharacterId> {
        ids.iter().copied().map(CharacterId).collect()
    }

    fn declare(pairs: &[(i64, LootDecision)]) -> BTreeMap<CharacterId, LootDecision> {
        pairs
            .iter()
            .map(|(id, decision)| (CharacterId(*id), *decision))
            .collect()
    }

    fn rng(seed: u64) -> DeterministicRng {
        DeterministicRng::new(seed)
    }

    #[test]
    fn round_robin_is_the_default_mode() {
        assert_eq!(
            LootDistributionMode::default(),
            LootDistributionMode::RoundRobin
        );
    }

    #[test]
    fn need_beats_greed_regardless_of_seed() {
        let members = cohort(&[1, 2, 3]);
        let declarations = declare(&[
            (1, LootDecision::Greed),
            (2, LootDecision::Need),
            (3, LootDecision::Greed),
        ]);
        // The single needer wins for every seed; greeders never contend.
        for seed in 1..=200u64 {
            let resolved = resolve_need_greed(&members, &declarations, &mut rng(seed)).unwrap();
            assert_eq!(resolved, LootResolution::Winner(CharacterId(2)));
        }
    }

    #[test]
    fn greed_wins_only_when_nobody_needs() {
        let members = cohort(&[4, 5, 6]);
        let declarations = declare(&[
            (4, LootDecision::Greed),
            (5, LootDecision::Pass),
            (6, LootDecision::Greed),
        ]);
        let resolved = resolve_need_greed(&members, &declarations, &mut rng(7)).unwrap();
        match resolved {
            LootResolution::Winner(id) => {
                assert!(id == CharacterId(4) || id == CharacterId(6));
            }
            other => panic!("expected a greed winner, got {other:?}"),
        }
    }

    #[test]
    fn resolution_is_deterministic_for_a_fixed_seed() {
        let members = cohort(&[10, 20, 30]);
        let declarations = declare(&[
            (10, LootDecision::Need),
            (20, LootDecision::Need),
            (30, LootDecision::Need),
        ]);
        let first = resolve_need_greed(&members, &declarations, &mut rng(4242)).unwrap();
        let second = resolve_need_greed(&members, &declarations, &mut rng(4242)).unwrap();
        assert_eq!(first, second);
        // A different seed is allowed to differ; it must still be a valid winner.
        let third = resolve_need_greed(&members, &declarations, &mut rng(9001)).unwrap();
        match third {
            LootResolution::Winner(id) => assert!(members.contains(&id)),
            other => panic!("expected a winner, got {other:?}"),
        }
    }

    #[test]
    fn tiebreak_between_two_equals_is_unbiased() {
        // Two needers, one root, many seeds: an unbiased sampler must split the
        // wins roughly evenly. A biased or constant sampler fails this.
        let members = cohort(&[100, 200]);
        let declarations = declare(&[(100, LootDecision::Need), (200, LootDecision::Need)]);
        let trials = 4000u64;
        let mut wins_low = 0u64;
        for seed in 1..=trials {
            match resolve_need_greed(&members, &declarations, &mut rng(seed)).unwrap() {
                LootResolution::Winner(CharacterId(100)) => wins_low += 1,
                LootResolution::Winner(CharacterId(200)) => {}
                other => panic!("unexpected resolution {other:?}"),
            }
        }
        // Expect ~50%. Assert a generous band to avoid flakiness while still
        // catching real bias (e.g. always-first).
        let fraction = wins_low as f64 / trials as f64;
        assert!(
            (0.40..=0.60).contains(&fraction),
            "tiebreak looks biased: low-id won {fraction} of {trials}"
        );
    }

    #[test]
    fn every_pass_falls_back_to_round_robin() {
        let members = cohort(&[1, 2, 3]);
        let declarations = declare(&[
            (1, LootDecision::Pass),
            (2, LootDecision::Pass),
            (3, LootDecision::Pass),
        ]);
        let resolved = resolve_need_greed(&members, &declarations, &mut rng(1)).unwrap();
        assert_eq!(resolved, LootResolution::FallbackRoundRobin);
    }

    #[test]
    fn silence_falls_back_to_round_robin() {
        let members = cohort(&[1, 2, 3]);
        let declarations = BTreeMap::new();
        let resolved = resolve_need_greed(&members, &declarations, &mut rng(1)).unwrap();
        assert_eq!(resolved, LootResolution::FallbackRoundRobin);
    }

    #[test]
    fn foreign_declaration_is_rejected() {
        let members = cohort(&[1, 2, 3]);
        let declarations = declare(&[(99, LootDecision::Need)]);
        assert!(matches!(
            resolve_need_greed(&members, &declarations, &mut rng(1)),
            Err(WorldError::LootUnavailable)
        ));
    }

    #[test]
    fn master_loot_assigns_to_a_cohort_member() {
        let members = cohort(&[1, 2, 3]);
        let resolved = resolve_master_loot(&members, Some(CharacterId(2))).unwrap();
        assert_eq!(resolved, LootResolution::Winner(CharacterId(2)));
    }

    #[test]
    fn master_loot_outside_cohort_is_rejected() {
        let members = cohort(&[1, 2, 3]);
        assert!(matches!(
            resolve_master_loot(&members, Some(CharacterId(4))),
            Err(WorldError::LootUnavailable)
        ));
    }

    #[test]
    fn master_loot_without_assignment_falls_back() {
        let members = cohort(&[1, 2, 3]);
        let resolved = resolve_master_loot(&members, None).unwrap();
        assert_eq!(resolved, LootResolution::FallbackRoundRobin);
    }

    #[test]
    fn unordered_or_invalid_cohort_is_rejected() {
        let declarations = BTreeMap::new();
        let rejected = |ids: &[i64]| {
            matches!(
                resolve_need_greed(&cohort(ids), &declarations, &mut rng(1)),
                Err(WorldError::LootUnavailable)
            )
        };
        // Out of ascending order.
        assert!(rejected(&[3, 1, 2]));
        // Duplicate identity (not strictly ascending).
        assert!(rejected(&[1, 1, 2]));
        // Non-positive identity.
        assert!(rejected(&[0, 1, 2]));
        // Empty cohort.
        assert!(rejected(&[]));
        // Over the maximum cohort size.
        assert!(rejected(&[1, 2, 3, 4, 5, 6, 7]));
    }

    #[test]
    fn stable_final_tiebreak_prefers_smallest_character_id() {
        // A stub RNG that always returns an out-of-range index forces the
        // documented final fallback: the smallest `CharacterId` in the stably
        // ordered candidate set.
        struct OutOfRangeRng;
        impl TieBreakRng for OutOfRangeRng {
            fn tiebreak_index(&mut self, _count: usize) -> Result<usize, WorldError> {
                Ok(usize::MAX)
            }
        }
        let members = cohort(&[7, 8, 9]);
        let declarations = declare(&[
            (7, LootDecision::Need),
            (8, LootDecision::Need),
            (9, LootDecision::Need),
        ]);
        let resolved = resolve_need_greed(&members, &declarations, &mut OutOfRangeRng).unwrap();
        assert_eq!(resolved, LootResolution::Winner(CharacterId(7)));
    }

    #[test]
    fn needers_are_selected_in_stable_order() {
        // Only the middle and last members need; the resolver must pick between
        // exactly those two, never the greeder, and preserve ascending order.
        let members = cohort(&[11, 22, 33]);
        let declarations = declare(&[
            (11, LootDecision::Greed),
            (22, LootDecision::Need),
            (33, LootDecision::Need),
        ]);
        for seed in 1..=200u64 {
            match resolve_need_greed(&members, &declarations, &mut rng(seed)).unwrap() {
                LootResolution::Winner(id) => {
                    assert!(id == CharacterId(22) || id == CharacterId(33));
                }
                other => panic!("unexpected resolution {other:?}"),
            }
        }
    }
}
