# ADR 0003: Tiamud product identity

- Status: Accepted
- Date: 2026-07-15

## Context

The project began under a temporary codename derived from one historical influence. That codename overemphasized EnvyMUD even though the engine is a clean implementation inspired by EnvyMUD, Merc, and other classic text MUDs of their era. A distinct, memorable product identity was therefore required before the public compatibility surface settled.

## Decision

- Name the product **Tiamud**.
- Treat Tiamud as the definitive product name in source, documentation, protocols, deployment examples, and user-facing text; the naming choice is not provisional.
- Use `tiamud.v1` for the experimental browser WebSocket subprotocol.
- Use the `TIAMUD_*` prefix for runtime configuration and Tiamud names for product binaries and packages.
- Describe EnvyMUD, Merc, and their contemporaries as inspirations, never as the product's identity or source-code lineage.
- Retain `envy-import` only as the format-specific name of the offline historical EnvyMUD/Merc-family area compatibility tool.
- Keep this definitive naming decision separate from the legal trademark and domain clearance required before a public release. Those checks validate use of Tiamud; they do not restore the former codename as a product option.

## Consequences

Documentation, UI, transport banners, package metadata, configuration examples, and protocol references use Tiamud consistently. Historical implementation logs may describe when a capability was first added, but their current identifiers are normalized to the accepted product name so contributors do not copy obsolete examples.
