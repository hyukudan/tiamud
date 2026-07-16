# Tiamud fuzz harnesses

These `cargo-fuzz` targets exercise only public, bounded parser surfaces. Every
target rejects inputs above 1 MiB before invoking product code.

- `world_pack_document` parses WorldPack authoring JSON and, for successful
  parses, exercises validation, canonical building, and embedded content tests.
- `envy_import` uses bits from the first input byte to select audit-only, ASCII
  import, or UTF-8 import paths. The complete input is always treated as inert
  historical area content.
- `asset_manifest` parses the public asset-manifest type and checks its JSON
  round trip. The asset pipeline has no public manifest validator, so this target
  deliberately makes no semantic-validity claim and never decodes image bytes.
- `action_parser` feeds bounded UTF-8 command text to the transport-independent
  gameplay parser and verifies that surrounding whitespace cannot change its
  result.
- `telnet_parser` feeds the production incremental Telnet codec both whole and
  in input-derived fragments, compares its application bytes and events, and
  enforces the 16 KiB subnegotiation bound.

The Telnet IAC/subnegotiation parser lives in the internal, non-publishable
`tiamud-telnet-codec` workspace crate. `tiamud-server` and this fuzz package both
depend on that single implementation; no fuzz-only parser copy exists. Its API
is deliberately limited to byte framing, events, protocol command constants,
and the public 16 KiB bound. Negotiation policy and game commands remain private
to the server. The Web envelope/depth parser is still private to the server
binary and retains deterministic unit coverage for its 16 KiB/16-level bounds.

Install the runner with a pinned Cargo resolution, a C++ compiler (required by
`libfuzzer-sys`), and a recent nightly:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo +nightly fuzz build
```

Without a C++ compiler, the Rust target and dependency graph can still be
checked without linking libFuzzer:

```sh
cargo +nightly check --manifest-path fuzz/Cargo.toml --bin telnet_parser \
  --no-default-features --locked
```

Run a bounded local campaign from the repository root:

```sh
cargo +nightly fuzz run world_pack_document -- -max_len=1048576 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run envy_import -- -max_len=1048576 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run asset_manifest -- -max_len=1048576 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run action_parser -- -dict=fuzz/dictionaries/action_parser.dict -max_len=65536 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run telnet_parser -- -dict=fuzz/dictionaries/telnet_parser.dict -max_len=262144 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
```

Crashes are written below `fuzz/artifacts/<target>/`. Re-run a single artifact
by passing its path after the target name, then minimize it with `cargo fuzz
tmin`. Keep only small, non-sensitive regression inputs in `fuzz/corpus/`.

The weekly/manual `fuzz.yml` workflow runs the production Telnet parser for a
bounded 120-second campaign with the checked-in dictionary and retains any
crash artifact. Long-running external fuzzing remains intentionally outside
the release workflow.
