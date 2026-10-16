# ADR 0033: instancias de historia compartidas, asientos y votación

- Estado: aceptada — implementación parcial. Implementada de forma incremental en este árbol como fase
  S7 de [ADR 0025](0025-narrative-ruleset-story-core.md) (parcial).
  2026-10-16: aceptado por el propietario.
- Fecha: 2026-10-15
- Alcance: `tiamud-server` (`story_sessions`, `story_sessions::shared`,
  adaptadores Web y Telnet), `tiamud-protocol` y el cliente navegador
- Relacionado: [ADR 0025](0025-narrative-ruleset-story-core.md),
  [ADR 0029](0029-story-session-instances-and-story-protocol.md),
  [ADR 0030](0030-story-save-slots-and-schema-v16.md),
  [especificación del protocolo](../protocol/tiamud-v1.md#shared-story-instances).
  Los números 0031 y 0032 están reservados (parser y misiones).

## Contexto

ADR 0025 deja la capa multijugador (instancias compartidas, votación,
espectadores) **encima** del mismo `step`: el actor de la instancia decide qué
entrada se aplica y el ruleset no cambia. Exige un ADR propio por sus
implicaciones de autorización, moderación y persistencia. S2 (ADR 0029) ya
tiene una tarea de escritor único por instancia con buzón acotado, vistas con
ids opacos por vista y recuperación; S3 (ADR 0030) conecta slots por cuenta.

## Decisión

### Modelo

- Una instancia es **solo** (la de S2) o **compartida**. Compartir no crea
  otra instancia: el dueño de una instancia solo la convierte
  (`compartir`/`share`, `story.share {}`) o la crea y comparte de una vez
  (`historia compartir <id>`/`story host <id>`). La instancia sigue siendo una
  tarea Tokio de escritor único; su tabla compartida
  (`story_sessions::shared::Table`) es lógica pura con reloj inyectado.
- Participantes: hasta `TIAMUD_STORY_MAX_SEATS` asientos (2..=8, por defecto
  8) y `TIAMUD_STORY_MAX_SPECTATORS` espectadores (0..=32, por defecto 32).
  Cada cuenta sigue en una sola instancia como máximo; una instancia
  compartida cuenta una vez contra `TIAMUD_STORY_MAX_INSTANCES`.
- El anfitrión es **siempre** el asiento más antiguo (`seats[0]`). Si sale o
  su asiento caduca, el rol migra al siguiente en orden de llegada. La
  instancia termina cuando no queda ningún asiento (los espectadores no la
  mantienen viva) y entonces se cierra para todos con `story.left {reason:
  ended}`.

### Unirse

- Código de unión opaco de 8 caracteres del alfabeto
  `23456789ABCDEFGHJKLMNPQRSTUVWXYZ` (40 bits de `OsRng`, único entre las
  instancias vivas, sin `0/O`/`1/I`). Entrada sin distinguir mayúsculas.
  `historia unirse <código>`/`story join <code>` (asiento) e `historia mirar
  <código>`/`story watch <code>` (espectador); `story.join {code, spectate}`.
  Solo el anfitrión ve el código en `story.lobby`.
- Intentos de unión limitados por cuenta (10/min, ráfaga 5). Un código
  desconocido, mal formado o el de una instancia que expulsó a la cuenta
  responden igual (`unknown_code`), sin oráculo.
- Invitación: el anfitrión envía `invitar <nombre>`/`invite <name>`
  (`story.invite {name}`); cada sesión conectada de ese personaje recibe
  `story.invitation {host, storyId, title, code}` (texto + GMCP
  `Tiamud.Story.Invitation` en Telnet). La respuesta es siempre la misma, esté
  o no conectado el personaje (sin oráculo de presencia). 10 invitaciones/min
  por anfitrión; buzón de 4 por sesión, se descarta al llenarse. **Límite
  honesto:** la invitación no consulta todavía ignore/block de la política
  social del mundo (vive en el actor del mundo); solo transporta nombre del
  anfitrión, título y código, sin texto libre.

### Modos de decisión

El anfitrión elige con `modo …`/`mode …` (`story.mode {mode, voteSeconds?}`);
cambiar de modo anula los votos abiertos. Por defecto: votación, ventana
`TIAMUD_STORY_VOTE_SECONDS` (30..=120, por defecto 60).

- **`vote` (mayoría):** cada asiento vota una acción de la vista; un número
  tecleado, `story.choose/menu/dialogue` o `story.vote {choiceId, viewRev}`
  cuentan como voto. La ventana empieza con el primer voto. Se cierra antes
  si **todos los asientos conectados** han votado; si no, al vencer la
  ventana se aplica la mayoría de los votos emitidos. Desempate
  determinista: el voto del anfitrión si está entre los empatados; si no, la
  acción de menor número. Cambiar el voto hasta el cierre está permitido;
  `votar 0`/`story.vote {viewRev}` sin `choiceId` lo retira (abstención). Si
  se retiran todos, la ventana se anula y nada se aplica por sí solo. Los
  asientos desconectados no cuentan para el cierre anticipado.
- **`host`:** solo la entrada del anfitrión se aplica; los demás reciben
  `host_only` al elegir y pueden dejar votos **orientativos** que nunca cierran.
- **`turns` (rotatorio):** decide quien tiene el turno, en orden de asiento;
  los demás reciben `not_your_turn`. Tras cada entrada aplicada el turno pasa
  al siguiente asiento conectado; si el titular no actúa dentro de la ventana
  o se desconecta, el turno pasa sin aplicar nada. Sin asientos conectados el
  turno no gira.

Los votos pertenecen a una revisión de vista (`viewRev`): un voto con otra
revisión es `stale_view`, un id nunca publicado es `unknown_choice`; ninguno
cambia el estado. Cada vista nueva anula los votos anteriores.

### Recuento público

`story.tally {viewRev, mode, choices: [{choiceId, votes}], voted, eligible,
remainingSeconds?, yourVote?}`: solo recuentos por acción (como mucho una
entrada por asiento), **nunca identidades de votantes**; `yourVote` es el voto
propio del destinatario. `remainingSeconds` acompaña a una ventana o turno en
marcha; el cliente cuenta hacia atrás localmente. Telnet muestra una línea
(`Votos (1/2): 1) 1 · cierra en 60 s · tu voto: 1`) y GMCP
`Tiamud.Story.Tally` por número de acción, sin ids opacos.

### Espectadores, deshacer y partidas

- Espectadores: solo lectura; cualquier entrada, voto o gestión responde
  `spectator`. Se cuentan, no se nombran, en `story.lobby`. Salen en cuanto
  se cierra su última sesión (sin periodo de gracia).
- Deshacer: **solo el anfitrión** (el resto ve `canUndo: false`). La variante
  «deshacer por unanimidad» queda fuera.
- Partidas: los slots de una instancia compartida son **de la cuenta del
  anfitrión**: solo él guarda, carga, lista o borra (`host_only` al resto);
  cargar reemplaza la partida para todos. El autoguardado se escribe en la
  cuenta del anfitrión **en el momento del cierre** (el último asiento al
  salir, el anfitrión vigente al desalojar por inactividad o al apagar). Un
  anfitrión que sale dejando a otros no recibe autoguardado: la partida sigue
  viva.

### Desconexión y reconexión

La presencia se deriva sin mensajes: cada sesión con un vínculo a la
instancia retiene un `Arc` de presencia por cuenta y el registro guarda solo
un `Weak`. Un reloj de la instancia (500 ms) marca los asientos sin sesión
como desconectados, los mantiene `TIAMUD_STORY_SEAT_GRACE_SECONDS` (30..=600,
por defecto 120) y después los retira. Reconectar dentro del plazo reanuda el
mismo asiento (`attach`); la recuperación reenvía `story.lobby`, la vista y
`story.tally`.

### Difusión y límites

- La instancia publica cada cambio en un `watch` de su `SharedFrame` (vista +
  tabla). Cada sesión sigue ese `watch` y traduce los cambios a
  `story.lobby`/`story.view`/`story.tally` para su participante; el valor más
  reciente gana, así que una sesión lenta coalesce en lugar de acumular.
  Ninguna cola nueva crece con el número de participantes.
- La respuesta a una entrada compartida siempre lleva al menos el recuento
  vigente (también un voto sin cambios), para que el cliente nunca quede
  pendiente.
- Cotas: 8 asientos, 32 espectadores, un voto por asiento, 64 cuentas
  expulsadas recordadas por instancia (la más antigua se olvida), nombres de
  ≤ 64 bytes, 16.384 buzones de invitación registrados, códigos con 8
  reintentos de colisión. Toda la aritmética es acotada o saturante.
- Expulsar (`expulsar <nombre>`/`kick <name>`, `story.kick {name}`): solo el
  anfitrión, nunca a sí mismo; la cuenta no puede volver a esa instancia. El
  expulsado recibe `story.left {reason: kicked}`.
- Métricas sin etiquetas de contenido: `tiamud_story_shared_instances`,
  `…_shared_started_total`, `…_shared_joins_total{result}`,
  `…_shared_votes_total`, `…_shared_decisions_total`,
  `…_shared_invitations_total`, `…_shared_kicks_total` y
  `…_shared_seat_lapses_total`.

### Protocolo (aditivo)

Servidor → cliente: `story.lobby`, `story.tally`, `story.invitation`; nuevos
motivos `kicked`/`ended` de `story.left` y códigos `host_only`, `spectator`,
`not_your_turn`, `unknown_code`, `seats_full`, `not_shared`,
`unknown_member`, `invalid_setting`. Cliente → servidor: `story.share {}`,
`story.join {code, spectate}`, `story.invite {name}`, `story.kick {name}`,
`story.mode {mode, voteSeconds?}` y `story.vote {choiceId?, viewRev}`. Los
tipos viven en `crates/protocol`, el TypeScript se regenera y el cliente
valida cada marco con claves exactas.

## Consecuencias

- `tiamud-core`, `story-core` y el actor del mundo no cambian. El riesgo se
  concentra en `story_sessions` y sus adaptadores.
- Las partidas compartidas siguen siendo volátiles salvo el autoguardado del
  anfitrión al cerrar y sus guardados manuales.
- El protocolo crece de forma aditiva; un cliente antiguo que no conozca los
  tipos nuevos no puede participar en una instancia compartida, pero las
  instancias solo no cambian.

## Fuera de alcance

- Turnos asíncronos persistidos (partidas que sobreviven sin nadie
  conectado), deshacer por unanimidad, votación pública con identidades,
  moderación de invitaciones con ignore/block del mundo, invitaciones a
  personajes desconectados, transferencia manual de anfitrión, indicador de
  «en una historia» para otros jugadores y una prueba de bot Telnet real.

## Alternativas descartadas

- **Un actor global de votaciones:** acopla instancias independientes y
  rompe el escritor único por instancia.
- **Difundir cada evento por un `mpsc` por participante:** obliga a decidir
  qué hacer con un participante lento; el `watch` del último marco es
  acotado por construcción.
- **Presencia por mensajes al desconectar:** un mensaje perdido con el buzón
  lleno dejaría un asiento «conectado» para siempre; el `Weak` no se pierde.
- **Votos con identidad pública:** innecesario para jugar y fuente de
  presión social; se publican recuentos.

## Nota de estado (2026-10-15): historias con parser compartidas

Añadida después de la decisión; no cambia el estado propuesto. En una
historia con parser (StoryPack v3) el texto libre **no se vota**: en los modos
`host` y `vote` solo el anfitrión teclea órdenes (los demás reciben
`host_only` y siguen votando las acciones numeradas de las vistas que las
tengan); en `turns` solo el titular del turno (`not_your_turn`), y el turno
pasa solo tras una línea que ejecutó órdenes del mundo (errores de análisis y
preguntas lo conservan); los espectadores nunca. `deshacer` y `reiniciar`
tecleados son solo del anfitrión. Ver la nota de
[ADR 0031](0031-parser-interactive-fiction-storypack-v3.md).
