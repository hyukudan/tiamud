# Tiamud documentation

This directory holds Tiamud's technical and project documentation: the
agent contract, specification, authoring guides, architecture decisions,
design, operation, protocol and status meta-documents.

## Language policy

The public-facing documents — this index, the specification, authoring guides,
protocol, operation guides and design docs — are maintained in **English** to
reach a wider audience. The internal governance documents — `../AGENTS.md`,
`../CLAUDE.md`, `project/PROJECT_MEMORY.md`, `project/COMPLETION_MATRIX.md`, the
ADRs under `decisions/` and the dated logs under `project/log/` — are
maintained in **Spanish**. The archived `archive/spec-v0.2.md` stays in Spanish
as a historical snapshot.

## Orientation order

1. [`../AGENTS.md`](../AGENTS.md) — canonical agent contract.
2. [`project/PROJECT_MEMORY.md`](project/PROJECT_MEMORY.md) — project memory and
   history.
3. [`project/COMPLETION_MATRIX.md`](project/COMPLETION_MATRIX.md) — completion
   matrix.
4. The relevant ADRs in [`decisions/`](decisions/).
5. The relevant design/specification, and the current implementation and tests.

## Single source of truth

`project/COMPLETION_MATRIX.md` is the SOLE AUTHORITY for test counts, versions
(WorldPack/AreaPack/snapshot/SQL schema) and each phase's status
(`Complete`/`Partial`). When the matrix conflicts with any text in the README,
CLAUDE.md, AGENTS.md or an older ADR, the matrix prevails. Accepted ADRs still
govern architecture decisions.

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
| [`project/`](project/) | Project meta-documents (Spanish): `PROJECT_MEMORY.md`, `COMPLETION_MATRIX.md`, `NAME_DOMAIN_PRECHECK.md` and `log/`. |
| [`project/log/`](project/log/) | Dated logs per phase/milestone (Spanish). See [`project/log/README.md`](project/log/README.md). |
| [`images/`](images/) | README graphic assets (logo, captures). |
| [`reviews/`](reviews/) | External reviews/audit (the "Fable" series). (Not versioned as a status source.) |
