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

Client-to-server action schemas remain strict: a client cannot probe new
actions by sending unknown fields. Capability names are the explicit extension
point, but advertising an unknown capability has no effect until the server
supports it. Server-to-client evolution is additive only inside a recognized
message shape. The official client discards unknown server envelope and
payload fields while validating and retaining every known field; this allows
an optional presentation or informational field to roll out without changing
the meaning of the v1 message.

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
combat.attack, combat.ability
progression.promote, progression.talent
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
| `map.snapshot` | Complete current discovered map. |
| `message.append` | Perception-filtered narrative domain event. |
| `chat.message` | Authorized `say` or `tell` delivery. |
| `sync.scene`, `sync.player` | Correlated current scene/player recovery pair after a gap request. |
| `media.play`, `media.stop` | Optional opted-in safe audio transition. |
| `server.heartbeat` | Unsequenced best-effort liveness challenge. |

State messages are coalescible, while narrative output has lower priority than
critical results. When a connection cannot drain critical output within the
bounded queue/write timeout, the server closes it explicitly. An
actor/broadcast lag is recovered with a notice and authoritative snapshots.

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

Ability `kind` is `martial`, `spell` or `utility`; `targeting` is `self`,
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
- `economy`: coin, commodity slugs/quantities and available shop stock/actions;
- `social.online`: opaque IDs, display name and self marker.

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
- Every post-authentication outbound WebSocket envelope is compact-serialized
  and checked against that same 16 KiB limit before entering the output queue.
  An individually oversized scene/player projection fails closed with an
  explicit close reason and
  `tiamud_outbound_budget_exceeded_total{transport="websocket",message="frame"}`
  increment rather than emitting an invalid frame.
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
