# Tiamud documentation

This directory holds Tiamud's technical and project documentation: the
agent contract, specification, authoring guides, architecture decisions,
design, operation and protocol.

## Language policy

The public-facing documents — this index, the specification, authoring guides,
protocol, operation guides and design docs — are maintained in **English** to
reach a wider audience. The internal governance documents — `../AGENTS.md`,
`../CLAUDE.md` and the ADRs under `decisions/` — are maintained in
**Spanish**. The archived `archive/spec-v0.2.md` stays in Spanish as a
historical snapshot.

## Orientation order

1. [`../AGENTS.md`](../AGENTS.md) — canonical agent contract.
2. The relevant ADRs in [`decisions/`](decisions/).
3. The relevant design/specification, and the current implementation and tests.

## Project records

The project's status records (completion matrix, project memory and dated
logs) are kept privately, outside this repository, and are not published.
Accepted ADRs govern architecture decisions; the code and tests remain the
authority over current behavior.

## Map of docs/

| Folder/File | Purpose |
| --- | --- |
| [`../AGENTS.md`](../AGENTS.md) | Canonical agent contract: invariants, verification and honest limits (Spanish). |
| [`spec-v0.3.md`](spec-v0.3.md) | Current specification (characters, inventory, media). |
| [`archive/spec-v0.2.md`](archive/spec-v0.2.md) | Historical specification in Spanish (archived; context, not current). |
| [`world-authoring.md`](world-authoring.md) | World authoring guide (WorldPack/AreaPack). |
| [`authoring/`](authoring/) | Authoring guide split by topic (README, media, modular worlds, progression/combat, validation/publishing, workflow). |
| [`decisions/`](decisions/) | Architecture Decision Records (ADR 0001–0024, Spanish). See [`decisions/README.md`](decisions/README.md). |
| [`design/`](design/) | Design documents and roadmaps (v3 progression, scene compositor, social/staff/loot roadmap). |
| [`operations/`](operations/) | Operations runbooks (releases, single-host deployment, soak testing). |
| [`protocol/`](protocol/) | Protocol specification (`tiamud-v1.md`). |
| [`images/`](images/) | README graphic assets (logo, captures). |
