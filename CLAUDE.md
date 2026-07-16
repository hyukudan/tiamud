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
`docs/spec-v0.2.md` es contexto histórico en castellano; `docs/spec-v0.3.md`
describe el baseline vigente de personajes y medios.

## Estado actual resumido

- Tiamud es el nombre definitivo; `hyukudan/tiamud` es el repositorio canónico
  y `main` la rama de desarrollo.
- Es un proyecto privado de prelanzamiento `0.1.0`, sin producción pública ni
  protocolo estable.
- Toolchain Rust 1.93.1, edition 2024, MSRV 1.88. CI usa Node 24 y pnpm 11.11;
  el paquete web admite pnpm 10+.
- Un servidor Rust posee un actor de mundo con escritor único. Navegador y
  Telnet comparten dominio, persistencia y proyección.
- PostgreSQL esquema v10 es el perfil público recomendado; SQLite esquema v10 es
  la ruta local sin servicios. Cada proceso abre uno y nunca doble-escribe.
- Buzones acotados por sesión encaminan proyecciones sin amplificación de
  broadcast. Conserva orden, recuperación por lag y rollback de checkpoint.
- WorldPack v5 y AreaPack v4 son actuales. WorldPack v4/v3/v2,
  AreaPack v3/v2/v1 y Envy se normalizan en fronteras explícitas. El servidor carga un paquete
  completo; ADR 0005 para carga caliente/LRU es futuro.
- El personaje v3 incluye linaje, trasfondo, clase base/avanzada, XP global,
  historial, talentos, reservas, promociones, recursos, cooldowns,
  resistencias y estados de extremo a extremo.
- La fase de inventario 3B separa `itemPrototypes` de instancias durables,
  persiste `WorldRuntimeSnapshot` v3 con conversiones explícitas desde v1/v2 y
  aplica capacidad por slots/peso. Las pilas fungibles se derivan por
  ubicación/prototipo y no persisten cantidad ni ID de pila. Los contenedores
  únicos tienen capacidad propia, peso recursivo, profundidad máxima cuatro y
  rechazo atómico de ciclos/sobrecapacidad. Rareza cerrada, `itemLevel`, peso
  unitario/total, cantidad, slots y capacidad se proyectan
  como texto con IDs opacos derivados de una sal aleatoria por sesión; Telnet
  muestra y acepta esas referencias para desambiguar. Los medios de objeto
  referencian el prototipo y `hiddenItems` la instancia. El arranque valida de
  forma acotada propietarios, equipo y capacidad también para desconectados,
  sin retener sus agregados en el mundo activo. SQLite/PostgreSQL conservan el
  snapshot único en esquema v10; restore conserva nuevas instancias iniciales
  del WorldPack y superpone el snapshot sin resucitar instancias runtime
  ausentes. No existe tabla espejo ni esquema v11. En 3B solo se acepta
  `transferable`; `give`, trade, políticas ligadas, derechos y tablas aleatorias
  siguen pendientes.
- La fase social 1 está implementada: `say`, `tell`, `reply` online, grupos de
  seis miembros con chat y liderazgo, y suscripciones durables a `general`,
  `newbie`, `staff`. `staff` exige perfil `moderator`, `administrator` o
  `implementor`; `helper` y `builder` no acceden. Web, texto Telnet y GMCP
  comparten política. No hay correo/historial ni telepatía.
- Accept, revoke, kick y transfer resuelven el nombre de un personaje conectado.
  La membresía de grupo sí persiste cuando alguien se desconecta, pero esas
  operaciones dirigidas no administran un objetivo offline.
- La fase staff 2 usa perfiles `helper`/`builder`/`moderator`/`administrator`/
  `implementor`, título narrativo opcional y doce capacidades cerradas. Incluye
  cola/resolución de reportes, inspect/where/goto/transfer/restore, anuncios y
  consulta de auditoría con motivos obligatorios y registro append-only. La
  primera cuenta registrada recibe `implementor`; los Admin migrados reciben
  `administrator`. Cambiar perfil sustituye las asignaciones por su bundle y
  solo se puede actuar sobre rangos estrictamente inferiores.
- Roles `Player`/`Moderator`/`Admin` quedan como tier legado de moderación y
  percepción; nunca bastan para ejecutar comandos ni abren por sí
  solos el canal `staff`. La autorización tampoco procede de título, XP, clase,
  talentos o WorldPack.
- La composición opcional de sprites está conectada y es segura ante
  percepción. Faltan sprites revisados, jugador separado, heal cue y lápida.
- Texto plano siempre completo. Medios, ANSI, GMCP rico y gráficos son
  opcionales, acotados y consentidos.
- Lua e IA editorial están aislados y limitados. Sin IA runtime ni cloud
  obligatorio.
- Software: `MIT OR Apache-2.0`; contenido original: `CC-BY-4.0`.

El árbol integrado de inventario 3A supera formato, pruebas bloqueadas del
workspace y clippy estricto; 77 pruebas de core, 80 de WorldPack, 28 del
importador, 195 de servidor, `svelte-check` 0/0, 100 pruebas web, build y 30/30
escenarios Playwright soportados. Licencias y avisos de 307 dependencias Rust y
76 web también pasan. La suite Node del harness pasa; el humo,
carga CI exacta de 16 usuarios (16 bootstraps/32 rondas) y soak corto de 30 s
con dos usuarios (64 rondas, readiness 8/8) terminan sin errores y con todos los
veredictos en `pass`. No se ejecutaron PostgreSQL real v10, WebKit local ni soak
de 24 horas. Esa evidencia operacional procede de un árbol dirty precommit: es
diagnóstico local, no evidencia publicable ni de CI.

La integración 3B pasa formato/check/clippy y 499/499 pruebas Rust: 88 core, 93
WorldPack, 28 importador, 202 servidor y 88 restantes. También pasan
`svelte-check` 0/0, 103/103 web, build, 30/30 Playwright Chromium/Firefox,
80/80 Node, humo, carga CI con 16 usuarios, soak corto de 66 rondas/10
readiness, `cargo-deny`, licencias y avisos 307 Rust/76 web. PostgreSQL real
v10, WebKit local y soak de 24 horas no se ejecutaron.

La evidencia histórica anterior informa 409 pruebas Rust, 158 de servidor, 96
web y 28 escenarios Chromium/Firefox; PostgreSQL 18.4 **esquema v8**, perfil
mixto limpio y 3.081.428 ejecuciones fuzz Telnet. Vincula aquel árbol: no es
aceptación v10 ni permite omitir nuevas pruebas. GitHub Actions puede bloquearse
por facturación antes de ejecutar.

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
  paridad Telnet/GMCP.
- Medios pasan por pipeline y manifiesto revisado; perderlos no cambia juego.
- La identidad durable prototype/instance, las pilas derivadas y los
  contenedores acotados ya existen. Antes de botín repetible/aleatorio,
  implementa transferencias/políticas, trade confirmado y después propiedad,
  procedencia y derechos de reparto atómicos. No añadas cajas de botín pagadas
  al baseline.
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
un workflow configurado en un pase. No afirmes áreas calientes, producción,
lector manual, release etiquetada ni soak 24 h sin evidencia.

Antes de entregar: revisa `git diff`, ejecuta `git diff --check`, verifica
enlaces/comandos modificados y declara exactamente qué se validó.
