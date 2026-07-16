# Progression, abilities and combat

The v3 character model in WorldPack v5 separates character identity from role:

- a lineage is an inherited or supernatural origin;
- an optional background is learned personal history;
- one base class is selected at creation;
- talents improve a path without changing class identity;
- an advanced class is an explicit, irreversible promotion in v1.

The full vocabulary, initial catalog and diagrams are in the
[character progression design](../design/character-progression-v3.md).

## Creation and levels

Creation starts from a neutral sheet: attributes, skills and resource capacity
are zero. It then applies lineage, optional background and base-class creation
grants. Every base class must author positive `max_health`; these grants are
the actual initial totals, not bonuses over hidden engine statistics. Creation
alone fills current health and resources.

There is one cumulative global XP curve. Level thresholds may grant bounded
points, while the active class supplies level-specific grants. Later health or
resource-capacity gains preserve current values rather than healing or
refilling them.

Promotion validates the current source class, requirements and exclusivity,
then applies its grants atomically. If a character promotes after a target
class milestone, already-reached target-class level grants are applied during
that one-time promotion; future levels apply only new milestones.

## Closed requirements and grants

Requirements support bounded combinations of level, permanent attributes,
skills, quests, progression signals, flags, lineage, current or historical
class, talent rank and known ability. Public compound requirements are
explainable. Hidden predicates expose only their reviewed safe hint.

Grants can change bounded attributes, skills, resource maximum/regeneration,
abilities, resistances, progression points and semantic tags. They cannot run
arbitrary code. WorldPack validation checks each independent creation
combination and runtime application checks the same caps again.

The reviewed implementation lives in the progression section of
[`square.area.json`](../../worlds/areas/square.area.json).

## Abilities and combat data

Abilities are stable, reusable definitions. They declare:

- category and targeting;
- bounded resource costs and cooldown;
- optional typed damage or healing with bounded attribute scaling;
- bounded regeneration, damage-over-time, stun or attribute status effects.

NPC prototypes author level, health, strength, defence, XP and respawn.
NPC ability sets attach resources, regeneration, resistances and known
abilities. Reusable item prototypes attach equipment slots, attack bonuses,
healing, consumption, weight, slot cost, rarity, item level, transfer/stack
policy, optional bounded-container capacity and short regeneration or strength
effects. Phase 3B accepts `unique` or `fungible` stacks but still accepts only
`transferable`; bound/non-transferable values are reserved and rejected until
their runtime semantics land. Fungible stacks are derived from durable units
and never author a quantity or stack ID. A container must be unique and
non-consumable. Fixed NPC loot is an `initialItemInstances` unit whose location
is the intended `npc_spawn`; it is not a list on the NPC prototype.

All actions still pass through the same authoritative world actor. Browser and
Telnet commands do not implement separate combat rules. Definitions are
bounded at publication and runtime; combat arithmetic saturates defensively
instead of wrapping.

## Author checklist

- Give every definition a stable namespaced ID.
- Ensure every base class is independently playable at level 1.
- Keep lineages modest so they do not become mandatory class choices.
- Make hidden requirements discoverable through safe hints without naming the
  secret flag or unseen quest.
- Test promotion at its minimum level and after any catch-up milestone.
- Exercise resource exhaustion, cooldown, resistance, defeat and missing
  target behavior.
- Treat sample values as demonstration balance, not universal balance.
