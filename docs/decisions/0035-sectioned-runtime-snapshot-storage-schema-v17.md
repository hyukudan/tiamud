# ADR 0035: almacenamiento seccionado del snapshot de runtime (esquema v17)

- Estado: propuesto (implementado el 2026-10-15 en SQLite y PostgreSQL;
  pendiente de aceptación del propietario y de una ejecución PostgreSQL real)
- Fecha: 2026-10-15
- Alcance: `world_runtime` y la nueva tabla `world_runtime_sections` en ambos
  adaptadores, migración v16→v17, mover SQLite→PostgreSQL, `tiamud-admin`,
  frontera de commit del actor de mundo y `tiamud-core::durability`
- Relacionado: [ADR 0002](0002-runtime-world-and-durable-identity.md),
  [ADR 0006](0006-character-progression-and-postgresql-first.md),
  [ADR 0027](0027-durable-dirty-tracking-for-tick-commits.md),
  [ADR 0030](0030-story-save-slots-and-schema-v16.md)

## Contexto

[ADR 0027](0027-durable-dirty-tracking-for-tick-commits.md) eliminó los
commits de tick sin cambios durables y los personajes sin cambios, pero cada
commit que sí incluye el runtime (todo comando que registra eventos, los ticks
con cambio de clima o NPC y el latido de reloj) seguía escribiendo el
`WorldRuntimeSnapshot` completo como un único documento JSON en
`world_runtime.snapshot_json`. En la medición local de rutas calientes
(200 personajes, ~7 000 instancias) eso son ~1,2 MB por commit de comando,
aunque un `move` no cambie ninguna parte del runtime y un clima solo cambie
`adventure`.

Garantías que no pueden debilitarse: escritor único, un backend, commit
atómico de snapshot + personajes + eventos + watermark, rollback exacto del
checkpoint en memoria, progresión exactly-once, recuperación desde el último
snapshot confirmado que **falla cerrada** ante cualquier dato inválido, el
techo de 2 MiB del snapshot (1 MiB para los DTO v1–v5) y la paridad
SQLite/PostgreSQL con migración.

## Opciones evaluadas

### (a) Snapshot seccionado

Dividir el documento por miembros de primer nivel en secciones almacenadas
como filas independientes, reescribir en cada commit solo las secciones
sucias junto con una fila de cabecera y un manifiesto (digest, longitud y
revisión por sección), todo en la misma transacción. La recuperación ensambla
y verifica.

- Atomicidad: idéntica a la actual; el commit sigue siendo una transacción
  que contiene cabecera, manifiesto y secciones reescritas.
- Recuperación: lectura acotada de una fila de cabecera y como mucho diez
  filas de sección, sin reproducción; el resultado es el mismo DTO v9.
- Coste: una cabecera y un manifiesto pequeños (~1,5 KB) por commit, más el
  tamaño de las secciones cambiadas.
- Complejidad: una tabla, un manifiesto y una función de ensamblado;
  la decisión de qué está sucio ya existe (ADR 0027).

### (b) Registro append-only de deltas con compactación periódica

Escribir por commit un delta del runtime y reconstruir aplicando deltas sobre
el último snapshot compacto.

- Requiere definir y versionar un formato de delta para cada mapa del DTO,
  una reproducción determinista en arranque, una política y un disparador de
  compactación (otra transacción grande, otra fuente de fallo), cotas de
  longitud de la cadena de deltas y tiempo de arranque proporcional a ella.
- Duplica la noción de "ledger" que hoy es solo diagnóstico tras el
  snapshot (`domain_events`) y reabre la pregunta de qué es autoridad.
- Ahorra más bytes en mapas grandes con cambios pequeños, pero obliga a
  calcular diffs por entrada en el hilo del actor.

### Decisión entre opciones

Se elige **(a)**. Mantiene una única autoridad (el snapshot), no introduce
reproducción ni compactación, reutiliza el seguimiento sucio existente y su
fallo es local y verificable (digest por sección). (b) queda descartada
mientras (a) baste; su ganancia adicional solo importa para mapas grandes con
cambios mínimos, y para eso hay una salida más simple (ver "Trabajo futuro").

## Decisión

1. **Secciones.** Los miembros `itemInstances`, `itemProvenance`, `npcs`,
   `adventure`, `progressionFlags`, `trades`, `lootEncounters`, `groundLoot`,
   `partyLootCursors` y `areaResidency` del `WorldRuntimeSnapshot` se guardan
   cada uno en una fila de `world_runtime_sections` (`section`, `revision`,
   `section_json`, `section_sha256`, `updated_at`). La lista es cerrada y la
   valida un `CHECK`; cada sección admite 1..=2 MiB. El resto de miembros
   (versión, revisión, reloj, RNG y contadores) forman el documento
   **cabecera**.
2. **Cabecera y manifiesto.** `world_runtime` (singleton) pasa a guardar
   `snapshot_version`, `head_json` (≤ 2 MiB), `manifest_json` (2..=8 KiB),
   `last_event_sequence` y `updated_at`; desaparece `snapshot_json`. El
   manifiesto, de formato cerrado `{"format":1,"revision":N,"sections":{…}}`,
   lista cada sección almacenada con `bytes`, `revision` (la revisión del
   manifiesto que la escribió) y `sha256` en hexadecimal minúsculo. La
   revisión del manifiesto avanza en cada escritura de runtime con aritmética
   comprobada.
3. **El DTO no cambia.** El documento ensamblado es exactamente el mismo
   objeto JSON del `WorldRuntimeSnapshot` v9 (miembros en otro orden). La
   versión del DTO sigue siendo una propiedad del documento, no del
   almacenamiento; un snapshot legacy (v1–v8) migrado se almacena seccionado
   con su versión original y se normaliza al decodificar, como antes.
4. **Escritura.** El actor pasa a la nueva operación
   `CharacterStore::commit_action_sections` el conjunto de secciones que
   `World::runtime_dirty_sections` no puede demostrar limpias: sin línea base
   confirmada, todas; con ella, solo aquellas cuyos manejadores copy-on-write
   ya no son el puntero confirmado (o cuyo manifiesto de áreas difiere). Es
   conservador: desconocido es sucio y un falso positivo solo cuesta bytes.
   Dentro de la transacción, el store lee el manifiesto anterior y además
   reescribe cualquier sección que ese manifiesto no avale (sin fila, otra
   versión de snapshot, manifiesto ilegible o sección ausente); en esos casos
   reemplaza todas las filas. La cabecera y el manifiesto se reescriben
   siempre; la fila espejo `world_adventure` solo cuando se reescribe la
   sección `adventure`. Las rutas raras (staff, generaciones y ciclo de vida
   de áreas, arranque) siguen reescribiendo todo mediante `commit_action`.
5. **Techo.** Antes de escribir, la longitud exacta del documento ensamblado
   se calcula a partir de la cabecera y las longitudes del manifiesto y debe
   ser ≤ 2 MiB; si no, el commit falla sin abrir escritura. Los 16 KiB de
   frame y las demás cotas no cambian.
6. **Recuperación.** Arranque, `load_runtime_snapshot`, retiro/rehidratación
   de áreas, el mover y `tiamud-admin` leen la cabecera y como mucho once
   filas de sección y **fallan cerrados** (`InvalidRuntimeSnapshot`, nunca el
   ledger) si: hay secciones sin cabecera, el manifiesto no es válido, el
   conjunto de filas no es exactamente el del manifiesto, una fila difiere en
   revisión, longitud o SHA-256, la cabecera no es un objeto o ya define una
   sección, o una sección no es un único valor JSON (no puede inyectar
   miembros aunque su digest cuadre). La longitud ensamblada se acota con el
   techo de su versión antes de reservar memoria, y después se aplica el
   decodificador versionado de siempre.
7. **Línea base.** La regla de ADR 0027 no cambia: la línea base solo avanza
   tras un commit confirmado y cualquier error (incluido uno ambiguo), la
   restauración de un snapshot o el rollback de ciclo de vida la descartan,
   de modo que el siguiente commit reescribe todas las secciones. Por eso una
   sección "limpia" coincide siempre con lo que el backend confirmó.
8. **Migración v16→v17.** En la misma transacción de migración, ambos
   adaptadores leen la fila única legacy, recrean `world_runtime` con la forma
   nueva, crean `world_runtime_sections` y reescriben el documento como
   secciones verificadas con revisión 1, conservando versión, watermark y
   `updated_at`. Un documento legacy que no sea un objeto JSON se conserva
   literal como cabecera sin secciones, de modo que el arranque sigue
   fallando cerrado exactamente como antes en vez de "repararlo".
9. **Mover y backup.** `tiamud-migrate-postgres` exige SQLite v17, verifica que
   las filas ensamblan un snapshot válido y copia cabecera, manifiesto y
   secciones byte a byte. `tiamud-admin check` valida tanto la forma v17 como
   documentos anteriores a v17; backup/verify/restore copian la base completa
   y por tanto ambas tablas.

## Consecuencias

- Un commit de comando que no cambia ninguna sección escribe cabecera +
  manifiesto (~1,5 KB) en vez del snapshot completo. Uno que cambia
  instancias de objeto sigue escribiendo la sección `itemInstances`, que en
  la medición local es la mayor (~0,8 MB de ~1,2 MB): el ahorro allí es
  parcial (ver el registro fechado).
- La latencia medida con el store en memoria no cambia de forma apreciable:
  ese store excluye E/S SQL, que es donde el ahorro de bytes actúa. No hay
  medición con SQL real ni con PostgreSQL.
- El esquema pasa a v17 en ambos adaptadores. Un binario v16 rechaza una base
  v17 por versión futura: volver atrás exige restaurar la copia previa a la
  migración, y una herramienta externa que leyera `snapshot_json` deja de
  funcionar de forma visible en lugar de leer datos parciales.
- Una corrupción o pérdida de una sola sección hace fallar el arranque entero
  (no se degrada a otra autoridad); la recuperación es restaurar la copia.
- Ninguna ruta nueva puede escribir secciones sin pasar por
  `plan_runtime_write`; un escritor que quiera saltarse secciones debe tener
  una línea base confirmada equivalente a la de ADR 0027.
- No cambian WorldPack, AreaPack, el DTO del snapshot (v9), el protocolo ni
  la equivalencia navegador/Telnet.

## Trabajo futuro (no decidido)

- Particionar `itemInstances`/`itemProvenance` en cubos estables (por ejemplo
  por hash del id) cuyo estado sucio se derive comparando el rango del cubo con
  la línea base, si una medición con SQL real lo justifica.
- Ejecutar las pruebas PostgreSQL v17 escritas contra un servidor real.
