# ADR 0031: ficción interactiva con parser en StoryPack v3

- Estado: propuesta. El núcleo (StoryPack v3, `StoryInput::Command`,
  `step_command`, biblioteca por defecto, vista con transcripción y aviso de
  entrada, validador, lints, CLI y fixture) existe como biblioteca en
  `crates/story-core`, `crates/story-parser` y `crates/story-tools`. No hay
  cableado de servidor, protocolo `story.command`/`story.prompt`, Telnet ni
  navegador. Ver [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
- Fecha: 2026-10-15
- Alcance: fase S5 de [ADR 0025](0025-narrative-ruleset-story-core.md)
  (parser de ficción interactiva en castellano), parte de modelo de
  contenido, transición y proyección
- Relacionado: [ADR 0025](0025-narrative-ruleset-story-core.md),
  [ADR 0028](0028-story-menus-portraits-and-dialogue-trees.md),
  [ADR 0029](0029-story-session-instances-and-story-protocol.md),
  [ADR 0030](0030-story-save-slots-and-schema-v16.md)

## Contexto

ADR 0025 reserva S5 para aventuras conversacionales con parser en castellano
y exige un ADR propio de parser e idioma. Existe ya `tiamud-story-parser`, una
biblioteca pura que convierte una línea (`coge la llave y ábrela`) en órdenes
tipadas `Command { action, obj, prep, obj2, direction, all, except, meta }`
contra un alcance (`Scope`) que aporta quien llama, con preguntas de
ambigüedad e incompletitud (`Ambiguous`/`Incomplete` + `Parser::answer`).
Faltaba todo lo demás: un mundo de objetos y salas en el StoryPack, el alcance
calculado desde `StoryState`, reglas de autor, comportamiento por defecto de
las acciones clásicas, preguntas que sobrevivan a un guardado y una vista con
transcripción.

Las aventuras españolas clásicas (PAWS/DAAD, la librería InformATE) fijan las
expectativas: salas con salidas por dirección canónica, objetos con nombre,
género, sustantivos y adjetivos, contenedores que se abren y cierran,
cerraduras con llave, oscuridad con fuente de luz, ropa, personajes con los que
hablar, reglas «antes/en lugar de/después», puntuación y mensajes por defecto
en castellano.

## Decisión

### Formato y compatibilidad

- `STORY_PACK_FORMAT_VERSION` pasa a **3**; el cargador acepta `1..=3`.
- v3 es un superconjunto estricto de v2. Todos los campos nuevos son
  opcionales y se omiten vacíos. El validador rechaza rasgos v3 (`world`,
  `room` en nodos y las condiciones/efectos de objetos) en documentos que
  declaran una versión anterior. `StoryPack::upgrade` y
  `tiamud-story fmt --upgrade` solo reescriben la versión.
- `STORY_STATE_FORMAT_VERSION` pasa a **3**. Los estados v1 y v2 sin campos
  nuevos se cargan; un estado v1/v2 con campos v3 se rechaza. La capa de
  persistencia S3 ya admite versiones de estado `1..=255` sin cambio de
  esquema SQL.

### Modelo de mundo

- **`world`** del pack: `lexicons` (1–4 léxicos publicados por id, `es`/`en`,
  cada uno con hasta 32 verbos propios del pack en la forma JSON del léxico,
  que pueden ser sinónimos de acciones de biblioteca o acciones nuevas que solo
  manejan reglas), `objects` (≤128), `rules` (≤512), `messages` (sustitución
  de mensajes de biblioteca por clave cerrada), `intro` y `max_score`.
- **Salas**: un nodo con `room` (nombre, `dark`, ≤16 `exits`) es un lugar;
  su `text` es la descripción. Cada salida usa un id de dirección canónico que
  conocen todos los léxicos del mundo, un destino, una condición opcional con
  texto `blocked`, y una puerta opcional (objeto abrible presente en la sala).
  Un nodo final no puede ser sala.
- **Objetos**: id durable, nombre localizado **sin artículo**, `gender`
  (masculino/femenino para los mensajes castellanos), `proper`, sustantivos y
  adjetivos por idioma de léxico (≤16, una palabra plegable cada uno),
  descripción, texto legible, ubicación inicial (`{"room": n}`, `"carried"`,
  `{"in": contenedor}`, `"nowhere"`), `found_in` para objetos fijos presentes
  en varias salas (puertas, decorado) y propiedades `portable`, `scenery`,
  `animate`, `container`, `openable`/`open`, `locked` + `key`,
  `lightable`/`lit`, `wearable`/`worn`. La contención es un bosque (sin
  ciclos) validado estáticamente y re-comprobado en cada estado.
- **Estado de objeto** en `StoryState.objects`: ubicación y banderas
  `open`/`locked`/`lit`/`worn`, coherentes con las propiedades declaradas.

### DSL

Se reutilizan `Condition` y `Effect` cerrados. Condiciones nuevas: `carried`,
`object_at`, `object_is` (bandera), `visible` (en alcance ahora), `player_in`
y `turn_cmp`. Efectos nuevos: `move_object` (un movimiento que metería un
contenedor dentro de sí mismo se ignora; salir de las manos quita `worn`) y
`set_object` (solo banderas que el objeto tiene). Ninguna regla lee prosa.

### Reglas y biblioteca

- Una regla se indexa por `(action, obj?, obj2?, direction?)` con fase
  `before`, `instead` o `after`, condición, efectos y texto. Dentro de cada
  fase gana la **primera** regla en orden de autor cuyo índice coincida y cuya
  condición se cumpla. `before` se ejecuta y la acción continúa salvo que la
  regla mueva al jugador o termine la historia; `instead` sustituye la acción;
  `after` se ejecuta solo si la acción de biblioteca tuvo éxito.
- La **biblioteca por defecto** implementa `look`, `examine`, `search`,
  `look_under`, `inventory`, `take`, `drop`, `open`, `close`,
  `unlock`/`lock` con llave, `put_in` (y `put_on` como rechazo), `go`,
  `enter`/`exit`, `wait`, `give`, `show`, `talk`, `read`, `wear`/`remove` y
  `switch_on`/`switch_off`, con mensajes castellanos e ingleses por defecto
  que el pack puede sustituir. Cualquier otra acción responde «No ocurre
  nada.» salvo que una regla la maneje.
- `todo`/`all` actúa sobre, como mucho, 32 objetos en orden de pack (`take`:
  objetos sueltos de la sala; `drop`/`put_in`: lo que se lleva en la mano).

### Alcance, luz y pronombre

- El alcance del parser se calcula desde el estado en cada orden: lo que se
  lleva (con el contenido de contenedores abiertos) y, si la sala está
  iluminada, lo que hay en ella o está fijo en ella (`found_in`), con todos los
  contenedores intermedios abiertos. Una sala oscura está iluminada si un
  objeto encendido está al alcance (llevado o en la sala). Las palabras de
  objetos fuera de alcance se declaran vocabulario conocido, de modo que la
  respuesta es «No ves eso por aquí.» y no «No conozco la palabra…».
- El referente del pronombre se guarda en `StoryState.pronoun` (id de objeto)
  y solo se ofrece al parser mientras el objeto es visible.

### Línea, preguntas y turnos

- `StoryInput::Command { text, language? }` y la función pública
  `step_command`. La línea se divide con `Parser::split_line` y cada orden se
  analiza con `Parser::parse_segment` contra un **alcance nuevo** tras ejecutar
  la anterior; la línea se detiene en el primer fallo (error de análisis,
  pregunta, acción fallida, orden meta o final).
- Cada orden del mundo ejecutada avanza el turno una unidad (`MAX_TURNS`
  sigue aplicando). Errores de análisis, preguntas y órdenes meta no avanzan
  el turno.
- Las preguntas se guardan como **entradas reproducibles**, no como
  estructuras internas del parser: `PendingQuestion { language, line,
  segment, answers }` (línea ≤256 bytes, ≤4 respuestas encadenadas). Como el
  estado no cambia mientras se pregunta, reanalizar la línea y las respuestas
  previas regenera exactamente la misma pregunta; así el estado guardado es
  pequeño, acotado e independiente de la representación privada de
  `story-parser`. La siguiente entrada se pasa a `Parser::answer`; si es una
  orden nueva o varias, se trata como línea nueva.

### Órdenes meta

`undo`, `save`, `restore`, `quit`, `restart`, `score` y `help` (las acciones
`meta` del léxico) no las ejecuta el ruleset: `step_command` las devuelve en
`CommandStep.meta` (`MetaRequest { action }`) para que el anfitrión las
gestione; `score` y `help` además responden en la transcripción. Se eligió un
valor de retorno separado en vez de nuevas variantes de `StoryEvent` para no
romper la proyección de eventos del servidor S2, que hace una coincidencia
exhaustiva sobre `StoryEvent`; `step(StoryInput::Command)` es compatible y
descarta las peticiones meta. El ADR de cableado decidirá si `story.*` las
expone como mensajes propios.

### Transcripción y vista

- `StoryState.transcript` (≤256 respuestas) guarda las respuestas de la última
  entrada como **claves e ids** (`Reply::Message { message, obj, obj2, list }`,
  `RoomName`, `RoomText`, `Describe`, `Read`, `Rule`, `Blocked`, `Score`,
  `Intro`); la única prosa almacenada es la pregunta o error ya renderizado
  por el parser (≤1024 bytes). La vista las renderiza en todos los idiomas de
  léxico del mundo como bloques de texto (`ViewBlock` sin hablante), detrás de
  las respuestas de menú y diálogo.
- En una sala, el texto de escena de la vista es el **nombre** de la sala (o
  «Oscuridad»); la descripción llega a la transcripción al mirar o entrar.
  Un estado nuevo empieza con `intro` y la descripción de la sala inicial.
- `StoryView.prompt: Option<ViewPrompt { text, question }>` indica que se
  acepta texto libre y si se espera respuesta a una pregunta.
  `PageItem::Scene` añade los campos opcionales `prompt` y `question`; ningún
  otro elemento de página cambia y la paginación de 12 KiB se mantiene.

### Dependencias

`tiamud-story-core` pasa a depender de `tiamud-story-parser`, también puro
(solo `serde`, `serde_json`, `thiserror`). `story-parser` sigue sin depender
de `story-core`. Los léxicos publicados se compilan por llamada con los
verbos del pack; su JSON base se analiza una sola vez por proceso.

### Cambios en los léxicos publicados

- Se añaden `lock` (`cerrar X con Y`, `bloquear`; `lock X with Y`),
  `wear` (`ponte`, `vestir`…; `wear`, `put on`), `remove` (`quítate`,
  `quitar`…; `remove`, `take off`) y, en inglés, `exit` (`exit`, `leave`,
  `get out`).
- Los enclíticos se prueban del sufijo más corto al más largo, de modo que
  gana la raíz verbal más larga: `póntela` es `ponte` + `la` (ponérsela), no
  `pon` + `tela`. Los resultados de los casos existentes no cambian.

## Fuera de alcance (pendiente)

- Actor de servidor, `story.command`/`story.prompt`, Telnet, GMCP y navegador
  (requieren cablear S2 y, si cambia el protocolo, su ADR).
- Locale por sesión y packs bilingües en el servidor (el servidor S2 fija `es`).
- Superficies (`put_on` solo rechaza), objetos transparentes, cantidades y
  plurales, preguntar *por* temas (`pregunta a X por Y`), PNJ que se mueven,
  temporizadores y daemons, y género/número de pronombres.
- Lints completos de resolubilidad (siguen siendo sobreaproximaciones).

## Consecuencias

- Las aventuras con parser se escriben como datos validados: salas, objetos,
  reglas y mensajes, sin Lua ni prosa interpretada.
- El estado de una partida con parser es mayor (objetos y transcripción),
  pero acotado por el pack y por `MAX_STATE_BYTES`.
- Los ids de objetos, reglas y salas son durables: los estados guardados los
  referencian.
- El servidor sigue sin cambios funcionales: un pack v3 cargado por S2 se
  valida, pero sin `story.command` no se puede jugar desde el servidor.

## Alternativas descartadas

- **Serializar `Pending` del parser**: expondría y congelaría estructuras
  internas de `story-parser` en los guardados; reproducir las entradas es más
  pequeño y robusto ante versiones.
- **Traducir órdenes a opciones de librojuego**: obligaría a enumerar
  combinaciones acción × objeto y perdería la biblioteca por defecto.
- **Lua para las reglas**: menos validable estáticamente (ADR 0025 lo deja
  para S6).
- **Nuevas variantes de `StoryEvent` para meta**: rompería la coincidencia
  exhaustiva del servidor S2 sin aportar nada a la transición pura.

## Nota de estado (2026-10-15): cableado al servidor

Añadida después de la decisión; no cambia el estado propuesto. Las sesiones
S2/S7 ([ADR 0029](0029-story-session-instances-and-story-protocol.md),
[ADR 0033](0033-shared-story-instances-and-voting.md)) juegan ya packs v3 con
la API pública `step_command`, sin cambiar `story-core` ni `story-parser`:

- Protocolo aditivo: cliente `story.command {text, viewRev}` (≤256 bytes,
  estricto); en `story.view` la escena lleva `prompt?: {text, question}` y hay
  bloques `command` (eco) y `reply` (transcripción y avisos del servidor);
  códigos `no_parser` e `invalid_command`. Se eligió `prompt` dentro de
  `story.view` en lugar de un mensaje `story.prompt` separado: el aviso
  pertenece a la vista y viaja con su `viewRev`.
- Idioma por instancia: `es` si el pack tiene léxico castellano, si no su
  primer léxico; la vista se resuelve en ese idioma. No hay elección por
  sesión.
- Órdenes meta → operaciones de sesión dentro de la instancia de escritor
  único: `deshacer` (una línea con órdenes previas deshace exactamente esa
  línea), `reiniciar` con confirmación tecleada sobre esa misma vista
  (deshacible), `puntos`/`ayuda` en la vista, `guardar`/`cargar` sin ranura →
  aviso y `story.slots`, `fin` → salir con autoguardado. Solo las líneas que
  ejecutan órdenes del mundo son pasos de deshacer y consumen turno.
- Límites: 256 bytes, `viewRev` vigente, 120 líneas/min por cuenta (ráfaga
  20). Compartidas: el texto libre nunca se vota (anfitrión en modos
  `host`/`vote`, titular del turno en `turns`).
- Telnet: toda línea no reservada es una orden; las palabras reservadas
  conservan su significado (`look`/`l` y `guardar`/`cargar` sin número van al
  parser). GMCP `Tiamud.Story.View.prompt`.
- El catálogo examina también los subdirectorios directos de
  `TIAMUD_STORY_DIR`.

Ver el [registro del cableado](../project/log/2026-10-15-story-parser-server-wiring-s5.md).
