# Guía para agentes de Tiamud

Este archivo se aplica a todo el repositorio. Es el contrato de trabajo para
agentes de programación y contribuidores. Debe permanecer alineado con el
código actual, las decisiones aceptadas y la evidencia retenida.

## Lectura inicial

Antes de un cambio material, lee en este orden:

1. [`docs/project/PROJECT_MEMORY.md`](docs/project/PROJECT_MEMORY.md), contexto
   técnico durable y notas de continuación.
2. [`docs/project/COMPLETION_MATRIX.md`](docs/project/COMPLETION_MATRIX.md),
   estado autoritativo de implementación y aceptación.
3. Los ADR pertinentes en [`docs/decisions/`](docs/decisions/).
4. [`docs/spec-v0.3.md`](docs/spec-v0.3.md), el diseño relevante y el código.
5. Los registros fechados de [`docs/project/log/`](docs/project/log/) solo si
   importan comandos o evidencia histórica exactos.

Precedencia de fuentes:

- los ADR aceptados definen decisiones arquitectónicas;
- el código y las pruebas enfocadas demuestran comportamiento;
- la matriz determina si un requisito está completo o parcial;
- la memoria es el resumen durable y punto de relevo;
- los registros fechados describen la entrega de aquel momento y pueden quedar
  superados;
- `docs/spec-v0.2.md` es la especificación histórica en castellano, no el
  contrato vigente de personajes, medios o sistemas sociales.

No deduzcas implementación porque un ADR o una hoja de ruta describa algo. No
cambies una decisión aceptada en silencio: añade o sustituye un ADR y actualiza
matriz, memoria y documentación pública a la vez.

## Identidad del producto y repositorio

- El nombre definitivo es **Tiamud**. No reintroduzcas un título provisional ni
  el antiguo nombre del repositorio como identidad.
- El repositorio canónico es `hyukudan/tiamud`; `main` es la rama predeterminada
  y la única de desarrollo activa.
- `envy-import` es solo la herramienta sin conexión para compatibilidad con
  datos EnvyMUD/Merc. Tiamud es una implementación limpia, no un fork.
- El proyecto privado está en prelanzamiento `0.1.0`; no existe despliegue
  público de producción ni versión estable del protocolo.
- Software, pruebas, configuración, scripts y binarios: `MIT OR Apache-2.0`.
  Prosa, documentación, mundos y medios originales: `CC-BY-4.0`. Conserva la
  frontera y la procedencia de cada recurso.
- Nunca supongas que un input histórico o de terceros puede redistribuirse.
  Compatibilidad no es licencia.

## Baseline implementado

### Entorno de ejecución y persistencia

- Workspace Rust 2024, toolchain 1.93.1 y MSRV 1.88.
- Un proceso Axum/Tokio `tiamud-server` posee un actor de mundo con escritor
  único e incluye el cliente web revisado y Lantern Square.
- Navegador y Telnet producen la misma `tiamud_core::PlayerAction` y entran por
  la misma autorización y dominio.
- El actor serializa mutaciones. Un commit durable fallido restaura exactamente
  el checkpoint en memoria; no debilites esa frontera.
- Mundo y conversación usan buzones acotados de 256 entradas por sesión,
  encaminados por `SessionId`. Conserva el fan-out O(S), recuperación por lag y
  limpieza segura ante cancelación.
- PostgreSQL esquema v10 es el perfil recomendado para un host público nuevo.
  SQLite esquema v10 es la ruta local/predeterminada sin servicios. Se abre un
  único sistema autoritativo; no hay doble escritura.
- `tiamud-migrate-postgres` traslada sin conexión una instantánea SQLite actual
  a PostgreSQL vacío en una transacción, incluidas suscripciones, perfiles,
  capacidades, reportes resueltos y auditoría de staff.
- Los datos consultables son relacionales. Las instantáneas JSON acotadas se
  guardan como `TEXT` opaco en ambos adaptadores; no uses `jsonb` sin necesidad
  medida y ADR.
- `WorldRuntimeSnapshot` v3 es la única autoridad durable de las instancias de
  objeto. SQLite y PostgreSQL la confirman con personajes/eventos en su
  transacción existente; no hay esquema v11 ni tabla espejo. Las lecturas v1
  y v2 usan DTO y migraciones explícitos dirigidos por `snapshotVersion`, y
  fallan cerradas ante campos de otra versión, referencias, grafo o contador
  inválidos. Al restaurar un snapshot anterior se conservan las nuevas
  instancias iniciales del WorldPack y se superponen las ubicaciones durables;
  una instancia creada en runtime que no esté en el snapshot no reaparece.
- Cada snapshot registra la secuencia cubierta y conserva atómicamente solo el
  sufijo diagnóstico cubierto de 10.000 secuencias.
- Autenticación registrada compartida entre web y Telnet. Trabajo Argon2id,
  vidas de sesión, conexiones y colas están acotados. Solo se almacenan
  resúmenes SHA-256 de credenciales bearer.

### Mundo, personajes y juego

- WorldPack v5 es canónico; AreaPack v4 es la autoría modular nativa.
  WorldPack v4/v3/v2, AreaPack v3/v2/v1 y Envy/Merc solo entran por
  normalización/importación.
- Se valida y carga un WorldPack completo antes de abrir listeners.
  `TIAMUD_WORLD_PACK` selecciona uno externo revisado; si no, se usa el
  embebido.
- `tiamud-world compose` realiza composición modular determinista sin conexión.
  Carga/reemplazo caliente y expulsión LRU **no están implementados**. ADR 0005
  es diseño futuro.
- El modelo de personaje v3 funciona de extremo a extremo dentro de WorldPack
  v5: linaje, trasfondo opcional, clase
  base/avanzada, XP global, clase activa, historial, reservas, talentos con
  rango y promoción atómica.
- `archetype` no es identidad canónica. Normalízalo en la frontera; no crees un
  modelo heredado paralelo.
- Concesiones y requisitos permanentes usan vocabularios cerrados y acotados.
  Aumentar capacidad no cura/recarga; un recurso nuevo empieza vacío.
- Habilidades marciales, mágicas y de utilidad comparten definición acotada con
  costes, cooldown, objetivos, daño/curación, resistencia y estados. Las
  habilidades del jugador respetan un cooldown global de un ciclo lógico.
- Los NPC eligen habilidades de forma determinista y tienen ataque básico de
  respaldo. No hay IA generativa en ejecución.
- Objetos/equipo, combate, derrota/recuperación, botín fijo, tiempo, clima,
  misiones, grupos, economía, conversación y moderación persisten en ambos
  adaptadores. Los objetos separan `itemPrototypes` de instancias durables;
  cada personaje tiene capacidad autoritativa por slots/peso y rareza cerrada
  `common|uncommon|rare|epic|legendary` más `itemLevel` textual en Web, Telnet
  y GMCP. Las unidades únicas reciben IDs opacos por instancia; una pila usa el
  selector opaco de su primer miembro visible ordenado. Todos se derivan de una
  sal criptográficamente aleatoria nueva por sesión. Telnet muestra la
  referencia opaca y permite usarla en comandos. En autoría,
  `entityMedia(kind=item)` referencia el prototipo reusable, mientras
  `roomPerception.hiddenItems` referencia la instancia concreta. En 3B las
  pilas fungibles son vistas derivadas por ubicación/prototipo: no persisten
  cantidad ni identidad de pila. Los contenedores son instancias únicas con
  slots/peso propios, peso recursivo, profundidad máxima cuatro y grafo
  validado. `take`/`drop`/`put`/`get` aceptan 1..=256 unidades y `contents`
  pagina 20 grupos; Web, Telnet y GMCP reciben cantidad, slots, peso total y
  capacidad con referencias opacas. Solo se acepta `transferable`. No amplíes
  la afirmación: no hay telepatía definida por el mundo, give/trade, reparto
  de grupo ni tablas de botín aleatorias.
- El arranque enumera de forma acotada los personajes que poseen instancias,
  carga temporalmente sus agregados y valida existencia, equipo y capacidad,
  incluidos los desconectados. Esos agregados no quedan retenidos como
  personajes conectados. Una incoherencia bloquea listeners.
- Los grupos admiten hasta seis miembros y soportan
  create/invite/accept/revoke/kick/transfer/disband/leave/status y `party say`
  (`gsay`/`gtell`). No hay follow/assist/rescue, selección general de aliados,
  XP/crédito de misión compartidos ni reparto de botín.
- La membresía permanece al desconectar y el snapshot muestra miembros offline,
  pero accept/revoke/kick/transfer resuelven un nombre conectado. No afirmes
  administración dirigida de un líder/invitado/miembro desconectado.
- La conversación incluye `say` local, `tell`, `reply` en memoria mientras el
  destinatario siga conectado, chat de grupo y canales `general`, `newbie` y
  `staff`. Suscripciones son durables en v10; `staff` requiere perfil
  `moderator`, `administrator` o `implementor` al unirse, enviar y recibir;
  `helper` y `builder` no acceden. Web, Telnet y `Comm.Channel.Text` GMCP
  comparten política. No hay correo/historial durable, shout/yell ni telepatía
  authored.
- La autoridad ejecutable de staff usa capacidades cerradas y nunca procede de
  XP, nivel, clase, talentos o WorldPack. Los perfiles `helper`, `builder`,
  `moderator`, `administrator` e `implementor` siembran bundles revisados y
  admiten un `lore_title` opcional que no autoriza nada. Las doce capacidades
  son `reports.view`, `reports.resolve`, `moderation.mute`, `moderation.ban`,
  `staff.assign`, `staff.inspect`, `staff.where`, `staff.goto`,
  `staff.transfer`, `staff.restore`, `staff.announce` y `staff.audit`.
- Los roles `Player`, `Moderator`, `Admin` se conservan solo como tier legado,
  moderación y percepción; poseer uno nunca basta para ejecutar un comando ni
  abre por sí solo el canal `staff`. La primera cuenta registrada recibe
  `implementor`; un Admin migrado recibe `administrator`. Cambiar un perfil
  elimina las asignaciones previas y siembra exactamente su bundle; toda acción
  sobre otra cuenta exige rango de perfil estrictamente inferior. Reportes,
  sanciones, asignaciones y acciones staff exigen motivo; la auditoría v10 es
  durable y append-only.
- `staff inspect|where|goto|transfer|restore <nombre> | <motivo>` actúa solo
  sobre objetivos conectados. `staff announce <texto> | <motivo>`, la cola y
  resolución cerrada de reportes, asignación de perfil/capacidad y consulta de
  auditoría comparten Web/Telnet y la misma autorización del actor.
- No existe entrenamiento general por uso. El incremento heredado de `combat`
  tras una derrota de NPC es deuda del mundo de ejemplo.

### Percepción, protocolo y clientes

- Las acciones producen hechos persistentes; percepción proyecta después
  escena, jugador y eventos específicos del observador.
- IDs estables, variantes secretas y contenido oculto quedan en el servidor.
  Los clientes reciben IDs opacos derivados de una sal criptográficamente
  aleatoria nueva por sesión.
- El texto contiene todos los hechos jugables. Imágenes, audio, ANSI, gráficos
  y composición son solo presentación.
- `tiamud.v1` experimental usa secuencias monótonas, deduplicación acotada,
  heartbeats y recuperación autoritativa; no hay replay general de eventos.
- Una recuperación devuelve `sync.scene` y `sync.player` correlacionados, cada
  uno bajo 16 KiB; el navegador valida y aplica el par atómicamente.
- Todo frame WebSocket normal está limitado a 16 KiB JSON compacto. No evites
  límites de frame, profundidad o cola.
- Una escena GMCP sobredimensionada degrada a un fallback estructurado acotado
  y métrica observable; nunca desaparece en silencio.
- El navegador Svelte 5 implementa acceso registrado, ficha v3, creación,
  habilidades, talentos/promociones, inventario/equipo, mapa, misiones,
  grupos/economía/conversación y preferencias accesibles persistentes.
- Scrollback visible: 500; historial de comandos: 100. Conserva posición de
  lectura y salto a entradas nuevas.
- El Service Worker solo cachea una shell acotada del mismo origen; excluye
  API, WebSocket, imágenes y audio.
- Telnet es cliente textual completo. `tiamud-telnet-codec` es el parser de
  producción compartido con fuzzing. Conserva RFC 1143, TTYPE/MTTS, NAWS,
  CHARSET, GMCP, MSSP, MCCP2 y salida enriquecida opt-in.
- Telnet TLS 1.2/1.3 directo es predeterminado y fail-closed. `plaintext` solo
  es excepción explícita para desarrollo/privado y nunca debe llevar
  credenciales públicas.

### Medios, scripts e IA opcional

- Recursos atmosféricos: sin conexión, inmutables y direccionados por
  contenido. Escenas opacas: AVIF, WebP/JPEG. Sprites: WebP alfa + PNG.
- Manifiestos conservan dimensiones, hashes, licencia, procedencia,
  clasificación, revisión y alt localizado.
- Presentación de entidades: autoría estricta, proyección segura y layout
  determinista. Servidor: 12 anotaciones/32 poses; navegador: 10 sprites más
  overflow semántico.
- Pistas de combate: ataque/golpe/derrota de NPC con IDs opacos. No se infieren
  de prosa. Faltan sprites demostrativos revisados, avatar separado, pista heal
  del servidor y lápida tras derrota.
- Modo lector, Save-Data y multimedia off no descargan sprites; movimiento
  reducido usa pose estática. La capa no es interactiva.
- Lua 5.4 se ejecuta en VM nueva, solo con capacidades, sin sistema de archivos,
  red, procesos, módulos nativos ni debug. Código, memoria, instrucciones,
  tiempo y eventos tienen cuotas; un batch admite ocho módulos y 50 ms.
- IA editorial es opcional, solo de autoría y fuera del juego. `none` es el
  proveedor predeterminado; endpoints locales usan loopback y remotos HTTPS.
  Ningún borrador se publica sin validación y revisión humana.

### Evidencia retenida de fase 3A y estado de fase 3B

El árbol integrado de inventario 3A supera formato, pruebas bloqueadas del
workspace y clippy estricto. Las suites enfocadas pasan 77 pruebas de core, 80
de WorldPack, 28 del importador y 195 de `tiamud-server`; la web pasa
`svelte-check` con 0 errores/avisos, 100/100 unitarias, build y 30/30 escenarios
Playwright soportados (15 Chromium y 15 Firefox). Licencias y avisos generados
de 307 dependencias Rust y 76 web también pasan. La suite Node del harness pasa;
humo, carga CI exacta de 16 usuarios
(16 bootstraps/32 rondas, 15 WebSocket + uno Telnet) y soak corto de 30 s con
dos usuarios (64 rondas, readiness 8/8) terminan sin errores y con todos los
veredictos en `pass`. No se ejecutaron PostgreSQL real v10, WebKit local ni soak
de 24 horas. La evidencia operacional procede de un árbol dirty precommit: es
diagnóstico local, no evidencia publicable ni de CI.

La fase 3B está implementada en el árbol de trabajo mediante WorldPack v5,
AreaPack v4, snapshot v3, pilas derivadas, contenedores acotados y paridad de
acciones/proyecciones. El árbol integrado pasa formato/check/clippy, 499/499
pruebas Rust (88 core, 93 WorldPack, 28 importador, 202 servidor y 88
restantes), 103/103 web, 30/30 Playwright Chromium/Firefox, 80/80 Node, humo,
carga CI 16 usuarios y soak corto de 66 rondas/10 readiness. También pasan
`cargo-deny`, licencias y avisos 307 Rust/76 web. No hubo PostgreSQL real v10,
WebKit local ni soak de 24 horas; no conviertas este pase local precommit en
evidencia de CI o producción.

### Evidencia histórica retenida

La memoria fechada conserva este baseline de 2026-07-16. Vincula aquel árbol,
no autoriza saltarse pruebas tras cambios:

- 409 pruebas Rust bloqueadas y clippy estricto; 158 pruebas enfocadas de
  `tiamud-server`.
- 96 pruebas web y 28 escenarios combinados Chromium/Firefox.
- PostgreSQL 18.4 validó **esquema v8**, restauración SQLite, mover, juego mixto
  y reinicio autenticado. No se relabela como aceptación v10.
- Perfil mixto de 30 s en `2b18cfe`: 66 rondas, 145 acciones, 10/10 readiness,
  cero errores/drops y cero buzones vivos al apagar.
- Fuzz del parser Telnet: 3.081.428 ejecuciones en 121 s sin crash, timeout, OOM
  ni artefacto de fallo.
- Una captura Chromium autenticada real detectó y cerró defectos de auth y
  cancelación del manifiesto. El hero es decorativo; la captura de juego es
  evidencia de producto.

GitHub Actions puede fallar antes de ejecutar por facturación/límite de gasto.
Inspecciona el job real; no conviertas una insignia en evidencia.

## Restricciones de diseño no negociables

1. Un host y un servidor modular siguen siendo el despliegue de referencia. No
   añadas Kubernetes, Redis, object storage obligatorio, microservicios o una
   segunda autoridad como atajo.
2. Conserva un escritor y un sistema de persistencia seleccionado. Nunca doble
   escritura ni consistencia checkpoint/rollback debilitada.
3. Dominio neutral al transporte. Una acción web no evita reglas de Telnet.
4. Autorización y percepción antes de serializar. Presentación nunca revela ni
   crea visibilidad.
5. Texto plano completo y accesible. Perder medios/color no pierde acciones.
6. Acota todo input/output no confiable: bytes, profundidad, cantidades, colas,
   concurrencia, descompresión, scripts, recursos, reintentos y tiempo.
7. Prefiere datos deterministas y transformaciones puras. Sin IA runtime,
   guiones arbitrarios de progresión, medios remotos ni estado derivado de
   prosa.
8. IDs publicados son identidad durable. Tras release v3, renombrar exige
   alias/tombstone/migración explícita.
9. Autoridad de staff separada de XP, nivel, clase y talentos. Usa la capacidad
   concreta y auditoría append-only; perfiles y títulos son asignación/presentación,
   no autorización implícita.
10. La fase 3B ya separa prototipo/instancia, deriva pilas y aplica capacidad
    recursiva de contenedores. Antes de botín repetible/aleatorio, implementa
    transferencias/políticas y trade confirmado, y después define propiedad,
    procedencia y reparto atómicos.
    Cajas, llaves y tiradas pagadas quedan fuera.
11. Optimiza con evidencia. Siguiente ruta: dirty tracking y deltas/coalescing,
    conservando snapshot atómico.
12. No afirmes áreas calientes actuales, producción pública, WebKit actual,
    prueba manual de lector, soak 24 h, release pública/etiquetada ni procedencia
    OIDC verificada.

## Mapa del repositorio

| Ruta | Responsabilidad |
| --- | --- |
| `crates/mud-core/` | Dominio, acciones, progresión y percepción |
| `crates/mud-server/` | Actor, auth, transportes, routing, carga y fachada de persistencia |
| `crates/persistence-sqlite/` | SQLite esquema v10 y almacenamiento local |
| `crates/persistence-postgres/` | PostgreSQL esquema v10, adaptador y mover |
| `crates/world-pack/` | WorldPack/AreaPack, validación, composición y CLI |
| `crates/scripting/` | Capacidades Lua con cuotas |
| `crates/asset-pipeline/` | Compilador de medios determinista |
| `crates/telnet-codec/` | Parser Telnet de producción compartido con fuzzing |
| `crates/cell-frame/` | Renderizado de celdas acotado |
| `crates/editorial-ai/` | Adaptadores opcionales de autoría |
| `crates/tiamud-admin/` | Integridad, migración y recuperación SQLite |
| `apps/web/` | Cliente Svelte y cobertura Playwright |
| `clients/` | Integraciones TinTin++ y Mudlet |
| `tools/envy-import/` | Importador histórico conservador |
| `worlds/` | Paquete canónico y áreas fuente |
| `scripts/` | Humo, carga, soak, release y licencias |
| `fuzz/` | Corpus, diccionarios y targets acotados |
| `docs/` | Especificaciones, ADR, operación, memoria y evidencia |
| `deploy/` | Caddy, systemd y entorno revisados |

## Flujo de cambios

### Antes de editar

- Inspecciona `git status` y conserva todo trabajo existente.
- Lee módulo, pruebas, ADR/diseño, memoria y fila de matriz relevantes.
- Identifica primero la frontera autoritativa: dominio, persistencia,
  percepción, protocolo, presentación o autoría.
- Prefiere cambios enfocados y reversibles; no tapes defectos de dominio solo
  en un cliente.

### Durante la edición

- El comportamiento vive en `tiamud-core`; transportes equivalentes lo adaptan.
- Persistencia suele exigir paridad SQLite/PostgreSQL, migración/restore y
  revisión de snapshot.
- WorldPack suele exigir esquema, validador, normalización, composición,
  loader, datos revisados y CLI tests.
- Protocolo web suele exigir serialización Rust, parser TypeScript estricto,
  socket/recovery, Playwright y revisión Telnet/GMCP.
- Medios pasan por manifiestos revisados y percepción.
- Usa aritmética checked/saturating y límites explícitos; falla cerrado ante
  autoridad, esquema o seguridad ambiguos.
- Un cambio material actualiza matriz, memoria y registro fechado.

### Archivos generados y sensibles

- No edites `THIRD_PARTY_NOTICES.md` a mano; regenera/verifica con el script.
- Conserva licencias completas y hashes de verificación.
- Construye artefactos mundo/medios con herramientas del repo.
- Nunca comitees `.env`, bases, logs operativos, reportes locales crudos,
  credenciales, tokens, claves o certificados.

## Verificación

Usa la puerta mínima al iterar y amplía según riesgo. No afirmes un pase de un
comando no ejecutado sobre el árbol final.

### Rust

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

PostgreSQL requiere una base desechable y autoridad explícita:

```sh
TIAMUD_TEST_POSTGRES_URL='postgresql://.../postgres?sslmode=disable' \
  cargo test --locked -p tiamud-persistence-postgres --all-targets
```

Nunca apuntes pruebas destructivas a producción.

### Navegador

```sh
pnpm --dir apps/web install --frozen-lockfile
pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web e2e
```

Reproduce CI con Node 24/pnpm 11.11. El paquete acepta pnpm 10+. En el host
actual Chromium/Firefox funcionan; WebKit carece de bibliotecas GTK/GStreamer.

### Proyecto, transporte y cadena de suministro

```sh
bash scripts/verify-project-licenses.sh
node scripts/generate-third-party-notices.mjs --check
node --test scripts/bot.test.mjs scripts/verify-soak-report.test.mjs scripts/telnet-bot.test.mjs
node scripts/smoke.mjs
node scripts/load.mjs --users 2 --telnet-users 1 --rounds 2 \
  --concurrency 2 --pace-ms 100
```

El harness carga/soak necesita Node 24. Un perfil corto es diagnóstico, no
prueba de 24 horas.

### Autoría

```sh
mkdir -p build/worlds
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.composed.world.json
cargo run --locked -p world-pack --bin tiamud-world -- validate \
  worlds/lantern-square.world.json
cargo test --locked -p world-pack
```

### Fuzzing

```sh
cargo +nightly-2026-07-01 fuzz run telnet_parser -- \
  -dict=fuzz/dictionaries/telnet_parser.dict \
  -max_len=262144 -timeout=5 -rss_limit_mb=1024 -max_total_time=120
```

## Límites actuales honestos

- sin soak de 24 horas retenido;
- sin WebKit local actual ni auditoría manual con lector real;
- sin sprites de demostración revisados, aunque el puente técnico existe;
- sin carga/reemplazo/LRU de áreas en ejecución;
- sin despliegue/repo público, hito estable, release etiquetada o procedencia
  verificada;
- sin entrenamiento general, migración general de IDs, multiclase, sprite de
  jugador, heal cue del servidor ni lápida de derrota;
- fase social 1 implementada, pero sin correo/historial, shout/yell o telepatía;
- fase staff 2 implementada; sin OLC, spawn/purge, acceso al host ni autoridad
  obtenible por rol legado, título, XP o WorldPack;
- fase 3B de inventario implementada: prototipos/instancias, creación durable,
  pilas derivadas, contenedores con capacidad/profundidad acotadas, cantidades,
  paginación, rareza e `itemLevel`; sin give/trade, políticas de vinculación,
  derechos/reparto ni botín aleatorio;
- backup/TLS/monitorización/recuperación PostgreSQL dependen del operador;
- marca/dominios y derechos de terceros siguen siendo trabajo legal externo.

Un workflow configurado no demuestra que se ejecutó. Inspecciona sus pasos.

## Criterio de finalización

Una tarea termina cuando el comportamiento existe en todas las fronteras
afectadas, hay pruebas enfocadas y puertas amplias proporcionales al riesgo, la
documentación es factual, los artefactos generados están sincronizados y
`git diff` contiene solo cambios intencionados. Expón límites; no conviertas una
arquitectura prevista o CI configurada en una afirmación de implementación.
