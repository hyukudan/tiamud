# ADR 0029: instancias de sesión de historia y protocolo `story.*`

- Estado: propuesta. Implementada de forma incremental en este árbol como fase
  S2 de [ADR 0025](0025-narrative-ruleset-story-core.md) (un jugador, estado
  vivo en memoria; slots persistidos conectados después, ver la nota de estado
  al final); ver
  [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md). Nada de este ADR
  cambia decisiones aceptadas hasta que se acepte.
- Fecha: 2026-10-15
- Alcance: `tiamud-server` (módulo `story_sessions`, adaptadores Web y Telnet),
  `tiamud-protocol` y el cliente navegador
- Relacionado: [ADR 0025](0025-narrative-ruleset-story-core.md),
  [ADR 0026](0026-typed-protocol-source-and-typescript-codegen.md),
  [ADR 0028](0028-story-menus-portraits-and-dialogue-trees.md),
  [especificación del protocolo](../protocol/tiamud-v1.md)

## Contexto

ADR 0025 exige un ADR propio para S2: actor de instancia en el servidor,
límites globales y por cuenta, desalojo por inactividad, protocolo `story.*`,
paridad Telnet/GMCP y Playwright. `story-core` ya ofrece `step`, `view`,
`view_pages` (páginas localizadas de hasta 12 KiB) y `UndoRing`, con
StoryPack v1 (librojuego) y v2 (menús de verbos, retratos y diálogos,
ADR 0028). Falta jugarlo desde el servidor real sin tocar `tiamud-core` ni el
actor del mundo.

## Decisión

### Catálogo y configuración

- `TIAMUD_STORY_DIR` habilita las historias; sin ella están **desactivadas**
  (no hay historia embebida por defecto). En desarrollo puede apuntar a
  `crates/story-core/fixtures`.
- Al arrancar, antes de abrir backend y listeners, se cargan todos los
  `*.story.json` del directorio (no recursivo, ≤ 64 packs, ≤ 1.024 entradas,
  cada pack ≤ 2 MiB) y se validan con `story-core`. Un pack inválido, un id
  duplicado o un directorio sin packs impide arrancar.
- `story-media.json` opcional en el mismo directorio mapea ids de activo de
  audio a rutas `assets/<sha256>/audio-<nombre>.{opus,ogg,mp3}` revisadas
  (≤ 64 KiB, ≤ 256 entradas, `deny_unknown_fields`). Un cue cuyo activo no
  está en el mapa nunca suena; nunca se acepta una URL arbitraria.
- `TIAMUD_STORY_MAX_INSTANCES` (1..=4.096, por defecto 256) y
  `TIAMUD_STORY_IDLE_SECONDS` (60..=86.400, por defecto 1.800). Valores fuera
  de rango o límites sin directorio fallan cerrados.

### Instancias

- Un `StoryManager` posee un mapa `cuenta → instancia`. Cada instancia es una
  tarea Tokio de **escritor único** con buzón acotado de 16 órdenes que posee
  su `StoryState`, su `UndoRing` (16 estados) y los ids opacos de su vista.
- Como máximo **una instancia por cuenta** en S2 y `max_instances` en total;
  superar un límite devuelve un error tipado y una métrica. Volver a entrar
  en la misma historia reanuda la instancia; otra historia se rechaza hasta
  salir.
- La semilla de cada partida sale de `OsRng` y queda registrada en la
  instancia; desde ahí `story-core` es determinista.
- Una instancia sin órdenes durante el plazo de inactividad se desaloja y se
  pierde (no hay slots hasta S3). Salir también la destruye. Un reinicio del
  servidor pierde todas las instancias.
- Las sesiones envían órdenes con `try_send` y esperan la respuesta como
  máximo 5 s; un buzón lleno o una respuesta tardía devuelven `busy` sin
  bloquear la sesión.
- Métricas sin etiquetas de contenido: `tiamud_story_catalog_stories`,
  `tiamud_story_instances`, `tiamud_story_instances_started_total`,
  `tiamud_story_instances_closed_total{reason=left|evicted}`,
  `tiamud_story_enter_rejected_total{reason=capacity|account|unknown}` y
  `tiamud_story_inputs_total{result=accepted|rejected}`.

### Presencia en el MUD

Se elige la opción más simple y segura: el personaje **queda aparcado** en su
sala, conectado y visible como cualquier jugador inactivo. `tiamud-core` y el
actor del mundo no cambian y no conocen la historia. Mientras la sesión está
en una historia, su entrada no llega al mundo: Web rechaza `action.perform` y
comandos de mundo con un `action.result` explicativo y Telnet suspende la
salida de escena (el chat sigue llegando) y la vuelve a mostrar al salir. No
hay indicador «absorto en una historia» para otros jugadores; añadirlo exige
cambiar la percepción del mundo y queda para otra fase.

### Proyección

- La instancia proyecta con `view_pages(pack, estado, "es")`, aplana sus
  elementos localizados y sustituye cada token de `story-core` por un id
  opaco aleatorio por vista (`c` + 16 hex de `OsRng`). El id nunca codifica
  turno, ordinal, id authored ni prosa; una vista nueva invalida todos los
  anteriores. Añade bloques de evento visibles (tiradas, objetos, puntuación,
  variables visibles) y los cues resueltos de la transición.
- Los cues de escena llegan como eventos al entrar en una escena; al
  reenviar una vista (reconexión, recuperación) solo se reanuda la música en
  curso. Fondos y retratos no se entregan como imagen en S2: el retrato se
  proyecta como nombre del personaje presente.
- El servidor re-empaqueta los elementos en partes cuyo marco secuenciado
  completo mide como máximo 15 KiB (bajo el límite de 16 KiB) porque el
  cambio de token por id altera el tamaño medido por `view_pages`. Ningún
  elemento se divide ni se trunca.
- El texto resuelve primero castellano (`es`); el locale por sesión es
  posterior.

### Protocolo `tiamud.v1` (aditivo)

Servidor → cliente: `story.entered {storyId,title}`, `story.view {viewRev,
part, parts, turn, canUndo, items[]}` con elementos etiquetados `scene`,
`cue`, `portrait`, `block`, `stat`, `item`, `choice`, `verb`, `target` y
`dialogueOption`, `story.left {reason: left|evicted}` y `story.error {code,
message}` con códigos cerrados.

Cliente → servidor: `story.choose {id, viewRev}` (opción de librojuego u
opción de diálogo), `story.dialogue {id, viewRev}` (solo diálogo),
`story.menu {verb, target?, viewRev}`, `story.undo {}` y `story.leave {}`.
Entrar y listar usan `command.submit` con `historia <id>`/`story <id>` y
`historias`/`stories`; `salir historia`/`story leave` también salen.

- `viewRev` distinto del vigente → `stale_view` y reenvío de la vista actual.
  Un id con forma inválida, nunca publicado, de otra clase o un objetivo de
  otro verbo → `unknown_choice`. Ninguno cambia el estado.
- Recuperación: tras `sync.request` y en cada reconexión de la cuenta el
  servidor reenvía `story.entered` y la vista vigente.
- Los tipos viven en `crates/protocol` y el TypeScript se genera (ADR 0026);
  el cliente valida cada marco con claves exactas, ids opacos, URLs de audio
  direccionadas por contenido y ensamblado de partes por `viewRev`.

### Telnet y GMCP

Texto plano con todas las acciones numeradas (`1) …`): opciones, opciones de
diálogo y cada verbo o par verbo → objetivo, en el mismo orden que el
navegador. Un número elige; `deshacer`/`undo`, `ver`/`look`, `salir`/`leave` e
`historias` funcionan dentro de la historia. GMCP `Tiamud.Story 1` emite
`Tiamud.Story.View` (resumen sin prosa ni ids: título, turno, puntuación,
final, acciones numeradas, presentes, estadísticas e inventario, con
degradación acotada bajo 8 KiB) y `Tiamud.Story.Left`. Los cues van por
`Client.Media` solo con opt-in de medios y salida enriquecida permitida.

## Consecuencias

- Las historias son jugables en Web y Telnet sin tocar el dominio del MUD;
  `tiamud-core` sigue sin conocer `story-core`.
- Toda partida es volátil hasta S3; la interfaz lo dice al desalojar.
- El protocolo crece de forma aditiva; un cliente antiguo ignora tipos
  desconocidos según la política aditiva vigente.

## Alternativas descartadas

- **Exponer los tokens de `story-core`** (`t{turno}c{n}`): son opacos respecto
  a ids authored pero predecibles y codifican turno y ordinal; un id aleatorio
  por vista separa protocolo y ruleset.
- **Ocultar el personaje o marcarlo como absorto**: exige variantes de
  percepción en `tiamud-core` y en el actor del mundo, que S2 no debe tocar.
- **Un actor global para todas las historias**: acopla latencia y fallos de
  partidas independientes.

## Nota de estado (2026-10-15): slots de guardado conectados

Añadida después de la decisión; no cambia el estado propuesto. Los slots de
[ADR 0030](0030-story-save-slots-and-schema-v16.md) están conectados a este
gestor de instancias, por lo que las frases anteriores sobre progreso perdido
al desalojar, salir o reiniciar quedan matizadas así:

- El gestor recibe el `CharacterStore` medido al arrancar
  (`attach_store`). Sin store, toda operación de slot responde
  `slots_unavailable` y salir no autoguarda.
- La instancia solo entrega su `StoryState` serializado (`to_json`) o acepta
  uno ya revalidado; las llamadas al store se hacen en la tarea de la sesión
  (o en la tarea de la instancia que ya se está cerrando al desalojar), con
  límite de 5 s y a través de la cola del worker de persistencia. El actor
  del mundo no participa.
- Slots manuales `1..=9`; el slot `0` es el autoguardado, escrito al salir,
  al desalojar por inactividad y en el apagado ordenado (que deja de admitir
  instancias nuevas y espera como máximo 10 s antes de drenar el mundo y el
  worker de persistencia), solo si la partida tiene progreso (turno > 0). Un
  autoguardado fallido se registra y cuenta, nunca bloquea la salida.
- Cargar exige la revisión exacta del pack (SHA-256 de sus bytes, conjunto
  compatible vacío) y revalida con `StoryState::from_json_slice`; un fallo es
  `slot_corrupt` o `slot_incompatible` sin cambiar la partida viva. Restaurar
  reinicia el anillo de deshacer: **el historial de deshacer no se persiste**.
- Al entrar en una instancia **nueva** se envía `story.slots` con
  `offerContinue` si hay un autoguardado compatible; reanudar una instancia
  viva no lo repite.
- Protocolo aditivo: cliente `story.save {slot}`, `story.restore {slot}`,
  `story.slots {}`, `story.deleteSlot {slot}`, `story.continue {}`; servidor
  `story.slots`, `story.saved`, `story.restored` (seguido de `story.view`) y
  siete códigos nuevos de `story.error`. Telnet `guardar/save N`,
  `cargar/load/restore N`, `partidas/saves`, `borrar/delete N`,
  `continuar/continue` y GMCP `Tiamud.Story.Slots`.
- Límite por cuenta: 6 escrituras (guardar/borrar) y 30 lecturas por minuto
  (ráfaga 10) con el limitador de cubeta por clave del servidor.

Ver el [registro de la conexión](../project/log/2026-10-15-story-save-slots-wired-s2-s3.md).

## Nota de estado (2026-10-15): catálogo con subdirectorios y parser

Añadida después de la decisión; no cambia el estado propuesto. El catálogo
carga ahora los `*.story.json` de `TIAMUD_STORY_DIR` **y de sus
subdirectorios directos** (sin ocultos ni enlaces simbólicos a directorios,
con el mismo límite de 1.024 entradas examinadas y 64 packs; `story-media.json`
solo en el directorio raíz), de modo que StoryPack v3 con parser
(`parser/…`) se juegan con `story.command`; ver la nota de
[ADR 0031](0031-parser-interactive-fiction-storypack-v3.md).

## Nota de estado (2026-10-15): imágenes, locale por sesión y prompt

Añadida después de la decisión; no cambia el estado propuesto. Sustituye lo
dicho arriba sobre fondos y retratos (ya se entregan como imagen revisada) y
sobre el locale fijo:

- `story-media.json` admite `formatVersion` 2 con `images` (≤256): id →
  `{manifest: "assets/<sha256>/manifest.json", alt: {locale: texto}}`;
  rutas de otra forma, alt ausente, con controles o de más de 512 bytes, o
  campos desconocidos impiden el arranque. `story.view` añade los campos
  opcionales `scene.background` y `portrait.image` (`{manifest, alt}`), sin
  depender del opt-in de audio. Los fixtures traen un mapa revisado con arte
  de muestra original de `lluvia-de-neon`.
- Locale por sesión: `historia <id> [locale]` (también `story`, y
  `historia compartir <id> [locale]`) elige entre los léxicos del mundo
  parser o los locales del título del pack; por defecto `es` si existe. Un
  locale no ofrecido es `unknown_locale`; una instancia viva conserva el
  suyo y quien se une a una compartida juega en el del anfitrión.
  `story.entered` añade `locale` y `locales`. Texto, eventos, alt y léxico
  siguen el locale; los mensajes del servidor siguen en castellano.
- El prompt del parser lo define el pack (`world.prompt`, ver
  [ADR 0031](0031-parser-interactive-fiction-storypack-v3.md)).

Ver el [registro](../project/log/2026-10-15-story-presentation-locale-compat.md).
