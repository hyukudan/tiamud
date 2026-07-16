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

Authenticated endpoints require `Authorization: Bearer <token>`.
`POST /api/characters` accepts `{"name":"Orin"}`. Account/character names are
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
action by themselves.

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
- Unknown client envelope/payload fields are rejected. Server/client parsers
  bound arrays, strings, URLs and numeric values.
- JSON object/array nesting may not exceed 16 levels.

`server.heartbeat` is deliberately unsequenced so best-effort liveness traffic
cannot displace an accepted command result.

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

The command is at most 1024 bytes without control characters. It reaches the
same `PlayerAction` parser as Telnet.

### Structured action

```json
{
  "v": 1,
  "type": "action.perform",
  "data": {"requestId":"cmd-28","action":"item.take","target":"o51a4c0311f262a90"}
}
```

Implemented action names are:

```text
look, inventory, move
item.take, item.drop, item.examine, item.use, item.equip, item.unequip
combat.attack
party.create, party.invite, party.accept, party.leave, party.status
quest.list, quest.accept, quest.status, quest.complete
economy.balance, economy.shop, economy.buy, economy.sell
weather, time
social.say, social.tell, social.who, social.help
social.ignore, social.unignore, social.block, social.unblock, social.report
moderation.mute, moderation.unmute, moderation.ban, moderation.unban,
moderation.role
```

Depending on the action, the strict payload may also contain `message`,
`durationSeconds`, `reason` or `role` (`player`, `moderator`, `admin`). Unexpected
fields or a missing/forbidden target are rejected. The normal authorization and
perception checks still apply; structured actions are not trusted shortcuts.

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

The server responds with a coalescible, critical `sync.snapshot` containing the
current authorized scene/player state and echoes `afterSeq`. v1 intentionally
does not retain/replay missing events. The client discards duplicate/backwards
sequences and suppresses stale snapshots while a sync is pending. The official
client closes and reconnects if no sync response arrives within ten seconds.

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
| `vitals.update` | Complete current character/vitals/skills/effects fragment. |
| `inventory.patch` | Complete current inventory/equipment fragment. |
| `map.snapshot` | Complete current discovered map. |
| `message.append` | Perception-filtered narrative domain event. |
| `chat.message` | Authorized `say` or `tell` delivery. |
| `sync.snapshot` | Current scene and player state after a gap request. |
| `media.play`, `media.stop` | Optional opted-in safe audio transition. |
| `server.heartbeat` | Unsequenced best-effort liveness challenge. |

State messages are coalescible; narrative is lower priority than critical
results. When a connection cannot drain critical output within the bounded
queue/write timeout, the server closes it explicitly. An actor/broadcast lag is
recovered with a notice and authoritative snapshots.

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
      "actions":[
        {"label":"Examine","command":"examine of75b645d851f21cf"},
        {"label":"Take","command":"take of75b645d851f21cf"}
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

`image` is optional. Its content-addressed root-relative URLs and contextual
localized alt text are presentation only. The official client resolves exact
locale, language, English, then deterministic first locale. Failure removes
only the image.

`scene.patch` always includes `viewId` and `revision` plus one or more of
`title`, `prose`, `image` (or `null`), `exits` and `weather`.
`entity.upsert/remove` also includes view/revision, `kind` (`item` or `npc`) and
opaque entity `id`.

## Player state

`player.snapshot` is the complete private projection:

- `revision`;
- `character`: name, level, experience/next threshold, health/max health,
  strength, defence, skills, effects and optional recovery countdown;
- `inventory` and `equipment`, with opaque item IDs and authorized actions;
- `map`: opaque current/known room IDs and known/unknown paths;
- `quests`: stable public content slug, description, status/progress/actions;
- optional `party` and `partyInvites`, with opaque character IDs;
- `economy`: coin, commodity slugs/quantities and available shop stock/actions;
- `social.online`: opaque IDs, display name and self marker.

Quest/commodity slugs are stable public content identifiers rather than hidden
world entity IDs. Persistence-only moderation/account state and private script
flags are absent.

`vitals.update`, `inventory.patch` and `map.snapshot` replace their complete
corresponding fragment at the stated revision. Other player changes use a new
`player.snapshot`. A full scene/player snapshot may follow semantic updates to
keep recovery deterministic under output coalescing.

## Narrative, chat and media

`message.append.data` contains `revision`, `kind` (`characterMoved`,
`itemTaken`, `itemDropped`, `gameplay` or `adventure`), safe text, opaque
`actorId`, and optional opaque `targetId`/`itemId`. It is projected separately
for every observer; inability to perceive a domain event means no message.

`chat.message.data` contains `channel` (`say`/`tell`), `sender` and bounded
plain text. Recipient ignore and either-side block policy is applied before
delivery. Muted/banned state and chat rate limits are enforced server-side.

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

## Limits and failure policy

- HTTP/WS body, frame and message: 16 KiB.
- JSON nesting: 16.
- command: 1024 bytes; request ID: 128; action target: 256.
- chat: 512 bytes; reason fields and all projected arrays/strings are bounded.
- initial hello timeout: 10 seconds; WebSocket write timeout: 5 seconds.
- three consecutive protocol/rate violations close the game session.
- `tiamud.v1` is required; browser `Origin`, when present, must exactly match
  the HTTP `Host`. Non-browser clients may omit `Origin` but still authenticate
  inside `client.hello`.
- static responses carry CSP, `X-Content-Type-Options: nosniff` and safe cache
  policy; external/data/file media URLs are rejected.

Errors disclose safe player-facing state only. Internal IDs, storage errors,
credentials, moderation internals and condition state are not serialized.
