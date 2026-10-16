# ADR 0001: Project foundation

- Status: Accepted
- Date: 2026-07-15

## Context

Tiamud needs to preserve what made EnvyMUD, Merc, and other text MUDs of their era expressive while becoming easy to enter from a browser. It must remain pleasant to develop and operate on one ordinary computer. Historical areas may help migrate legally usable content, but the new engine should not inherit an old C architecture or make any historical file format canonical.

## Decision

- Build a clean Rust implementation with one authoritative, transport-independent world model.
- Use a single-process modular monolith as the reference deployment. Containers are optional; Kubernetes, Redis, object storage, and microservices are not v1 dependencies.
- Make the official Svelte client browser-first over a versioned WebSocket protocol. Keep basic Telnet text fully playable.
- Treat text as authoritative and illustrations as atmospheric accompaniment.
- Publish raster assets offline as AVIF first, WebP second, and JPEG for compatibility. Begin with 640 px and 1280 px derivatives.
- Use SQLite in WAL mode as the default persistence adapter. A dedicated owner thread performs blocking SQLite work outside the async runtime.
- Preserve a narrow persistence interface so PostgreSQL can become an alternative backend. Do not dual-write. Migrate explicitly from one authoritative backend to another.
- Keep portable gameplay data relational. A future PostgreSQL adapter may use `jsonb` for bounded metadata, not as an unstructured substitute for the domain model.
- Keep AI optional and outside the authoritative game loop. AI-assisted development and editorial work must be reviewed; lightweight local models may later be offered as opt-in authoring tools.
- Develop the canonical WorldPack format and the historical `envy-import` importer independently. The importer is an offline EnvyMUD/Merc-family compatibility tool, not part of runtime world loading.
- Recover a connection with a fresh authoritative snapshot in v1 rather than a general event replay system.

## Consequences

The early system is easy to run, back up, and profile. Boundaries exist where measured demand could later justify PostgreSQL, external asset storage, richer MUD protocols, or separated processes. Some scaling features are intentionally deferred, and content tools must compile assets before deployment.

Open-source licensing was intentionally left for a separate decision and is
now resolved by [ADR 0004](0004-open-source-licensing.md).

Implementation progress does not amend this foundation decision. The current evidence and remaining full-scope work are maintained in the project's private completion records.

## Later amendment

[ADR 0006](0006-character-progression-and-postgresql-first.md) supersedes the
storage operating recommendation above now that both adapters are implemented:
new public single-host installations are PostgreSQL-first, while the
zero-configuration executable and local/legacy path remain SQLite. It does not
change this ADR's one-process topology, exactly-one-backend rule, explicit
offline migration or prohibition on required Kubernetes and microservices.
The implemented adapters keep queryable data relational and store bounded,
non-queryable application snapshots as JSON in `TEXT`, not PostgreSQL `jsonb`.
