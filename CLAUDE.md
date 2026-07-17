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
describe el baseline vigente de personajes, inventario y medios.

## Estado actual resumido

- Tiamud es el nombre definitivo; `hyukudan/tiamud` es el repositorio canónico
  y `main` la rama de desarrollo.
- Es un proyecto privado de prelanzamiento `0.1.0`, sin producción pública ni
  protocolo estable.
- Toolchain Rust 1.93.1, edition 2024, MSRV 1.88. CI usa Node 24 y pnpm 11.11;
  el paquete web admite pnpm 10+.
- Un servidor Rust posee un actor de mundo con escritor único. Navegador y
  Telnet comparten dominio, persistencia y proyección.
- PostgreSQL esquema v13 es el perfil público recomendado; SQLite esquema v13 es
  la ruta local sin servicios. Cada proceso abre uno y nunca doble-escribe.
- Buzones acotados por sesión encaminan proyecciones sin amplificación de
  broadcast. Conserva orden, recuperación por lag y rollback de checkpoint.
- WorldPack v9 y AreaPack v8 son actuales; WorldProject permanece en v2.
  WorldPack v8/v7/v6/v5/v4/v3/v2, AreaPack v7/v6/v5/v4/v3/v2/v1 y Envy se normalizan en fronteras explícitas. El servidor carga un paquete
  completo; ADR 0005 para carga caliente/LRU es futuro.
- El personaje v3 incluye linaje, trasfondo, clase base/avanzada, XP global,
  historial, talentos, reservas, promociones, recursos, cooldowns,
  resistencias y estados de extremo a extremo.
- La fase de inventario 4B separa `itemPrototypes` de instancias durables y
  persiste `WorldRuntimeSnapshot` v7 con DTO exactos v1-v7. v1-v3 se
  convierten a vínculo `unbound`; schema-smuggling, vínculos y políticas
  incoherentes fallan cerrados; v4 normaliza a contador inicial y trades vacíos.
  SQLite/PostgreSQL conservan el snapshot único en esquema v13, sin tabla
  espejo ni custodia SQL. Los snapshots v1-v5 conservan el límite de 1 MiB y
  v6-v7 admiten 2 MiB; v6 rechaza campos v7 y normaliza a afijos vacíos y
  contador de aperturas inicial. Restore conserva nuevas
  instancias iniciales del WorldPack y superpone el snapshot sin resucitar
  instancias runtime ausentes.
- Las pilas fungibles se derivan por ubicación/prototipo/vínculo/afijo y no persisten
  cantidad ni ID de pila. Los contenedores únicos tienen capacidad propia, peso
  recursivo, profundidad máxima cuatro y rechazo atómico de
  ciclos/sobrecapacidad. Rareza cerrada, `itemLevel`, peso unitario/total,
  cantidad, slots, capacidad, política y vínculo se proyectan como texto con IDs
  opacos derivados de una sal aleatoria por sesión; Telnet muestra y acepta esas
  referencias. Los medios de objeto referencian el prototipo y `hiddenItems` la
  instancia. Un afijo durable por instancia añade un prefijo/sufijo y deltas
  acotados de ataque/curación sin exponer IDs ni digests internos.
- WorldPack v9/AreaPack v8 aceptan `transferable`, `character_bound` y
  `non_transferable`, igual que WorldPack v6/v5 y AreaPack v5/v4; WorldPack
  v4/AreaPack v3 y anteriores solo `transferable`. La adquisición
  liga recursivamente, y ninguna raíz puede soltar o cambiar de dueño un
  descendiente restringido. `give [cantidad] <objeto> to <personaje>` mueve
  1..=256 raíces portadas a un receptor distinto, conectado y en la misma sala,
  con capacidad bilateral, ignore/block, commit y proyección atómicos. El
  arranque valida propietarios raíz y ligados, incluidos consumidos y
  desconectados. `trade` reserva hasta 128 raíces transferibles y moneda entre
  dos personajes conectados de la misma sala, con revisión/digest, doble
  confirmación, liquidación simultánea y cancelación restart-safe. Web, Telnet
  y `Char.Trade 1` GMCP comparten dominio y proyección.
- Botín 4A resuelve botín fijo y tablas enteras repetibles exactamente una vez
  por derrota, con RNG sin sesgo, procedencia durable y máximo de 64 raíces.
  Congela la cohorte de grupo, asigna round-robin, admite `loot pass`, reserva
  20 ticks, publica después y desaparece al 120. La capacidad suprime sin
  bloquear derrota ni reroll; reiniciar cancela encuentros abiertos pero no
  derechos resueltos. La fase 4B añade sellos obtenidos solo como botín NPC:
  `open` simula y confirma atómicamente hasta 16 resultados, vínculo, capacidad
  y afijo, mientras `reward odds` publica fracciones exactas paginadas. Web,
  Telnet y `Char.Reward 1` GMCP comparten la misma autoridad. Need/greed,
  contribución y master-loot siguen pendientes; no hay cajas, llaves, moneda
  premium, pity ni tiradas pagadas.
- La fase social 1 está implementada: `say`, `tell`, `reply` online, grupos de
  seis miembros con chat y liderazgo, y suscripciones durables a `general`,
  `newbie`, `staff`. `staff` exige perfil `moderator`, `administrator` o
  `implementor`; `helper` y `builder` no acceden. La fase 5A añade telepatía
  directa online como habilidad WorldPack v8: exige conocimiento, recurso,
  cooldown y sala emisora permitida; ignore/block o la receptividad privada del
  receptor suprimen sin filtrar y el commit precede a la entrega. No enlaza
  `reply`, no persiste texto/destino y comparte mute/bucket. Web, texto Telnet
  y GMCP comparten política. La fase 5B añade emotes locales authored,
  plantillas cerradas por perspectiva y targeting local cerrado —sin objetivo,
  opcional u obligatorio— mediante
  `emotes`/`emote`, sin texto libre, historial o SQL. Ignore/block se aplica por
  receptor y la entrega espera al commit de `OnTalk`. La fase 5C añade correo
  offline explícito con 64 mensajes/128 KiB/30 días por buzón, páginas de 10,
  cuotas por cuenta 20/h y 50/día, aceptación opaca ante política/capacidad y
  denuncia con evidencia de 90 días. Web, Telnet y `Tiamud.Mail 1` comparten
  dominio. La fase 5D añade tablones públicos globales `general`/`newbie` bajo
  demanda: diez resúmenes por página, retención de 90 días, cuotas 5/h y 20/día
  por cuenta, 64 posts vivos por cuenta y capacidades 1.000/4 MiB por tablón y
  2.000/8 MiB globales. El autor-personaje puede borrar; otra cuenta puede
  crear la única denuncia global, cuya evidencia temporal entra en la cola
  staff. Web, Telnet y `Tiamud.Board 1` comparten dominio sin fan-out ni
  `Comm.Channel.Text`. La fase 5E está implementada: `yell` usa radio uno y
  `shout` radio tres mediante BFS dirigido determinista que incluye origen,
  excluye salidas secretas, visita como máximo 259 salas y no revela rumbo o
  distancia. El mensaje ocupa 1..=512 bytes; Web reserva `social.yell` y
  `social.shout` exactos, texto Telnet usa los verbos y GMCP reutiliza
  `Comm.Channel.Text`. Mute/bucket bloquean emisión; ignore, block bilateral y
  `condition.deaf` suprimen `say`/`yell`/`shout` por receptor sin impedir que
  un sordo hable ni cambiar la confirmación del actor. `OnTalk` se ejecuta una
  vez en origen y la entrega comienza post-commit. No hay historial general,
  persistencia de voz, hilos/edición/búsqueda de tablones, emotes globales/de
  grupo ni telepatía de grupo/offline.
  La línea Telnet máxima es 8 KiB; un detalle GMCP sobredimensionado por escape
  conserva resumen, prefijo UTF-8 y overflow, mientras texto conserva el cuerpo
  completo. Web rechaza claves JSON duplicadas recursivamente y espera
  `action.result` correlacionado antes de aplicar post/delete/report, sin perder
  borradores en fallo.
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
- Escenas de botín se acotan semánticamente a 14 KiB Web y 48 KiB Telnet,
  priorizando propio, público y reservado, con lotes máximos de 16 eventos,
  avisos de coalescing y snapshot autoritativo.
- Lua e IA editorial están aislados y limitados. Sin IA runtime ni cloud
  obligatorio.
- Software: `MIT OR Apache-2.0`; contenido original: `CC-BY-4.0`.

La evidencia histórica de inventario 3A supera formato, pruebas bloqueadas del
workspace y clippy estricto; 77 pruebas de core, 80 de WorldPack, 28 del
importador, 195 de servidor, `svelte-check` 0/0, 100 pruebas web, build y 30/30
escenarios Playwright soportados. Licencias y avisos de 307 dependencias Rust y
76 web también pasan. La suite Node del harness pasa; el humo,
carga CI exacta de 16 usuarios (16 bootstraps/32 rondas) y soak corto de 30 s
con dos usuarios (64 rondas, readiness 8/8) terminan sin errores y con todos los
veredictos en `pass`. No se ejecutaron PostgreSQL real v10, WebKit local ni soak
de 24 horas. Esa evidencia operacional procede de un árbol dirty precommit: es
diagnóstico local, no evidencia publicable ni de CI.

La evidencia histórica de integración 3B pasó formato/check/clippy y 499/499
pruebas Rust: 88 core, 93
WorldPack, 28 importador, 202 servidor y 88 restantes. También pasaron
`svelte-check` 0/0, 103/103 web, build, 30/30 Playwright Chromium/Firefox,
80/80 Node, humo, carga CI con 16 usuarios, soak corto de 66 rondas/10
readiness, `cargo-deny`, licencias y avisos 307 Rust/76 web. PostgreSQL real
v10, WebKit local y soak de 24 horas no se ejecutaron.

La integración histórica de 3C usó un WorldPack canónico con 9 prototipos, 10
instancias iniciales, 49.603 bytes y SHA-256
`f3ab27cb93a2b3c80fe0a9a6ecf8acdd856a45ec1edbda257dd33046a21772c8`.
En aquel árbol pasaron 513/513 pruebas Rust: 93 core, 95 WorldPack, 28
importador, 207 servidor y 90 restantes; `svelte-check` 0/0, 103/103 web, 81/81
Node y 32/32 Playwright soportados, 16 Chromium y 16 Firefox. WebKit no arrancó
en aquel gate por bibliotecas GTK/GStreamer. No se ejecutaron PostgreSQL real
v10 ni soak de 24
horas; es evidencia local de un árbol dirty, no de CI, release o producción.

La integración histórica 3D añadió snapshot v5 y reservas durables de trade. La
evidencia integrada pasa 550 pruebas Rust: 102 core (80+3+14+5), 97 WorldPack,
28 importador, 231 servidor y 92 restantes —incluidas 23 admin—; también 107
Vitest y 83 Node/bot.
El pase Playwright final completa 36/36 escenarios, 18 Chromium y 18 Firefox.
No se ejecutaron
PostgreSQL real v10, WebKit local ni soak de 24 horas.

La integración histórica 4A añadió WorldPack v6/AreaPack v5, snapshot v6 y esquema
SQL v11. Pasa 587/587 pruebas Rust (113 core, 240 servidor, 112 WorldPack y 122
restantes), clippy/formato, 109 Vitest, 84 Node/bot y 19/19 Chromium. También
pasan licencias/avisos 307 Rust/76 web, humo, 16 bootstraps/32 rondas de carga
y el soak corto de 58 rondas/127 acciones/readiness 8/8. Lantern Square ocupa
50.498 bytes y tiene SHA-256
`3d6c7f40e7a5e56d509b46aeb89439aeea8f5a3168d249dd46609df1fe54c816`.
No se ejecutaron PostgreSQL real v11, Firefox, WebKit, lector manual ni soak de
24 horas.

La integración histórica 4B añade WorldPack v7/AreaPack v6 y snapshot v7 sobre el
mismo esquema SQL v11. Lantern Square contiene 10 prototipos, 9 instancias
iniciales, 2 tablas, 1 fuente, 1 afijo y 1 pool; ocupa 52.349 bytes y tiene
SHA-256
`183e7bc012a9214d68350c1f8fc1f8e6e505bb35c408c14e734901a671b5c7ef`.
Pasan formato, check, build, clippy estricto, 608/608 pruebas Rust (122 core,
245 servidor, 117 WorldPack y 124 restantes), 112 Vitest, 86 Node/bot y 40/40
Playwright (20 Chromium y 20 Firefox). También pasan cargo-deny,
licencias/avisos 307 Rust/76 web, humo, carga 16/32 con 15 WebSocket+1 Telnet y
soak corto de 30 s con 58 rondas, 127 acciones y readiness 8/8, todos sin
errores. La carga crece 55.365.632 bytes RSS, bajo 96 MiB; el soak informa
45.211.648 bytes totales y 1.536.000 sostenidos. Son diagnósticos locales sobre
un árbol precommit dirty: el verificador rechaza correctamente el reporte corto
por duración/perfil/warmup/árbol no limpio. No acreditan PostgreSQL real v11,
WebKit, lector manual, candidato limpio ni soak de 24 h.

La integración histórica 5A añadió WorldPack v8/AreaPack v7 sin cambiar snapshot
v7 ni esquema SQL v11. La telepatía directa exige habilidad, recurso,
cooldowns y sala emisora permitida; confirma antes de entregar y no filtra la
receptividad privada del receptor. Lantern Square mide 53.040 bytes y tiene
SHA-256
`109670f74056ab67d711fba520f7ad74d1cd16b8c72c06bd257839d89827dbc6`.
Pasan 630/630 pruebas Rust (131 core, 252 servidor, 123 WorldPack y 124
restantes), formato/diff/clippy, check web 0/0, 113 Vitest, build, 86 Node/bot,
42/42 Playwright soportado (21 Chromium y 21 Firefox), `cargo-deny` y
licencias/avisos 307 Rust/76 web. No se ejecutaron PostgreSQL real v11,
WebKit, lector manual ni soak de 24 h.

La integración actual 5B añade WorldPack v9/AreaPack v8 sin cambiar snapshot
v7 ni esquema SQL v11. Los emotes de sala usan catálogo authored, plantillas
cerradas por perspectiva, targeting local cerrado y sustitución en una pasada;
no admiten texto libre ni persistencia. Lantern Square mide 53.567 bytes y
tiene SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.
Pasan 652/652 pruebas Rust (139 core, 257 servidor, 132 WorldPack y 124
restantes), formato/diff/clippy, check web 0/0, 114 Vitest, build, 86 Node/bot,
44/44 Playwright soportado (22 Chromium y 22 Firefox), `cargo-deny` y
licencias/avisos 307 Rust/76 web. Dos auditorías dejan 0 P1/P2/P3 abiertos.
No se ejecutaron PostgreSQL real v11, WebKit, lector manual, carga/soak ni soak
de 24 h.

La integración actual 5C mantiene WorldPack v9/AreaPack v8 y snapshot v7, y
avanza SQLite/PostgreSQL al esquema v12. Añade correo offline separado con
buzón 64/128 KiB/30 días, cuotas 20/h y 50/día, referencias opacas, supresión
no reveladora y denuncia con evidencia de 90 días; Web, Telnet y
`Tiamud.Mail 1` tienen paridad. Pasan 669/669 pruebas Rust (142 core, 268
servidor, 132 WorldPack y 127 restantes), formato/diff/clippy, check web 0/0,
117 Vitest, build, 87 Node/bot, 44/44 Playwright soportado (22 Chromium y 22
Firefox), humo, `cargo deny check advisories sources` y licencias/avisos 307
Rust/76 web. La auditoría deja 0 P1/P2/P3 abiertos. No se ejecutaron
PostgreSQL real v12/mover, WebKit, lector manual, carga/soak ni soak de 24 h.

La implementación actual 5D mantiene WorldPack v9/AreaPack v8 y snapshot v7,
y avanza SQLite/PostgreSQL al esquema v13. Añade catálogo cerrado, publicación,
lista, lectura, borrado propio y denuncia de tablones con cuota/capacidad
transaccional, purga acotada, evidencia de 90 días y paridad Web/Telnet/
`Tiamud.Board 1`. El adaptador v13 pasa 19/19 pruebas contra PostgreSQL 18.4
real con el mover actual y migración representativa; las pruebas de servidor
incluyen flujo board/runtime y concurrencia advisory reales. La auditoría cerró
línea Telnet insuficiente, pérdida GMCP silenciosa, claves JSON duplicadas y
mutación Web optimista.

El árbol final pasa formato/diff/clippy estricto, 691 pruebas Rust (146 core,
283 servidor, 132 WorldPack y 130 restantes), PostgreSQL adaptador+mover 19/19
y focused servidor 5/5. Web pasa check 0/0, 122 Vitest, build y 46/46
Playwright soportado (23 Chromium, 23 Firefox); Node 88, humo,
advisories/sources y licencias/avisos 307 Rust/76 web también pasan. Auditoría:
0 P1/P2/P3. No se ejecutaron WebKit por bibliotecas ausentes, lector manual,
carga/soak mixtos ni soak de 24 h. TLS/backup/recuperación PostgreSQL de
producción siguen siendo específicos de instalación.

ADR 0019 y la fase 5E de voz espacial están implementados. WorldPack v9,
AreaPack v8, WorldProject v2, snapshot v7 y SQL v13 no cambian. Core, actor,
Web, Telnet/GMCP, cliente oficial y bot demuestran radio 1/3, BFS dirigido,
ciclos y convergencia, exclusión de salidas secretas, máximo 259,
mute/bucket, ignore/block/deaf y rollback sin entrega. El
[registro 5E](docs/project/log/2026-07-17-bounded-spatial-voice-phase-5e.md)
conserva la evidencia integrada y los límites no ejecutados.

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
un workflow configurado en un pase. No afirmes áreas calientes, producción,
lector manual, release etiquetada ni soak 24 h sin evidencia.

Antes de entregar: revisa `git diff`, ejecuta `git diff --check`, verifica
enlaces/comandos modificados y declara exactamente qué se validó.
