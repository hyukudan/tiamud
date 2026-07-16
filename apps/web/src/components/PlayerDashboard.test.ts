import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { PlayerStateSnapshot } from '../lib/protocol';
import PlayerDashboard from './PlayerDashboard.svelte';
import dashboardSource from './PlayerDashboard.svelte?raw';

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
    lineage: { id: 'embermarked', label: 'Embermarked', description: 'Fire answers their blood.' },
    background: { id: 'archive', label: 'Archive Apprentice', description: 'Trained among old records.' },
    baseClass: { id: 'arcanist', label: 'Arcanist', description: 'Shapes disciplined magic.' },
    activeClass: { id: 'spellblade', label: 'Spellblade', description: 'Steel guided by ember lore.' },
    classHistory: [
      { id: 'arcanist', label: 'Arcanist', description: 'Shapes disciplined magic.' },
      { id: 'spellblade', label: 'Spellblade', description: 'Steel guided by ember lore.' },
    ],
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
    progressionPoints: { talent: 2 },
    talents: [{
      id: 'ember-path', label: 'Ember Path', description: 'Control fire without being consumed.',
      pointPoolId: 'talent', availablePoints: 2,
      nodes: [{
        id: 'steady-flame', label: 'Steady Flame', description: 'Keep spells stable under pressure.',
        rank: 1, maxRank: 3, nextCost: 2, available: true,
        requirements: [{ met: true, text: 'Reach level 2' }],
        command: 'talent learn ember-path steady-flame',
      }],
    }],
    promotions: [{
      class: { id: 'stormcaller', label: 'Stormcaller', description: 'Calls lightning from rain.' },
      available: false,
      requirements: [{ met: false, text: 'Hear the lantern whisper' }],
      command: 'promote stormcaller',
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
        onpromote: () => undefined,
        ontalent: () => undefined,
      },
    });

    for (const label of [
      'Health', 'Inventory', 'Discovered map', 'Quests', 'Party', 'Coin and market',
      'Chat and adventurers', 'Manage ignore list', 'Abilities', 'Mana', 'Agility', 'Resistances',
      'Progression', 'Class evolution', 'Advanced classes', 'Talent paths',
      'Invite or manage an adventurer', 'Revoke invitation', 'Remove from party',
      'Transfer leadership', 'Disband party', 'Message to party',
      'Reply to your latest direct message', 'World channels', 'Channel',
      'General', 'Newbie', 'Staff',
    ]) {
      expect(body).toContain(label);
    }
    expect(body).toContain('blue lantern');
    expect(body).toContain('First Hunt');
    expect(body).toContain('Lantern Market');
    expect(body).toContain('Spellblade');
    expect(body).toContain('Embermarked');
    expect(body).toContain('Archive Apprentice');
    expect(body).toContain('Stormcaller');
    expect(body).toContain('Hear the lantern whisper');
    expect(body).toContain('Steady Flame');
    expect(body).toContain('Rank 1 / 3');
    expect(body).toContain('Ember Bolt');
    expect(body).toContain('Target for Ember Bolt');
    expect(body).toContain('rain wolf');
    expect(body).not.toContain('optional ally');
    expect(body).toContain('<button');
    expect(body).not.toContain('accountId');
    expect(body).not.toContain('characterId');
  });

  it('wires party and world-channel controls to structured actions', () => {
    for (const action of [
      'party.say', 'party.revoke', 'party.kick', 'party.transfer', 'party.disband',
      'social.reply', 'social.channel.list', 'social.channel.join',
      'social.channel.leave', 'social.channel.say',
    ]) {
      expect(dashboardSource).toContain(action);
    }
    expect(dashboardSource).toContain('{ channel: communityChannel, message }');
    expect(dashboardSource).toContain('{ target }');
  });
});
