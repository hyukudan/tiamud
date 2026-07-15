# Web client

The browser client is a small Svelte 5 single-page application. It is an atmospheric companion to the authoritative text game: missing or failed images never prevent play.

## Development

Node.js 20.19+ and pnpm 10+ are required.

```sh
pnpm install
pnpm dev
```

The development server proxies `/ws` to `ws://127.0.0.1:8080`. A production build is static and expects the WebSocket endpoint on the same origin.

```sh
pnpm check
pnpm test
pnpm build
```

TypeScript 6.0.3 is intentionally pinned. TypeScript 7.0.2 builds the application, but
`svelte-check` 4.7.2 currently fails during startup because TypeScript 7 changed its
CommonJS export shape. The pinned version is the newest release verified by the commands
above; it can be upgraded once the checker supports TypeScript 7.

## Protocol slice

The client opens `/ws` with the `mud.v1` WebSocket subprotocol. On every connection it sends `client.hello` and expects a fresh `scene.snapshot`; it does not replay commands after reconnecting.

Handled server messages are `server.welcome`, `scene.snapshot`, `action.result`, and `server.notice`. Commands are sent as `command.submit` with a unique `requestId`.

Scene images prefer AVIF and fall back through WebP to JPEG. The parser accepts either direct `image` fields, `presentation.backdrop`, or an Envy-style `visual.variants` array while the backend contract settles.
