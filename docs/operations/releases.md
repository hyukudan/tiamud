# Release construction and verification

The repository contains a tagged-release workflow, but no actual published
release is claimed as evidence by this document. When a matching tag succeeds,
artifacts are produced by the
[`Reproducible release`](../../.github/workflows/release.yml) workflow. A tag
must be named `v<workspace-version>`. The workflow checks out that exact
revision, derives `SOURCE_DATE_EPOCH` from its commit, builds in two isolated
target directories, packages both results, and rejects the release unless the
two package directories are byte-for-byte identical.

The packaging script fixes archive ordering, timestamps, numeric ownership,
gzip metadata, executable modes, and web file modes. It rejects symbolic links
and special files in the web distribution, and requires the Tiamud MIT,
Apache-2.0 and original-content license notices in both bundles. The workflow
is designed to retain:

- `tiamud-<version>-x86_64-unknown-linux-gnu.tar.gz` with the server and tools;
- `tiamud-web-<version>.tar.gz` with the browser distribution;
- `SHA256SUMS` with portable, basename-only SHA-256 entries;
- `REPRODUCIBILITY.txt` with the tested revision and source epoch;
- `tiamud-<version>.intoto.jsonl` with OIDC-signed build provenance.

The generated provenance is the release authenticity mechanism. GitHub Actions obtains a
short-lived OIDC identity, and the attestation records the repository, workflow,
revision, and artifact digests. There is deliberately no long-lived signing key
on a build host and no separate GPG signature to trust in place of provenance.

The three bundled project licenses do not replace notices required by Rust or
web dependencies. `THIRD_PARTY_NOTICES.md` is generated from the release Cargo
graph and the complete frozen pnpm graph, embeds the license/notice texts
shipped by those packages, and is included in both archives. Regenerate and
review it whenever either lockfile changes:

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

## Consumer verification

Download both archives, `SHA256SUMS`, and the attestation bundle into one clean
directory. First verify the bytes:

```sh
sha256sum --check --strict SHA256SUMS
```

Then verify each archive against the expected repository and the portable
bundle. Replace `OWNER/REPOSITORY` only after the canonical public repository
has been established:

```sh
repository=OWNER/REPOSITORY
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
