# ADR 0006: Data-driven character progression and PostgreSQL-first operations

- Status: Accepted; implemented
- Date: 2026-07-16

## Context

The first playable loop persisted a compact fixed character model and advanced
levels with a legacy formula. That proved combat, restart and projection, but
authors could not define distinct classes, resource economies, spells,
techniques or level rewards without changing Rust. NPC combat likewise had no
shared authored ability vocabulary.

PostgreSQL has since gained a schema-v8 runtime adapter and retained acceptance
evidence. SQLite remains valuable for local play and existing installations,
but new public single-host deployments benefit from PostgreSQL's standard
operational monitoring and recovery ecosystem. This does not justify splitting
the authoritative server or introducing orchestration.

## Decision

- Keep one Rust server process and one authoritative world actor on one
  ordinary host. PostgreSQL, when selected, runs as the persistence service for
  that host; Kubernetes, Redis, object storage and microservices are not
  requirements.
- Recommend PostgreSQL for new public installations. Retain the executable's
  current zero-configuration SQLite fallback at `data/tiamud.db` for local play
  and legacy installations. Open exactly one backend per process, never
  dual-write, and use the explicit offline SQLite-to-PostgreSQL mover when
  changing authority.
- Keep account, ownership, moderation and other data queried by the server in
  relational columns and tables. Character gameplay and world runtime snapshots
  are bounded opaque application payloads stored as JSON in `TEXT` on both
  adapters because they are loaded atomically, not queried ad hoc. Do not use
  PostgreSQL `jsonb` merely because the payload syntax is JSON.
- Supersede only ADR 0001's operating recommendation that described PostgreSQL
  as a future alternative. ADR 0001's single-process, text-complete and bounded
  architecture remains accepted. PostgreSQL-first is an operating profile, not
  a claim that the zero-variable executable default has changed.
- Make the character rules optional, bounded WorldPack data: archetypes define
  initial attributes, initial skill ranks, resource pools and regeneration,
  resistances and known abilities. Skill growth comes from authored level
  milestones; the existing post-combat `combat` rank increment remains the
  current legacy gameplay rule, not a general training system. A pack with
  archetypes declares one authored default; packs without them retain the
  legacy character path.
- Express advancement as cumulative XP milestones with contiguous authored
  levels. A milestone may grant attributes, skills, resource capacity and new
  abilities. Persist the resulting sheet and dynamic combat state rather than
  recomputing it from the current pack on every connection.
- Use one ability definition for martial techniques, spells and innate NPC
  actions. Definitions declare resource costs, targeting, per-ability cooldown,
  damage or healing with optional attribute scaling, and bounded timed status
  effects. Damage types and authored resistance percentages are authoritative
  domain rules, not presentation hints.
- Apply a one-logical-tick global ability cooldown to player abilities in
  addition to authored cooldowns. The world actor validates learned ability,
  target, resources, cooldowns, recovery and status capacity before committing
  the action.
- Give NPCs bounded resource pools, resistances and authored ability sets. NPC
  combat uses deterministic rule-based scoring once per combat turn and falls
  back to the existing basic attack when no ability is usable. This is gameplay
  logic, not runtime generative AI.
- Expose the same domain behavior through structured browser actions and text
  commands. Browser character creation and the character sheet add accessible
  native controls; Telnet retains complete text commands and offers archetype
  selection during creation. Older browser servers and WorldPacks degrade to
  the legacy flow instead of becoming unusable.
- Keep AI optional and outside play. Lightweight local or remote-compatible
  editorial adapters may draft offline WorldPack material, and development may
  be AI-assisted, but every material output remains subject to human review,
  validators and ordinary test evidence.

## Consequences

Most class balance, magic, techniques and level evolution can now be changed in
reviewed data without branching the engine. The shared definition model avoids
parallel hard-coded systems for spells, skills and NPC powers, while strict
limits and deterministic canonicalization keep validation and review tractable.

Compatibility has an explicit cost: the engine retains the old fixed defaults
and level formula when authored archetypes are absent, and clients must accept
projections that omit new optional fields. Saved characters keep their durable
progression state across restart; changing a WorldPack does not silently rebuild
or rebalance existing characters.

Published archetype, ability, resource and stat IDs are durable content
identity. Snapshot restoration filters unknown ephemeral state and safely drops
or ignores learned abilities that no longer exist in the active definitions.
That makes removal fail safely, but it does not make renames transparent:
renaming a published ID still needs an explicit alias or content-migration
facility, which remains future work.

PostgreSQL-first changes the recommendation for a new public host, not the
single-host topology or the code's no-configuration SQLite behavior. PostgreSQL
backup, TLS, monitoring and recovery remain operator responsibilities;
`tiamud-admin` backup/check/restore covers SQLite, while PostgreSQL deployments
use native PostgreSQL procedures. The repository's disposable acceptance run
is not proof of a production installation. No Kubernetes support or
distributed-world behavior is implied.

The implementation does not claim a complete game's class catalog, an initial
stat preview in the creation form, a visual skill tree, automated balance, PvP,
general ally selection in the current web scene, a successful 24-hour soak,
current WebKit execution or a manual screen-reader pass. Those are content
choices, future features or separate acceptance work.
