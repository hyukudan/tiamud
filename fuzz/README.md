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

The Web envelope/depth parser and the Telnet IAC/subnegotiation parser currently
live as private types inside the `tiamud-server` binary. The Web boundary is 16
KiB and 16 JSON levels; Telnet bounds commands at 4 KiB and subnegotiations at 16
KiB. Their deterministic unit tests cover malformed JSON, depth limits, split IAC
commands, escaped IAC bytes, bare `SE`, and oversized subnegotiations. They are
intentionally not copied into this crate: a duplicate parser would fuzz a model
rather than the production surface. Moving those parsers into a small public
library module is the required prerequisite for direct libFuzzer coverage.

Install the runner with a pinned Cargo resolution, a C++ compiler (required by
`libfuzzer-sys`), and a recent nightly:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo +nightly fuzz build
```

Run a bounded local campaign from the repository root:

```sh
cargo +nightly fuzz run world_pack_document -- -max_len=1048576 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run envy_import -- -max_len=1048576 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run asset_manifest -- -max_len=1048576 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
cargo +nightly fuzz run action_parser -- -dict=fuzz/dictionaries/action_parser.dict -max_len=65536 -timeout=10 -rss_limit_mb=2048 -max_total_time=300
```

Crashes are written below `fuzz/artifacts/<target>/`. Re-run a single artifact
by passing its path after the target name, then minimize it with `cargo fuzz
tmin`. Keep only small, non-sensitive regression inputs in `fuzz/corpus/`.

GitHub-hosted fuzz jobs should use the same length, timeout, and RSS bounds plus
an explicit `-max_total_time`; long-running external fuzzing is intentionally
outside the release workflow.
