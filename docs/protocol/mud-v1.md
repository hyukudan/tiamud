# Browser protocol `mud.v1`

Status: experimental. Breaking changes are allowed until the first public compatibility milestone.

## Transport

The browser connects to `/ws` and offers the WebSocket subprotocol `mud.v1`. Every message is one UTF-8 JSON text frame with this envelope:

```json
{
  "v": 1,
  "type": "scene.snapshot",
  "seq": 2,
  "data": {}
}
```

- `v` is the envelope major version.
- `type` identifies the payload shape.
- `seq` is a monotonically increasing server-session sequence when supplied.
- Unknown fields are ignored. Unknown message types do not become game commands.
- v1 reconnects with a fresh authoritative snapshot. It never replays an unconfirmed command automatically.

## Handshake

The first client message must be `client.hello`:

```json
{
  "v": 1,
  "type": "client.hello",
  "data": {
    "client": "web",
    "capabilities": {
      "images": ["avif", "webp", "jpeg"],
      "reducedMotion": true
    }
  }
}
```

The server answers with `server.welcome` and then a complete `scene.snapshot`. The current guest identity is temporary and is not an authentication design.

## Commands

```json
{
  "v": 1,
  "type": "command.submit",
  "data": {
    "requestId": "2e6d…",
    "command": "north"
  }
}
```

The server sends exactly one `action.result` for a syntactically valid envelope. `requestId` correlates the response with browser history; it is not currently a durable idempotency key. A successful action that changes the view is followed by a fresh `scene.snapshot`.

## Scene snapshot

```json
{
  "v": 1,
  "type": "scene.snapshot",
  "seq": 4,
  "data": {
    "viewId": "demo:lantern-square",
    "revision": 7,
    "title": "Lantern Square",
    "prose": ["Rain has polished the old stones."],
    "image": {
      "alt": "A rain-dark square lit by a blue lantern.",
      "avif": "/assets/…/scene-1280.avif",
      "webp": "/assets/…/scene-1280.webp",
      "jpeg": "/assets/…/scene-1280.jpg",
      "width": 1280,
      "height": 720
    },
    "exits": [
      {"id": "demo:north-gate", "label": "north", "command": "north"}
    ]
  }
}
```

`image` is optional and atmospheric. Its absence or failure must not hide an exit, entity, state change, or other actionable fact. The official client selects AVIF first, then WebP, then JPEG.

## Notices and limits

`server.notice` carries non-command information with an `info`, `warning`, or `error` level. The prototype bounds WebSocket frames and messages to 16 KiB, command strings to 1 KiB, request IDs to 128 bytes, and closes a session after three consecutive protocol errors. Per-IP rate limits are still required before public exposure.
