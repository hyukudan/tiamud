# ADR 0002: Runtime world data and durable player identity

- Status: Accepted; transitional clauses completed
- Date: 2026-07-15

## Context

The bootstrap server constructed its demo graph in Rust and used a character name as the SQLite primary key. That proved the transport and persistence boundaries but could not support authored worlds, account-owned rosters, stable character selection, or a media reference that belongs to content rather than client code.

This ADR records the intermediate runtime-world and durable-identity decision. Statements below about a compatibility entry path and implicit start-room convention describe that checkpoint, not the current alpha.

## Decision

- Load one bounded, validated WorldPack before starting network listeners. `TIAMUD_WORLD_PACK` selects it and the server fails startup on invalid authoritative world data.
- Convert authoring data into transport-independent `WorldDefinition` values before the single-owner actor starts.
- Keep stable room IDs compatible with persisted locations. Missing saved rooms resolve to the pack's declared start room and are repaired durably.
- Introduce durable SQLite account and character IDs. Names are case-insensitive presentation/lookup data, not identity.
- Migrate the bootstrap name-keyed character table transactionally into account-owned character rows without losing room locations.
- Keep authentication and account session policy separate from durable identity. At this checkpoint, the existing unauthenticated WebSocket and Telnet entry remained only as a temporary compatibility path until authenticated selection was integrated.
- Let a WorldPack room reference a safe relative atmospheric asset manifest. The web client validates the content-addressed manifest and uses responsive AVIF/WebP/JPEG sources.
- Treat all media failure as optional: missing, malformed, or broken assets remove only the illustration and never the narrative or actions.

## Consequences

World content and scene media are no longer compiled into domain code or mapped by room ID in the browser. Accounts can own multiple characters and character ownership is checked before entry. Startup depends on a valid configured pack, which is deliberate because silently substituting different authoritative world data would be unsafe.

## Follow-up status

The authenticated exploration alpha completed the transition recorded here:

- WorldPack v2 requires and validates an explicit `startRoomId`, plus item definitions and initial item placement.
- Web and Telnet now require registered-account authentication and durable character creation/selection.
- Passwords use Argon2id; revocable bearer sessions expire after 30 days and persist only a token digest.
- The actor rejects a second live connection to the same character. Disconnect releases that lease, and reconnect returns a fresh authoritative snapshot.
- Character position and complete per-character item locations persist through take/drop actions, reconnects, and restarts.

The temporary unauthenticated transport entry described by this ADR is no longer the current behavior. The broader engine remains incomplete; the project's private completion records track the active gaps without changing this accepted decision.
