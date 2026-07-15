# ADR 0004: Open-source and original-content licensing

- Status: Accepted
- Date: 2026-07-15

## Context

Tiamud is ready to be shared for collaboration. The repository contains both
software and project-authored narrative/media, while the historical importer
may process third-party material whose ownership cannot be inferred. The
license boundary must therefore be explicit before pushing the completed v0.2
work for review.

## Decision

- License program source code, tests, built binaries, configuration,
  build/deployment material and scripts under the SPDX expression
  `MIT OR Apache-2.0`. Recipients may choose either license and need not comply
  with both.
- License original Tiamud narrative/world content, documentation prose,
  artwork, audio and derived image/audio renditions under `CC-BY-4.0` with
  attribution to Tiamud contributors.
- Treat a source tree, executable or release bundle containing both categories
  as a mixed artifact: the software portions remain dual-licensed and each
  content component retains CC BY 4.0. Neither grant is applied to the whole
  bundle in a way that relicenses the other category.
- Record the software expression in every Rust package and the official web
  package. Ship the root boundary notice, all three complete license texts and
  the content boundary notice in release archives.
- Keep asset-level provenance and license metadata even when the repository-
  level content license applies.
- Do not relicense historical or third-party input. `envy-import` must continue
  to mark unknown rights as requiring review, and publication requires its own
  compatible license or permission.
- Do not treat these copyright licenses as trademark registration or exclusive
  clearance of the Tiamud name.

## Consequences

The repository and its original default world can be redistributed and
modified under standard open licenses without weakening provenance controls.
Contributors submit changes under the terms applicable to the files or
components they change. Mixed artifacts must preserve all applicable notices.
Trademark/domain clearance and third-party rights review remain separate
release activities.
