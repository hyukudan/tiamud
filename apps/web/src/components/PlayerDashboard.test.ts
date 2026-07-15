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
      props: { value: player, oncommand: () => undefined, onaction: () => undefined },
    });

    for (const label of [
      'Health', 'Inventory', 'Discovered map', 'Quests', 'Party', 'Coin and market',
      'Chat and adventurers', 'Manage ignore list',
    ]) {
      expect(body).toContain(label);
    }
    expect(body).toContain('blue lantern');
    expect(body).toContain('First Hunt');
    expect(body).toContain('Lantern Market');
    expect(body).toContain('<button');
    expect(body).not.toContain('accountId');
    expect(body).not.toContain('characterId');
  });
});
