# ADR 0030: slots de guardado de historias y esquema SQL v16

- Estado: aceptada — implementación parcial. La capa de persistencia (esquema v16 en SQLite y
  PostgreSQL, migración v15→v16, mover, API de store y pruebas) está
  implementada y, desde la nota de estado del final, conectada a sesiones de
  historia, protocolo y clientes. Ver
  [`COMPLETION_MATRIX.md`](../project/COMPLETION_MATRIX.md).
  2026-10-16: aceptado por el propietario.
- Fecha: 2026-10-15
- Alcance: fase S3 de [ADR 0025](0025-narrative-ruleset-story-core.md)
  (slots persistidos), solo la frontera de persistencia
- Relacionado: [ADR 0025](0025-narrative-ruleset-story-core.md),
  [ADR 0028](0028-story-menus-portraits-and-dialogue-trees.md); el ADR 0029
  queda reservado para el actor de instancias y el protocolo de S2

## Contexto

ADR 0025 define `StoryState` como documento JSON acotado que
`StoryState::from_json_slice` revalida contra su pack antes de usarlo, y exige
para S3 un esquema SQL v16 en ambos adaptadores, con migración y paridad, y su
propio ADR. También fija que un slot debe ligar el estado a la identidad exacta
del pack y que cargarlo contra otra revisión se rechaza o se migra de forma
explícita, nunca implícitamente.

El baseline conserva un único backend autoritativo (SQLite o PostgreSQL), datos
consultables relacionales con documentos JSON acotados como `TEXT` opaco,
SHA-256 de bytes exactos para documentos durables (checkpoints de área v15) y
un mover sin conexión que valida cada fila antes de copiarla.

## Decisión

### Tabla `story_saves` (esquema v16)

Una fila por `(account_id, story_id, slot)`:

| Columna | Contrato |
| --- | --- |
| `account_id` | FK a `accounts(id)` con `ON DELETE CASCADE` |
| `story_id` | gramática de id StoryPack: 1..=64 bytes, `[a-z0-9][a-z0-9._-]*` |
| `slot` | entero `0..=9`; `0` es el autoguardado y `1..=9` son manuales |
| `pack_id` | misma gramática; debe coincidir con `pack_id` del documento |
| `pack_sha256` | SHA-256 hexadecimal en minúsculas de los bytes exactos del StoryPack usado |
| `state_format_version` | `1..=255`; debe coincidir con `format_version` del documento |
| `state_json` | `TEXT` opaco de 2..=65.536 bytes |
| `state_sha256` | SHA-256 hexadecimal en minúsculas de los bytes exactos de `state_json` |
| `created_at`, `updated_at` | segundos Unix, `updated_at >= created_at` |

Clave primaria `(account_id, story_id, slot)`; sirve también para listar y
sumar por cuenta. No hay `jsonb`, índices adicionales ni tabla espejo. El
documento es opaco para SQL: el servidor nunca deriva estado, autorización ni
pistas de su contenido.

`story_id` y `pack_id` se guardan por separado: hoy coinciden para historias
de un solo pack, pero el id de catálogo de la historia es la clave del slot y
el id del pack es la identidad del estado. Renombrar un pack publicado exige
migración explícita, nunca reinterpretación.

### Límites

- Estado por slot: 64 KiB (`MAX_STORY_SAVE_STATE_BYTES`). Es más estricto que
  `story_core::MAX_STATE_BYTES` (256 KiB, cota de análisis): un estado mayor se
  rechaza al guardar con el error tipado `StorySaveTooLarge`; nunca se trunca.
  Los estados de las fixtures miden pocos KiB.
- Por historia: 10 slots, impuesto por el `CHECK` de `slot`.
- Por cuenta: como máximo 16 historias distintas con algún slot y 1 MiB de
  estado agregado. Sobrescribir un slot solo cuenta la diferencia de tamaño.
  Superar un límite devuelve `StorySaveCapacity` sin escribir.
- Conjunto de revisiones compatibles declaradas en una carga: como máximo 16.
- `list_story_slots` lee solo metadatos y está acotado a 10 filas; leer más es
  corrupción.

### Integridad

- El store calcula el SHA-256 de los bytes exactos al guardar; el llamador no
  lo aporta.
- Guardar valida identidad, slot, digest del pack, versión, límites de bytes,
  que el documento sea un objeto JSON y que su `format_version`/`pack_id`
  coincidan con los metadatos. La validación narrativa completa no ocurre en
  persistencia: `persistence` no depende de `story-core`.
- Cargar revalida la fila completa (límites, metadatos, SHA-256 exacto,
  timestamps y tipos SQL). Cualquier discrepancia devuelve
  `StorySaveCorrupt`; nunca hay pánico. Un slot corrupto puede repararse
  sobrescribiéndolo con un guardado válido.
- El llamador (el gestor de instancias de S2) debe pasar después los bytes por
  `StoryState::from_json_slice(bytes, pack)`, que vuelve a comprobar el estado
  contra el pack antes de usarlo.
- El guardado es un upsert atómico: SQLite usa una transacción `IMMEDIATE`;
  PostgreSQL bloquea la fila de la cuenta con `SELECT .. FOR UPDATE` antes de
  comprobar límites y escribir. `created_at` se conserva al sobrescribir y
  `updated_at` nunca retrocede por debajo de `created_at`.

La política está definida una sola vez: el contrato de fila puro
(`tiamud_persistence_sqlite::story_saves`) lo comparten los dos stores, a
través de `persistence/common.rs`, y el mover.

### Compatibilidad de revisión del pack

Cargar exige una `StoryPackCompatibility { pack_id, pack_sha256,
compatible_pack_sha256 }`. La carga falla cerrada con `StorySavePackMismatch`
salvo que el slot tenga el mismo `pack_id` y su `pack_sha256` sea el actual o
uno de los declarados compatibles. No hay migración implícita entre
revisiones.

StoryPack todavía no tiene un campo que declare revisiones compatibles. Hasta
que un formato posterior lo añada (con su validación y su propia decisión),
el llamador pasa un conjunto vacío y solo la revisión exacta carga. Declarar
una revisión compatible solo afirma que el estado guardado sigue validando con
el pack nuevo; `from_json_slice` lo comprueba igualmente.

### Retención y borrado

- Los slots no caducan: son datos del jugador, acotados por cuenta.
- `delete_story_slot` borra un slot e informa si existía.
- Borrar una cuenta borra todos sus slots por la FK `ON DELETE CASCADE` en
  ambos adaptadores. El baseline no expone todavía borrado de cuentas; la
  cascada es la garantía de esquema para cuando exista, y las pruebas la
  ejercen con SQL directo.
- Ningún slot sobrevive a la restauración de una copia anterior: la copia de
  seguridad del backend incluye la tabla y la restauración la reemplaza con
  el resto de autoridades.

### Exportación, copia y traslado

- `tiamud-admin backup`/`verify-backup`/`restore` copian la base SQLite entera,
  incluida `story_saves`; no hay exportación por cuenta.
- `tiamud-migrate-postgres` exige SQLite v16, valida en streaming cada fila
  con el contrato compartido **antes** de abrir la transacción PostgreSQL y la
  vuelve a validar al copiarla. Una fila corrupta, manipulada o con metadatos
  incoherentes aborta el traslado entero (`InvalidStorySave`). Los bytes se
  copian exactos.
- Los backups PostgreSQL siguen siendo responsabilidad del operador y deben
  incluir la tabla.

### Migración

- SQLite: `migrate` crea la tabla con `CREATE TABLE IF NOT EXISTS` y fija
  `user_version = 16` en la transacción existente; v15→v16 solo añade la
  tabla vacía.
- PostgreSQL: instalación nueva v16 o `UPGRADE_V15_TO_V16_SQL` (v6..=v15 llegan
  a v16 encadenados); la verificación exige la tabla. Una base v17 se rechaza
  sin modificarla.
- No cambia `WorldRuntimeSnapshot`, WorldPack, AreaPack ni ninguna otra tabla.

### API de store

Métodos nuevos de `CharacterStore` (implementados en SQLite, PostgreSQL y el
envoltorio medido; por defecto `StorySaveUnsupported`):

- `save_story_slot(StorySaveWrite) -> StorySaveSummary`
- `load_story_slot(account, story_id, StorySlot, StoryPackCompatibility)
  -> Option<StorySaveRecord>`
- `list_story_slots(account, story_id) -> Vec<StorySaveSummary>`
- `delete_story_slot(account, story_id, StorySlot) -> bool`

Errores tipados: `InvalidStorySave`, `StorySaveTooLarge`, `StorySaveCapacity`,
`StorySaveCorrupt`, `StorySavePackMismatch`, `StorySaveUnsupported`, además de
`AccountNotFound`.

## Fuera de alcance (pendiente)

- Conectar los slots al gestor de instancias y al protocolo `story.save` /
  `story.restore` (S2, ADR 0029), incluida la autorización por cuenta, la
  política de autoguardado y la paridad Web/Telnet/GMCP.
- Declarar revisiones compatibles en StoryPack.
- Exportación de partidas por cuenta y cuotas globales del servidor.
- Aceptación real PostgreSQL de v16: las pruebas PostgreSQL existen pero
  requieren `TIAMUD_TEST_POSTGRES_URL` y no se han ejecutado.

## Consecuencias

- El esquema SQL pasa a v16 en ambos adaptadores; un binario v15 rechaza una
  base v16 por versión futura, así que el rollback de binario exige restaurar
  la copia previa a la migración.
- Las partidas guardadas quedan ligadas a la revisión exacta del pack: publicar
  una revisión nueva deja inaccesibles los slots anteriores hasta que exista
  una declaración de compatibilidad explícita.
- La persistencia sigue sin conocer el dominio narrativo; la revalidación
  contra el pack es obligatoria en el llamador.

## Alternativas descartadas

- **Guardar el estado dentro de `character_adventure` o del snapshot del
  mundo**: mezcla una sesión instanciada con la autoridad global del MUD.
- **`jsonb` o columnas por campo del estado**: el estado es opaco para SQL y
  el proyecto evita `jsonb` sin necesidad medida.
- **Migrar automáticamente estados a la revisión nueva del pack**: viola la
  regla de ADR 0025 de no migrar implícitamente.
- **Confiar en el digest aportado por el llamador**: el store calcula el
  digest de los bytes que realmente escribe.

## Nota de estado (2026-10-15): conexión con las sesiones S2

Añadida después de la decisión; no cambia el estado propuesto ni el contrato
de la tabla. El primer punto de «Fuera de alcance» está implementado (ver
[ADR 0029, nota de estado](0029-story-session-instances-and-story-protocol.md#nota-de-estado-2026-10-15-slots-de-guardado-conectados)):

- El gestor de instancias es el único llamador. La autorización por cuenta es
  la de la sesión autenticada: cada operación usa la cuenta de la sesión y la
  historia en curso; no hay forma de nombrar otra cuenta ni otra historia.
- `pack_sha256` es el SHA-256 de los bytes exactos del `*.story.json` leído al
  arrancar; el conjunto compatible se pasa vacío. Cambiar el archivo del pack
  deja los slots anteriores listados como `compatible: false` y su carga se
  rechaza con `slot_incompatible`.
- Los errores tipados del store se traducen a códigos públicos:
  `StorySaveCapacity` y `StorySaveTooLarge` → `slot_capacity`,
  `StorySaveCorrupt` (y un fallo de `from_json_slice`) → `slot_corrupt`,
  `StorySavePackMismatch` → `slot_incompatible`, `StorySaveUnsupported` →
  `slots_unavailable`, y cualquier otro error o un plazo agotado → `internal`
  con un mensaje genérico; los detalles quedan en el registro del servidor.
- El historial de deshacer no se persiste.

Sigue pendiente: declarar revisiones compatibles en StoryPack, exportación
por cuenta, cuotas globales y la ejecución real de las pruebas PostgreSQL v16.

## Nota de estado (2026-10-15): revisiones compatibles declaradas

Añadida después de la decisión; no cambia el estado propuesto ni el esquema
v16. StoryPack v3 admite `compatible_revisions`: hasta 16 SHA-256 en
hexadecimal minúscula, únicos, de revisiones anteriores del pack. La sesión
los pasa como `compatible_pack_sha256` al cargar (antes, conjunto vacío) y los
cuenta como compatibles en `story.slots`. El estado cargado de una revisión
declarada se revalida contra el pack actual con
`StoryState::from_json_slice`; si ya no encaja, la carga falla con
`slot_incompatible` sin cambiar la partida viva (un fallo de la revisión
exacta sigue siendo `slot_corrupt`). No hay migración. Ver el
[registro](../project/log/2026-10-15-story-presentation-locale-compat.md).
