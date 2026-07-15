# Tiamud optional editorial AI

This crate is an authoring-only adapter for `none` (the safe default), Ollama,
llama.cpp, and an OpenAI-compatible HTTP endpoint. It is intentionally not a
dependency of `tiamud-server`; provider failure cannot affect login, simulation,
or play.

Requests and responses are bounded, redirects and ambient proxies are disabled,
timeouts are mandatory, local providers are restricted to loopback, and remote
compatible endpoints require HTTPS. Authorization values are redacted from
debug output. Returned text is an untrusted `EditorialSuggestion` and is not
publishable until a named human review explicitly accepts it. Provider/model,
task, provenance, reviewer, decision, and review timestamp remain attached to
the result.

The adapter never downloads a model and never invokes a process. Operators run
their chosen provider separately and call this library only from offline
authoring tools.

## Authoring workflow

`tiamud-world ai-draft`, enabled by the authoring-only `editorial-ai` Cargo
feature, is the supported CLI integration. The feature is disabled for normal
WorldPack library consumers, so `tiamud-server` does not inherit the HTTP client
or an AI adapter. The command reads bounded UTF-8 input files, invokes the
explicitly selected provider, and creates a new JSON artifact in `draft` state.
The safe default provider is `none`, so omitting the provider fails without
making a network request or writing an artifact.

```sh
cargo run -p world-pack --features editorial-ai --bin tiamud-world -- ai-draft \
  --provider ollama \
  --endpoint http://127.0.0.1:11434/ \
  --model gemma3 \
  --task draft-description \
  --input notes/room-prompt.txt \
  --context worlds/lantern-square.world.json \
  --output reviews/room-draft.json
```

For an authenticated OpenAI-compatible endpoint, put the complete Authorization
header in an environment variable and pass only its name with
`--authorization-env`; secrets are not accepted as command-line values. Remote
compatible endpoints require HTTPS. Ollama and llama.cpp endpoints are limited
to loopback.

After inspecting the draft, a named reviewer records an explicit decision in a
different file:

```sh
cargo run -p world-pack --features editorial-ai --bin tiamud-world -- ai-review \
  reviews/room-draft.json \
  --reviewer "World editor" \
  --reviewed-at 2026-07-15T00:00:00Z \
  --decision accept \
  --output reviews/room-reviewed.json
```

Neither command edits a WorldPack, activates a registry pointer, or calls the
publish path. Existing output files are never replaced. Even an accepted review
remains a separate authoring artifact that an editor must apply manually and
validate through the normal WorldPack workflow.
