# ADR 0028: menús de verbos, retratos y árboles de diálogo en StoryPack v2

- Estado: aceptada — implementación parcial. El modelo de núcleo (StoryPack v2 en `crates/story-core`
  y su soporte offline en `crates/story-tools`) está implementado como
  biblioteca; no existe protocolo, actor de servidor ni interfaz. Ver
  [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
  2026-10-16: aceptado por el propietario.
- Fecha: 2026-10-15
- Alcance: fase S4 de [ADR 0025](0025-narrative-ruleset-story-core.md)
  (aventuras por menús de verbos), solo la parte de modelo de contenido,
  transición y proyección
- Relacionado: [ADR 0025](0025-narrative-ruleset-story-core.md),
  [ADR 0008](0008-optional-scene-compositor.md)

## Contexto

ADR 0025 reserva para S4 las aventuras narrativas por menús de verbos al
estilo de las aventuras españolas de los 80 o de *Snatcher*: una escena con
fondo, personajes con retrato, un menú de verbos (MIRAR, HABLAR, INVESTIGAR,
MOSTRAR, USAR, MOVERSE) aplicados a objetivos, respuestas que cambian al
repetir una acción («nada nuevo») y árboles de diálogo, y exige un ADR propio
para el modelo de menús y diálogo.

StoryPack v1 solo expresa opciones de librojuego (`choices`). Simular menús con
opciones obligaría a enumerar cada combinación verbo × objetivo como opción,
perdería los textos por defecto, no permitiría contar repeticiones sin
variables auxiliares por objetivo y mezclaría el diálogo con la navegación de
escenas.

## Decisión

### Formato y compatibilidad

- `STORY_PACK_FORMAT_VERSION` pasa a **2**. El cargador acepta `1..=2`.
- v2 es un superconjunto estricto de v1: todos los campos nuevos son opcionales
  y se omiten vacíos en forma canónica. Un documento v1 es un documento v2
  válido salvo por su `format_version`.
- El validador rechaza rasgos v2 (verbos, personajes, diálogos, escena, menú o
  la condición `uses`) en un documento que aún declara `format_version: 1`.
- La ruta de actualización es explícita y sin pérdidas:
  `StoryPack::upgrade()` y `tiamud-story fmt --upgrade` solo reescriben la
  versión. No hay conversión implícita al guardar.
- `StoryState` pasa a v2 con campos nuevos opcionales; un estado v1 sin ellos
  se carga y se normaliza a v2. Un estado v1 con campos v2 se rechaza.

### Modelo de contenido

- **Verbos** (`verbs`): id durable, etiqueta localizada, clase de objetivo
  (`none`, `hotspot`, `character`, `item`) y respuesta por defecto (1–8
  bloques).
- **Personajes** (`characters`): id, nombre localizado y mapa
  `expresión → id de activo de retrato` (≤ 16), con `default_expression`
  opcional (por defecto `neutral`).
- **Escena** (por nodo): `background` (id de activo), `cues` (≤ 4, el mismo
  `Cue` existente, emitidas al entrar), `hotspots` (≤ 32, con condición de
  visibilidad), `present` (≤ 8 personajes con expresión y condición) y `menu`.
- **Menú**: verbos ofrecidos en orden (≤ 16, con condición), entradas por
  `(verbo, objetivo)` (≤ 128 por nodo, ≤ 4096 en total) y textos por defecto de
  escena por verbo. Cada objetivo debe ser un hotspot del nodo, un personaje
  presente o un objeto declarado según la clase del verbo; las referencias
  colgantes son **errores** del validador.
- **Entrada de menú**: respuestas ordenadas (1–8), cada una con condición,
  `once`, bloques de texto (1–8), efectos del DSL existente, destino opcional y
  diálogo opcional; y texto `exhausted` («nada nuevo»).
- **Bloques de texto**: `{speaker?, expression?, text}`; el hablante debe
  estar declarado. Texto solo de presentación.
- **Diálogos**: árbol con `start`, nodos (≤ 128 por diálogo, ≤ 4096 en total),
  cada uno con líneas (1–8 bloques) y opciones (1–16) con condición, `once`,
  efectos y `next` = `goto` a otro nodo del mismo diálogo, `return` al inicio o
  `exit` (por defecto). No hay pila: a lo sumo un diálogo activo y `return`
  solo vuelve al inicio, de modo que la profundidad está acotada por
  construcción.

### Semántica

- Selección de respuesta: la primera respuesta cuya condición se cumple y que
  no sea un `once` ya gastado. Si ninguna aplica: `exhausted`, después el texto
  por defecto de la escena para el verbo y por último el del verbo. Un verbo
  sin entrada para el objetivo usa los mismos textos por defecto.
- **Repetición acotada**: cada uso de una entrada incrementa un contador por
  `(nodo, entrada)` en `StoryState`, saturado en `MAX_MENU_USES` (9999). La
  respuesta se elige con el contador *anterior* al uso. La condición
  `{"uses": {"node", "entry", "op", "value"}}` permite textos que cambian al
  repetir; el avance de la trama sigue gobernado por banderas, objetos y
  contadores, nunca por prosa.
- Un `goto`/`end` en efectos de respuesta u opción gana; mover de escena o
  terminar cierra el diálogo. Una respuesta que no mueve ni termina puede abrir
  un diálogo.
- Un diálogo solo bloquea el menú mientras su nodo ofrece al menos una opción
  disponible; si ninguna lo está, la escena recupera su menú.
- Cada entrada aceptada avanza el turno una unidad, como en v1.

### Entradas y tokens

- `StoryInput::MenuAction { verb, target? }` y
  `StoryInput::DialogueChoice { token }` se añaden a `Choose`.
- Tokens opacos por vista: verbo `t{turno}v{n}`, objetivo `t{turno}v{n}o{m}`
  (ligado al turno **y** al verbo), opción de diálogo `t{turno}d{n}`. Ordinales
  sobre elementos *disponibles*; nunca codifican ids authored ni prosa.
- Errores tipados: token de turno anterior → `StaleChoice`; objetivo ausente,
  sobrante o de otro verbo → `BadTarget`; clase de token equivocada →
  `MalformedToken`; menú u opción durante un diálogo → `InDialogue`; respuesta
  de diálogo sin diálogo → `NotInDialogue`.

### Proyección y límite de 16 KiB

- `StoryView` añade fondo, cues de la escena, retratos de presentes, bloques
  (respuesta de la última acción seguida de las líneas del diálogo, con nombre
  del hablante y retrato resuelto), menú con objetivos por verbo y opciones de
  diálogo. Ids authored, contadores, conjuntos gastados y RNG nunca se exponen.
- `view_pages(pack, state, locale)` proyecta una vista localizada en elementos
  autocontenidos y los empaqueta en páginas ordenadas cuyo JSON mide como
  máximo `MAX_VIEW_BYTES` = 12 KiB, dejando 4 KiB del marco vigente de 16 KiB
  para la envoltura futura del protocolo. Nunca se trunca contenido: un
  elemento que no cupiera solo (imposible en un pack validado, porque el mayor
  es un bloque de 4096 bytes escapado como mucho al doble) es un error tipado.

### Medios

Fondos, retratos y cues son ids de activo revisado, nunca URL, validados con la
misma gramática de id. Un retrato ausente para una expresión solo pierde
medios: la vista lleva `portrait: null` y `tiamud-story validate` lo avisa como
lint. La entrega de activos sigue el pipeline y el manifiesto revisados.

### Herramientas

`tiamud-story` añade lints (verbos o entradas nunca ofrecidos, respuestas
imposibles o ensombrecidas, «nada nuevo» inalcanzable, diálogos nunca
iniciados, nodos de diálogo inalcanzables, opciones imposibles y expresiones de
retrato no declaradas), `preview --dialogue`, menús en `play` (`N`, `N M`,
`cN` y palabras visibles como `mirar mesa`, comparadas sin mayúsculas ni
tildes con las etiquetas que ofrece la vista), pasos `menu`/`act`/`say` en
scripts v2, `fmt --upgrade` y el esquema JSON v2 publicado.

## Fuera de alcance (pendiente)

- Mensajes `story.menu`/`story.dialogue`, paginación correlacionada en el
  protocolo, paridad Telnet/GMCP y UI de navegador: dependen del actor y
  protocolo de S2, que requieren su propio ADR.
- Mostrar un objeto *a* un personaje concreto cuando hay varios presentes
  (verbo de dos objetivos): hoy `MOSTRAR` toma un objeto y la escena decide.
- Persistencia de estados v2 en slots (S3).

## Consecuencias

- Las aventuras por menús se expresan sin enumerar combinaciones como
  opciones y sin variables auxiliares por objetivo.
- El formato tiene dos versiones legibles; la v1 sigue cargando y la
  actualización es una reescritura de versión verificable.
- Los ids de verbos, entradas, respuestas, diálogos, nodos y opciones son
  durables: los contadores y conjuntos gastados del estado los referencian.
- El tamaño del estado sigue acotado por el pack: claves de contadores y
  conjuntos deben resolver contra el pack al cargar.

## Alternativas descartadas

- **Menús como opciones v1 generadas**: explosión combinatoria, sin textos por
  defecto ni repetición acotada, y tokens que filtrarían estructura.
- **Diálogo con pila de llamadas**: más expresivo, pero profundidad y estado no
  acotados por construcción; `return` al inicio cubre los árboles habituales.
- **Repetición derivada del texto mostrado**: viola la regla de no derivar
  estado de la prosa.
- **Truncar vistas grandes**: perdería contenido jugable; se pagina.

## Nota de estado (2026-10-15): fondos y retratos entregados como imagen

Añadida después de la decisión; no cambia el estado propuesto. Los ids de
`background` y de retrato siguen siendo ids de activo revisado, nunca URL. El
servidor los resuelve solo mediante el mapa revisado `story-media.json`
(formato 2, `images`: manifiesto de pipeline `assets/<sha256>/manifest.json`
con variantes AVIF/WebP/JPEG, licencia y procedencia, más texto alternativo
localizado) y envía `scene.background` y `portrait.image` como
`{manifest, alt}`; un id sin entrada no produce imagen y el texto conserva
todo lo jugable. El navegador muestra fondo y retratos solo con multimedia
permitida y cae al texto alternativo; Telnet imprime el alt del fondo. Ver el
[registro](../project/log/2026-10-15-story-presentation-locale-compat.md).

## Nota de estado (2026-10-16): retratos en las líneas de diálogo

Añadida después de la decisión; no la cambia. Los bloques con hablante
(respuestas de menú y líneas de diálogo) ya resolvían en `story-core` el id
de retrato de la expresión; el servidor lo convierte ahora en imagen con el
mismo mapa revisado `story-media.json` y envía `block.portrait {manifest,
alt}` (alt en el locale de la sesión) solo en una línea con `speaker`. Una
expresión sin entrada en el mapa no produce imagen. El navegador la muestra
junto a la línea solo con multimedia permitida y Telnet conserva
`Nombre: «texto»` sin repetir el alt. Ver el
[registro](../project/log/2026-10-16-story-language-picker-and-dialogue-portraits.md).
