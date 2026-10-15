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

### Typed source and generated TypeScript

The Rust crate `crates/protocol` (`tiamud-protocol`, proposed
[ADR 0026](../decisions/0026-typed-protocol-source-and-typescript-codegen.md))
is the single typed source of the envelope, the server/client message type
names, every client payload except `action.perform`, and the server payloads
whose shape belongs to the transport. Payloads that embed a `tiamud-core`
projection (scene, player, item, map, narrative event) are generic over it. The
server builds every WebSocket message from these types; the encoded JSON is
byte-identical to the former hand-built objects, which golden tests in both
crates pin.

`apps/web/src/lib/generated/protocol.ts` is generated from those types with
ts-rs (a test-only dependency) and committed. `cargo test --workspace --locked`
fails when it is stale; regenerate with
`TIAMUD_UPDATE_GENERATED_PROTOCOL=1 cargo test --locked -p tiamud-protocol`.
Generated declarations describe the wire, not trust: the official client still
validates every inbound frame at runtime in `protocol.ts`, and its validators
return the generated types. Still hand-mirrored: the `action.perform` field set
and the inner scene/player/item/map/event/reward-odds/presentation shapes.

### Additive v1 rollout policy

`tiamud.v1` has not yet reached its first compatibility milestone nor been used
in public production. During this pre-release interval, phases 4A and 4B
deliberately replace the source contract in an incompatible way: the server and
the official client must come from the same checkout. There is no negotiated
capability that makes an older client compatible with the 4A and 4B fields or
enums, and that compatibility must not be claimed.

The following additive policy will be binding starting from the first explicit
compatibility milestone. From that milestone on, the client-server schemas will
remain strict: a client will not be able to probe for new actions through
unknown fields. Capability names will be the explicit extension point, but
advertising an unknown capability will have no effect until the server
implements it. Server-to-client evolution will be additive only within a
recognized shape. The official client will discard unknown fields inside a
recognized envelope/payload while validating and preserving all known ones;
this will allow adding optional presentation or information without changing the
meaning of the v1 message.

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

```mermaid
sequenceDiagram
    accTitle: Authenticated pre-welcome handshake
    accDescr: The first frame within ten seconds must be a valid client.hello. The server rechecks bearer session, ownership, ban state and the single-live-character lease. Success sends sequenced server.welcome, scene.snapshot and player.snapshot. A rejection sends a sequenced server.notice with one stable access code and closes the connection.

    participant C as Client
    participant S as Server

    C->>S: client.hello (first frame, within 10 s)
    S->>S: resolve/recheck bearer session, ownership,<br/>ban state and single-live-character lease
    alt entry accepted
        S-->>C: server.welcome (sequenced)
        S-->>C: scene.snapshot
        S-->>C: player.snapshot
        Note over C,S: connection enters play with a fresh observer salt
    else entry rejected (pre-welcome)
        S-->>C: server.notice with one stable access.* code
        Note over C,S: access.protocol_rejected, access.session_expired,<br/>access.session_invalid, access.character_invalid,<br/>access.character_incompatible, access.character_connected,<br/>access.session_limit, access.temporarily_unavailable
        S--xC: close connection
        Note over C: official client stops auto-reconnect; clears saved<br/>credentials only for expired/invalid sessions
    end
```

## Client messages

### Text command

```json
{
  "v": 1,
  "type": "command.submit",
  "data": {"requestId":"cmd-27","command":"north"}
}
```

The command is at most 1024 bytes and contains no control characters. It reaches
the same `PlayerAction` parser as Telnet.

Textual item delivery uses exactly:

```text
give [quantity] <item> to <character>
```

The default quantity is one and must be in `1..=256`. Item and recipient may be
opaque session selectors; the recipient may also be an unambiguous exact name.
The server requires another connected character in the same room and rechecks
policy, subtree, capacity and ignore/block.

### Structured action

The server parses the WebSocket JSON with recursive duplicate-key detection. Two
occurrences of a key in any object, nested included, reject the frame before
building the typed payload; no action depends on the parser's last-key-wins
semantics.

```json
{
  "v": 1,
  "type": "action.perform",
  "data": {"requestId":"cmd-28","action":"item.take","target":"o51a4c0311f262a90"}
}
```

Implemented action names:

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

Depending on the action, the strict payload may also contain `message`,
`channel` (`general`, `newbie` or `staff`), `durationSeconds`, `reason` or
`role` (`player`, `moderator`, `admin`). `party.revoke`, `party.kick` and
`party.transfer` use `target`; `party.say` and `social.reply` use `message`;
channel join/leave uses `channel`, and sending requires `channel` and `message`.
`social.telepathy` requires exactly `abilityId`, `target` and `message`: the
authored ID is at most 64 bytes, the character 64 and the visible UTF-8 thought
512. It accepts no combat, channel or persistence fields, even when a known
additional field arrives as `null`. An ability whose purpose is `telepathy` is
rejected on `combat.ability`; there is no second generic path.
`social.emote.list` accepts no action fields. `social.emote` requires `emote`,
an authored ASCII command of 1–32 bytes matching `[a-z][a-z0-9-]*`, and accepts
only `target`, a UTF-8 name of up to 64 bytes. It does not accept `message`: the
phrase comes from the WorldPack's reviewed templates and the target, when the
gesture allows or requires one, must be another connected, unambiguous character
in the same room. Unexpected fields and missing/forbidden targets are rejected.
The same authorization and perception rules apply; a structured action is not a
reliable shortcut. Accept/revoke/kick/transfer resolve `target` as the name of a
connected character, not as offline membership administration.

Phase 5E implements two additional actions:

| Action | Exact additional fields |
| --- | --- |
| `social.yell` | `message` |
| `social.shout` | `message` |

`message` is `1..=512` visible UTF-8 bytes. These shapes accept no `target`,
`channel`, radius, direction, distance or additional fields, even when a known
one arrives as `null`. Their text equivalents are `yell <message>` and
`shout <message>`.

Mail uses separate closed shapes:

| Action | Exact additional fields |
| --- | --- |
| `mail.list` | None or `page` |
| `mail.read` | `mailId` |
| `mail.delete` | `mailId` |
| `mail.send` | `target`, `subject`, `body` |
| `mail.report` | `mailId`, `reason` |

`mailId` is `m` followed by 32 lowercase hexadecimal digits; `page` is in
1..7. Requesting a page past `totalPages` is valid and returns an empty list,
which prevents a changing total from turning navigation into an error.
Target/subject/body/reason are at most 64/128/2,048/2,048 visible UTF-8 bytes,
with no controls or bidi formatting marks. The target name matches exactly,
case included. The pipe `|` is reserved as a separator in target/subject, but
body and reason may keep it. The equivalent text actions are
`mail [list [page]]`, `mail read/delete <ref>`,
`mail send <character> | <subject> | <body>` and
`mail report <ref> | <reason>`. Sending shares the social mute and bucket; the
other operations do not. A repeated `requestId` only replays the same exact
signature, and the private structured response belongs to that session.

Boards use separate closed shapes:

| Action | Exact additional fields |
| --- | --- |
| `board.catalog` | None |
| `board.list` | `board`, `page` |
| `board.read` | `postId` |
| `board.post` | `board`, `subject`, `body` |
| `board.delete` | `postId` |
| `board.report` | `postId`, `reason` |

`board` is `general` or `newbie`; `postId` is `b` followed by 32 lowercase
hexadecimal digits and `page` is in `1..=100`. Subject, body and reason are at
most 128/4,096/2,048 visible UTF-8 bytes. C0/C1 controls, DEL and bidi marks are
rejected. `|` is forbidden in the subject because it separates the text command,
but body and reason may keep it. The exact text shapes are `board`,
`board list <general|newbie> [page]`, `board read/delete <ref>`,
`board post <board> | <subject> | <body>` and
`board report <ref> | <reason>`.

Catalog, list and read do not mutate nor publish to other sessions. `post`
shares the social mute, ban and bucket before its own durable quota; the other
actions do not. A repeated `requestId` requires the same signature, including
the entire body, and only replays that session's response.

The browser records pending `board.post`, `board.delete` and `board.report` in a
bounded way and does not apply their effect on the mere submission. It waits for
the `action.result` whose `requestId` matches: only `ok: true` invalidates the
list, removes the deleted detail or marks the entry as reported. A failure
preserves the view and the drafts; the post/report forms block a second
submission while their request is still pending.

Except for `staff.status`, which only shows the caller's own assignment, every
`staff.*` action requires a concrete capability and `reason`; targeted actions
additionally recheck the hierarchy at the actor. A legacy role is never enough.
Resolution uses a positive `reportId` and `resolution`
(`dismissed`, `action_taken` or `no_action`). `staff.profile.set` requires
`target`, `profile` and `loreTitle`, where the latter two accept `null` to
remove the assignment or the title; the non-null profiles are `helper`,
`builder`, `moderator`, `administrator` and `implementor`.
`staff.capability.set` requires `target`, `enabled` and a capability from this
closed vocabulary: `reports.view`, `reports.resolve`, `moderation.mute`,
`moderation.ban`, `staff.assign`, `staff.inspect`, `staff.where`, `staff.goto`,
`staff.transfer`, `staff.restore`, `staff.announce` or `staff.audit`. The target
is limited to 64 bytes, the reason to 1,024 and the lore title to 128. Fields
that do not belong to the action are rejected.

Item actions use closed combinations of `target`, `container`, `recipient`,
`quantity` and `page`. `quantity` is optional, defaults to one and must be in
`1..=256`. `item.put`/`item.get` require both opaque selectors; `item.contents`
requires `container`, allows a positive page and forbids quantity. `item.give`
requires `target` and `recipient`, allows quantity and forbids container/page.
`recipient` is at most 64 bytes, contains no controls and is rejected for any
other action. All accepted fields are part of the deduplication signature:

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

For `item.give`, `target`, `recipient` and `quantity` participate in the
signature. Repeating `requestId` with the same signature returns the cached
result without re-executing; changing any of those values with the same ID is a
protocol error. Opaque selectors belong to the session that created them; the
cache stays indexed by character/`requestId` and can only replay the same exact
signature per the general rules described below.

Loot actions have closed shapes:

| Action | Exact additional fields |
| --- | --- |
| `loot.status` | None |
| `loot.pass` | `target` |

`target` is the opaque selector or visible exact name of a reserved root and is
at most 256 bytes with no controls. It is part of the deduplication signature;
changing it under the same `requestId` is rejected. `loot.pass` can only cede an
own right to the next identity in the frozen cohort, does not renew the deadline
and does not reveal that identity to the client.

Sealed rewards use separate, closed shapes:

| Action | Exact additional fields |
| --- | --- |
| `reward.open` | `target` |
| `reward.odds` | `target`, `page` |

`target` identifies a sealed reward the character carries and `page` is
positive. Both are part of the deduplication signature. The equivalent text
commands, shared by Web and Telnet, are `open <selector>` and
`reward odds <selector> [page]`; omitting the text page uses the first. Opening
rechecks authority, space and weight and confirms, in a single durable mutation,
the consumption of the seal and the creation of the entire result. Querying
odds does not mutate the world.

Confirmed trading uses these closed shapes; every selector is opaque and belongs
to the session that projected it:

| Action | Exact additional fields |
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

`quantity` is `1..=128`, `coins` `0..=1,000,000,000`, `page` is positive and
`stateRevision` must be the exact positive revision seen. The request requires a
connected, nearby opaque recipient; the other actions require the private opaque
trade. All fields are part of the deduplication signature. Repeating an
identical assignment or a confirmation already recorded by the same identity is
a no-op; changing the signature under the same `requestId` is rejected.

The equivalent commands are `trade request`,
`trade accept|decline|status|cancel`, `trade offer item [quantity]`,
`trade remove item`, `trade offer coins`, `trade contents` and
`trade confirm <revision>`. The text describes the state without depending on
Web or GMCP.

The text equivalents use `party say` (`gsay`/`gtell`), `party revoke`,
`party kick`, `party transfer`, `party disband`, `reply`, `telepathy`, `emotes`,
`emote <command> [character]`, `channel(s)`, `channel join`, `channel leave` and
`channel say`. The aliases `gossip`, `newbie` and `wiznet` send to `general`,
`newbie`, `staff`. Joining, sending to or receiving `staff` requires a
`moderator`, `administrator` or `implementor` profile.

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

```mermaid
sequenceDiagram
    accTitle: Snapshot recovery pair
    accDescr: A sync.request with afterSeq is answered by a sync.scene and a sync.player frame that echo afterSeq and share a syncId. Scene and player parts coalesce independently, a newer request replaces an older incomplete pair, and the client applies state only after it holds the matching pair. No matching pair within ten seconds closes and reconnects.

    participant C as Client
    participant S as Server

    C->>S: sync.request{afterSeq}
    S-->>C: sync.scene{syncId, afterSeq, scene}
    S-->>C: sync.player{syncId, afterSeq, player, complete}
    Note over C: parts coalesce independently;<br/>apply state only with the matching syncId pair,<br/>advance accepted seq through the player part
    alt newer sync.request before pair completes
        C->>S: sync.request{afterSeq'}
        Note over C: older incomplete pair discarded,<br/>newer request replaces it
        S-->>C: sync.scene{syncId', afterSeq'}
        S-->>C: sync.player{syncId', afterSeq', complete}
    else no complete pair within 10 s
        Note over C,S: client closes and reconnects;<br/>v1 does not retain/replay missing events
    end
```

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
| `container.contents` | Authorized page of up to 20 direct child groups, with the container's capacity. |
| `trade.contents` | Authorized private page of up to 20 direct child groups of an offered container. |
| `reward.odds` | Private page of exact odds for a carried sealed reward. |
| `mail.list` | Private page of up to ten headers from the offline mailbox. |
| `mail.message` | Complete private letter marked as read. |
| `board.catalog` | Two public board descriptors and the current posting permission. |
| `board.list` | Public page of up to ten summaries, only for the requester. |
| `board.entry` | Complete public post, only for the requester. |
| `map.snapshot` | Complete current discovered map. |
| `message.append` | Perception-filtered narrative domain event. |
| `chat.message` | Authorized delivery of `announcement`, `say`, `yell`, `shout`, `tell`, `telepathy`, `emote`, `party`, `general`, `newbie` or `staff`. |
| `sync.scene`, `sync.player` | Correlated current scene/player recovery pair after a gap request. |
| `media.play`, `media.stop` | Optional opted-in safe audio transition. |
| `server.heartbeat` | Unsequenced best-effort liveness challenge. |

State messages are coalescible, while narrative output has lower priority than
critical results. When a connection cannot drain critical output within the
bounded queue/write timeout, the server closes it explicitly. An
actor/broadcast lag is recovered with a notice and authoritative snapshots.

A domain delivery projects at most 16 narrative events per batch. If a burst
contains more, Web and Telnet emit a single bounded notice, coalesce the excess
narration and preserve a critical authoritative snapshot; the client must not
infer that the notice is a journal nor request replay of the omitted events.

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

`loot` appears only on a ground item with an exclusive right. Its shape is
`{"access":"yours","expiresInTicks":1..20}` for the beneficiary or
`{"access":"reserved"}` for any third party; a public item omits the field. The
server removes `Take`/`Pass` when the observer cannot use them. `CharacterId`,
`PartyId`, cohort, drop, table, digest and durable provenance are never
projected. Stacks are partitioned by access before their selector is assigned,
so a reference never mixes own, reserved and public units.

Authoring limits the room title to 128 bytes and the description to 8,192 bytes.
In addition, before serializing, Web reduces each scene to a 14 KiB semantic
window and Telnet to 48 KiB. The retention order is deterministic: own loot,
public items and others' reserved loot, followed by the rest of the
entities/prose as the budget allows. Any reduction adds a textual notice and the
next authoritative projection gradually reveals the remaining content. The Web
window leaves room for the physical 16 KiB envelope; it does not raise that
frame limit.

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
- `social.online`: opaque ID, visible name and the `selfCharacter` and `nearby`
  booleans. `nearby` only helps build recipient selectors; the server rechecks
  connection and room on execution.

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

Scene, inventory, equipment and contents items keep `weightGrams` as the unit
weight and add `quantity`, `stackSlots` and `totalWeightGrams`. A container
instance adds `containerCapacity` with used and maximum slots/weight. A stack is
not durable wire identity: its opaque selector represents the current visible
group and may change when the session, location or members vary.
`container.contents` contains `container`, `page`, `totalPages`, `capacity` and
`items`; each page includes at most twenty groups and must fit within the normal
16 KiB limit.

A modified unique unit adds `affix` with `label`, `position` (`prefix` or
`suffix`), `attackBonusDelta` and `healAmountDelta`. That item's projected name
already incorporates the prefix or suffix. The view never publishes the affix's
definition ID, pool, hashes or durable provenance; the modifier is authorized
presentation and statistics, not authoring authority. Items with no modifier
omit `affix`.

In inventory, `availableQuantity` and `reservedForTrade` separate mutable units
from reserved ones and `canTrade` only proposes fully transferable, unlocked
roots. The offer projects grouped lines with selector, label, quantity and
container mark, without `ItemInstanceId`, prototype or durable participant.
`trade.contents` contains `tradeId`, `container`, `page`, `totalPages`,
`capacity` and `items`. The view and each selector are regenerated with the
session's salt; the counterparty receives no internal IDs or social policy.

Every item mandatorily includes `transferPolicy`, with value `transferable`,
`character_bound` or `non_transferable`, and the boolean `boundToYou`. The
latter can only be true for `character_bound` and never reveals the owner's
durable `CharacterId`. Inventory items add the mandatory boolean `canGive`,
computed over the entire subtree; a container's content always carries
`canGive: false` because it is not a directly carried root, and equipment does
not publish that field. The authorized actions still list `Drop`; `canGive`, not
an incomplete text action, decides whether the client offers `Give`.

In GMCP, `Char.Items.List` projects `transferPolicy` and `boundToYou` for
`location: "inv"`, `"worn"` and `"container"`. Inventory and container also
include `canGive`; inventory adds availability/reservation and `canTrade`, and in
a container give/trade are always false. `Char.Trade 1` and
`Char.Trade.Contents` project the same private state and the same bounded
fallbacks as the text. Each regeneration uses the Telnet session's salt and
carries no durable binding, owner, trade or instance.

A Web `reward.odds` response contains `selector`, `page`, `totalPages` and three
lists whose sum does not exceed twenty rows per page: `guaranteed`, `weighted`
and `affixes`. Guaranteed rows expose label and quantity; weighted ones expose
pool label, rolls, result label, minimum/maximum quantity and
`probabilityNumerator`/`probabilityDenominator`; affix ones expose parent
result, label, position and deltas, along with
`conditionalProbabilityNumerator`/`conditionalProbabilityDenominator`.
`nothing` is an explicit row when applicable. The integer fractions are the
exact contract: the client may additionally show a computed percentage, but must
not replace them with rounding. `affixPoolLabel` is only an optional reviewed
label; it is not an identifier and must not be used for actions.

Telnet always receives the same playable information in text. If it negotiates
`Char.Reward 1`, it additionally receives `Char.Reward.Odds` with the same page,
the exact fractions and the private revision. Each package keeps the 8 KiB GMCP
limit; on overflow only labels are bounded and `truncated: true` and
`overflow.reason: "messageTooLarge"` are added, without removing selectors,
rows, deltas or numerators/denominators.

`mail.list` contains `page`, `totalPages`, `totalMessages`, `unreadCount`,
`capacity: 64` and up to ten rows
`{id,sender,subject,createdAt,expiresAt,read}`. `mail.message` adds `body`. Times
are Unix seconds; the body never enters the list. References and fields are
bounded before serializing and do not expose accounts, characters or SQL keys.
Telnet offers equivalent text; if it negotiates `Tiamud.Mail 1`, it receives
`Tiamud.Mail.List` or `Tiamud.Mail.Message`, each package under the 8 KiB GMCP
limit. As defense in depth, if a non-canonical internal value were to grow the
escaped JSON, `Tiamud.Mail.Message` keeps the header, the body's UTF-8 prefix,
`truncated: true` and the same `overflow` object described for boards; the
canonical mail input limits the body to 2,048 bytes and normally fits in full.

`board.catalog` contains `retentionDays: 90` and exactly two rows
`{key,label,description,canPost}` for `general` and `newbie`. `board.list`
contains `board`, `page`, `totalPages`, `totalPosts`, `capacity: 1000` and up to
ten summaries `{postId,board,author,subject,createdAt,expiresAt,canDelete,
canReport}`. `totalPages` is never less than one; a valid page past the total
contains `posts: []`. `board.entry` contains the same summary plus `body`.
`canDelete` is computed for the same author character and `canReport` for another
account while the global report does not yet exist.

The responses contain no account, durable character, SQL PK, ignore/block state
or body in the listing. Svelte inserts its strings as text, with no HTML,
Markdown or autolinks. Telnet offers the textual equivalent; if it negotiates
`Tiamud.Board 1`, it receives `Tiamud.Board.Catalog`, `.List` or `.Entry`, each
package under 8 KiB counting package, space and JSON. If the complete detail
exceeds that maximum due to JSON escaping, `Tiamud.Board.Entry` keeps the
summary and the largest UTF-8 prefix of `body` that fits, adds `truncated: true`
and
`overflow: {package:"Tiamud.Board.Entry",maximumBytes:8192,originalBytes}`.
`originalBytes` measures the UTF-8 body prior to escaping. The Telnet text keeps
the full body. Boards produce no `chat.message`, `message.append` or online
notification.

`vitals.update`, `inventory.patch` and `map.snapshot` replace their complete
corresponding fragment at the stated revision. Other player changes use a new
`player.snapshot`. A full scene/player snapshot may follow semantic updates to
keep recovery deterministic under output coalescing.

## Narrative, conversation and media

`message.append.data` contains `revision`, `kind` (`characterMoved`,
`itemTaken`, `itemDropped`, `itemGiven`, `tradeRequested`, `tradeChanged`,
`tradeCompleted`, `tradeCancelled`, `gameplay` or `adventure`), safe text,
opaque `actorId` and optional opaque `targetId`/`itemId`. It is projected
separately for each observer; not perceiving a domain event means not receiving
a message.

For `itemGiven`, `actorId` identifies the sender and `targetId` the recipient
with the observer's own salt. Sender and recipient additionally receive
`itemId`, derived with their session from the first transferred root; it is not
promised to be a durable stack identity. Another observer in the same room
receives the narration and the opaque characters, but not `itemId`; a remote
observer does not receive the event. The durable `ItemsGiven` event keeps at
most 256 internal root IDs, never exposed by this message.

Trade request, change, confirmation and cancellation produce narrative only for
their two participants; `tradeCompleted` adds a generic narration for observers
in the same room. No message exposes offers, quantities, currency, social
reason, `TradeId` or instance IDs. The private sheet and the inventory/economy
updates carry the authoritative state.

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

Telnet keeps the full textual narration. If it negotiated `Char.Items`, sender
and recipient receive inventory lists regenerated independently with their own
salt; the room observer does not receive someone else's list.

`chat.message.data` contains `channel` (`announcement`, `say`, `yell`, `shout`,
`tell`, `telepathy`, `emote`, `party`, `general`, `newbie` or `staff`), `sender`
and bounded plain text. Ignore and block by either party are applied before
delivery. Mute/ban and rate limit are applied on the server. `general`, `newbie`
and `staff` require a durable subscription in schema v13; `staff` additionally
requires a `moderator`, `administrator` or `implementor` profile when joining,
sending and receiving. `helper`, `builder` and the legacy roles
`Player`/`Moderator`/`Admin` do not grant that access. `announcement` is server
delivery, not a subscribable channel. The subscription is durable, but the
`reply` state and the history are memory only.

`telepathy` comes from `social.telepathy`, not from `tell`: the actor must know
the ability, pay resource and cooldowns, and be in a permitted room. The server
confirms that mutation before delivering. Ignore/block and the recipient's room
or private state may suppress delivery without changing the sender's
confirmation. The body, destination and `abilityId` are not included in durable
events; reception does not change the `reply` target.

`emote` comes from `social.emote`: `sender` identifies the actor and `text`
contains the complete phrase already rendered for that observer. The client does
not prepend the sender again. The command and name are discoverable through the
listing, but the template, the target and the IDs are not exposed as structured
message fields; the target's visible name may be part of the rendered text.
Ignore/block may suppress recipients without changing the actor's confirmation
or third parties' authorized perception. It creates no history, `reply`, durable
event, snapshot or SQL row.

`yell` and `shout` come only from `social.yell` and `social.shout`. The server
computes a directed, deterministic BFS that includes the origin room: radius one
for `yell`, radius three for `shout`, at most 259 unique rooms and zero
traversals of secret exits. The message contains no direction, distance, route
or origin room. Ignore, bilateral block and `condition.deaf` suppress each
recipient's copy for `say`/`yell`/`shout`; deafness does not prevent speaking
and the actor's confirmation does not change. Mute and the common bucket are
checked before running `OnTalk` once at the origin. Only a successful commit
starts the fan-out; rollback delivers zero copies. There is no retention,
replay, subscription, `reply`, SQL or snapshot for these channels.

`mail` is not a `chat.message`: it is queried on demand and persists up to 30
days in a 64-message/128 KiB mailbox. The per-account quota is 20 attempts per
hour and 50 per day. A delivery suppressed by ignore/block, ban or capacity uses
the same acceptance as a save; an unknown, ambiguous or own target uses a single
generic error. Only the recipient lists, reads, deletes or reports. A report
captures evidence for 90 days without enabling general mailbox browsing by
staff. Each account allows at most eight open mail reports and the system keeps
up to 10,000 active pieces of evidence.

The full persistence, privacy and retention decision is in
[ADR 0017](../decisions/0017-bounded-offline-mail.md).

Telnet represents the same authorized `SocialMessage` as bounded text. If it
negotiated GMCP `Comm`, it also receives `Comm.Channel.Text` with the same
scope, including `announcement`, `yell` and `shout`, and the `talker` and `text`
fields; GMCP does not bypass authorization nor create another source of truth.
When MXP is negotiated, text, senders and domain narrative are escaped as data
exactly once; plain text and the GMCP JSON keep the original content.

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
- semantic scene before the envelope: 14 KiB on Web; semantic scene used for
  text/GMCP/cells: 48 KiB on Telnet. A reduction keeps own, public and others'
  reserved loot first and always publishes a notice;
- projected events per batch: 16; an overflow is coalesced with a notice and an
  authoritative snapshot;
- Every post-authentication outbound WebSocket envelope is compact-serialized
  and checked against that same 16 KiB limit before entering the output queue.
  The scene is already reduced to its semantic window; any remaining envelope or
  individual private projection that still exceeds the limit fails closed with an
  explicit reason and a
  `tiamud_outbound_budget_exceeded_total{transport="websocket",message="frame"}`
  increment rather than emitting an invalid frame.
- JSON nesting: 16.
- `command.submit` WebSocket: 1,024 bytes; Telnet command line: 8 KiB, discarded
  entirely if it exceeds the maximum; request ID: 128; action target: 256;
  `recipient`: 64.
- chat: 512 bytes; mail target/subject/body/report reason: 64/128/2,048/2,048
  bytes; board subject/body/report reason: 128/4,096/2,048 bytes, ten posts
  per page and pages 1..=100; reason fields and all projected arrays/strings
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
