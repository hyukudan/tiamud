# Tiamud scripting runtime

This crate executes WorldPack Lua 5.4 hooks in a fresh VM with explicit host
capabilities and hard defaults: 64 KiB source, 2 MiB VM memory, 100,000
instructions, 25 ms wall time, 32 emitted events, and 1 KiB per event field.

Only table, bounded string/math helpers, and safe base operations are loaded.
Filesystem, network, processes, package/native loading, binary chunks, and
dynamic `load` are unavailable. Lua pattern functions (`find`, `match`,
`gmatch`, `gsub`) are removed because their C implementation cannot be
preempted by the VM instruction hook. `math.random` and `math.randomseed` are
also removed: authored randomness must use the host-seeded deterministic
`tiamud.random`. `print`, `warn`, and `collectgarbage` are unavailable, avoiding
host output injection and another uninterruptible C boundary. The only mutation
boundary is `tiamud.emit`, whose event kinds must be granted up front. Each
execution is discarded after one hook.

WorldPack authoring rejects more than eight room-local modules for one hook and
the server enforces the same shared cap. A batch gets at most 50 ms, and all
scripting in one persisted world tick gets at most 100 ms. Per-module
instruction, memory, event, and effective remaining wall-time quotas still
apply inside those aggregate limits. If the global tick budget is exhausted,
the server discards every partial script effect and narration, restores the
checkpoint, and commits the base tick without hooks so logical time cannot
freeze.

Supported hooks are `on_enter`, `on_leave`, `on_look`, `on_use`, `on_talk`,
`on_combat_start`, `on_defeat`, `on_timer`, and `on_quest_event`.

The runtime returns inert `ScriptEvent` values. The authoritative world decides
whether and how to translate them into domain actions; scripts never receive a
direct reference to game state.

`call_function` and `validate_functions` are a generic sibling API for
non-WorldPack hosts (StoryPack scripts, ADR 0034): they call a named global
function with bounded copied `ScriptValue` arguments (depth 4, 16,384 values,
1 KiB strings), accept only a caller-supplied closed set of event kinds and
return the first scalar result. The same sandbox, hardening and quotas apply.
