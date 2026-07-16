import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { PlayerStateSnapshot } from '../lib/protocol';
import PlayerDashboard from './PlayerDashboard.svelte';

const player: PlayerStateSnapshot = {
  revision: 4,
  character: {
    name: 'Aria',
    level: 2,
    experience: 120,
    nextLevelExperience: 200,
    health: 21,
    maxHealth: 25,
    strength: 7,
    defence: 3,
    archetype: { id: 'spellblade', label: 'Spellblade', description: 'Steel guided by ember lore.' },
    attributes: [
      { id: 'agility', label: 'Agility', value: 8 },
      { id: 'intellect', label: 'Intellect', value: 11 },
    ],
    resistances: [{ damageType: 'fire', percent: 15 }],
    statuses: [{
      id: 'focused', label: 'Focused', kind: 'attribute_modifier', magnitude: 2, remainingTicks: 2,
    }],
    resources: [{ id: 'mana', label: 'Mana', current: 12, max: 20 }],
    abilities: [{
      id: 'ember-bolt', name: 'Ember Bolt', description: 'Hurls a compact flame.', kind: 'spell',
      resourceCosts: [{ resourceId: 'mana', label: 'Mana', amount: 4 }], cooldownRemaining: 0, usable: true,
      targeting: 'explicitTarget',
    }, {
      id: 'mending-light', name: 'Mending Light', description: 'Restores the caster.', kind: 'spell',
      resourceCosts: [], cooldownRemaining: 0, usable: true, targeting: 'optionalTarget',
    }],
    skills: [{ id: 'combat', label: 'Combat', rank: 14 }],
    effects: [],
  },
  inventory: [{
    id: 'o1111111111111111',
    label: 'blue lantern',
    description: 'Cold light rests behind rain-speckled glass.',
    actions: [{ label: 'Drop', command: 'drop o1111111111111111' }],
  }],
  equipment: [{
    id: 'o2222222222222222',
    slot: 'weapon',
    label: 'bronze sword',
    actions: [{ label: 'Unequip', command: 'unequip weapon' }],
  }],
  map: {
    currentRoomId: 'o3333333333333333',
    rooms: [{ id: 'o3333333333333333', title: 'Lantern Square', current: true }],
    paths: [{ from: 'o3333333333333333', direction: 'north' }],
  },
  quests: [{
    id: 'first-hunt',
    label: 'First Hunt',
    summary: 'Defeat one hostile creature.',
    status: 'active',
    current: 0,
    target: 1,
    actions: [],
  }],
  party: {
    leader: true,
    members: [{ id: 'o4444444444444444', name: 'Aria', leader: true, online: true }],
    actions: [{ label: 'Leave party', command: 'party leave' }],
  },
  partyInvites: [],
  economy: {
    coins: 40,
    commodities: [],
    shopAvailable: true,
    shop: [{
      id: 'tonic', label: 'healing tonic', buyPrice: 12, sellPrice: 6,
      stock: 20, owned: 0, actions: [{ label: 'Buy', command: 'buy tonic' }],
    }],
  },
  social: {
    online: [
      { id: 'o4444444444444444', name: 'Aria', selfCharacter: true },
      { id: 'o5555555555555555', name: 'Bram', selfCharacter: false },
    ],
  },
};

describe('PlayerDashboard', () => {
  it('renders every playable panel with labelled, keyboard-native controls', () => {
    const { body } = render(PlayerDashboard, {
      props: {
        value: player,
        abilityTargets: [{ id: 'owolf000000000001', label: 'rain wolf' }],
        oncommand: () => undefined,
        onaction: () => undefined,
        onability: () => undefined,
      },
    });

    for (const label of [
      'Health', 'Inventory', 'Discovered map', 'Quests', 'Party', 'Coin and market',
      'Chat and adventurers', 'Manage ignore list', 'Abilities', 'Mana', 'Agility', 'Resistances',
    ]) {
      expect(body).toContain(label);
    }
    expect(body).toContain('blue lantern');
    expect(body).toContain('First Hunt');
    expect(body).toContain('Lantern Market');
    expect(body).toContain('Spellblade');
    expect(body).toContain('Ember Bolt');
    expect(body).toContain('Target for Ember Bolt');
    expect(body).toContain('rain wolf');
    expect(body).not.toContain('optional ally');
    expect(body).toContain('<button');
    expect(body).not.toContain('accountId');
    expect(body).not.toContain('characterId');
  });
});
