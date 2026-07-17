# Tiamud browser protocol `tiamud.v1`

Status: experimental. Breaking changes remain possible before the first public
compatibility milestone. This document describes the implemented contract, not
the broader historical proposal.

## HTTP account and roster API

The official browser uses same-origin JSON before opening a WebSocket:

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/auth/register` | Create account and 30-day session (`201`). |
| `POST` | `/api/auth/login` | Verify credentials and create session. |
| `GET` | `/api/auth/session` | Resolve bearer session. |
| `POST` | `/api/auth/logout` | Revoke bearer session (`204`). |
| `GET` | `/api/character-options` | List the bounded public lineage/background/base-class catalog. |
| `GET` | `/api/characters` | List account-owned characters. |
| `POST` | `/api/characters` | Create an account-owned character (`201`). |

Registration/login body:

```json
{"name":"Aster","password":"at least eight bytes"}
```

Session response (`expiresAt` is Unix seconds):

```json
{
  "account": {"id": 1, "name": "Aster", "role": "player"},
  "token": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "expiresAt": 1786723200
}
```

`GET /api/character-options` is public and returns the defaults plus the
definitions players may choose during character creation. The catalog exposes
identity, label and description, not creation grants or the derived initial
sheet:

```json
{
  "defaultLineageId": "tiamud:mortal",
  "defaultBackgroundId": "tiamud:caravan-scout",
  "defaultBaseClassId": "tiamud:wanderer",
  "lineages": [
    {
      "id": "tiamud:mortal",
      "label": "Mortal",
      "description": "Adaptable people whose identity is shaped by choices."
    }
  ],
  "backgrounds": [
    {
      "id": "tiamud:caravan-scout",
      "label": "Caravan Scout",
      "description": "You learned to read roads, weather and distant movement."
    }
  ],
  "baseClasses": [
    {
      "id": "tiamud:wanderer",
      "label": "Wanderer",
      "description": "A resilient traveller with balanced training."
    }
  ]
}
```

`defaultBackgroundId` is `null` when a world has no recommended background.
There are at most 16 lineages, 16 backgrounds and 32 base classes. Each option
is a lowercase safe content ID of at most 128 bytes, a non-empty label of at
most 128 bytes and a non-empty description of at most 1,024 bytes.

Authenticated endpoints require `Authorization: Bearer <token>`; the options
catalog does not. `POST /api/characters` accepts the clean v3 body:

```json
{
  "name": "Orin",
  "lineageId": "tiamud:stonekin",
  "backgroundId": null,
  "baseClassId": "tiamud:vanguard"
}
```

Omitting `lineageId` or `baseClassId` selects its catalog default. Background
selection is deliberately tri-state: omit `backgroundId` to use the world
default, send `null` to choose no background, or send a string to select that
background. Sending JSON `null` for lineage/base class has the same defaulting
effect as omission; it never means “no lineage” or “no base class”. Unknown
option IDs, the retired `archetypeId` field and all other unknown request fields
are rejected. Account/character names are
1-64 safe bytes and passwords 8-1024 bytes. Passwords are Argon2id hashes; the
token contains 256 random bits, only its SHA-256 digest is stored, it expires
after 30 days and logout revokes it. Public use requires HTTPS/WSS.

Argon2 work is isolated from the async request path. The default service runs
at most two password jobs concurrently, admits no more than 256 outstanding
jobs without creating an unbounded waiter queue, and applies a five-second work
timeout. Saturation and timeout fail as temporary service unavailability; they
do not fall back to weaker password handling.

HTTP numeric account/character IDs are durable identity references used for
ownership and selection. They are not world entity IDs and never authorize an
action by themselves. Roster and creation responses retain the legacy
`CharacterSnapshot` shape; the authoritative lineage, background, classes and
derived sheet arrive in `player.snapshot` after entry. Consequently, the
current creation UI does not preview initial attributes, resources or
abilities; adding that preview requires an additive catalog contract.

## WebSocket transport and envelope

Connect to same-origin `/ws`, offer exactly subprotocol `tiamud.v1`, and send
UTF-8 JSON text frames. Binary frames are protocol errors. The maximum frame,
message and HTTP JSON body is 16 KiB.

Every sequenced server message uses:

```json
{"v":1,"type":"scene.snapshot","seq":2,"data":{}}
```

- `v` is exactly `1`.
- `type` selects a strict payload shape.
- `seq` increases per WebSocket session. Retained frames are delivered in
  sequence order; coalescing may create a gap.
- Unknown client envelope/payload fields are rejected except for additive keys
  inside `client.hello.capabilities`, which an older server may ignore. The
  official client ignores unknown fields sent by the server inside a
  recognized v1 envelope, but still rejects a malformed known field.
  Server/client parsers bound arrays, strings, URLs and numeric values.
- JSON object/array nesting may not exceed 16 levels.

`server.heartbeat` is deliberately unsequenced so best-effort liveness traffic
cannot displace an accepted command result.

### Additive v1 rollout policy

`tiamud.v1` aún no ha alcanzado su primer hito de compatibilidad ni se ha usado
en producción pública. Durante este intervalo de prelanzamiento, las fases 4A y
4B sustituyen deliberadamente el contrato fuente de forma incompatible: el
servidor y el cliente oficial deben proceder del mismo checkout. No existe una
capacidad negociada que haga compatible un cliente anterior con los campos o
enums de 4A y 4B, y no debe afirmarse esa compatibilidad.

La política aditiva siguiente será vinculante a partir del primer hito explícito
de compatibilidad. Desde ese hito, los esquemas cliente-servidor permanecerán
estrictos: un cliente no podrá sondear acciones nuevas mediante campos
desconocidos. Los nombres de capacidad serán el punto de extensión explícito,
pero anunciar una capacidad desconocida no tendrá efecto hasta que el servidor
la implemente. La evolución servidor-cliente será aditiva solo dentro de una
forma reconocida. El cliente oficial descartará campos desconocidos dentro de
un envelope/payload reconocido mientras valida y conserva todos los conocidos;
esto permitirá añadir presentación o información opcional sin cambiar el
significado del mensaje v1.

A new required field, enum value, independently sequenced message type or
behavior that an older client must understand is not a safe field addition. It
requires an explicitly negotiated client capability or a new protocol version.
Servers must continue emitting the established semantic fields to clients that
do not advertise the capability. An unknown server message type is not applied
by the official client and is reported as protocol-invalid input, so it must
never be the sole carrier of authoritative state or create an unexplained
sequence dependency.

## Authenticated handshake

The first message, within ten seconds, must be:

```json
{
  "v": 1,
  "type": "client.hello",
  "data": {
    "client": "web",
    "capabilities": {
      "images": ["avif", "webp", "jpeg"],
      "reducedMotion": true,
      "audio": false
    },
    "auth": {
      "token": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "characterId": 7
    }
  }
}
```

`images` is either the ordered list above or empty. Audio is opt-in and false by
default. The server resolves/rechecks the bearer session, ownership, ban state
and single-live-character lease before entering. Success begins with sequenced
`server.welcome`, `scene.snapshot` and `player.snapshot`; failure sends a safe
notice and closes the connection.

Disconnect releases the live lease. Reconnect repeats the handshake and gets a
fresh observer salt and authoritative snapshots. It does not replay commands or
general event history.

A rejected pre-welcome handshake is a sequenced error `server.notice` with one
of these stable codes:

- `access.protocol_rejected`: the first frame was not a valid authenticated
  `client.hello`;
- `access.session_expired` or `access.session_invalid`: discard the saved
  credential and require sign-in;
- `access.character_invalid`: the selected character is unavailable;
- `access.character_incompatible`: the character needs an operator-managed
  WorldPack content migration;
- `access.character_connected`: another live connection owns its lease;
- `access.session_limit`: an account/IP connection cap rejected entry;
- `access.temporarily_unavailable`: a bounded dependency could not admit or
  finish the request.

The official browser stops automatic reconnect after any of these entry
rejections. It clears saved credentials only for expired or invalid sessions;
the other codes preserve an otherwise reusable login and return the player to
character entry with a safe explanation.

## Client messages

### Text command

```json
{
  "v": 1,
  "type": "command.submit",
  "data": {"requestId":"cmd-27","command":"north"}
}
```

El comando ocupa como máximo 1024 bytes y no contiene caracteres de control.
Llega al mismo analizador de `PlayerAction` que Telnet.

La entrega textual de objetos usa exactamente:

```text
give [cantidad] <objeto> to <personaje>
```

La cantidad predeterminada es uno y debe estar en `1..=256`. Objeto y receptor
pueden ser selectores opacos de la sesión; el receptor también puede ser un
nombre exacto no ambiguo. El servidor exige que sea otro personaje conectado y
en la misma sala y vuelve a validar política, subárbol, capacidad e
ignore/block.

### Acción estructurada

El servidor analiza el JSON WebSocket con detección recursiva de claves
duplicadas. Dos apariciones de una clave en cualquier objeto, también anidado,
rechazan el frame antes de construir el payload tipado; ninguna acción depende
de la semántica last-key-wins del parser.

```json
{
  "v": 1,
  "type": "action.perform",
  "data": {"requestId":"cmd-28","action":"item.take","target":"o51a4c0311f262a90"}
}
```

Nombres de acción implementados:

```text
look, inventory, move
item.take, item.drop, item.contents, item.put, item.get, item.give,
item.examine, item.use, item.equip, item.unequip
loot.status, loot.pass
reward.open, reward.odds
combat.attack, combat.ability
progression.promote, progression.talent
party.create, party.invite, party.accept, party.revoke, party.kick,
party.transfer, party.disband, party.leave, party.status, party.say
quest.list, quest.accept, quest.status, quest.complete
economy.balance, economy.shop, economy.buy, economy.sell
weather, time
social.say, social.tell, social.reply, social.channel.list,
social.channel.join, social.channel.leave, social.channel.say, social.telepathy,
social.emote.list, social.emote,
mail.list, mail.read, mail.delete, mail.send, mail.report,
board.catalog, board.list, board.read, board.post, board.delete, board.report,
social.who, social.help
social.ignore, social.unignore, social.block, social.unblock, social.report
moderation.mute, moderation.unmute, moderation.ban, moderation.unban,
moderation.role
staff.status, staff.reports, staff.report.resolve,
staff.inspect, staff.where, staff.goto, staff.transfer, staff.restore,
staff.announce, staff.audit, staff.profile.set, staff.capability.set
trade.request, trade.accept, trade.decline, trade.item.set, trade.item.remove,
trade.coins.set, trade.contents, trade.confirm, trade.status, trade.cancel
```

Según la acción, el payload estricto también puede contener `message`,
`channel` (`general`, `newbie` or `staff`), `durationSeconds`, `reason` or
`role` (`player`, `moderator`, `admin`). `party.revoke`, `party.kick` y
`party.transfer` usan `target`; `party.say` y `social.reply` usan `message`;
alta/baja de canal usa `channel`, y el envío requiere `channel` y `message`.
`social.telepathy` exige exactamente `abilityId`, `target` y `message`: el ID
authored ocupa como máximo 64 bytes, el personaje 64 y el pensamiento UTF-8
visible 512. No acepta campos de combate, canal ni persistencia, aunque un
campo adicional conocido llegue como `null`. Una habilidad cuyo propósito es
`telepathy` se rechaza en `combat.ability`; no existe una segunda ruta genérica.
`social.emote.list` no admite campos de acción. `social.emote` exige `emote`,
un comando authored ASCII de 1–32 bytes con patrón `[a-z][a-z0-9-]*`, y admite únicamente `target`, un
nombre UTF-8 de hasta 64 bytes. No acepta `message`: la frase procede de las
plantillas revisadas del WorldPack y el objetivo, cuando el gesto lo admite o
exige, debe ser otro personaje conectado y no ambiguo de la misma sala.
Los campos inesperados y objetivos ausentes/prohibidos se rechazan. Se aplican
las mismas reglas de autorización y percepción; una acción estructurada no es
un atajo confiable. Accept/revoke/kick/transfer resuelven `target` como nombre
de un personaje conectado, no como administración de membresía offline.

El correo usa formas cerradas separadas:

| Acción | Campos exactos adicionales |
| --- | --- |
| `mail.list` | Ninguno o `page` |
| `mail.read` | `mailId` |
| `mail.delete` | `mailId` |
| `mail.send` | `target`, `subject`, `body` |
| `mail.report` | `mailId`, `reason` |

`mailId` es `m` seguido de 32 dígitos hexadecimales minúsculos; `page` está en
1..7. Pedir una página posterior a `totalPages` es válido y devuelve una lista
vacía, lo que evita que el total cambiante convierta la navegación en error.
Objetivo/asunto/cuerpo/motivo ocupan como máximo 64/128/2.048/2.048
bytes UTF-8 visibles, sin controles ni marcas bidi de formato. El nombre
objetivo coincide exactamente, incluidas mayúsculas/minúsculas. La barra `|`
se reserva como separador en objetivo/asunto, pero cuerpo y motivo pueden
conservarla. Las acciones
textuales equivalentes son `mail [list [página]]`, `mail read/delete <ref>`,
`mail send <personaje> | <asunto> | <cuerpo>` y
`mail report <ref> | <motivo>`. Enviar comparte mute y bucket social; las
demás operaciones no. Un `requestId` repetido solo reproduce la misma firma
exacta y la respuesta estructurada privada pertenece a esa sesión.

Los tablones usan formas cerradas separadas:

| Acción | Campos exactos adicionales |
| --- | --- |
| `board.catalog` | Ninguno |
| `board.list` | `board`, `page` |
| `board.read` | `postId` |
| `board.post` | `board`, `subject`, `body` |
| `board.delete` | `postId` |
| `board.report` | `postId`, `reason` |

`board` es `general` o `newbie`; `postId` es `b` seguido de 32 dígitos
hexadecimales minúsculos y `page` está en `1..=100`. Asunto, cuerpo y motivo
ocupan como máximo 128/4.096/2.048 bytes UTF-8 visibles. Controles C0/C1, DEL y
marcas bidi se rechazan. `|` se prohíbe en asunto porque separa el comando de
texto, pero cuerpo y motivo pueden conservarlo. Las formas textuales exactas
son `board`, `board list <general|newbie> [página]`, `board read/delete <ref>`,
`board post <tablón> | <asunto> | <cuerpo>` y
`board report <ref> | <motivo>`.

Catálogo, lista y lectura no mutan ni se publican a otras sesiones. `post`
comparte mute, ban y bucket social antes de la cuota durable propia; las demás
acciones no. Un `requestId` repetido exige la misma firma, incluido todo el
cuerpo, y solo reproduce la respuesta de aquella sesión.

El navegador registra de forma acotada `board.post`, `board.delete` y
`board.report` pendientes y no aplica su efecto al mero envío. Espera el
`action.result` cuyo `requestId` coincida: solo `ok: true` invalida la lista,
retira el detalle borrado o marca la entrada como denunciada. Un fallo conserva
la vista y los borradores; los formularios de publicación/denuncia bloquean un
segundo envío mientras su solicitud siga pendiente.

Salvo `staff.status`, que solo muestra la asignación propia, cada acción
`staff.*` exige una capacidad concreta y `reason`; las acciones dirigidas
vuelven a comprobar además la jerarquía en el actor. Un rol legado nunca basta.
La resolución usa `reportId` positivo y `resolution`
(`dismissed`, `action_taken` o `no_action`). `staff.profile.set` exige
`target`, `profile` y `loreTitle`, donde estos dos últimos aceptan `null` para
retirar la asignación o el título; los perfiles no nulos son `helper`,
`builder`, `moderator`, `administrator` e `implementor`.
`staff.capability.set` exige `target`, `enabled` y una capacidad de este
vocabulario cerrado: `reports.view`, `reports.resolve`, `moderation.mute`,
`moderation.ban`, `staff.assign`, `staff.inspect`, `staff.where`, `staff.goto`,
`staff.transfer`, `staff.restore`, `staff.announce` o `staff.audit`. El objetivo
está limitado a 64 bytes, el motivo a 1.024 y el título narrativo a 128. Los
campos que no pertenecen a la acción se rechazan.

Las acciones de objeto usan combinaciones cerradas de `target`, `container`,
`recipient`, `quantity` y `page`. `quantity` es opcional, vale uno por defecto y
debe estar en `1..=256`. `item.put`/`item.get` exigen ambos selectores opacos;
`item.contents` exige `container`, admite una página positiva y prohíbe
cantidad. `item.give` exige `target` y `recipient`, admite cantidad y prohíbe
contenedor/página. `recipient` ocupa como máximo 64 bytes, no contiene controles
y se rechaza para cualquier otra acción. Todos los campos admitidos forman
parte de la firma de deduplicación:

```json
{"v":1,"type":"action.perform","data":{
  "requestId":"put-1",
  "action":"item.put",
  "target":"o51a4c0311f262a90",
  "container":"o848103fdc4e0992b",
  "quantity":12
}}
```

```json
{"v":1,"type":"action.perform","data":{
  "requestId":"give-1",
  "action":"item.give",
  "target":"o51a4c0311f262a90",
  "recipient":"o7f72cd623eb86c31",
  "quantity":2
}}
```

Para `item.give`, `target`, `recipient` y `quantity` participan en la firma.
Repetir `requestId` con la misma firma devuelve el resultado cacheado sin
reejecutar; cambiar cualquiera de esos valores con el mismo ID es un error de
protocolo. Los selectores opacos pertenecen a la sesión que los creó; la caché
continúa indexada por personaje/`requestId` y solo puede reproducir la misma
firma exacta según las reglas generales descritas más abajo.

Las acciones de botín tienen formas cerradas:

| Acción | Campos exactos adicionales |
| --- | --- |
| `loot.status` | Ninguno |
| `loot.pass` | `target` |

`target` es el selector opaco o nombre exacto visible de una raíz reservada y
ocupa como máximo 256 bytes sin controles. Forma parte de la firma de
deduplicación; cambiarlo bajo el mismo `requestId` se rechaza. `loot.pass` solo
puede ceder un derecho propio a la siguiente identidad de la cohorte congelada,
no renueva el plazo y no revela esa identidad al cliente.

Las recompensas selladas usan formas separadas y cerradas:

| Acción | Campos exactos adicionales |
| --- | --- |
| `reward.open` | `target` |
| `reward.odds` | `target`, `page` |

`target` identifica una recompensa sellada que el personaje lleva y `page` es
positivo. Ambos forman parte de la firma de deduplicación. Los comandos de
texto equivalentes, compartidos por Web y Telnet, son `open <selector>` y
`reward odds <selector> [página]`; al omitir la página textual se usa la
primera. Abrir vuelve a comprobar autoridad, espacio y peso y confirma en una
sola mutación durable el consumo del sello y la creación de todo el resultado.
Consultar probabilidades no muta el mundo.

El intercambio confirmado usa estas formas cerradas; todo selector es opaco y
pertenece a la sesión que lo proyectó:

| Acción | Campos exactos adicionales |
| --- | --- |
| `trade.request` | `recipient` |
| `trade.accept` | `tradeId` |
| `trade.decline` | `tradeId` |
| `trade.item.set` | `tradeId`, `target`, `quantity` |
| `trade.item.remove` | `tradeId`, `target` |
| `trade.coins.set` | `tradeId`, `coins` |
| `trade.contents` | `tradeId`, `target`, `page` |
| `trade.confirm` | `tradeId`, `stateRevision` |
| `trade.status` | `tradeId` |
| `trade.cancel` | `tradeId` |

`quantity` vale `1..=128`, `coins` `0..=1.000.000.000`, `page` es positivo y
`stateRevision` debe ser la revisión positiva exacta vista. La solicitud exige
un receptor opaco conectado y cercano; las demás acciones exigen el trade
opaco privado. Todos los campos forman parte de la firma de deduplicación.
Repetir una asignación idéntica o una confirmación ya registrada por la misma
identidad es un no-op; cambiar la firma bajo el mismo `requestId` se rechaza.

Los comandos equivalentes son `trade request`, `trade accept|decline|status|cancel`,
`trade offer item [cantidad]`, `trade remove item`, `trade offer coins`,
`trade contents` y `trade confirm <revisión>`. El texto describe el estado sin
depender de Web ni GMCP.

Los equivalentes de texto usan `party say` (`gsay`/`gtell`), `party revoke`,
`party kick`, `party transfer`, `party disband`, `reply`, `telepathy`, `emotes`,
`emote <comando> [personaje]`, `channel(s)`, `channel join`, `channel leave` y
`channel say`. Los alias `gossip`, `newbie` y
`wiznet` envían a `general`, `newbie`, `staff`. Unirse, enviar o recibir
`staff` exige perfil `moderator`, `administrator` o `implementor`.

An ability use has its own additive `abilityId` field; it does not overload the
opaque target:

```json
{
  "v": 1,
  "type": "action.perform",
  "data": {
    "requestId": "cmd-ability-1",
    "action": "combat.ability",
    "abilityId": "ember-bolt",
    "target": "o51a4c0311f262a90"
  }
}
```

`abilityId` is required for `combat.ability` and forbidden on unrelated
actions. A `self` ability forbids `target`; `explicitTarget` requires an opaque
visible NPC target; `optionalTarget` may omit it and currently resolves to the
caster. Availability, known-ability membership, recovery/stun state, resources,
cooldown, target perception and combat engagement are rechecked by the server.

Promotion and talent acquisition use stable authored content IDs and have
their own strict fields:

```json
{"v":1,"type":"action.perform","data":{
  "requestId":"promote-1",
  "action":"progression.promote",
  "classId":"tiamud:stormcaller"
}}
```

```json
{"v":1,"type":"action.perform","data":{
  "requestId":"talent-1",
  "action":"progression.talent",
  "graphId":"tiamud:wayfarer-talents",
  "nodeId":"tiamud:trail-sense"
}}
```

`classId` is required only for `progression.promote`; `graphId` and `nodeId`
are required only for `progression.talent`. IDs use the 128-byte definition-ID
grammar. The server rechecks current class, history, requirements, exclusions,
rank, cost, points and all resulting grants atomically. The equivalent text
commands, shared with Telnet, are `promote <class-id>` and
`talent learn <graph-id> <node-id>`.

`requestId` is 1-128 safe bytes. The server keeps a process-local bounded cache
of 4096 results keyed by character/request ID. Repeating the same signature
returns its cached `action.result`; reusing an ID for different input is a
protocol error. This survives WebSocket reconnect while the entry remains in
the cache, but not server restart or eviction; it is not a durable command
ledger.

### Snapshot recovery

```json
{"v":1,"type":"sync.request","data":{"afterSeq":41}}
```

The server responds with two independently bounded, critical frames carrying
the current authorized scene and player state:

```json
{"v":1,"type":"sync.scene","seq":50,"data":{"syncId":"sync-50","afterSeq":41,"scene":{}}}
{"v":1,"type":"sync.player","seq":51,"data":{"syncId":"sync-50","afterSeq":41,"player":{},"complete":true}}
```

Both frames echo `afterSeq` and share a transport-local `syncId`. Scene and
player parts coalesce independently, so a newer request replaces an older
incomplete pair. While recovery is pending, a client treats recognized
`sync.scene`/`sync.player` sequence jumps as part of that recovery rather than
recursively issuing another `sync.request`; it applies state only after it has
the matching pair and advances its accepted sequence through the player part.
An incomplete/mismatched older pair is discarded. v1 intentionally does not
retain/replay missing events. The official client closes and reconnects if no
complete recovery pair arrives within ten seconds.

### Heartbeat acknowledgement

```json
{"v":1,"type":"client.heartbeat","data":{"nonce":17}}
```

The nonce must match the latest server heartbeat. The server emits a heartbeat
every 15 seconds and disconnects after 45 seconds without a valid response. The
official client also closes/reconnects after 45 seconds without any server
traffic.

## Server messages

| Type | Purpose / priority |
| --- | --- |
| `server.welcome` | Server name, optional MOTD and transport-session ID. |
| `action.result` | Critical correlated accepted/rejected action result. |
| `server.notice` | `info`, `warning` or `error` operational/protocol notice. Pre-welcome access rejections include a stable `access.*` code. |
| `scene.snapshot` | Complete authorized scene. |
| `player.snapshot` | Complete private player state. |
| `scene.patch` | Typed changed scene scalars/weather/image/exits. |
| `entity.upsert`, `entity.remove` | Changed visible item/NPC. |
| `vitals.update` | Complete current character sheet/vitals/resources/abilities fragment. |
| `inventory.patch` | Complete current inventory/equipment fragment. |
| `container.contents` | Página autorizada de hasta 20 grupos hijos directos, con capacidad del contenedor. |
| `trade.contents` | Página privada autorizada de hasta 20 grupos hijos directos de un contenedor ofrecido. |
| `reward.odds` | Página privada de probabilidades exactas de una recompensa sellada portada. |
| `mail.list` | Página privada de hasta diez cabeceras del buzón offline. |
| `mail.message` | Carta privada completa marcada como leída. |
| `board.catalog` | Dos descriptores públicos de tablón y permiso actual de publicación. |
| `board.list` | Página pública de hasta diez resúmenes, solo para quien la solicita. |
| `board.entry` | Publicación pública completa, solo para quien la solicita. |
| `map.snapshot` | Complete current discovered map. |
| `message.append` | Perception-filtered narrative domain event. |
| `chat.message` | Entrega autorizada de `announcement`, `say`, `tell`, `telepathy`, `emote`, `party`, `general`, `newbie` o `staff`. |
| `sync.scene`, `sync.player` | Correlated current scene/player recovery pair after a gap request. |
| `media.play`, `media.stop` | Optional opted-in safe audio transition. |
| `server.heartbeat` | Unsequenced best-effort liveness challenge. |

State messages are coalescible, while narrative output has lower priority than
critical results. When a connection cannot drain critical output within the
bounded queue/write timeout, the server closes it explicitly. An
actor/broadcast lag is recovered with a notice and authoritative snapshots.

Una entrega de dominio proyecta como máximo 16 eventos narrativos por batch.
Si una ráfaga contiene más, Web y Telnet emiten un único aviso acotado,
coalescen la narración excedente y conservan un snapshot autoritativo crítico;
el cliente no debe inferir que el aviso sea un journal ni solicitar replay de
los eventos omitidos.

Before `server.welcome`, a rejected connection is reported as the typed error
notice documented under the authenticated handshake. Codes after welcome are
ordinary operational notices and do not retroactively reject authenticated
entry.

## Scene state

Representative `scene.snapshot`:

```json
{
  "viewId": "o1a34f72d87c00e52",
  "revision": 7,
  "title": "Lantern Square",
  "prose": ["Rain has polished the old stones."],
  "image": {
    "alt": "Lantern Square after rain, lit by a blue-glass lantern.",
    "localizedAlt": {
      "en": "Lantern Square after rain, lit by a blue-glass lantern.",
      "es": "La Plaza del Farol tras la lluvia, iluminada por un farol azul."
    },
    "manifest": "/assets/8ade88f0df1adb294333e72d3fa2d0863543dc0263ebe3038208f3c9e415924c/manifest.json"
  },
  "exits": [
    {"id":"o613eaed48b4d7580","label":"north","command":"north"}
  ],
  "items": [
    {
      "id":"of75b645d851f21cf",
      "label":"blue-glass lantern",
      "description":"Cold blue light pools inside the glass.",
      "rarity":"common",
      "itemLevel":1,
      "weightGrams":1200,
      "quantity":1,
      "stackSlots":2,
      "totalWeightGrams":1200,
      "transferPolicy":"transferable",
      "boundToYou":false,
      "loot":{"access":"yours","expiresInTicks":12},
      "actions":[
        {"label":"Examine","command":"examine of75b645d851f21cf"},
        {"label":"Take","command":"take of75b645d851f21cf"},
        {"label":"Pass","command":"loot pass of75b645d851f21cf"}
      ]
    }
  ],
  "npcs": [],
  "weather":{"condition":"rain","day":1,"hour":2,"nextChangeInTicks":6}
}
```

`viewId` and world entity IDs match `o` plus 16 lowercase hexadecimal digits,
are observer/session specific and cannot be correlated with durable WorldPack
IDs. Scene revision belongs to the authorized view. Hidden items/NPCs/exits,
private flags, author variant IDs and conditions are filtered before this
payload exists.

`loot` solo aparece en un objeto de suelo con derecho exclusivo. Su forma es
`{"access":"yours","expiresInTicks":1..20}` para el beneficiario o
`{"access":"reserved"}` para cualquier tercero; un objeto público omite el
campo. El servidor elimina `Take`/`Pass` cuando el observador no puede usarlos.
Nunca se proyectan `CharacterId`, `PartyId`, cohorte, drop, tabla, digest o
procedencia durable. Las pilas se particionan por acceso antes de asignar su
selector, por lo que una referencia no mezcla unidades propias, reservadas y
públicas.

La autoría limita el título de sala a 128 bytes y la descripción a 8.192
bytes. Además, antes de serializar, Web reduce cada escena a una ventana
semántica de 14 KiB y Telnet a 48 KiB. El orden de retención es determinista:
botín propio, objetos públicos y botín reservado ajeno, seguido del resto de
entidades/prosa según el presupuesto. Toda reducción añade un aviso textual y
la siguiente proyección autoritativa revela gradualmente el contenido restante.
La ventana Web deja margen para el envelope físico de 16 KiB; no aumenta ese
límite de frame.

`image` is optional. Its content-addressed root-relative URLs and contextual
localized alt text are presentation only. The official client resolves exact
locale, language, English, then deterministic first locale. Failure removes
only the image.

`scene.patch` always includes `viewId` and `revision` plus one or more of
`title`, `prose`, `image` (or `null`), `exits` and `weather`.
`entity.upsert/remove` also includes view/revision, `kind` (`item` or `npc`) and
opaque entity `id`.

`scene.snapshot` and `sync.scene.data.scene` may add a presentation-only
sidecar for entities that are already present in the same semantic scene:

```json
{
  "presentation": {
    "entities": [
      {
        "id": "of75b645d851f21cf",
        "alt": "",
        "decorative": true,
        "footprint": {"width": 0.18, "height": 0.42},
        "preferredAnchor": {"x": 0.28, "y": 0.91},
        "priority": 10,
        "layer": "foreground",
        "poses": {
          "idle": {
            "webp": "/assets/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/sprite-idle-640.webp",
            "png": "/assets/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/sprite-idle-640.png"
          }
        }
      }
    ]
  }
}
```

The `id` must equal an already visible item/NPC opaque ID; an annotation never
creates an entity. Sources are reviewed same-origin content-addressed WebP/PNG
rasters. The server projects at most 12 annotations and 32 total poses, ordered
by priority and opaque ID, then removes the lowest-priority optional entries if
the compact normal or recovery frame would exceed 16 KiB. Stable WorldPack IDs
never appear, and Telnet/GMCP shapes are unchanged.

The official browser renders at most ten sprites with deterministic semantic
overflow and disables requests under multimedia-off, Save-Data or
screen-reader policy. An existing `message.append` may carry one optional
presentation cue:

```json
{
  "revision": 42,
  "kind": "gameplay",
  "text": "The gate-stalker recoils from your strike.",
  "actorId": "o1111111111111111",
  "targetId": "o2222222222222222",
  "cue": {
    "kind": "hit",
    "entityId": "o2222222222222222",
    "pose": "hit",
    "intensity": 2,
    "durationMs": 320
  }
}
```

The implemented server mapping is deliberately closed and batch-aware:

| Projected authoritative event | Emitted cue |
| --- | --- |
| `NpcAbilityUsed` | `attack`, on the projected NPC actor |
| `CharacterDamaged` without a matching `NpcAbilityUsed` from that NPC in the same batch | `attack`, on the projected NPC actor |
| `NpcDamaged` with positive remaining health | `hit`, on the projected NPC target |
| `NpcDefeated` | `defeat`, on the projected NPC target |
| lethal `NpcDamaged`, the damage event paired with `NpcAbilityUsed`, or any other event | none |

This derivation happens only after the observer receives a perception-safe
`ViewEvent`; unprojected events produce no cue. It never examines narrative
text or copies a stable domain ID. The web adapter adds the cue only when the
same opaque NPC ID occurs in the previous or current authorized presentation
sidecar. Exact emitted values are `attack`/`attack`/2/420 ms,
`hit`/`hit`/2/320 ms and `defeat`/`defeated`/3/700 ms for
kind/pose/intensity/duration respectively.

The browser's bounded compatibility vocabulary also accepts `heal`, but the
server does not emit it in v1. Player characters are not separate compositor
entities, so player-targeted poses are not rendered. A normal same-view
snapshot retains an active cue only while that opaque entity remains in the
new semantic/presentation join; resync, view change or removal clears it.
Consequently defeat does not linger after authoritative NPC removal: v1 has no
visual tombstone or guaranteed 700 ms defeat fade. Reduced-motion mode selects
the authored static pose without movement. There is not yet a reviewed sample
sprite set in the bundled world, so this bridge is verified with fixtures
rather than claimed as finished demonstration art. Semantic item/NPC lists and
narrative remain the only source of gameplay state.

## Player state

`player.snapshot` is the complete private projection. Its `character` member is
the authoritative, presentation-safe character sheet; a representative shape
is:

```json
{
  "name": "Aria",
  "lineage": {
    "id": "tiamud:embermarked",
    "label": "Embermarked",
    "description": "People touched by a harmless inner ember that can be trained into arcane resilience."
  },
  "background": {
    "id": "tiamud:archive-apprentice",
    "label": "Archive Apprentice",
    "description": "Years among difficult records taught you disciplined study and careful observation."
  },
  "baseClass": {
    "id": "tiamud:arcanist",
    "label": "Arcanist",
    "description": "A scholar who turns mana and disciplined intellect into precise arcane effects."
  },
  "activeClass": {
    "id": "tiamud:arcanist",
    "label": "Arcanist",
    "description": "A scholar who turns mana and disciplined intellect into precise arcane effects."
  },
  "classHistory": [
    {
      "id": "tiamud:arcanist",
      "label": "Arcanist",
      "description": "A scholar who turns mana and disciplined intellect into precise arcane effects."
    }
  ],
  "level": 5,
  "experience": 1600,
  "nextLevelExperience": 2500,
  "health": 21,
  "maxHealth": 25,
  "strength": 7,
  "defence": 3,
  "attributes": [
    {"id":"strength","label":"Strength","value":7},
    {"id":"defence","label":"Defence","value":3},
    {"id":"intellect","label":"Intellect","value":11}
  ],
  "skills": [{"id":"combat","label":"Combat","rank":14}],
  "resources": [{"id":"mana","label":"Mana","current":12,"max":20}],
  "abilities": [
    {
      "id": "ember-bolt",
      "name": "Ember Bolt",
      "description": "Hurl a compact flame at a visible foe.",
      "kind": "spell",
      "resourceCosts": [{"resourceId":"mana","label":"Mana","amount":4}],
      "cooldownRemaining": 0,
      "usable": true,
      "purpose": "gameplay",
      "targeting": "explicitTarget"
    }
  ],
  "resistances": [{"damageType":"fire","percent":10}],
  "statuses": [
    {
      "id":"battle-focus",
      "label":"Battle Focus",
      "kind":"attribute_modifier",
      "magnitude":2,
      "remainingTicks":3
    }
  ],
  "progressionPoints": {"talent": 2, "lineage": 1},
  "talents": [
    {
      "id": "tiamud:arcane-talents",
      "label": "Arcane Talents",
      "description": "Choices for efficient channeling and deeper study of magical forces.",
      "pointPoolId": "talent",
      "availablePoints": 2,
      "nodes": [
        {
          "id": "tiamud:focused-channel",
          "label": "Focused Channel",
          "description": "Reduce waste when drawing power into a stable pattern.",
          "rank": 1,
          "maxRank": 2,
          "nextCost": 1,
          "available": true,
          "requirements": [{"met":true,"text":"Reach rank 1 in Focused Channel."}],
          "command": "talent learn tiamud:arcane-talents tiamud:focused-channel"
        }
      ]
    }
  ],
  "promotions": [
    {
      "class": {
        "id": "tiamud:spellblade",
        "label": "Spellblade",
        "description": "A martial or arcane practitioner who has deliberately mastered the discipline they once lacked."
      },
      "available": false,
      "requirements": [
        {"met":false,"text":"Reach level 6."},
        {"met":true,"text":"One of: All of: Be a Vanguard. AND Raise Intellect to 1. OR All of: Be a Arcanist. AND Raise Combat to 8."}
      ],
      "command": "promote tiamud:spellblade"
    }
  ],
  "effects": [],
  "recoveryInTicks": null
}
```

`lineage`, `baseClass` and `activeClass` are always present.
`background` is omitted, rather than serialized as `null`, when the character
has none. `classHistory` is a non-empty ordered list containing both the base
and active class without duplicate IDs. Each identity/class value has exactly
the public `{id,label,description}` shape. `progressionPoints` maps point-pool
IDs to current balances. `talents` contains only graphs currently owned by the
character; every node reports its current rank, optional next cost,
availability, safe requirement statuses and executable text command.
`promotions` contains only transitions reachable from the current active class
and likewise reports safe requirements and its text command.

Ability `kind` is `martial`, `spell` or `utility`; `purpose` is the closed
`gameplay|telepathy` presentation contract; `targeting` is `self`,
`explicitTarget` or `optionalTarget`. Status kinds are `attribute_modifier`,
`regeneration`, `damage_over_time` and `stun`. Damage types are `physical`,
`arcane`, `fire`, `frost` and `lightning`. Resistances and active statuses are
projected; v1 does not currently project a derived character damage range.
The category vocabulary is `martial`, `spell` and `utility` in both WorldPack
authoring and the wire; the former `innate` authoring term is not accepted by
the clean v3 model. `self_only`, `single_enemy` and `single_ally` authoring
targets project as `self`, `explicitTarget` and `optionalTarget`.
`optionalTarget` is self-only in the current clients because the scene does not
yet expose authorized ally target IDs; it reserves an additive future path, not
a current ally-selection promise.
The generic `attributes` list includes strength and defence; their scalar fields
remain in parallel for backward compatibility. `statuses` is the extensible
ability-status projection, while `effects` retains the older regeneration and
strength consumable effects.

The rest of the snapshot contains:

- `revision`;
- `inventory` and `equipment`, with opaque item IDs and authorized actions;
- `map`: opaque current/known room IDs and known/unknown paths;
- `quests`: stable public content slug, description, status/progress/actions;
- optional `party` and `partyInvites`, with opaque character IDs;
- `economy`: `coins`, `reservedCoins`, `spendableCoins`, commodity
  slugs/quantities and available shop stock/actions;
- optional `trade`: one private requested/negotiating trade with opaque ID and
  counterparty, ticks remaining, revision, both confirmation flags, both
  bounded offers and `canAccept`/`canConfirm`;
- `social.online`: ID opaco, nombre visible y booleanos `selfCharacter` y
  `nearby`. `nearby` solo ayuda a construir selectores de receptor; el servidor
  vuelve a comprobar conexión y sala al ejecutar.

Quest/commodity slugs are stable public content identifiers rather than hidden
world entity IDs. Persistence-only moderation/account state and private script
flags are absent.

Lineage, background, class, talent, ability and status definition IDs are
lowercase ASCII identifiers of at most 128 bytes using letters, digits, `-`,
`_`, `.` and `:`; attribute, resource and skill IDs use the same alphabet with
a 64-byte limit. Every ID starts and ends with a letter or digit. Character
sheets contain at most 64 attributes, 64 skills, 16 resources, 64 known
abilities and 32 statuses. The current closed damage vocabulary allows at most
five resistance entries (the generic runtime safety cap is 16). Progression
has at most four point pools, 16 visible talent graphs, 32 nodes per graph, five
ranks per node, 32 selected nodes, 64 promotion candidates and eight entries
in class history. A point balance or node cost is at most 10,000. An ability
has at most four resource costs. A projected requirement list has at most 128
entries and each safe requirement description is at most 512 bytes. Level is
at most 100; skill rank is at most 1,000; resources at most 1,000,000;
resistance at most 90 percent; cooldown/status duration at most 10,000 ticks.
These are projection/parser limits, not policy hints for clients to bypass
server validation.

Los objetos de escena, inventario, equipo y contenidos conservan
`weightGrams` como peso unitario y añaden `quantity`, `stackSlots` y
`totalWeightGrams`. Una instancia contenedor añade `containerCapacity` con
slots/peso usados y máximos. Una pila no es identidad wire durable: su selector
opaco representa el grupo visible actual y puede cambiar al variar sesión,
ubicación o miembros. `container.contents` contiene `container`, `page`,
`totalPages`, `capacity` e `items`; cada página incluye como máximo veinte
grupos y debe caber en el límite normal de 16 KiB.

Una unidad única modificada añade `affix` con `label`, `position`
(`prefix` o `suffix`), `attackBonusDelta` y `healAmountDelta`. El nombre
proyectado de ese objeto ya incorpora el prefijo o sufijo. La vista nunca
publica el ID de definición, el pool, hashes ni procedencia durable del afijo;
el modificador es presentación y estadística autorizadas, no autoridad de
autoría. Los objetos sin modificador omiten `affix`.

En inventario, `availableQuantity` y `reservedForTrade` separan unidades
mutables de las reservadas y `canTrade` solo propone raíces completamente
transferibles no bloqueadas. La oferta proyecta líneas agrupadas con selector,
etiqueta, cantidad y marca de contenedor, sin `ItemInstanceId`, prototipo ni
participante durable. `trade.contents` contiene `tradeId`, `container`, `page`,
`totalPages`, `capacity` e `items`. La vista y cada selector se regeneran con
la sal de la sesión; la contraparte no recibe IDs internos ni política social.

Cada objeto incluye obligatoriamente `transferPolicy`, con valor
`transferable`, `character_bound` o `non_transferable`, y el booleano
`boundToYou`. Este último solo puede ser verdadero para `character_bound` y
nunca revela el `CharacterId` durable del propietario. Los objetos de inventario
añaden el booleano obligatorio `canGive`, calculado sobre todo el subárbol; un
contenido de contenedor siempre lleva `canGive: false` porque no es una raíz
portada directamente, y el equipo no publica ese campo. Las acciones
autorizadas siguen indicando `Drop`; `canGive`, no una acción textual
incompleta, decide si el cliente ofrece `Give`.

En GMCP, `Char.Items.List` proyecta `transferPolicy` y `boundToYou` para
`location: "inv"`, `"worn"` y `"container"`. Inventario y contenedor incluyen
también `canGive`; inventario añade disponibilidad/reserva y `canTrade`, y en
contenedor entrega/trade siempre son falsos. `Char.Trade 1` y
`Char.Trade.Contents` proyectan el mismo estado privado y los mismos fallbacks
acotados que el texto. Cada regeneración usa la sal de la sesión Telnet y no
transporta vínculo, propietario, trade ni instancia durables.

Una respuesta Web `reward.odds` contiene `selector`, `page`, `totalPages` y
tres listas cuya suma no supera veinte filas por página: `guaranteed`,
`weighted` y `affixes`. Las filas garantizadas exponen etiqueta y cantidad;
las ponderadas exponen etiqueta de pool, tiradas, etiqueta del resultado,
cantidad mínima/máxima y `probabilityNumerator`/`probabilityDenominator`; las
de afijo exponen resultado padre, etiqueta, posición y deltas, junto con
`conditionalProbabilityNumerator`/`conditionalProbabilityDenominator`.
`nothing` es una fila explícita cuando corresponde. Las fracciones enteras son
el contrato exacto: el cliente puede mostrar además un porcentaje calculado,
pero no sustituirlas por redondeos. `affixPoolLabel` es solo una etiqueta
revisada opcional; no es un identificador y no debe usarse para acciones.

Telnet siempre recibe la misma información jugable en texto. Si negocia
`Char.Reward 1`, recibe además `Char.Reward.Odds` con la misma página, las
fracciones exactas y la revisión privada. Cada paquete conserva el límite GMCP
de 8 KiB; ante un exceso se acotan solo etiquetas y se añaden
`truncated: true` y `overflow.reason: "messageTooLarge"`, sin eliminar
selectores, filas, deltas ni numeradores/denominadores.

`mail.list` contiene `page`, `totalPages`, `totalMessages`, `unreadCount`,
`capacity: 64` y hasta diez filas `{id,sender,subject,createdAt,expiresAt,read}`.
`mail.message` añade `body`. Los tiempos son segundos Unix; el cuerpo nunca
entra en la lista. Referencias y campos están acotados antes de serializar y
no exponen cuentas, personajes o claves SQL. Telnet ofrece texto equivalente;
si negocia `Tiamud.Mail 1`, recibe `Tiamud.Mail.List` o
`Tiamud.Mail.Message`, cada paquete bajo el límite GMCP de 8 KiB. Como defensa
en profundidad, si un valor interno no canónico hiciera crecer el JSON
escapado, `Tiamud.Mail.Message` conserva cabecera, prefijo UTF-8 del cuerpo,
`truncated: true` y el mismo objeto `overflow` descrito para tablones; el input
canónico de correo limita el cuerpo a 2.048 bytes y normalmente cabe completo.

`board.catalog` contiene `retentionDays: 90` y exactamente dos filas
`{key,label,description,canPost}` para `general` y `newbie`. `board.list`
contiene `board`, `page`, `totalPages`, `totalPosts`, `capacity: 1000` y hasta
diez resúmenes `{postId,board,author,subject,createdAt,expiresAt,canDelete,
canReport}`. `totalPages` nunca es menor que uno; una página válida posterior
al total contiene `posts: []`. `board.entry` contiene el mismo resumen más
`body`. `canDelete` se calcula para el mismo personaje autor y `canReport` para
otra cuenta mientras no exista la denuncia global.

Las respuestas no contienen cuenta, personaje durable, PK SQL, estado de
ignore/block ni cuerpo en el listado. Svelte inserta sus cadenas como texto,
sin HTML, Markdown o autolinks. Telnet ofrece el equivalente textual; si
negocia `Tiamud.Board 1`, recibe `Tiamud.Board.Catalog`, `.List` o `.Entry`,
cada paquete bajo 8 KiB contando paquete, espacio y JSON. Si el detalle
completo excede ese máximo por escape JSON, `Tiamud.Board.Entry` conserva el
resumen y el mayor prefijo UTF-8 de `body` que quepa, añade
`truncated: true` y
`overflow: {package:"Tiamud.Board.Entry",maximumBytes:8192,originalBytes}`.
`originalBytes` mide el cuerpo UTF-8 previo al escape. El texto Telnet conserva
el cuerpo completo. Los tablones no producen `chat.message`, `message.append`
ni notificación online.

`vitals.update`, `inventory.patch` and `map.snapshot` replace their complete
corresponding fragment at the stated revision. Other player changes use a new
`player.snapshot`. A full scene/player snapshot may follow semantic updates to
keep recovery deterministic under output coalescing.

## Narrativa, conversación y medios

`message.append.data` contiene `revision`, `kind` (`characterMoved`,
`itemTaken`, `itemDropped`, `itemGiven`, `tradeRequested`, `tradeChanged`,
`tradeCompleted`, `tradeCancelled`, `gameplay` o `adventure`), texto seguro,
`actorId` opaco y `targetId`/`itemId` opacos opcionales. Se proyecta por separado
para cada observador; no percibir un evento de dominio significa no recibir
mensaje.

Para `itemGiven`, `actorId` identifica al emisor y `targetId` al receptor con la
sal propia del observador. Emisor y receptor reciben además `itemId`, derivado
con su sesión de la primera raíz transferida; no se promete que sea una
identidad durable de pila. Otro observador de la misma sala recibe la narración
y los personajes opacos, pero no `itemId`; un observador remoto no recibe el
evento. El evento durable `ItemsGiven` conserva como máximo 256 IDs de raíces
internos, nunca expuestos por este mensaje.

Solicitud, cambio, confirmación y cancelación de trade solo producen narrativa
para sus dos participantes; `tradeCompleted` añade una narración genérica para
observadores de la misma sala. Ningún mensaje expone ofertas, cantidades,
moneda, motivo social, `TradeId` o IDs de instancia. La ficha privada y las
actualizaciones de inventario/economía llevan el estado autoritativo.

```json
{"v":1,"type":"message.append","seq":52,"data":{
  "revision":19,
  "kind":"itemGiven",
  "text":"Aria gives 2 × ember tonic to Bram.",
  "actorId":"o51a4c0311f262a90",
  "targetId":"o7f72cd623eb86c31",
  "itemId":"o9c07330f19a0e04d"
}}
```

Telnet conserva la narración textual completa. Si negoció `Char.Items`, emisor
y receptor reciben listas de inventario regeneradas independientemente con su
propia sal; el observador de sala no recibe una lista ajena.

`chat.message.data` contiene `channel` (`announcement`, `say`, `tell`,
`telepathy`, `emote`, `party`, `general`, `newbie` o `staff`), `sender` y texto plano
acotado. Ignore y block
por cualquiera de las partes se aplican antes de entregar. Mute/ban y rate
limit se aplican en servidor. `general`, `newbie` y `staff` requieren
suscripción durable en esquema v13; `staff` exige además perfil `moderator`,
`administrator` o `implementor` al unirse, enviar y recibir. `helper`,
`builder` y los roles legados `Player`/`Moderator`/`Admin` no conceden ese
acceso. `announcement` es entrega del servidor, no un canal suscribible. La
suscripción es durable, pero el estado de `reply` y el historial son solo
memoria.

`telepathy` procede de `social.telepathy`, no de `tell`: el actor debe conocer
la habilidad, pagar recurso y cooldowns, y estar en una sala permitida. El
servidor confirma esa mutación antes de entregar. Ignore/block y la sala o el
estado privado del receptor pueden suprimir la entrega sin cambiar la
confirmación del emisor. El cuerpo, destino y `abilityId` no se incluyen en
eventos durables; la recepción no modifica el destinatario de `reply`.

`emote` procede de `social.emote`: `sender` identifica al actor y `text`
contiene la frase completa ya renderizada para ese observador. El cliente no
antepone otra vez el remitente. El comando y nombre son descubribles mediante
el listado, pero la plantilla, el objetivo y los IDs no se exponen como campos
estructurados del mensaje; el nombre visible del objetivo puede formar parte
del texto renderizado. Ignore/block puede suprimir receptores sin
cambiar la confirmación del actor ni la percepción autorizada de terceros.
No crea historial, `reply`, evento durable, snapshot o fila SQL.

`mail` no es un `chat.message`: se consulta bajo demanda y persiste hasta 30
días en un buzón de 64 mensajes/128 KiB. La cuota por cuenta es 20 intentos por
hora y 50 por día. Una entrega suprimida por ignore/block, ban o capacidad usa
la misma aceptación que un guardado; un objetivo desconocido, ambiguo o propio
usa un único error genérico. Solo el destinatario lista, lee, borra o denuncia.
Una denuncia captura evidencia durante 90 días sin habilitar exploración
general de buzones por staff. Cada cuenta admite como máximo ocho denuncias de
correo abiertas y el sistema conserva hasta 10.000 evidencias activas.

La decisión de persistencia, privacidad y retención completa está en
[ADR 0017](../decisions/0017-bounded-offline-mail.md).

Telnet representa el mismo `SocialMessage` autorizado como texto acotado. Si
negoció GMCP `Comm`, recibe también `Comm.Channel.Text` con el mismo alcance,
incluido `announcement`, y los campos `talker` y `text`; GMCP no evita
autorización ni crea otra fuente de verdad. Cuando MXP está negociado, texto,
remitentes y narrativa de dominio se escapan como datos exactamente una vez;
texto plano y el JSON GMCP conservan el contenido original.

Audio messages are sent only when `client.hello.capabilities.audio` is true:

```json
{"v":1,"type":"media.play","seq":20,"data":{
  "channel":"ambience",
  "url":"/assets/<64-lowercase-hex>/audio-rain.opus",
  "loop":true
}}
```

Channels are `music`, `ambience` or `effects`; formats are Opus/Ogg/MP3 under a
content-addressed same-origin asset path. The browser independently requires a
user gesture and enabled preference before playback. `media.stop` identifies a
channel. Media never carries exclusive gameplay information.

## Telnet GMCP scene overflow

Each GMCP package payload (package name, one separating space and compact JSON)
is limited to 8 KiB. A complete `Tiamud.Scene` which exceeds that budget is not
sliced or silently omitted. The server instead sends a valid bounded
`Tiamud.Scene` containing `viewId`, `revision`, a title capped at 256 Unicode
characters, `truncated: true`, `titleTruncated`, and:

```json
{"overflow":{"package":"Tiamud.Scene","maximumBytes":8192}}
```

A client which negotiated `Tiamud.Overflow 1` also receives at most one
`Tiamud.Overflow` notice per GMCP session, with package, `messageTooLarge`
reason and byte limit. Every overflowing scene still increments the operational
`tiamud_outbound_budget_exceeded_total{transport="gmcp",message="scene"}`
counter even after that notice has been deduplicated. The normal complete text
rendering remains authoritative and is never removed by this fallback.

## Limits and failure policy

- HTTP/WS body, frame and message: 16 KiB.
- escena semántica antes del envelope: 14 KiB en Web; escena semántica usada
  para texto/GMCP/celdas: 48 KiB en Telnet. Una reducción conserva primero
  botín propio, público y reservado ajeno y siempre publica aviso;
- eventos proyectados por batch: 16; un exceso se coalesce con aviso y
  snapshot autoritativo;
- Every post-authentication outbound WebSocket envelope is compact-serialized
  and checked against that same 16 KiB limit before entering the output queue.
  La escena ya se reduce a su ventana semántica; cualquier envelope restante o
  proyección privada individual que aún exceda el límite falla de forma cerrada
  con motivo explícito y
  `tiamud_outbound_budget_exceeded_total{transport="websocket",message="frame"}`
  increment rather than emitting an invalid frame.
- JSON nesting: 16.
- `command.submit` WebSocket: 1.024 bytes; línea de comando Telnet: 8 KiB,
  descartada completa si excede el máximo; request ID: 128; action target: 256;
  `recipient`: 64.
- chat: 512 bytes; mail target/subject/body/report reason: 64/128/2,048/2,048
  bytes; board subject/body/report reason: 128/4,096/2,048 bytes, diez posts
  por página y páginas 1..=100; reason fields and all projected arrays/strings
  are bounded.
- initial hello timeout: 10 seconds; WebSocket write timeout: 5 seconds.
- three consecutive protocol/rate violations close the game session.
- `tiamud.v1` is required; browser `Origin`, when present, must exactly match
  the HTTP `Host`. Non-browser clients may omit `Origin` but still authenticate
  inside `client.hello`.
- static responses carry CSP, `X-Content-Type-Options: nosniff` and safe cache
  policy; external/data/file media URLs are rejected.

Errors disclose safe player-facing state only. Internal IDs, storage errors,
credentials, moderation internals and condition state are not serialized.
