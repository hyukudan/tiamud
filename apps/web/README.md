# Tiamud web client

The official Svelte 5 client is an accessible, responsive companion to the
authoritative text game. Images and audio are optional presentation: disabling
them or losing an asset never removes an action or gameplay fact.

## Development

Node.js 20.19+ (or 22.12+) and pnpm 10+ are required.

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
restoration/revocation, roster loading, character creation/selection and a
text-complete welcome screen. After selection it exposes:

- atmospheric scene prose, localized alternative text, exits, items, NPCs and
  weather;
- feed/command history, structured contextual actions and free-text commands;
- vitals, skills, effects, inventory/equipment, discovered map, quests,
  parties, economy, chat, who and ignore controls;
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

## Protocol and recovery

The client opens `/ws` with subprotocol `tiamud.v1` and authenticates its first
`client.hello` with the bearer token and character ID. It declares image,
reduced-motion and audio capabilities.

It accepts complete scene/player snapshots and typed scene/entity/vitals/
inventory/map/chat/media updates. Server messages are bounded and validated;
opaque view/entity IDs are required where appropriate. It discards duplicate
or backwards sequence numbers, requests `sync.snapshot` after a gap and answers
unsequenced server heartbeats. A 45-second silence watchdog closes the socket so
the bounded exponential reconnect can obtain a fresh authoritative snapshot.
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
command checks across Chromium, Firefox and WebKit in CI.

This automation is not a substitute for a real assistive-technology pass. A
manual screen-reader audit is still pending. Chromium, Firefox and WebKit have
all passed the automated suite; the retained project log records WebKit's
temporary user-space host-library setup without treating it as a runtime
dependency.

## License

The client software is available under `MIT OR Apache-2.0`. Original bundled
artwork and compiled media use `CC-BY-4.0`; see the repository root license
files for the complete boundary and attribution requirements.
