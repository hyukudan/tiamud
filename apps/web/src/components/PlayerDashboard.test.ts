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
    rarity: 'uncommon',
    itemLevel: 3,
    weightGrams: 800,
    quantity: 2,
    availableQuantity: 2,
    reservedForTrade: 0,
    stackSlots: 1,
    totalWeightGrams: 1600,
    transferPolicy: 'transferable',
    boundToYou: false,
    canGive: true,
    canTrade: true,
    actions: [{ label: 'Drop', command: 'drop o1111111111111111' }],
  }, {
    id: 'o6666666666666666',
    label: 'archive satchel',
    description: 'A compact field satchel.',
    rarity: 'common',
    itemLevel: 1,
    weightGrams: 400,
    quantity: 1,
    availableQuantity: 1,
    reservedForTrade: 0,
    stackSlots: 1,
    totalWeightGrams: 650,
    transferPolicy: 'character_bound',
    boundToYou: true,
    canGive: false,
    canTrade: false,
    containerCapacity: {
      usedSlots: 1, maxSlots: 8, usedWeightGrams: 250, maxWeightGrams: 5000,
    },
    actions: [{ label: 'Contents', command: 'contents o6666666666666666' }],
  }],
  equipment: [{
    id: 'o2222222222222222',
    slot: 'weapon',
    label: 'bronze sword',
    rarity: 'common',
    itemLevel: 2,
    weightGrams: 2200,
    quantity: 1,
    stackSlots: 1,
    totalWeightGrams: 2300,
    transferPolicy: 'non_transferable',
    boundToYou: false,
    containerCapacity: {
      usedSlots: 2, maxSlots: 5, usedWeightGrams: 100, maxWeightGrams: 1000,
    },
    actions: [{ label: 'Unequip', command: 'unequip weapon' }],
  }],
  inventoryCapacity: {
    usedSlots: 1,
    maxSlots: 24,
    usedWeightGrams: 3000,
    maxWeightGrams: 30000,
  },
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
    reservedCoins: 0,
    spendableCoins: 40,
    commodities: [],
    shopAvailable: true,
    shop: [{
      id: 'tonic', label: 'healing tonic', buyPrice: 12, sellPrice: 6,
      stock: 20, owned: 0, actions: [{ label: 'Buy', command: 'buy tonic' }],
    }],
  },
  social: {
    online: [
      { id: 'o4444444444444444', name: 'Aria', selfCharacter: true, nearby: true },
      { id: 'o5555555555555555', name: 'Bram', selfCharacter: false, nearby: true },
    ],
  },
};

describe('PlayerDashboard', () => {
  it('renders every playable panel with labelled, keyboard-native controls', () => {
    const { body } = render(PlayerDashboard, {
      props: {
        value: player,
        containerContents: {
          container: 'o6666666666666666',
          items: [{
            id: 'o7777777777777777', label: 'healing tonic', rarity: 'uncommon',
            itemLevel: 2, weightGrams: 250, quantity: 3, stackSlots: 1,
            availableQuantity: 3, reservedForTrade: 0,
            totalWeightGrams: 750, transferPolicy: 'transferable', boundToYou: false,
            canGive: false, canTrade: false, actions: [],
          }],
          page: 1,
          totalPages: 2,
          capacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 750, maxWeightGrams: 5000 },
        },
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
      'Confirmed trade', 'Request trade',
      'Invite or manage an adventurer', 'Revoke invitation', 'Remove from party',
      'Transfer leadership', 'Disband party', 'Message to party',
      'Reply to your latest direct message', 'World channels', 'Channel',
      'General', 'Newbie', 'Staff', 'Nearby recipient', 'Transferable',
      'Bound to this character', 'Non-transferable',
    ]) {
      expect(body).toContain(label);
    }
    expect(body).toContain('blue lantern');
    expect(body).toContain('blue lantern ×2');
    expect(body).toContain('archive satchel');
    expect(body).toContain('0.3/');
    expect(body).toContain('5.0 kg');
    expect(body).toContain('bronze sword');
    expect(body).toContain('stack slot');
    expect(body).toContain('2/5');
    expect(body).toContain('0.1/');
    expect(body).toContain('1.0 kg');
    expect(body).toContain('Container contents');
    expect(body).toContain('Page 1 of 2');
    expect(body).toContain('healing tonic');
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
    expect(dashboardSource).toContain("'item.contents'");
    expect(dashboardSource).toContain("'item.put'");
    expect(dashboardSource).toContain("'item.get'");
    expect(dashboardSource).toContain("'item.give'");
    expect(dashboardSource).toContain('recipient: recipient.id');
    expect(dashboardSource).toContain('<option value="staff">');
    expect(dashboardSource).not.toContain('<option value="announcement">');
  });

  it('renders private offers, reservations, and revision-bound double confirmation', () => {
    const tradingPlayer: PlayerStateSnapshot = {
      ...player,
      inventory: [{
        ...player.inventory[0],
        availableQuantity: 1,
        reservedForTrade: 1,
        canGive: false,
      }],
      economy: { ...player.economy, reservedCoins: 5, spendableCoins: 35 },
      trade: {
        id: 'o8888888888888888',
        phase: 'negotiating',
        counterparty: 'o5555555555555555',
        expiresInTicks: 80,
        stateRevision: 6,
        ownConfirmed: false,
        counterpartyConfirmed: true,
        ownOffer: {
          coins: 5,
          lines: [{
            target: 'o1111111111111111', label: 'blue lantern', quantity: 1, container: false,
          }],
        },
        counterpartyOffer: {
          coins: 3,
          lines: [{
            target: 'o9999999999999999', label: 'field satchel', quantity: 1, container: true,
          }],
        },
        canAccept: false,
        canConfirm: true,
      },
    };
    const { body } = render(PlayerDashboard, {
      props: {
        value: tradingPlayer,
        oncommand: () => undefined,
        onaction: () => undefined,
        onability: () => undefined,
        onpromote: () => undefined,
        ontalent: () => undefined,
      },
    });
    for (const label of [
      'Exchange with Bram', 'Revision 6', 'Your offer', "Bram's offer", '5 coins',
      '3 coins', 'reserved for trade 1', 'Review before confirming', 'Confirm revision 6',
      'Inspect offered contents', 'Cancel trade',
    ]) expect(body).toContain(label);
    for (const action of [
      'trade.request', 'trade.item.set', 'trade.item.remove', 'trade.coins.set',
      'trade.contents', 'trade.confirm',
    ]) expect(dashboardSource).toContain(action);
    expect(dashboardSource).toContain('`trade.${action}`');
  });
});
