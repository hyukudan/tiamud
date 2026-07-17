# Tiamud web client

The official Svelte 5 client is an accessible, responsive companion to the
authoritative text game. Images and audio are optional presentation: disabling
them or losing an asset never removes an action or gameplay fact.

## Development

The client package supports Node.js 20.19+ (or 22.12+) and pnpm 10+. The
repository's load and soak tooling requires Node.js 24, which is also used by
the pinned complete-project verification profile.

```sh
pnpm install --frozen-lockfile
pnpm dev
```

Vite proxies `/api` and `/ws` to `127.0.0.1:8080`. A production build is static,
uses same-origin HTTP/WebSocket endpoints and is embedded in `tiamud-server`.

```sh
pnpm check
pnpm test
pnpm build
pnpm exec playwright install --with-deps chromium firefox webkit
pnpm e2e
```

TypeScript 6.0.3 is pinned because the current `svelte-check` release is not
compatible with the TypeScript 7 CommonJS export shape.

## User-facing implementation

Before connecting, the client provides registration/login, durable session
restoration/revocation, roster loading, optional archetype-aware character
creation/selection and a text-complete welcome screen. After selection it
exposes:

- atmospheric scene prose, localized alternative text, exits, items, NPCs and
  weather;
- 500-entry feed/command scrollback that preserves a reader's position and
  offers an explicit jump-to-latest control, plus structured contextual actions
  and free-text commands;
- a complete character sheet with archetype, level/XP threshold, legacy and
  authored attributes, vitals, resource meters, skills, resistances and active
  statuses;
- known abilities with kind, description, bounded resource costs, cooldown,
  availability and targeting, plus keyboard-native target selection and use;
- inventory/equipment, discovered map, quests, parties, economy, chat, who and
  ignore controls;
- a separate offline-mail inbox with bounded paging, read/delete, send and
  report controls; it does not merge durable correspondence into chat history;
- a separate “Tablones” panel for the closed `general` and `newbie` community
  boards, with bounded public summaries, text-only detail, 90-day retention,
  author deletion and evidence-backed reporting; it never interprets HTML,
  Markdown, links or remote content;
- persistent validated command history (100 entries), suggestions and
  up/down keyboard recall;
- responsive panels and a fixed reachable command form on narrow screens;
- persistent image, audio, separate music/ambience/effects volume, motion,
  Save-Data, contrast, text-size and screen-reader preferences.

Audio starts disabled. Enabling it is the required user gesture; the controller
then accepts only same-origin content-addressed audio URLs announced by the
server. Save-Data and screen-reader mode prevent scene images from being
requested. Motion preferences combine the system setting and the manual mode.

The production Service Worker caches at most 32 same-origin shell entries. It
does not cache API, WebSocket, image or audio traffic and deletes old cache
versions on activation.

## Archetypes and abilities

After account access, the client requests the public `GET /api/archetypes`
catalog alongside the authenticated roster. When choices are advertised, the
creation form presents their label and description. Its initial `World default`
choice omits `archetypeId`, allowing the authoritative authored default to win;
an explicitly selected class sends its ID in `POST /api/characters`. A
`404`/`405` catalog response is
treated as a pre-archetype server: the form remains usable and sends the exact
legacy `{name}` body. Other malformed/error responses remain visible failures
instead of being silently trusted.

The catalog deliberately does not expose initial attributes, resources or
abilities, so creation currently previews only label and description. The
complete authoritative sheet arrives in `player.snapshot` after entry. A richer
initial-stat preview requires an additive catalog contract and remains pending.

The WebSocket character projection is additive for old servers. Missing
archetype, authored attributes, resources, abilities, resistances or statuses
normalize to empty/absent presentation fields, while current servers are
strictly bounded against the core schema. Strength and defence remain available
as legacy scalar fields; their entries in the generic attribute list are
de-duplicated in the rendered sheet.

Ability buttons use the structured v1 action rather than synthesizing a text
command:

```json
{
  "v": 1,
  "type": "action.perform",
  "data": {
    "requestId": "<idempotency-key>",
    "action": "combat.ability",
    "abilityId": "ember-bolt",
    "target": "<optional-opaque-npc-id>"
  }
}
```

The client never treats projected `usable` as authorization. The server still
validates the known ability, resource balance, cooldown, recovery/status state,
target and combat engagement. See the wire contract for the exact character
shape, enums and limits.

## Protocol and recovery

The client opens `/ws` with subprotocol `tiamud.v1` and authenticates its first
`client.hello` with the bearer token and character ID. It declares image,
reduced-motion and audio capabilities.

Changing a declared presentation capability performs a clean reconnect and a
new v1 hello without clearing the scene or scrollback. A typed pre-welcome
access rejection stops automatic reconnect and returns to the account flow.
Only an invalid or expired session clears local credentials; capacity,
selection and temporary operational rejections retain the reusable account
session.

It accepts complete scene/player snapshots and typed scene/entity/vitals/
inventory/map/chat/mail/board/media updates. Server messages are bounded and validated;
opaque view/entity IDs are required where appropriate. It discards duplicate
or backwards sequence numbers, requests `sync.snapshot` after a gap and answers
unsequenced server heartbeats. A 45-second silence watchdog closes the socket so
the bounded exponential reconnect can obtain a fresh authoritative snapshot.
An unanswered synchronization request similarly reconnects after ten seconds,
preventing the client from suppressing state snapshots forever.
The client never automatically replays an unconfirmed command.

The server keeps a short bounded result cache by character and `requestId`, so
an explicitly repeated identical request returns the prior result; reusing an
ID for a different action is a protocol error. General event replay is
intentionally outside v1.

See the authoritative [wire contract](../../docs/protocol/tiamud-v1.md) and
[completion matrix](../../docs/project/COMPLETION_MATRIX.md).

## Accessibility evidence

The application uses semantic landmarks, a `role="log"` narrative history,
selective live announcements, meters, keyboard-operable native controls,
forced-colors styles, configurable text and reduced-motion/reader modes.
Playwright includes axe A/AA/WCAG 2.2 tags, keyboard, mobile overflow and fixed
command checks for Chromium, Firefox and WebKit in CI.

This automation is not a substitute for a real assistive-technology pass. A
manual screen-reader audit is still pending. All 26 current Chromium and
Firefox scenarios passed together in the final combined invocation. WebKit
discovers its thirteen scenarios but could not load
on the most recent validation host because the required GTK/GStreamer, media
and related runtime libraries were absent; this is neither an observed
application failure nor a current WebKit pass. A retained
historical log records an earlier WebKit run with a temporary user-space
host-library setup. The authoritative current status is kept in the
[completion matrix](../../docs/project/COMPLETION_MATRIX.md).

## License

The client source, tests, configuration, scripts and built JavaScript/CSS are
software available under `MIT OR Apache-2.0`. Original bundled artwork/audio
and their derived image/audio renditions use `CC-BY-4.0`. A production web
distribution can contain both categories; inclusion does not license the
software under CC BY 4.0 or the media under the software licenses. See
[`LICENSE`](../../LICENSE) and
[`LICENSE-CONTENT.md`](../../LICENSE-CONTENT.md) for the complete boundary and
[`LICENSE-CC-BY-4.0`](../../LICENSE-CC-BY-4.0) for the unmodified CC legal
code.
