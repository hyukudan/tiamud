# Tiamud scripting runtime

This crate executes WorldPack Lua 5.4 hooks in a fresh VM with explicit host
capabilities and hard defaults: 64 KiB source, 2 MiB VM memory, 100,000
instructions, 25 ms wall time, 32 emitted events, and 1 KiB per event field.

Only table, string, math, and safe base operations are loaded. Filesystem,
network, processes, package/native loading, binary chunks, and dynamic `load`
are unavailable. The only host boundary is `tiamud.emit`, whose event kinds
must be granted up front. `tiamud.random` uses a host-provided deterministic
seed. Each execution is discarded after one hook.

Supported hooks are `on_enter`, `on_leave`, `on_look`, `on_use`, `on_talk`,
`on_combat_start`, `on_defeat`, `on_timer`, and `on_quest_event`.

The runtime returns inert `ScriptEvent` values. The authoritative world decides
whether and how to translate them into domain actions; scripts never receive a
direct reference to game state.
