# Single-host authoring workflow

Tiamud is designed to run on one ordinary host. World authoring does not need
Kubernetes, a build farm or a live content service. A normal workflow uses the
repository, the Rust CLI and a reviewable asset directory on a developer or
builder machine, then deploys one validated artifact to the server.

```text
AreaPack sources + reviewed assets
                |
                v
        compose and validate
                |
                v
      test, preview and diff
                |
                v
       immutable publication
                |
                v
    one WorldPack loaded at startup
```

## Recommended loop

1. Work on the canonical `main` branch and edit the smallest owning AreaPack;
   Tiamud currently has no parallel development branch.
2. Compile new raster assets offline; record provenance, license and review.
3. Compose a fresh candidate path.
4. Validate, run embedded content tests and preview affected scenes.
5. Compare the candidate with the current reviewed artifact using `diff`.
6. Run repository tests and request human review.
7. Publish an immutable artifact, deploy it beside the binary and restart the
   single server intentionally.
8. Keep the prior artifact available for operational rollback.

The runtime never composes areas, invokes image encoders, calls an AI provider
or downloads third-party media. See the [single-host operations guide](../operations/single-host.md)
for service, database, TLS, backup and rollback procedures.

## Historical content

EnvyMUD/Merc files are offline source material, not runtime data. Use the
[conservative importer](../../tools/envy-import/README.md), inspect every
diagnostic and explicitly resolve provenance and licensing. Unsupported
records remain inert; imported programs are never executed.

## Optional AI

AI is an authoring assistant only. The default provider is `none`; local
Ollama or llama.cpp and explicit compatible HTTPS endpoints are optional. An
AI result is an untrusted draft, cannot publish itself and requires a separate
human decision. The complete bounded flow is documented in the
[editorial AI guide](../../crates/editorial-ai/README.md).

## Runtime boundary

The server loads either the embedded reviewed pack or the external path named
by `TIAMUD_WORLD_PACK`. It loads one complete artifact before opening game
ports. Area hot-loading remains future architecture, not part of the current
authoring or deployment path.
