# Registro de decisiones (ADR)

Los ADR gobiernan decisiones de arquitectura aceptadas; el estado de
implementación por fase vive en
[`../project/COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).

| # | Título | Estado | Fase |
| --- | --- | --- | --- |
| [0001](0001-project-foundation.md) | Project foundation | Aceptado | Fundación |
| [0002](0002-runtime-world-and-durable-identity.md) | Runtime world and durable identity | Aceptado | Núcleo runtime |
| [0003](0003-tiamud-product-identity.md) | Tiamud product identity | Aceptado | Identidad |
| [0004](0004-open-source-licensing.md) | Open-source licensing | Aceptado | Licencias |
| [0005](0005-hot-area-lifecycle.md) | Hot area lifecycle | Aceptado | Áreas 6A |
| [0006](0006-character-progression-and-postgresql-first.md) | Character progression and PostgreSQL-first | Aceptado | Personajes v3 |
| [0007](0007-worldpack-v3-character-model.md) | WorldPack v3 character model | Aceptado | Personajes v3 |
| [0008](0008-optional-scene-compositor.md) | Optional scene compositor | Aceptado | Compositor |
| [0009](0009-item-prototypes-instances-and-capacity.md) | Item prototypes, instances and capacity | Aceptado | Inventario 3A |
| [0010](0010-derived-stacks-and-bounded-containers.md) | Derived stacks and bounded containers | Aceptado | Inventario 3B |
| [0011](0011-atomic-give-and-item-transfer-policies.md) | Atomic give and item transfer policies | Aceptado | Inventario 3C |
| [0012](0012-confirmed-trade-reservations.md) | Confirmed trade reservations | Aceptado | Inventario 3D |
| [0013](0013-repeatable-loot-rights-and-provenance.md) | Repeatable loot rights and provenance | Aceptado | Botín 4A |
| [0014](0014-earned-sealed-rewards-and-deterministic-affixes.md) | Earned sealed rewards and deterministic affixes | Aceptado | Recompensas 4B |
| [0015](0015-authored-online-telepathy.md) | Authored online telepathy | Aceptado | Social 5A |
| [0016](0016-authored-room-emotes.md) | Authored room emotes | Aceptado | Social 5B |
| [0017](0017-bounded-offline-mail.md) | Bounded offline mail | Aceptado | Social 5C |
| [0018](0018-bounded-moderable-community-boards.md) | Bounded moderable community boards | Aceptado | Social 5D |
| [0019](0019-bounded-spatial-voice.md) | Bounded spatial voice | Aceptado | Social 5E |
| [0020](0020-additive-local-area-loading.md) | Additive local area loading | Aceptado | Áreas 6B |
| [0021](0021-quiescent-area-generation-cutover.md) | Quiescent area generation cutover | Aceptado | Áreas 6C |
| [0022](0022-reversible-area-unit-retirement.md) | Reversible area-unit retirement | Aceptado (implementación parcial) | Áreas 6D |
| [0023](0023-need-greed-and-master-loot-distribution.md) | Need/greed and master-loot distribution | Aceptado (implementación parcial) | Botín 4C |
| [0024](0024-phase-6d-acceptance-closure-plan.md) | Phase 6D acceptance closure plan | Aceptado (implementación parcial: Gate 2) | Áreas 6D |
| [0025](0025-narrative-ruleset-story-core.md) | Narrative ruleset (story-core), StoryPack and instanced story sessions | Propuesto (prototipo S0 de biblioteca) | Narrativa S0–S8 |
| [0026](0026-typed-protocol-source-and-typescript-codegen.md) | Typed `tiamud.v1` source and generated TypeScript | Propuesto (implementación incremental) | Protocolo navegador |
| [0028](0028-story-menus-portraits-and-dialogue-trees.md) | Story verb menus, portraits and dialogue trees (StoryPack v2) | Propuesto (modelo de núcleo de biblioteca) | Narrativa S4 |
| [0030](0030-story-save-slots-and-schema-v16.md) | Story save slots and schema v16 | Propuesto (capa de persistencia, sin conectar a sesiones) | Narrativa S3 |
| [0029](0029-story-session-instances-and-story-protocol.md) | Story session instances and `story.*` protocol | Propuesto (implementación parcial S2: un jugador, en memoria) | Narrativa S2 |

El estado de fase (`Complete`/`Partial`) es autoridad de
[`../project/COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
