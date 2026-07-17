# Release construction and verification

The repository contains a tagged-release workflow, but no actual published
release is claimed as evidence by this document. When a matching tag succeeds,
artifacts are produced by the
[`Reproducible release`](../../.github/workflows/release.yml) workflow. A tag
must be named `v<workspace-version>`. The workflow checks out that exact
revision, derives `SOURCE_DATE_EPOCH` from its commit, builds in two isolated
target directories, packages both results, and rejects the release unless the
two package directories are byte-for-byte identical.

Publication is create-only. If a GitHub release already exists for the tag,
the workflow fails instead of uploading into it or replacing any published
asset; investigate the existing release and use a new version for changed
bytes.

The packaging script fixes archive ordering, timestamps, numeric ownership,
gzip metadata, executable modes, and web file modes. It rejects symbolic links
and special files in the web distribution. Both bundles must contain the root
software/content boundary notice, the complete MIT and Apache-2.0 texts, the
original-content boundary notice, and the complete unmodified CC BY 4.0 legal
code. CI and release packaging pin the reviewed MIT text and verify the
vendored Apache-2.0 and CC BY 4.0 legal codes against their official-text
digests. The workflow is designed to retain:

- `tiamud-<version>-x86_64-unknown-linux-gnu.tar.gz` with the server and tools;
- `tiamud-web-<version>.tar.gz` with the browser distribution;
- `SHA256SUMS` with portable, basename-only SHA-256 entries;
- `REPRODUCIBILITY.txt` with the tested revision and source epoch;
- `tiamud-<version>.intoto.jsonl` with OIDC-signed build provenance.

The generated provenance is the release authenticity mechanism. GitHub Actions obtains a
short-lived OIDC identity, and the attestation records the repository, workflow,
revision, and artifact digests. There is deliberately no long-lived signing key
on a build host and no separate GPG signature to trust in place of provenance.

The five bundled project licensing files do not replace notices required by
Rust or web dependencies. `THIRD_PARTY_NOTICES.md` is generated from the
release Cargo graph and the complete frozen pnpm graph, embeds the
license/notice texts shipped by those packages, and is included in both
archives. Regenerate and review it whenever either lockfile changes:

```sh
pnpm --dir apps/web install --frozen-lockfile
node scripts/generate-third-party-notices.mjs
node scripts/generate-third-party-notices.mjs --check
```

The generator also enforces `scripts/third-party-license-policy.json`. An
unreviewed declared-license expression, a dependency without a discoverable
notice, a stale lockfile digest, or a stale generated file fails closed. The
release workflow installs the frozen web graph and runs the same check before
building; the packaging script also runs the check and requires the reviewed
notice file in every archive, including local reproducibility audits.

Release archives are mixed artifacts. The program source/binaries,
configuration and scripts remain `MIT OR Apache-2.0`; original narrative,
documentation prose, artwork/audio and derived media renditions retain
`CC-BY-4.0`. Packaging the categories together does not relicense either one.

## Generaciones de área y release 6C

La procedencia del binario no autentica por sí sola un registro de áreas del
operador. Catálogo, fallback y bundles conservan sus digests, licencias y
revisión propios; `unsigned_local` sigue presuponiendo un host confiable y no se
convierte en firma remota porque el CLI viaje dentro de un archivo atestado.

Antes de actualizar el binario de una instalación en modo catálogo, conserva
la raíz del registro, el catálogo activo y el anterior y una copia verificada
del backend. SQL v14 referencia la generación por digest, no por una ruta
incluida en el paquete: borrar esos blobs puede impedir el siguiente arranque o
un rollback. `area-select` solo selecciona una generación local ya publicada;
no descarga contenido ni lo obtiene del release de GitHub.

La sustitución quiescente 6C, su migración v14 y un rollback explícito deben
probarse como parte del candidato instalado según la
[guía de un único host](single-host.md#sustitución-quiescente-de-generaciones-de-área).
Una actualización 6B/v13 sin liga pero con recovery compartido o cualquier
personaje debe ligar G0 una sola vez mediante
`TIAMUD_ACK_LEGACY_AREA_GENERATION=<sha256-exacto-G0>` y retirar la variable.
Cuentas solas no son autoridad de mundo; un release tampoco convierte
implícitamente una autoridad monolítica existente a catálogo.
La puerta final 6C está retenida; PostgreSQL real no se abrió sin URL. Esa
evidencia y una futura atestación remota son pruebas distintas.

## Consumer verification

Download both archives, `SHA256SUMS`, and the attestation bundle into one clean
directory. First verify the bytes:

```sh
sha256sum --check --strict SHA256SUMS
```

Then verify each archive against the canonical repository and the portable
bundle:

```sh
repository=hyukudan/tiamud
# Set this to the exact workspace/package version in the release tag.
version=0.1.0
for artifact in tiamud-*.tar.gz; do
  gh attestation verify "$artifact" \
    --repo "$repository" \
    --bundle "tiamud-${version}.intoto.jsonl" \
    --signer-workflow "${repository}/.github/workflows/release.yml" \
    --source-ref "refs/tags/v${version}"
done
```

Verification must fail closed: do not install an archive when its checksum,
repository identity, provenance signature, or expected tag/version does not
match. Extract into a new staging directory, run `tiamud-admin check`, and only
then follow the update and rollback procedure in
[the single-host runbook](single-host.md).

## Local reproducibility audit

The release workflow is the authoritative audit because it supplies the pinned
toolchains and attestation identity. A maintainer can reproduce its packaging
step locally by building the same revision twice with the same
`SOURCE_DATE_EPOCH`, invoking `scripts/package-release.sh` for both isolated
target directories, and comparing the output directories with:

```sh
diff --no-dereference --recursive build/first/packages build/second/packages
```

An empty diff is necessary but not sufficient for publication: a tagged
release must pass the locked notice/policy check, pass the workflow, publish its
signed provenance, and be independently verified with the commands above.
Until that happens for a specific tag, release acceptance remains partial.
