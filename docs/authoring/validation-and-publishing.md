# Validation and publication

Publication is the end of review, not a substitute for it. Run the following
gates against the exact candidate bytes intended for deployment.

## Validate and test

```sh
cargo run --locked -p world-pack --bin tiamud-world -- validate \
  build/worlds/candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- test \
  build/worlds/candidate.world.json
```

`validate` checks schema versions, bounds, IDs, references, progression graphs,
creation combinations, scripts, media records and resolved local assets.
For objects, media subjects must be prototype IDs and `hiddenItems` keys must be
instance IDs; the validator rejects the opposite pairing. It also rejects an
unknown transfer policy, invalid stack bounds, fungible or consumable
containers, containment cycles, depth above four and container over-capacity.
Base and variant room titles must contain 1..=128 safe UTF-8 bytes and their
descriptions 1..=8,192;
these limits count encoded bytes.

Loot validation checks canonical IDs, all prototype/spawn/table references,
one source per spawn, use of every table, integer weights and ordered quantity
ranges. It enforces 256 tables, 1,024 sources, 8 guaranteed entries and 8 pools
per table, 8 rolls and 64 entries per pool, 4,096 weighted entries globally,
and no more than 64 fixed plus repeatable roots for one defeat. Native loot
fields require WorldPack v6/AreaPack v5; injecting them into an older schema,
even as empty arrays, fails closed instead of upgrading it implicitly.
WorldPack v7/AreaPack v6 additionally validate sealed reward shape, earned-only
provenance, absence from initial instances, no sealed-to-sealed output and a
16-unit reward-table ceiling. A sealed table cannot be an NPC source directly,
and its base plus conditional-affix expansion may expose at most 1,024 odds
rows. At most 512 affixes, 256 pools, 64 entries per pool and 4,096 total pool
entries are accepted; labels are at most 64 bytes and each positive attack or
heal delta is at most 1,000. A loot entry may choose at most one compatible
affix for exactly one unique, non-sealed item, and its effective statistics
must remain within runtime caps. Injecting any of these fields into v6/v5 or
older input also fails closed.

WorldPack v9/AreaPack v8 preserve the closed telepathy definition introduced
in WorldPack v8/AreaPack v7.
Validation requires category `spell|utility`, `single_ally`, a non-empty
positive resource cost, positive cooldown, no damage/heal/status payload and
exclusion from every NPC ability set. The world admits at most 32 communication
abilities, each with at most 64 unique existing blocked rooms. WorldPack v2-v7
and AreaPack v1-v6 reject a smuggled `communication` field instead of silently
normalizing it.

WorldPack v9/AreaPack v8 additionally validate at most 32 authored room emotes,
globally unique lowercase ASCII commands, exact targeting/template shapes,
exact `{actor}` and `{target}` placeholder counts and a 512-byte worst-case
render bound. Control characters, CR/LF, ESC, `<`, `>` and `&` fail closed.
WorldPack v2-v8 and AreaPack v1-v7 reject `socialEmotes` even when empty;
normalization and `envy-import` produce an empty catalog.

`test` runs the document's embedded content expectations. Run both explicitly;
publishing does not replace content tests or human play review.

## Preview and compare

Preview affected scenes under relevant state combinations:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- preview \
  build/worlds/candidate.world.json \
  --room demo:lantern-square --format ansi \
  --time-period night --weather rain --actor-level 5
```

Then inspect the deterministic semantic diff:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- diff \
  worlds/lantern-square.world.json \
  build/worlds/candidate.world.json
```

Review hidden-state cases separately. Preview and tests must not expose variant
IDs, secret predicates, hidden entities or undiscovered asset paths.

For the reviewed Lantern Square sources, composition must reproduce three
rooms, ten item prototypes, nine initial instances, two loot tables, one loot
source, one affix, one affix pool, one authored direct-online telepathy ability
and one authored room emote. The canonical file is 53,567 bytes with
SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.
Its gate-stalker combines a fixed stormglass talon, a guaranteed road token,
an earned sealed oath and one 60% wolf-clasp/40% nothing roll under party
round-robin rights. The ordinary clasp drop and the oath's one talon each use
a single bounded affix roll.

## Repository gates

At minimum, changes to WorldPack content should pass:

```sh
cargo test -p world-pack
cargo test -p tiamud-core
cargo test -p tiamud-server
cargo clippy -p world-pack -p tiamud-core -p tiamud-server --all-targets -- -D warnings
```

Run relevant browser checks when scene or entity presentation changes, and the
asset-pipeline suite when encoded media policy changes.

## Immutable publication

```sh
cargo run --locked -p world-pack --bin tiamud-world -- publish \
  build/worlds/candidate.world.json \
  --registry build/world-registry
```

The publisher validates and canonicalizes the document, stores it under its
SHA-256 and atomically updates the registry's active pointer. Existing
artifacts are immutable. Asset bundles are published separately and must
already exist at their reviewed same-project paths.

The artifact digest identifies the complete WorldPack. Separately, each loot
table has a canonical SHA-256 that the runtime records in durable item
provenance. Review a table diff as a gameplay and audit change even when all
IDs remain stable.

Rollback only selects a previously published digest:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- rollback \
  --registry build/world-registry \
  --world lantern-square \
  --sha256 <published-sha256>
```

Deploy the selected immutable artifact through the normal
[single-host procedure](../operations/single-host.md). Preserve the prior
binary, WorldPack, assets and database backup. A content rollback does not
reverse database migrations or reconstruct missing asset bundles.

## Human review checklist

- IDs and references express the intended durable identity.
- Text contains all actionable information and alternative text is accurate.
- Hidden content does not leak through names, manifests, previews or prefetch.
- Progression combinations and advanced-class paths are reachable and bounded.
- Combat values remain understandable under ordinary and maximum inputs.
- Loot probabilities, quantities, fixed drops, rights policy and the combined
  64-root worst case match the intended encounter.
- Every image, sound and imported record has reviewed provenance and license.
- Optional AI output is traceable to a separate accepted review artifact.
- The candidate diff, tests and deployment rollback plan were reviewed by a
  named person.
