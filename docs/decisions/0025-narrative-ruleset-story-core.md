# ADR 0025: ruleset narrativo (story-core), StoryPack y sesiones de historia instanciadas

- Estado: propuesta. La fase S0 (crate puro `crates/story-core`, sin servidor,
  protocolo, persistencia ni cliente) está implementada como prototipo de
  biblioteca, y la parte de herramientas de S1 (CLI offline `tiamud-story` en
  `crates/story-tools` y esquema JSON publicado) también existe. El modelo de
  núcleo de S4 (StoryPack v2 con menús, retratos y diálogos) se propone en
  [ADR 0028](0028-story-menus-portraits-and-dialogue-trees.md) y existe solo
  como biblioteca; ver
  [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
  Nota de estado (2026-10-15): existe además una biblioteca aislada de parser
  de ficción interactiva castellano/inglés (`crates/story-parser`, entregada
  como «núcleo S4 del parser»; en la tabla de fases de este ADR el parser
  figura como S5). No depende de `story-core` ni está integrada: ningún
  StoryPack, actor, protocolo `story.command` ni cliente la usa, y su
  integración sigue requiriendo el ADR propio de parser e idioma.
  Nada de este ADR modifica decisiones aceptadas hasta que se acepte.
- Fecha: 2026-10-15
- Alcance: nuevo runtime narrativo hermano de `tiamud-core`
- Relacionado: [ADR 0001](0001-project-foundation.md),
  [ADR 0002](0002-runtime-world-and-durable-identity.md),
  [ADR 0005](0005-hot-area-lifecycle.md),
  [ADR 0008](0008-optional-scene-compositor.md)

## Contexto

El propietario quiere que el motor sirva también para géneros narrativos
distintos del MUD persistente:

- aventuras conversacionales con parser (ficción interactiva en castellano);
- librojuegos de «elige tu aventura» con estadísticas y dados (estilo Lobo
  Solitario o Lucha-ficción);
- aventuras narrativas por menús de verbos (MIRAR/HABLAR/INVESTIGAR/MOSTRAR al
  estilo de *Snatcher* o de las aventuras españolas de los 80), con retratos,
  árboles de diálogo y señales musicales;
- y, opcionalmente, juego compartido: instancias con varios jugadores, votos,
  espectadores y turnos asíncronos.

Un análisis de arquitectura concluye que **no** conviene doblar `tiamud-core`
para ello. `World`/`PlayerAction` es un dominio de ~31 000 líneas pensado para
un único mundo global y persistente con tiempo por ticks; contiene una misión
cableada (`first-hunt`), banderas de texto con tope de 64, un parser de
prefijos en inglés y supuestos de sala/salida/NPC que no existen en un
librojuego. Cada uno de esos rasgos es correcto para el MUD y erróneo para una
historia ramificada; añadir excepciones rompería la frontera autoritativa y
mezclaría dos modelos de tiempo.

Lo que sí es reutilizable es la infraestructura: actor de escritor único,
fachada de persistencia con backend único, transportes navegador/Telnet con
paridad, autenticación y cuentas, pipeline de medios revisados y sandbox Lua
con cuotas.

## Decisión

Se añade un **ruleset narrativo hermano**, `story-core` (paquete
`tiamud-story-core`), puro y determinista, que reutiliza la infraestructura de
Tiamud pero no comparte dominio con `tiamud-core`:

1. **Contenido**: un formato propio, **StoryPack**, versionado
   (`format_version`), con `deny_unknown_fields` en cada estructura, validador,
   lints y límites constantes. No es un WorldPack ni una extensión de él.
2. **Estado**: `StoryState` serializable (nodo, variables tipadas acotadas,
   inventario como conjunto de ids declarados, nodos visitados, estado del RNG,
   turno, puntuación y final).
3. **Transición**: `step(&StoryPack, &StoryState, StoryInput) ->
   Result<(StoryState, Vec<StoryEvent>), StoryError>`, pura, sin E/S, sin
   async, sin reloj y sin entropía externa.
4. **Proyección**: `view(&StoryPack, &StoryState) -> StoryView`, que expone
   solo el texto del nodo, las opciones **disponibles** tras tokens opacos, la
   hoja de estadísticas visibles y el inventario.
5. **Sesiones instanciadas**: cada partida es una instancia aislada con su
   propio escritor único; no existe un «mundo de historia» global.

### Frontera con `tiamud-core`

- `story-core` no depende de `tiamud-core`, `world-pack`, `tiamud-server` ni
  de ningún crate con E/S. Solo usa `serde`, `serde_json` y `thiserror`.
- `tiamud-core` no conoce `story-core`. Ni `PlayerAction` ni `World` ganan
  variantes narrativas; ninguna regla del MUD cambia.
- Un personaje del MUD y una sesión de historia no comparten estado de juego.
  Un puente futuro (p. ej. una historia lanzada desde una sala) sería una
  decisión aparte con su propio ADR.
- Lo compartido es infraestructura: cuentas/autenticación, transportes,
  límites de sesión, pipeline de medios y, en fases posteriores, la fachada de
  persistencia y el sandbox Lua.

### Modelo de instancias

- Cada sesión de historia activa es un **actor de escritor único por
  instancia**, con cola de entrada acotada, que posee su `StoryState` y su
  `UndoRing`. La transición es `step`; el actor solo serializa entradas y
  publica vistas.
- Límites globales configurables y con valor por defecto finito: instancias
  vivas totales, instancias por cuenta e inactividad máxima antes de expulsar
  la instancia de memoria (desalojo por inactividad). Superar un límite
  rechaza la creación con un error tipado; nunca crece sin cota.
- Antes de existir slots persistidos (S3), desalojar una instancia la pierde;
  la superficie lo dirá explícitamente.
- Las instancias no comparten memoria mutable; el pack validado es inmutable y
  puede compartirse por referencia.

### Tiempo por turnos

El tiempo narrativo es el **turno**: un contador `u32` que avanza una unidad
por entrada aceptada, con tope `MAX_TURNS`. No hay ticks, relojes ni
temporizadores dentro del ruleset. Los temporizadores de sesión (inactividad,
votación en multijugador) viven fuera, en el actor, y entran como entradas
explícitas.

### Determinismo y RNG sembrado

- El generador es un SplitMix64 implementado localmente; su secuencia está
  fijada por prueba para que una actualización de dependencias nunca altere
  partidas guardadas.
- La semilla la elige la sesión al crear la instancia (fuente segura del
  servidor) y el estado del generador forma parte de `StoryState`. Mismo pack,
  misma semilla y mismas entradas producen exactamente los mismos estados y
  eventos.
- Las tiradas usan muestreo por rechazo acotado. El estado del RNG nunca se
  expone en la vista.
- Todo `BTreeMap`/`BTreeSet`; ningún orden de iteración no determinista influye
  en el resultado.

### DSL cerrado de condiciones y efectos

- Condiciones: `all`, `any`, `not`, `var_cmp`, `has_item`, `visited`, `flag`,
  con profundidad y número de nodos acotados.
- Efectos: `set`, `add` (aritmética comprobada en ancho mayor, saturada y
  ajustada a los límites declarados), `give`, `take`, `goto`, `roll` (dados,
  variable modificadora, almacenamiento y prueba con destinos), `if`
  (anidamiento acotado), `cue` (música/sfx/parada, por id de activo revisado,
  nunca URL), `score` y `end`.
- El presupuesto de efectos por paso está acotado y la validación garantiza que
  un pack aceptado nunca lo excede. Ninguna regla lee prosa.
- Lua **no** forma parte de S0. Si una fase posterior necesita lógica que el
  DSL no expresa, se evaluará el sandbox existente con cuotas en su propio ADR.

### Modelo de guardado y deshacer

- **Deshacer**: un `UndoRing` en memoria con capacidad `1..=64`; al llenarse
  expulsa la entrada más antigua. Es memoria de sesión, no persistencia.
- **Guardado**: `StoryState` se serializa a JSON acotado (`MAX_STATE_BYTES`) y
  `StoryState::check_against` revalida todo estado leído contra su pack antes de
  usarlo. Los **slots persistidos** requerirán un esquema SQL futuro (v16 en
  ambos adaptadores, SQLite y PostgreSQL, con migración y paridad) y **su
  propio ADR**; este ADR no reserva ni define ese esquema.
- Un slot deberá ligar el estado a la identidad exacta del pack (id y digest);
  cargar un slot contra otra revisión del pack se rechaza o se migra de forma
  explícita, nunca implícitamente.

### Superficie de protocolo (boceto, no normativo)

Mensajes servidor → cliente, bajo `tiamud.v1` o una extensión versionada:

- `story.scene` (texto del nodo, título, retrato/fondo opcional por id de
  activo), `story.choices` (lista de `{token, label}`), `story.menu` (verbos y
  objetivos para aventuras por menús), `story.dialogue` (turno de diálogo con
  hablante), `story.cue` (música/sfx/parada, solo a clientes con medios
  activados), `story.prompt` (entrada libre para parser), `story.ended`
  (tipo de final y puntuación).

Mensajes cliente → servidor: `story.choose {token}`, `story.command {text}`
(parser, fase posterior), `story.undo`, `story.save {slot}`,
`story.restore {slot}`.

Reglas:

- **Tokens opacos por vista**: el token de opción está ligado al turno de la
  vista y al ordinal entre las opciones *disponibles*. Nunca codifica el id
  authored ni deriva de la prosa; un token de una vista anterior se rechaza
  (`StaleChoice`) y una opción oculta no tiene token.
- **Marcos de 16 KiB**: ningún mensaje supera el límite vigente de 16 KiB; un
  nodo largo o un menú amplio se **paginan** en partes correlacionadas, nunca
  se trunca contenido jugable ni se relaja el límite.
- **Telnet**: opciones numeradas en texto plano (`1) Atacar`), con la misma
  semántica que el navegador; GMCP opcional `Tiamud.Story` (paquetes bajo el
  límite GMCP vigente de 8 KiB). Perder medios o GMCP nunca pierde acciones.
- Percepción antes de serializar: la vista ya excluye variables ocultas,
  opciones no disponibles, ids authored y estado del RNG.

### Capa multijugador (posterior)

Instancias compartidas, votación de opciones, espectadores de solo lectura y
turnos asíncronos se construyen **encima** del mismo `step`: el actor de la
instancia decide qué entrada se aplica (p. ej. la opción ganadora de una
votación con plazo acotado). El ruleset no cambia. Requiere su propio ADR por
sus implicaciones de autorización, moderación y persistencia.

## Fases

| Fase | Contenido | ¿ADR propio? |
| --- | --- | --- |
| S0 | Crate puro `story-core`: StoryPack, validador/lints, DSL, RNG sembrado, `step`/`view`, `UndoRing`, fixture en castellano y pruebas deterministas | No (este ADR) |
| S1 | CLI de autoría (`validate`/`play` en terminal) y esquema JSON publicado del StoryPack | No, salvo cambio de formato |
| S2 | Actor de instancia en el servidor, límites globales/por cuenta, desalojo por inactividad, protocolo `story.*`, paridad Telnet/GMCP, Playwright | **Sí** (protocolo y actor) |
| S3 | Slots de guardado persistidos, esquema SQL v16 SQLite/PostgreSQL, migración y paridad | **Sí** (persistencia) |
| S4 | Aventuras por menús de verbos: `story.menu`, diálogos, retratos y cues vía pipeline de medios | **Sí** (modelo de menús/diálogo: [ADR 0028](0028-story-menus-portraits-and-dialogue-trees.md), propuesto; su modelo de núcleo StoryPack v2 existe como biblioteca) |
| S5 | Parser de ficción interactiva en castellano (`story.command`/`story.prompt`), gramática acotada y determinista | **Sí** (parser e idioma) |
| S6 | Integración opcional de scripts con el sandbox Lua y cuotas | **Sí** |
| S7 | Multijugador: instancias compartidas, votos, espectadores, asíncrono | **Sí** |
| S8 | Herramientas de autoría avanzadas, importadores y aceptación (carga, soak de instancias) | Solo si cambia arquitectura |

## Consecuencias

- El MUD conserva su dominio intacto; el riesgo de regresión de S0 sobre el
  baseline es nulo porque ningún crate existente depende de `story-core`.
- Hay dos rulesets que mantener. Se acepta a cambio de no contaminar
  `tiamud-core` con un modelo de tiempo y de contenido ajeno.
- StoryPack es un formato nuevo con ids durables una vez publicado; renombrar
  nodos, variables u objetos publicados exigirá alias o migración explícita.
- Mientras S2/S3 no existan, `story-core` no es jugable desde el servidor ni
  persiste partidas; la matriz lo registra como `Partial`.

## Alternativas descartadas

- **Extender `World`/`PlayerAction`**: obligaría a generalizar misiones,
  banderas, parser y modelo de salas de un dominio de 31 000 líneas con un único
  mundo global; mezcla tiempo por ticks y por turnos y rompe la frontera
  autoritativa.
- **Un mundo global de historias dentro del actor del MUD**: acopla latencia y
  fallos de partidas aisladas al mundo compartido y complica límites por
  cuenta.
- **Motor externo (Ink, Twine, Inform) embebido**: añade otro lenguaje, otra
  semántica de guardado y un runtime que no controla límites ni determinismo
  con las garantías de Tiamud; las importaciones pueden ser herramientas
  offline posteriores.
- **Lua como lenguaje principal de historias**: más expresivo pero menos
  validable estáticamente; el DSL cerrado permite reachability, lints y
  garantías de límites antes de ejecutar.
- **Generación de texto con LLM en runtime**: excluida por las restricciones
  del proyecto.
