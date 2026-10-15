# Guía de Claude para Tiamud

Lee [`AGENTS.md`](AGENTS.md) completo antes de modificar este repositorio. Es
el contrato canónico entre agentes. Este archivo es una entrada resumida para
Claude Code y no sustituye los invariantes, verificaciones ni límites de estado
honesto de `AGENTS.md`.

## Orden de orientación

1. `AGENTS.md`.
2. `docs/project/PROJECT_MEMORY.md`.
3. `docs/project/COMPLETION_MATRIX.md`.
4. ADR y diseño/especificación pertinentes.
5. Implementación y pruebas actuales.

Ante conflictos de estado, la matriz prevalece sobre registros antiguos y
texto obsoleto de un ADR. Los ADR siguen gobernando decisiones aceptadas.
`docs/archive/spec-v0.2.md` es contexto histórico en castellano; `docs/spec-v0.3.md`
describe el baseline vigente de personajes, inventario y medios.

## Estado actual resumido

Tiamud es un MUD con servidor Rust (actor de mundo con escritor único),
clientes navegador y Telnet que comparten dominio, persistencia y proyección, y
un backend único PostgreSQL o SQLite (esquema v17). El baseline vigente cubre
personajes v3, inventario con instancias/pilas/contenedores,
`give`/trade/botín/recompensas/afijos, sistemas sociales 1–5E, y áreas locales
6A–6C con retirada reversible 6D **parcial**. El esquema v16 añadió la capa
de persistencia de slots de historia S3 (ADR 0030 propuesto); el v17 guarda el
snapshot de runtime seccionado y reescribe solo las secciones sucias
(ADR 0035 propuesto).

No dupliques aquí el detalle: la descripción normativa completa vive en
[`AGENTS.md`](AGENTS.md) (sección "Baseline implementado", invariantes y límites
honestos). Los conteos de pruebas exactos, las versiones
(WorldPack/AreaPack/snapshot/esquema SQL) y el estado por fase
(`Complete`/`Partial`) son autoridad única de
[`docs/project/COMPLETION_MATRIX.md`](docs/project/COMPLETION_MATRIX.md); la
historia y la evidencia por fase, de
[`docs/project/PROJECT_MEMORY.md`](docs/project/PROJECT_MEMORY.md). La evidencia
local no es evidencia de CI, release ni producción.

## Reglas de decisión

- Corrige defectos en la frontera autoritativa; no añadas excepciones de
  transporte.
- Mantén equivalencia navegador/Telnet y percepción antes de protocolo.
- Conserva escritor único, un backend, snapshots transaccionales y rollback.
- Acota bytes, profundidad, cantidades, colas, concurrencia, tiempo, memoria,
  scripts, recursos y reintentos; usa aritmética checked/saturating.
- No debilites los 16 KiB ni la recuperación atómica para hacer caber algo.
- No derives pistas, estado o autorización de prosa.
- No añadas Kubernetes, Redis, microservicios, LLM runtime, medios remotos
  arbitrarios ni layout de píxeles del servidor sin necesidad medida y ADR.
- IDs publicados son durables. `archetype` queda en compatibilidad.
- Aumentar capacidad no cura. Progresión permanente es atómica y exactly-once.
- Persistencia requiere normalmente paridad SQLite/PostgreSQL y migración.
- WorldPack requiere esquema, validador, normalización, composición, loader,
  datos revisados y CLI tests.
- Protocolo requiere Rust, TypeScript estricto, socket/recovery, Playwright y
  paridad Telnet/GMCP. Los tipos de cable viven en `crates/protocol`; regenera
  `apps/web/src/lib/generated/protocol.ts` con
  `TIAMUD_UPDATE_GENERATED_PROTOCOL=1 cargo test --locked -p tiamud-protocol`.
- Medios pasan por pipeline y manifiesto revisado; perderlos no cambia juego.
- La identidad durable prototype/instance, las pilas derivadas y los
  contenedores acotados, `give`, trade confirmado y botín repetible con
  procedencia/derechos, recompensas selladas y afijos ya existen. Conserva su
  atomicidad antes de reparto avanzado. No añadas cajas de botín pagadas al
  baseline.
- Las acciones staff que tocan el mundo resuelven un personaje conectado y
  comprueban una capacidad concreta. Conserva la sintaxis textual con `|` para
  separar cuerpo y motivo en inspect/where/goto/transfer/restore, announce,
  profile y capability.

## Verificación mínima

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings

pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web e2e

bash scripts/verify-project-licenses.sh
node scripts/generate-third-party-notices.mjs --check
node --test scripts/bot.test.mjs scripts/verify-soak-report.test.mjs scripts/telnet-bot.test.mjs
```

PostgreSQL necesita `TIAMUD_TEST_POSTGRES_URL` desechable y explícita. Las
pruebas de carga necesitan Node 24. Nunca uses datos de producción.

El host actual ejecuta Chromium/Firefox pero carece de bibliotecas nativas
WebKit. No afirmes WebKit. Un soak corto es diagnóstico; no hay evidencia
retenida de 24 horas.

## Documentación y relevo

Ante un cambio material:

1. actualiza pruebas y artefactos generados;
2. actualiza `docs/project/COMPLETION_MATRIX.md`;
3. actualiza `docs/project/PROJECT_MEMORY.md`;
4. añade o modifica el registro fechado pertinente;
5. corrige README, especificación, diseño y operación que quedarían falsos; y
6. añade un ADR antes de cambiar arquitectura aceptada.

No reescribas registros históricos para que parezcan actuales. No conviertas
un workflow configurado en un pase. No confundas la carga aditiva 6B con el
corte completo 6C, ni presentes 6C como reemplazo por área, descarga o LRU.
Tampoco afirmes su gate final, producción, lector manual, release etiquetada o
soak 24 h sin evidencia.

Antes de entregar: revisa `git diff`, ejecuta `git diff --check`, verifica
enlaces/comandos modificados y declara exactamente qué se validó.
