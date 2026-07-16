# ADR 0007: WorldPack v3 character composition and class promotion

- Status: Accepted; implementation in progress
- Date: 2026-07-16

## Context

ADR 0006 established a useful data-driven archetype, level and ability system.
The implementation proved persistence, browser/Telnet parity and authored
combat, but `archetype` now means creation package, base class, resource model
and permanent level track at the same time. There is no independent lineage,
background, specialization or advanced class.

Tiamud has no public deployment, stable protocol release or production
character saves. Preserving the overloaded term in a new public API would
therefore create permanent complexity without protecting users. Historical
Envy/Merc areas and WorldPack v2 remain valuable importer inputs.

## Decision

- Publish the clean character model as WorldPack v3 and strict AreaPack v2.
  Normalize v2/AreaPack v1/Envy content at import or composition boundaries;
  do not make the v3 runtime carry `archetype` as its canonical identity.
- Use `lineage`, optional `background`, `base class`, `talent` or
  `specialization`, and `advanced class` as the canonical terms. Reserve
  `transformation` for a later independent overlay. Treat `race` and
  `archetype` as importer/UI aliases only.
- Keep one global cumulative XP and level curve. Milestones may award bounded
  lineage and class development points. Do not add class XP, general
  multiclassing or separate lineage XP in the MVP.
- Keep exactly one active class. Acquiring or ranking a talent improves the
  character but never changes class identity. Promoting to an advanced class
  is a separate atomic domain action that records class history, applies
  one-time grants and changes future class rewards without resetting XP.
- Express eligibility through a closed, depth- and count-bounded requirement
  tree. Express permanent effects through a closed, checked grant vocabulary.
  Permanent grants cannot execute Lua or arbitrary expressions.
- Resolve package and grant composition deterministically. Maximum-health and
  resource-capacity increases are not implicit combat refills. Validate final
  cumulative caps during publication and application.
- Ship a deliberately small demonstration catalog: Mortal, Stonekin and
  Embermarked lineages; Caravan Scout, Archive Apprentice and City Guard
  backgrounds; Wanderer, Vanguard and Arcanist base classes; Pathfinder,
  Bastion, Stormcaller and cross-entry Spellblade advanced classes.
- Keep IDs stable, lowercase and namespaced, with explicit UI order. Add full
  alias, tombstone and save-migration machinery before a public version needs
  functional ID renames; do not implement a production migration framework
  for deployments that do not exist.
- Keep PostgreSQL-first public operation, SQLite fallback, the shared ability
  model and the single-host/single-writer architecture from ADR 0006.

The normative details are in
[`character-progression-v3.md`](../design/character-progression-v3.md).

## Consequences

World authors can combine identity, personal history and role without defining
every lineage × class pair. Advanced class requirements can demonstrate level,
attributes, skills, nodes and world progress without hard-coded Rust branches.
One global curve and one active class keep balance, UI and persistence
tractable.

WorldPack v3 is a deliberate pre-release schema break. The repository must
update its reviewed world, composition tests, server loader, durable snapshot,
creation flows and clients together before claiming the model is shipped. A
v2 normalizer preserves practical importer value but is not a promise that an
old binary understands v3 fields.

This ADR supersedes the character-package and per-archetype progression parts
of ADR 0006. ADR 0006 remains authoritative for abilities, combat validation,
persistence backend selection and single-host operations.

Multiclassing, class ranks, reversible promotion, arbitrary progression
scripts, culture content, transformations, a graph editor and automated game
balance are explicitly deferred.
