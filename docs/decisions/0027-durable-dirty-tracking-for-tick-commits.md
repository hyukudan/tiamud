# ADR 0027: seguimiento durable de cambios para los commits de tick

- Estado: aceptado (implementado el 2026-10-15)
  2026-10-16: aceptado por el propietario, incluida expresamente la consecuencia
  de que un crash no ordenado pueda rebobinar el reloj lógico y la revisión del
  mundo hasta 60 ticks (retroceso acotado).
- Fecha: 2026-10-15
- Alcance: frontera de commit del actor de mundo (`world_actor.rs`),
  `tiamud-core` (`durability.rs`, `cow.rs`) y la fachada `CharacterStore`
- Relacionado: [ADR 0001](0001-project-foundation.md),
  [ADR 0002](0002-runtime-world-and-durable-identity.md),
  [ADR 0012](0012-confirmed-trade-reservations.md),
  [ADR 0013](0013-repeatable-loot-rights-and-provenance.md)

## Contexto

Hasta ahora cada tick confirmaba incondicionalmente, en una transacción, el
`WorldRuntimeSnapshot` completo (hasta 2 MiB de JSON) y el agregado durable de
**cada** personaje conectado, aunque el tick no hubiese cambiado nada durable.
Cada comando hacía lo mismo con todos los personajes conectados. Una medición
local (ver el [registro](../project/log/2026-10-15-hot-path-dirty-tracking-and-indexes.md))
con 200 personajes y ~7 000 instancias escribía ~1,4 MB por tick.

Las garantías existentes que no pueden debilitarse son: escritor único, un
backend, commits atómicos con rollback exacto del checkpoint en memoria ante
fallo, progresión exactly-once, eventos persistidos cubiertos por el
`last_event_sequence` del snapshot y recuperación desde el último snapshot
confirmado.

## Decisión

1. `World` guarda una **línea base durable**: manejadores copy-on-write de
   todo lo que alimenta el snapshot de runtime (instancias, procedencia, NPC,
   aventura, banderas, trades, encuentros/derechos de botín, cursores),
   contadores escalares (RNG, secuencias, ids), residencia de áreas, el reloj
   lógico confirmado y el agregado exacto confirmado por personaje. No forma
   parte de ningún checkpoint: deshacer estado en memoria no cambia lo que
   guarda el store.
2. Toda mutación de esos mapas pasa por `DerefMut` del manejador
   copy-on-write; mientras la línea base conserva una referencia, cualquier
   acceso mutable crea una asignación nueva. La identidad de puntero con la
   línea base demuestra "sin cambios"; cualquier otra cosa cuenta como sucia
   (un acceso mutable sin cambio real es un falso positivo conservador). Los
   caminos de tick silenciosos se protegen con predicados de solo lectura para
   no tomar préstamos mutables innecesarios.
3. Un tick sin eventos persistibles, sin runtime sucio y sin personajes
   cambiados **no escribe nada**; sigue avanzando el reloj y refrescando a los
   observadores. Si solo cambian personajes, se escriben esos personajes en una
   transacción (`CharacterStore::commit_characters`, paridad SQLite/PostgreSQL)
   sin tocar snapshot, aventura global ni ledger. Si el runtime está sucio o
   hay eventos, se usa `commit_action` con los personajes cambiados y el
   snapshot completo, como antes. Los comandos usan el mismo filtro de
   personajes.
4. Reloj y revisión del mundo no ensucian por sí solos. El snapshot confirmado
   puede quedar por detrás del reloj en memoria como mucho
   `DURABLE_CLOCK_HEARTBEAT_TICKS` (60) ticks: al alcanzarlo se escribe el
   snapshot. El apagado ordenado confirma el estado pendiente antes de cerrar
   (mejor esfuerzo; un fallo se registra y la recuperación usa el último
   snapshot, como tras un crash).
5. La línea base solo avanza tras un commit que el backend confirmó con
   exactamente esos valores. Cualquier error de commit (incluido uno ambiguo),
   la restauración de un snapshot de runtime y el rollback de ciclo de vida de
   áreas la descartan, de modo que el siguiente commit reescribe todo.

## Consecuencias

- Un crash puede rebobinar el reloj lógico y la revisión del mundo hasta 60
  ticks (antes, como mucho el tick en curso). Plazos absolutos (trades, botín,
  cooldowns, reaparición) se desplazan como mucho esa cantidad; ningún estado
  durable distinto del reloj se pierde, porque cualquier otro cambio se
  confirma en el mismo tick. Los ids de vista son por sesión y se renuevan al
  reconectar.
- Exactly-once no cambia: un tick fallido se restaura exacto y se reintenta
  entero en el siguiente; los eventos solo se escriben junto al snapshot.
- Un commit que el store aplicó pero informó como fallido ya no se "cura" con
  la reescritura incondicional del tick siguiente; lo hace la invalidación de
  la línea base, que fuerza esa reescritura completa.
- Una ruta nueva que escriba personajes sin pasar por `commit_durable`,
  `commit_tick` o `World::mark_durable_commit` debe invalidar la línea base.
- No cambian el formato de snapshot, el esquema SQL, WorldPack ni el
  protocolo.
