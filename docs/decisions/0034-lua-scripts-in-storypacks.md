# ADR 0034: scripts Lua opcionales en StoryPacks

- Estado: propuesta. El núcleo existe como biblioteca: puntos de enganche
  `ScriptHost` en `crates/story-core`, el anfitrión Lua en
  `crates/story-script` (`tiamud-story-script`), una API aditiva
  `call_function`/`validate_functions` en `crates/scripting` y el soporte de
  `tiamud-story` (validar, lints, `preview`, `test`, `play`). Desde la
  [nota de estado del cableado](#nota-de-estado-2026-10-15-cableado-a-las-sesiones-del-servidor)
  las sesiones del servidor ejecutan scripts solo con `TIAMUD_STORY_SCRIPTS=1`
  (desactivado por defecto: entonces el catálogo rechaza esos packs al
  arrancar). Ver [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
- Fecha: 2026-10-15
- Alcance: fase S6 de [ADR 0025](0025-narrative-ruleset-story-core.md)
  («integración opcional de scripts con el sandbox Lua y cuotas»)
- Relacionado: [ADR 0025](0025-narrative-ruleset-story-core.md),
  [ADR 0029](0029-story-session-instances-and-story-protocol.md),
  [ADR 0031](0031-parser-interactive-fiction-storypack-v3.md),
  [ADR 0033](0033-shared-story-instances-and-voting.md)

## Contexto

El DSL cerrado de StoryPack (condiciones, efectos, tiradas, `if`, reglas del
parser) cubre librojuegos, menús y ficción interactiva clásicos, y permite
validación estática, lints y cotas antes de ejecutar. Algunas reglas son
incómodas o imposibles de expresar en él: un asalto de combate con críticos
según la diferencia de tiradas, una fórmula de daño con varios modificadores,
un acertijo con aritmética. ADR 0025 descartó Lua como lenguaje principal de
historias, pero reservó S6 para una válvula de escape opcional sobre el
sandbox existente (`crates/scripting`: VM nueva por invocación, sin E/S,
64 KiB de fuente, 2 MiB de memoria, 100 000 instrucciones, 25 ms, 32 eventos,
semilla determinista, intenciones con capacidades validadas por Rust).

ADR 0025 también fija que `story-core` solo depende de `serde`, `serde_json`
y `thiserror` (más el parser puro de ADR 0031), sin E/S ni runtime de
scripts.

## Decisión

### Los scripts son funciones puras que emiten las mismas intenciones

- Un pack v3 puede declarar una sección opcional `scripts` (≤32 scripts,
  ≤64 KiB de fuente cada uno, ≤256 KiB en total, sin NUL). Cada script tiene
  `id` durable, `kind` (`condition` o `effect`) y `source` Lua 5.4.
- Se referencia desde el DSL con `{"script": {"id": …, "args": {…}}}` como
  **condición** (`Condition::Script`) o como **efecto** (`Effect::Script`),
  con ≤8 argumentos constantes enteros (|v| ≤ 10⁹) o booleanos.
- Un script de condición define `check(story, args)` y devuelve un booleano;
  no puede emitir nada (conjunto de capacidades vacío) y no admite grants.
- Un script de efecto define `run(story, args)` y solo **emite intenciones**
  con los ayudantes `fx` (`fx.set`, `fx.add`, `fx.give`, `fx.take`, `fx.go`,
  `fx.score`, `fx.finish`, `fx.move`, `fx.set_object`). El anfitrión las
  convierte en valores `Effect` ordinarios del DSL cerrado; `story-core`
  comprueba cada una contra los **grants** declarados del script (`writes`
  para variables, `items`, `goto`, `objects`, `score`, `ends`), contra los
  tipos y cotas del DSL (`MAX_VAR_ABS`, `MAX_SCORE_DELTA`, id de final válido,
  sala/contenedor existentes, bandera que el objeto tiene) y solo después las
  aplica con el mismo código que los efectos escritos a mano, cargándolas al
  mismo presupuesto de 64 efectos por paso. `roll`, `if`, `cue` y `script` no
  se pueden emitir. ≤32 intenciones por invocación.
- El script recibe una **copia** de una proyección acotada y de solo lectura
  del estado (`ScriptSnapshot`): nodo, turno, puntuación, todas las variables
  (enteras y booleanas), objetos del inventario, nodos visitados y, en mundos
  v3, la ubicación y las banderas de cada objeto. Nunca ve el estado del RNG,
  la transcripción, contadores de menú ni el historial de deshacer. Escribir
  en esa tabla no tiene efecto alguno.

Así Lua nunca es una segunda autoridad: la transición sigue siendo
`step` de `story-core`, el estado solo cambia mediante el DSL cerrado y todo
lo que un script puede tocar está declarado y es auditable en el pack.

### `story-core` sigue puro: anfitrión opcional

- `story-core` no depende de Lua. Expone el trait `ScriptHost`
  (`check`/`run` sobre un `ScriptCall` con declaración, argumentos,
  proyección y semilla) y variantes aditivas `step_scripted`,
  `view_scripted`, `view_pages_scripted`, `step_command_scripted` y
  `resolve_menu_scripted`. Las funciones existentes (`step`, `view`, …) no
  cambian de firma.
- El anfitrión Lua vive en un crate aparte, `tiamud-story-script`
  (`story-core` + `tiamud-scripting`): `LuaStoryHost` y `validate_pack`.
- **Sin anfitrión** cualquier referencia a un script falla cerrada con
  `StoryError::ScriptUnavailable`. Además `StoryPack::validate()` (la
  validación de un runtime sin anfitrión, la que usa el servidor) **rechaza**
  cualquier pack que declare scripts, así que no se puede cargar donde no
  podría ejecutarse. `StoryPack::validate_scripted()` comprueba declaraciones,
  grants y referencias; `tiamud_story_script::validate_pack` añade la
  compilación y la inicialización de cada fuente en el sandbox (sin llamar a
  `check`/`run`) y exige la función de entrada.
- La API de `tiamud-scripting` crece de forma aditiva: `call_function`
  (función global con argumentos `ScriptValue` acotados — profundidad ≤4,
  ≤16 384 valores, cadenas ≤1 KiB —, conjunto cerrado de clases de evento y
  devolución escalar) y `validate_functions`. `execute_hook`,
  `validate_module`, `ScriptError` y `ScriptLimits` no cambian.

### Determinismo

- Cada invocación usa una VM nueva sin `os`, `io`, `package`, `require`,
  `load`, `debug`, `print`, `collectgarbage`, `math.random` ni funciones de
  patrón. El preludio de historia elimina además `next` y `string.format`
  (`%p` imprime direcciones), hace que `tostring` devuelva solo el tipo para
  tablas/funciones y redefine `pairs` para iterar en orden de clave ordenado
  (números antes que cadenas; otras claves son error), porque el orden de las
  tablas hash de Lua 5.4 depende de una semilla por estado.
- La aleatoriedad solo entra por `tiamud.random(n)` y `dice(n, caras)`,
  sembrados por el anfitrión: un script de efecto recibe un `u64` extraído del
  SplitMix64 de la historia (que por tanto avanza y queda en el estado); un
  script de condición recibe una semilla derivada del estado (RNG, turno e id)
  sin avanzar nada, de modo que `view` y el `step` siguiente evalúan igual.
- Mismo pack, misma semilla y mismas entradas producen los mismos estados y
  eventos. La cuota de instrucciones es la cota determinista; el tope de
  25 ms de reloj es una red de seguridad cuyo disparo solo puede **fallar**
  un paso (sin cambiar el estado), nunca producir un resultado distinto.

### Cuotas y presupuesto por paso

- Por invocación rigen las cuotas del sandbox (64 KiB de fuente del autor más
  el preludio, 2 MiB, 100 000 instrucciones, 25 ms, 32 eventos de ≤1 KiB por
  campo).
- Por paso (o por vista) se permiten como mucho
  `MAX_SCRIPT_CALLS_PER_STEP` = 16 invocaciones entre condiciones y efectos;
  superarlo es `StoryError::ScriptBudget`. Una línea del parser comparte un
  único presupuesto.
- El peor caso es por tanto 16 × 25 ms por entrada. Antes de cablear scripts a
  las sesiones del servidor, el ADR de cableado debe fijar un presupuesto
  agregado de reloj por entrada (como el de 50 ms por lote del mundo) y
  ejecutar los scripts fuera de cualquier actor compartido.

### Semántica de fallo

- Un error de compilación, error Lua, cuota agotada, valor de retorno no
  booleano en una condición, intención mal formada o no concedida, o
  presupuesto excedido hace fallar el **paso entero** con un error tipado
  (`StoryError::Script { script, message }`, `ScriptBudget`,
  `ScriptUnavailable`). Como `step` es puro, el estado de entrada queda
  intacto y sigue siendo utilizable; no hay aplicación parcial.
- El mensaje es para autores: nombra el script y, gracias al nombre de chunk
  `=<id>` y a que el preludio ocupa la misma primera línea, la línea real del
  fuente (`asalto:3: attempt to index a nil value`), sin la traza de pila.
  `tiamud-story validate` lo sitúa en `scripts[i].source`. Un transporte debe
  mostrar a jugadores solo un error genérico (el servidor S2 ya traduce
  cualquier error no previsto a `internal`).

### Formato

- No hay StoryPack v4: `scripts` y las variantes `script` son campos
  opcionales de v3, omitidos cuando están vacíos, y el validador los rechaza
  en documentos que declaran v1 o v2. Un pack v3 sin scripts no cambia en
  absoluto. `StoryState` no cambia (los scripts no añaden estado).
- El esquema JSON publicado de v3 describe `scripts` y las variantes
  `script`.

## Consecuencias

- Los autores pueden expresar reglas no triviales sin ampliar el DSL, y la
  validación sigue garantizando qué puede tocar cada script.
- Los lints de `tiamud-story` no ven dentro de Lua: tratan los grants de un
  script de efecto como posibles mutaciones y sus `goto`/`ends` como salidas,
  y avisan de scripts declarados que nada referencia. Su silencio prueba aún
  menos que antes.
- Un pack con scripts depende del sandbox Lua (mlua/Lua 5.4 vendorizado) en
  cualquier runtime que lo ejecute.

## Alternativas descartadas

- **Lua que muta el estado directamente** (tablas escribibles enlazadas a
  `StoryState`): crearía una segunda autoridad sin validación de tipos,
  cotas ni grants.
- **Ampliar el DSL con aritmética general**: acerca el DSL a un lenguaje de
  propósito general sin sandbox ni cuotas.
- **Dependencia opcional (feature) de Lua dentro de `story-core`**: rompería
  la regla de dependencias de ADR 0025 para quien compile con la feature y
  mezclaría el runtime de scripts con el ruleset puro.
- **StoryPack v4**: innecesario; el cambio es aditivo y opcional dentro de v3.

## Nota de estado (2026-10-15): cableado a las sesiones del servidor

Esta nota fija los dos requisitos previos que la sección «Cuotas y
presupuesto por paso» exigía antes de cablear y describe lo entregado; no
cambia la decisión ni acepta el ADR, que sigue **propuesto**.

- **Interruptor de arranque.** `TIAMUD_STORY_SCRIPTS` (`0`/`1`, por defecto
  desactivado). Desactivado, un `*.story.json` que declara scripts detiene el
  arranque como cualquier pack inválido. Activado, cada pack con scripts se
  valida con `tiamud_story_script::validate_pack` (declaraciones, grants,
  referencias y compilación de cada fuente en el sandbox) antes de abrir
  backend o listeners. Los packs sin scripts siguen pasando por
  `StoryPack::validate()`.
- **Fuera de cualquier actor compartido y del runtime async.** Cada instancia
  ya es su propia tarea de escritor único; para una historia con scripts,
  toda operación que puede evaluar un script (proyección inicial, elegir,
  menú/diálogo, línea de parser, voto, deshacer, cargar partida y el tick de
  una instancia compartida) mueve el estado de la instancia a un pool
  bloqueante acotado (`spawn_blocking`) y lo recupera al terminar. El pool
  limita globalmente los trabajos simultáneos (`TIAMUD_STORY_SCRIPT_WORKERS`,
  `1..=64`, por defecto el paralelismo del host hasta 16) y los admitidos en
  espera (`TIAMUD_STORY_SCRIPT_QUEUE`, `0..=1024`, por defecto 4 × workers);
  saturado, la entrada se rechaza con `busy` antes de ejecutar nada y el
  estado no cambia. Un tick compartido rechazado se reintenta en el
  siguiente; salir y expulsar dejan su tick oportunista al reloj.
- **Presupuesto agregado por entrada.** `TIAMUD_STORY_SCRIPT_BUDGET_MS`
  (`10..=400`, por defecto 100 ms) de reloj para todas las llamadas de una
  entrada (paso y proyección), además de las cuotas por llamada: cada
  llamada recibe como tope de reloj el mínimo de 25 ms y lo que queda del
  presupuesto, y una llamada que empieza sin presupuesto falla. Como hasta
  ahora, agotar algo solo hace fallar el paso.
- **Fallos.** Un error de script, cuota o presupuesto llega al jugador como
  un error genérico (`story.error` con código `internal`, en castellano o
  inglés según el idioma de la sesión), con el detalle de autor solo en el
  log del servidor; el estado queda intacto. Tres fallos de script seguidos
  en una instancia la cierran con autoguardado del último estado bueno.
- **Compartidas.** Igual semántica; la mesa resuelve votos o turnos antes de
  ejecutar los scripts de la acción ganadora.
- **Métricas** sin etiquetas de historia ni script:
  `tiamud_story_script_invocations_total`,
  `tiamud_story_script_failures_total`,
  `tiamud_story_script_budget_exceeded_total`,
  `tiamud_story_script_busy_total` y
  `tiamud_story_instances_closed_total{reason="script_failures"}`.
- Sin cambios de protocolo, `StoryState`, `story-core` ni del actor del
  mundo. Evidencia local solamente (pruebas Rust del servidor); no hay
  escenario Playwright del fixture con scripts.
