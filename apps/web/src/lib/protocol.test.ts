import { describe, expect, expectTypeOf, it } from 'vitest';
import {
  type ActionPerformFields,
  createAbilityPerform,
  createActionPerform,
  createClientHello,
  createCommandSubmit,
  createHeartbeatAck,
  createPromotionPerform,
  createSyncRequest,
  createTalentPerform,
  localizedImageAlt,
  parseServerMessage,
} from './protocol';
import { WEBSOCKET_SUBPROTOCOL } from './brand';
import { websocketUrl } from './socket';

const validPlayerState = () => ({
  revision: 3,
  character: {
    name: 'Aria', level: 2, experience: 120, nextLevelExperience: 200,
    health: 21, maxHealth: 25, strength: 7, defence: 3, effects: [], recoveryInTicks: null,
    skills: [{ id: 'combat', label: 'Combat', rank: 14 }],
    lineage: { id: 'mortal', label: 'Mortal', description: 'Adaptable and ambitious.' },
    background: null,
    baseClass: { id: 'wanderer', label: 'Wanderer', description: 'A versatile explorer.' },
    activeClass: { id: 'wanderer', label: 'Wanderer', description: 'A versatile explorer.' },
    classHistory: [{ id: 'wanderer', label: 'Wanderer', description: 'A versatile explorer.' }],
    progressionPoints: { talent: 1 },
    talents: [],
    promotions: [],
  },
  inventory: [{
    id: 'o1111111111111111', label: 'blue lantern', description: 'Cold light.',
    rarity: 'uncommon', itemLevel: 3, weightGrams: 800,
    quantity: 1, availableQuantity: 1, reservedForTrade: 0,
    stackSlots: 1, totalWeightGrams: 800,
    transferPolicy: 'transferable', boundToYou: false,
    canGive: true, canTrade: true,
    actions: [{ label: 'Drop', command: 'drop o1111111111111111' }],
  }],
  equipment: [{
    id: 'o2222222222222222', slot: 'weapon', label: 'bronze sword',
    rarity: 'common', itemLevel: 2, weightGrams: 2200,
    quantity: 1, stackSlots: 1, totalWeightGrams: 2200,
    transferPolicy: 'transferable', boundToYou: false,
    actions: [{ label: 'Unequip', command: 'unequip weapon' }],
  }],
  inventoryCapacity: {
    usedSlots: 1, maxSlots: 24, usedWeightGrams: 3000, maxWeightGrams: 30000,
  },
  map: {
    currentRoomId: 'o3333333333333333',
    rooms: [{ id: 'o3333333333333333', title: 'Lantern Square', current: true }],
    paths: [{ from: 'o3333333333333333', direction: 'north' }],
  },
  quests: [{
    id: 'first-hunt', label: 'First Hunt', summary: 'Defeat one hostile creature.',
    status: 'active', current: 0, target: 1, actions: [],
  }],
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
    online: [{ id: 'o4444444444444444', name: 'Aria', selfCharacter: true, nearby: true }],
  },
});

describe('protocol messages', () => {
  it('announces modern image formats with AVIF first', () => {
    const hello = createClientHello(true, { token: 'token', characterId: 7 });
    expect(hello.data.capabilities).toEqual({
      images: ['avif', 'webp', 'jpeg'],
      reducedMotion: true,
      audio: false,
    });
    expect(hello.data.auth).toEqual({ token: 'token', characterId: 7 });
    expect(createClientHello(false, { token: 'token', characterId: 7 }, false).data.capabilities)
      .toEqual({ images: [], reducedMotion: false, audio: false });
    expect(createClientHello(false, { token: 'token', characterId: 7 }, true, true).data.capabilities)
      .toEqual({ images: ['avif', 'webp', 'jpeg'], reducedMotion: false, audio: true });
  });

  it('creates commands with an idempotency identifier', () => {
    expect(createCommandSubmit('north', 'request-1')).toMatchObject({
      type: 'command.submit',
      data: { command: 'north', requestId: 'request-1' },
    });
  });

  it('creates structured actions, sync requests, and heartbeat acknowledgements', () => {
    expect(createActionPerform('item.take', { target: 'lantern' }, 'request-2')).toEqual({
      v: 1,
      type: 'action.perform',
      data: { requestId: 'request-2', action: 'item.take', target: 'lantern' },
    });
    expect(createActionPerform(
      'item.put',
      { target: 'o1111111111111111', container: 'o2222222222222222', quantity: 12 },
      'request-put',
    )).toMatchObject({
      data: {
        action: 'item.put', target: 'o1111111111111111',
        container: 'o2222222222222222', quantity: 12,
      },
    });
    expect(createActionPerform(
      'item.contents',
      { container: 'o2222222222222222', page: 2 },
      'request-contents',
    )).toMatchObject({
      data: { action: 'item.contents', container: 'o2222222222222222', page: 2 },
    });
    expect(createActionPerform(
      'item.give',
      { target: 'o1111111111111111', recipient: 'o3333333333333333', quantity: 2 },
      'request-give',
    )).toMatchObject({
      data: {
        action: 'item.give', target: 'o1111111111111111',
        recipient: 'o3333333333333333', quantity: 2,
      },
    });
    expect(createActionPerform('loot.status', undefined, 'loot-status')).toEqual({
      v: 1,
      type: 'action.perform',
      data: { requestId: 'loot-status', action: 'loot.status' },
    });
    expect(createActionPerform(
      'loot.pass',
      { target: 'o4444444444444444' },
      'loot-pass',
    )).toMatchObject({
      data: {
        requestId: 'loot-pass', action: 'loot.pass', target: 'o4444444444444444',
      },
    });
    expect(createActionPerform('party.say', { message: 'Hold the bridge.' }, 'request-party')).toMatchObject({
      data: { requestId: 'request-party', action: 'party.say', message: 'Hold the bridge.' },
    });
    expect(createActionPerform(
      'social.channel.say',
      { channel: 'newbie', message: 'Where is the market?' },
      'request-channel',
    )).toMatchObject({
      data: {
        requestId: 'request-channel', action: 'social.channel.say',
        channel: 'newbie', message: 'Where is the market?',
      },
    });
    expect(createSyncRequest(12)).toEqual({
      v: 1,
      type: 'sync.request',
      data: { afterSeq: 12 },
    });
    expect(createHeartbeatAck(9)).toEqual({
      v: 1,
      type: 'client.heartbeat',
      data: { nonce: 9 },
    });
    expect(createAbilityPerform('ember-bolt', 'o9999999999999999', 'request-3')).toEqual({
      v: 1,
      type: 'action.perform',
      data: {
        requestId: 'request-3',
        action: 'combat.ability',
        abilityId: 'ember-bolt',
        target: 'o9999999999999999',
      },
    });
    expect(createPromotionPerform('pathfinder', 'request-4')).toMatchObject({
      data: { requestId: 'request-4', action: 'progression.promote', classId: 'pathfinder' },
    });
    expect(createTalentPerform('wayfarer', 'sure-step', 'request-5')).toMatchObject({
      data: {
        requestId: 'request-5', action: 'progression.talent', graphId: 'wayfarer', nodeId: 'sure-step',
      },
    });
  });

  it('bounds structured item quantities and pages before sending', () => {
    expect(() => createActionPerform('item.take', { target: 'tonic', quantity: 0 }, 'bad')).toThrow(RangeError);
    expect(() => createActionPerform('item.drop', { target: 'tonic', quantity: 257 }, 'bad')).toThrow(RangeError);
    expect(() => createActionPerform('item.contents', { container: 'bag', page: 0 }, 'bad')).toThrow(RangeError);
    expect(() => createActionPerform('item.contents', { container: '', page: 1 }, 'bad')).toThrow(RangeError);
    expect(() => createActionPerform('item.give', { target: 'tonic', recipient: '' }, 'bad')).toThrow(RangeError);
    expect(() => createActionPerform(
      'item.give', { target: 'tonic', recipient: 'x'.repeat(65) }, 'bad',
    )).toThrow(RangeError);
  });

  it('creates only the exact closed loot action forms', () => {
    expect(() => createActionPerform(
      'loot.status', { target: 'o1111111111111111' }, 'extra',
    )).toThrow('exact closed field set');
    expect(() => createActionPerform('loot.pass', undefined, 'missing'))
      .toThrow('exact closed field set');
    expect(() => createActionPerform('loot.pass', { target: '' }, 'empty'))
      .toThrow('bounded visible selector');
    expect(() => createActionPerform(
      'loot.pass', { target: 'x'.repeat(257) }, 'long',
    )).toThrow('bounded visible selector');
    expect(() => createActionPerform(
      'loot.pass', { target: 'o1111111111111111', quantity: 1 }, 'extra',
    )).toThrow('exact closed field set');
  });

  it('creates only the exact closed sealed-reward action forms', () => {
    const target = 'o1111111111111111';
    expect(createActionPerform('reward.open', { target }, 'reward-open')).toMatchObject({
      data: { action: 'reward.open', target },
    });
    expect(createActionPerform('reward.odds', { target, page: 2 }, 'reward-odds')).toMatchObject({
      data: { action: 'reward.odds', target, page: 2 },
    });
    expect(() => createActionPerform('reward.open', { target, page: 1 }, 'extra'))
      .toThrow('exact closed field set');
    expect(() => createActionPerform('reward.open', { target: 'internal:id' }, 'internal'))
      .toThrow('opaque selector');
    expect(() => createActionPerform('reward.odds', { target }, 'missing'))
      .toThrow('exact closed field set');
    expect(() => createActionPerform('reward.odds', { target, page: 0 }, 'bad-page'))
      .toThrow(RangeError);
  });

  it('creates only the exact closed trade action forms', () => {
    const tradeId = 'oaaaaaaaaaaaaaaaa';
    const itemId = 'obbbbbbbbbbbbbbbb';
    const recipient = 'occcccccccccccccc';
    const cases: Array<[string, ActionPerformFields]> = [
      ['trade.request', { recipient }],
      ['trade.accept', { tradeId }],
      ['trade.decline', { tradeId }],
      ['trade.item.set', { tradeId, target: itemId, quantity: 128 }],
      ['trade.item.remove', { tradeId, target: itemId }],
      ['trade.coins.set', { tradeId, coins: 1_000_000_000 }],
      ['trade.contents', { tradeId, target: itemId, page: 2 }],
      ['trade.confirm', { tradeId, stateRevision: 7 }],
      ['trade.status', { tradeId }],
      ['trade.cancel', { tradeId }],
    ];
    for (const [action, fields] of cases) {
      expect(createActionPerform(action, fields, `request-${action}`).data).toEqual({
        requestId: `request-${action}`, action, ...fields,
      });
    }
    expect(() => createActionPerform('trade.accept', { tradeId, page: 1 }, 'extra'))
      .toThrow('exact closed field set');
    expect(() => createActionPerform('trade.item.set', {
      tradeId, target: itemId, quantity: 129,
    }, 'quantity')).toThrow('between 1 and 128');
    expect(() => createActionPerform('trade.request', { recipient: 'Aria' }, 'recipient'))
      .toThrow('opaque selector');
    expect(() => createActionPerform('trade.confirm', {
      tradeId, stateRevision: 0,
    }, 'revision')).toThrow('positive safe integer');
    for (const [action, fields] of [
      ['trade.request', { recipient: undefined }],
      ['trade.accept', { tradeId: undefined }],
      ['trade.item.set', { tradeId, target: undefined, quantity: 1 }],
      ['trade.item.set', { tradeId, target: itemId, quantity: undefined }],
      ['trade.coins.set', { tradeId, coins: undefined }],
      ['trade.contents', { tradeId, target: itemId, page: undefined }],
      ['trade.confirm', { tradeId, stateRevision: undefined }],
    ] as Array<[string, ActionPerformFields]>) {
      expect(() => createActionPerform(action, fields, `undefined-${action}`))
        .toThrow(RangeError);
    }
    expect(() => createActionPerform(
      'trade.accept',
      Object.create({ tradeId }) as ActionPerformFields,
      'inherited-trade-id',
    )).toThrow('exact closed field set');
    for (const [action, fields] of [
      ['trade.accept', { tradeId: 7 }],
      ['trade.coins.set', { tradeId, coins: '1' }],
      ['trade.contents', { tradeId, target: itemId, page: '1' }],
      ['trade.confirm', { tradeId, stateRevision: '7' }],
    ] as Array<[string, unknown]>) {
      expect(() => createActionPerform(
        action,
        fields as unknown as ActionPerformFields,
        `wrong-type-${action}`,
      )).toThrow(RangeError);
    }
  });

  it('creates the exact structured staff phase-2 payloads', () => {
    const cases: Array<{
      action: string;
      fields?: ActionPerformFields;
      requestId: string;
    }> = [
      { action: 'staff.status', requestId: 'staff-status' },
      {
        action: 'staff.reports',
        fields: { reason: 'weekly review' },
        requestId: 'staff-reports',
      },
      {
        action: 'staff.report.resolve',
        fields: {
          reportId: 7,
          resolution: 'action_taken',
          reason: 'evidence verified',
        },
        requestId: 'staff-resolve',
      },
      {
        action: 'staff.inspect',
        fields: { target: 'Aria', reason: 'support request' },
        requestId: 'staff-inspect',
      },
      {
        action: 'staff.where',
        fields: { target: 'Aria', reason: 'locate incident' },
        requestId: 'staff-where',
      },
      {
        action: 'staff.goto',
        fields: { target: 'Aria', reason: 'respond to incident' },
        requestId: 'staff-goto',
      },
      {
        action: 'staff.transfer',
        fields: { target: 'Aria', reason: 'unstick character' },
        requestId: 'staff-transfer',
      },
      {
        action: 'staff.restore',
        fields: { target: 'Aria', reason: 'recover after incident' },
        requestId: 'staff-restore',
      },
      {
        action: 'staff.announce',
        fields: { message: 'The shard will restart soon.', reason: 'planned maintenance' },
        requestId: 'staff-announce',
      },
      {
        action: 'staff.audit',
        fields: { reason: 'weekly review' },
        requestId: 'staff-audit',
      },
      {
        action: 'staff.profile.set',
        fields: {
          target: 'Aria', profile: 'moderator', loreTitle: 'Warden', reason: 'approved promotion',
        },
        requestId: 'staff-profile',
      },
      {
        action: 'staff.profile.set',
        fields: { target: 'Aria', profile: null, loreTitle: null, reason: 'assignment ended' },
        requestId: 'staff-profile-clear',
      },
      {
        action: 'staff.capability.set',
        fields: {
          target: 'Aria', capability: 'reports.resolve', enabled: true, reason: 'temporary duty',
        },
        requestId: 'staff-capability',
      },
    ];

    for (const { action, fields, requestId } of cases) {
      expect(createActionPerform(action, fields, requestId)).toEqual({
        v: 1,
        type: 'action.perform',
        data: { requestId, action, ...fields },
      });
    }

    expect(createActionPerform(
      'staff.profile.set',
      { target: 'Aria', reason: 'unchanged optional fields' },
      'staff-profile-omitted',
    ).data).not.toHaveProperty('profile');
    expect(createActionPerform(
      'staff.profile.set',
      { target: 'Aria', profile: null, loreTitle: null, reason: 'clear assignment' },
      'staff-profile-null',
    ).data).toMatchObject({ profile: null, loreTitle: null });
  });

  it('rejects invalid report ids and never emits the server-only announcement channel', () => {
    expectTypeOf<NonNullable<ActionPerformFields['channel']>>()
      .toEqualTypeOf<'general' | 'newbie' | 'staff'>();

    for (const reportId of [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(() => createActionPerform(
        'staff.report.resolve',
        { reportId, resolution: 'dismissed', reason: 'invalid report id' },
        'staff-bad-report',
      )).toThrow(RangeError);
    }

    expect(() => createActionPerform(
      'social.channel.say',
      { channel: 'announcement', message: 'forged' } as unknown as ActionPerformFields,
      'forged-announcement',
    )).toThrow('channel is not sendable');
  });

  it('normalises the current visual scene shape', () => {
    const hash = 'a'.repeat(64);
    const message = parseServerMessage(
      JSON.stringify({
        v: 1,
        type: 'scene.snapshot',
        seq: 8,
        data: {
          title: 'The old road',
          description: 'Rain softens the outlines of the forest.',
          weather: { condition: 'rain', day: 2, hour: 21, nextChangeInTicks: 4 },
          visual: {
            alt: 'A forest road in light rain.',
            variants: [
              { format: 'avif', url: `/assets/${hash}/scene-1280.avif` },
              { format: 'webp', url: `/assets/${hash}/scene-1280.webp` },
              { format: 'jpeg', url: `/assets/${hash}/scene-1280.jpg` },
            ],
          },
          exits: [{ id: 'north', label: 'North', command: 'north' }],
        },
      }),
    );

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: {
        title: 'The old road',
        prose: ['Rain softens the outlines of the forest.'],
        weather: { condition: 'rain', day: 2, hour: 21, nextChangeInTicks: 4 },
        image: {
          avif: `/assets/${hash}/scene-1280.avif`,
          webp: `/assets/${hash}/scene-1280.webp`,
          jpeg: `/assets/${hash}/scene-1280.jpg`,
        },
        exits: [{ id: 'north', label: 'North', command: 'north' }],
        items: [],
        npcs: [],
      },
    });
  });

  it('bounds malformed scene weather without dropping the text snapshot', () => {
    const message = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 9,
      data: {
        title: 'Lantern Square',
        prose: ['The weather instrument is recalibrating.'],
        weather: { condition: 'x'.repeat(80), day: -1, hour: 99, nextChangeInTicks: -2 },
        exits: [],
        items: [],
        npcs: [],
      },
    }));

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: {
        weather: { condition: 'unknown', day: 0, hour: 23, nextChangeInTicks: 0 },
      },
    });
  });

  it('normalises visible NPCs, bounded health, and combat actions', () => {
    const message = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 4,
      data: {
        title: 'Lantern Square',
        prose: [],
        exits: [],
        items: [],
        npcs: [{
          id: 'opaque-wolf',
          label: 'rain wolf',
          description: 'It watches from the archway.',
          health: 99,
          maxHealth: 12,
          actions: [{ label: 'Attack', command: 'attack opaque-wolf' }],
        }],
      },
    }));

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: { npcs: [{
        id: 'opaque-wolf',
        health: 12,
        maxHealth: 12,
        actions: [{ command: 'attack opaque-wolf' }],
      }] },
    });
  });

  it('normalises visible items and their direct actions', () => {
    const message = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 3,
      data: {
        title: 'Lantern Square',
        prose: [],
        exits: [],
        items: [{
          id: 'tiamud:blue-lantern',
          label: 'blue-glass lantern',
          description: 'Its flame is calm.',
          rarity: 'uncommon',
          itemLevel: 3,
          weightGrams: 800,
          quantity: 2,
          stackSlots: 1,
          totalWeightGrams: 1600,
          transferPolicy: 'transferable',
          boundToYou: false,
          loot: { access: 'yours', expiresInTicks: 12 },
          actions: [
            { label: 'Examine', command: 'examine tiamud:blue-lantern' },
            { label: 'Take', command: 'take tiamud:blue-lantern' },
            { label: 'Pass', command: 'loot pass tiamud:blue-lantern' },
          ],
        }],
      },
    }));

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: { items: [{
        id: 'tiamud:blue-lantern',
        label: 'blue-glass lantern',
        loot: { access: 'yours', expiresInTicks: 12 },
        actions: [
          { command: 'examine tiamud:blue-lantern' },
          { command: 'take tiamud:blue-lantern' },
          { command: 'loot pass tiamud:blue-lantern' },
        ],
      }] },
    });

    const reserved = JSON.parse(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 4,
      data: {
        title: 'Lantern Square', prose: [], exits: [], npcs: [],
        items: [{
          id: 'o1111111111111111', label: 'wolf clasp', description: '', rarity: 'rare',
          itemLevel: 2, weightGrams: 100, quantity: 1, stackSlots: 1,
          totalWeightGrams: 100, transferPolicy: 'transferable', boundToYou: false,
          loot: { access: 'reserved' }, actions: [],
        }],
      },
    }));
    expect(parseServerMessage(JSON.stringify(reserved))).toMatchObject({
      data: { items: [{ loot: { access: 'reserved' } }] },
    });
    for (const loot of [
      { access: 'yours' },
      { access: 'yours', expiresInTicks: 0 },
      { access: 'yours', expiresInTicks: 21 },
      { access: 'reserved', expiresInTicks: 4 },
      { access: 'reserved', beneficiary: 7 },
      { access: 'public' },
    ]) {
      reserved.data.items[0].loot = loot;
      const parsed = parseServerMessage(JSON.stringify(reserved));
      expect(parsed?.type).toBe('scene.snapshot');
      if (parsed?.type !== 'scene.snapshot') throw new Error('expected scene snapshot');
      expect(parsed.data.items).toEqual([]);
    }
    reserved.data.items[0].loot = { access: 'reserved' };
    reserved.data.items[0].actions = [{
      label: 'Take', command: 'take o1111111111111111',
    }];
    const unauthorized = parseServerMessage(JSON.stringify(reserved));
    expect(unauthorized?.type).toBe('scene.snapshot');
    if (unauthorized?.type !== 'scene.snapshot') throw new Error('expected scene snapshot');
    expect(unauthorized.data.items).toEqual([]);
  });

  it('accepts bounded entity annotations only in the scene presentation sidecar', () => {
    const hash = 'b'.repeat(64);
    const message = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 5,
      data: {
        viewId: 'o9999999999999999',
        title: 'Lantern Square',
        prose: [],
        exits: [],
        items: [],
        npcs: [{
          id: 'o8888888888888888',
          label: 'rain wolf',
          description: 'It watches from the archway.',
          health: 12,
          maxHealth: 12,
          actions: [{ label: 'Attack', command: 'attack o8888888888888888' }],
        }],
        presentation: {
          entities: [{
            id: 'o8888888888888888',
            alt: '',
            decorative: true,
            footprint: { width: 0.18, height: 0.42 },
            preferredAnchor: { x: 0.7, y: 0.96 },
            priority: 20,
            layer: 'foreground',
            poses: {
              idle: {
                avif: `/assets/${hash}/sprite-idle-640.avif`,
                png: `/assets/${hash}/sprite-idle-640.png`,
              },
              attack: { webp: `/assets/${hash}/sprite-attack-640.webp` },
              hit: { manifest: `/assets/${hash}/manifest.json` },
            },
          }],
        },
      },
    }));

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: {
        npcs: [{
          id: 'o8888888888888888',
        }],
        presentation: {
          entities: [{
            id: 'o8888888888888888',
            decorative: true,
            footprint: { width: 0.18, height: 0.42 },
            priority: 20,
            layer: 'foreground',
            poses: {
              idle: { avif: `/assets/${hash}/sprite-idle-640.avif` },
              attack: { webp: `/assets/${hash}/sprite-attack-640.webp` },
              hit: { manifest: `/assets/${hash}/manifest.json` },
            },
          }],
        },
      },
    });
  });

  it('strips unsafe sidecar annotations without dropping semantic entities', () => {
    const unsafeVisuals = [
      {
        alt: '', decorative: true, footprint: { width: 0.2, height: 0.4 }, priority: 1,
        layer: 'foreground', poses: { idle: { png: 'https://tracker.invalid/wolf.png' } },
      },
      {
        alt: '', decorative: true, footprint: { width: 0.2, height: 0.4 }, priority: 1,
        layer: 'foreground', poses: { idle: { png: 'data:image/png;base64,AAAA' } },
      },
      {
        alt: '', decorative: true, footprint: { width: 0.2, height: 0.4 }, priority: 1,
        layer: 'foreground', poses: { idle: { png: `/assets/${'c'.repeat(64)}/sprite-idle.svg` } },
      },
      {
        alt: 'x'.repeat(257), decorative: false, footprint: { width: 0.2, height: 0.4 }, priority: 1,
        layer: 'foreground', poses: { idle: { png: `/assets/${'d'.repeat(64)}/sprite-idle.png` } },
      },
      {
        alt: '', decorative: true, footprint: { width: 2, height: 0.4 }, priority: 1,
        layer: 'foreground', poses: { idle: { png: `/assets/${'e'.repeat(64)}/sprite-idle.png` } },
      },
      {
        alt: '', decorative: true, footprint: { width: 0.2, height: 0.4 }, priority: 1,
        layer: 'foreground',
        poses: { idle: { manifest: { url: `/assets/${'f'.repeat(64)}/manifest.json` } } },
      },
    ];
    const message = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 6,
      data: {
        title: 'Lantern Square', prose: [], exits: [],
        items: unsafeVisuals.map((_visual, index) => ({
          id: `o${String(index + 1).padStart(16, '0')}`,
          label: `item ${index}`, description: 'Still usable.',
          rarity: 'common', itemLevel: 1, weightGrams: 100,
          quantity: 1, stackSlots: 1, totalWeightGrams: 100,
          transferPolicy: 'transferable', boundToYou: false, actions: [],
        })),
        npcs: [],
        presentation: {
          entities: unsafeVisuals.map((visual, index) => ({
            id: `o${String(index + 1).padStart(16, '0')}`,
            ...visual,
          })),
        },
      },
    }));

    expect(message?.type).toBe('scene.snapshot');
    if (message?.type !== 'scene.snapshot') throw new Error('expected scene snapshot');
    expect(message.data.items).toHaveLength(6);
    expect(message.data.items.every((item) => item.description === 'Still usable.')).toBe(true);
    expect(message.data.presentation).toBeUndefined();
  });

  it('decorates message.append with only bounded semantic cues', () => {
    const cue = {
      kind: 'hit',
      entityId: 'o8888888888888888',
      pose: 'hit',
      intensity: 2,
      durationMs: 480,
    };
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 12,
      data: {
        revision: 7,
        kind: 'gameplay',
        text: 'The rain wolf recoils from the blow.',
        actorId: 'o7777777777777777',
        targetId: 'o8888888888888888',
        cue,
      },
    }))).toMatchObject({ type: 'message.append', data: { cue, text: 'The rain wolf recoils from the blow.' } });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'visual.cue', seq: 12, data: cue,
    }))).toBeUndefined();

    for (const invalid of [
      { ...cue, kind: 'status' },
      { ...cue, pose: 'javascript:alert(1)' },
      { ...cue, intensity: 4 },
      { ...cue, durationMs: 801 },
      { ...cue, entityId: 'o6666666666666666' },
    ]) {
      const message = parseServerMessage(JSON.stringify({
        v: 1,
        type: 'message.append',
        seq: 13,
        data: {
          revision: 8,
          kind: 'gameplay',
          text: 'Combat text remains authoritative.',
          actorId: 'o7777777777777777',
          targetId: 'o8888888888888888',
          cue: invalid,
        },
      }));
      expect(message).toMatchObject({
        type: 'message.append',
        data: { text: 'Combat text remains authoritative.' },
      });
      if (message?.type !== 'message.append') throw new Error('expected semantic message');
      expect(message.data.cue).toBeUndefined();
    }
  });

  it('bounds the sidecar annotation count and keeps it out of incremental entity messages', () => {
    const visual = {
      alt: '', decorative: true, footprint: { width: 0.2, height: 0.4 }, priority: 1,
      layer: 'foreground', poses: { idle: { png: `/assets/${'a'.repeat(64)}/sprite-idle.png` } },
    };
    const oversized = Array.from({ length: 13 }, (_, index) => ({
      id: `o${String(index).padStart(16, '0')}`,
      ...visual,
    }));
    const snapshot = parseServerMessage(JSON.stringify({
      v: 1, type: 'scene.snapshot', seq: 14,
      data: {
        title: 'Lantern Square', prose: [], items: [], npcs: [], exits: [],
        presentation: { entities: oversized },
      },
    }));
    expect(snapshot).toMatchObject({ type: 'scene.snapshot', data: { presentation: undefined } });

    const fourPoses = {
      ...visual,
      poses: Object.fromEntries((['idle', 'attack', 'hit', 'defeated'] as const).map((pose) => [
        pose,
        { png: `/assets/${'b'.repeat(64)}/sprite-${pose}.png` },
      ])),
    };
    const tooManyPoses = parseServerMessage(JSON.stringify({
      v: 1, type: 'scene.snapshot', seq: 14,
      data: {
        title: 'Lantern Square', prose: [], items: [], npcs: [], exits: [],
        presentation: {
          entities: Array.from({ length: 9 }, (_, index) => ({
            id: `o${String(index).padStart(16, '0')}`,
            ...fourPoses,
          })),
        },
      },
    }));
    expect(tooManyPoses).toMatchObject({
      type: 'scene.snapshot', data: { presentation: undefined },
    });

    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'scene.patch', seq: 15,
      data: {
        viewId: 'o9999999999999999', revision: 8, title: 'Night falls',
        presentation: { entities: [{ id: 'o8888888888888888', ...visual }] },
      },
    }))).toMatchObject({
      type: 'scene.patch',
      data: { viewId: 'o9999999999999999', revision: 8, title: 'Night falls' },
    });

    const upsert = parseServerMessage(JSON.stringify({
      v: 1, type: 'entity.upsert', seq: 16,
      data: {
        viewId: 'o9999999999999999', revision: 8, kind: 'item', id: 'o8888888888888888',
        entity: {
          id: 'o8888888888888888', label: 'lantern', description: 'Still usable.', actions: [],
          rarity: 'uncommon', itemLevel: 3, weightGrams: 800,
          quantity: 1, stackSlots: 1, totalWeightGrams: 800,
          transferPolicy: 'transferable', boundToYou: false,
          visual,
          presentation: { entities: [{ id: 'o8888888888888888', ...visual }] },
        },
      },
    }));
    expect(upsert).toMatchObject({
      type: 'entity.upsert', data: { entity: { id: 'o8888888888888888' } },
    });
    if (upsert?.type !== 'entity.upsert') throw new Error('expected entity upsert');
    expect('presentation' in upsert.data.entity).toBe(false);
    expect('visual' in upsert.data.entity).toBe(false);
  });

  it('validates typed scene, entity, vitals, inventory, and map updates', () => {
    const viewId = 'o9999999999999999';
    const itemId = 'o8888888888888888';
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'scene.patch', seq: 10,
      data: {
        viewId, revision: 4, title: 'Lantern Square at Night',
        weather: { condition: 'rain', day: 2, hour: 21, nextChangeInTicks: 4 },
      },
    }))).toMatchObject({ type: 'scene.patch', data: { viewId, revision: 4, weather: { condition: 'rain' } } });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'entity.upsert', seq: 11,
      data: {
        viewId, revision: 4, kind: 'item', id: itemId,
        entity: {
          id: itemId, label: 'blue lantern', description: 'Cold light.',
          rarity: 'uncommon', itemLevel: 3, weightGrams: 800,
          quantity: 1, stackSlots: 1, totalWeightGrams: 800,
          transferPolicy: 'transferable', boundToYou: false,
          actions: [{ label: 'Take', command: `take ${itemId}` }],
        },
      },
    }))).toMatchObject({ type: 'entity.upsert', data: { kind: 'item', id: itemId } });

    const player = validPlayerState();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'vitals.update', seq: 12,
      data: { revision: 4, character: player.character },
    }))).toMatchObject({ type: 'vitals.update', data: { revision: 4, character: { name: 'Aria' } } });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'inventory.patch', seq: 13,
      data: {
        revision: 4,
        inventory: player.inventory,
        equipment: player.equipment,
        inventoryCapacity: player.inventoryCapacity,
      },
    }))).toMatchObject({ type: 'inventory.patch', data: { revision: 4, inventory: [{ label: 'blue lantern' }] } });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'map.snapshot', seq: 14,
      data: { revision: 4, map: player.map },
    }))).toMatchObject({ type: 'map.snapshot', data: { revision: 4, map: { currentRoomId: 'o3333333333333333' } } });
  });

  it('validates bounded paginated container summaries', () => {
    const item = {
      id: 'o1111111111111111', label: 'healing tonic', rarity: 'uncommon', itemLevel: 2,
      weightGrams: 250, quantity: 12, availableQuantity: 12, reservedForTrade: 0,
      stackSlots: 1, totalWeightGrams: 3000,
      transferPolicy: 'transferable', boundToYou: false,
      canGive: false, canTrade: false,
      actions: [{ label: 'Get', command: 'get 12 o1111111111111111 from o2222222222222222' }],
    };
    const envelope = (overrides: Record<string, unknown> = {}) => ({
      v: 1, type: 'container.contents', seq: 21,
      data: {
        container: 'o2222222222222222', items: [item], page: 1, totalPages: 2,
        capacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 3000, maxWeightGrams: 5000 },
        ...overrides,
      },
    });
    expect(parseServerMessage(JSON.stringify(envelope()))).toMatchObject({
      type: 'container.contents',
      data: { container: 'o2222222222222222', page: 1, totalPages: 2, items: [{ quantity: 12 }] },
    });
    expect(parseServerMessage(JSON.stringify(envelope({ page: 3 })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({ items: Array.from({ length: 21 }, () => item) })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({
      items: [{ ...item, canGive: true }],
    })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({
      items: [{ ...item, totalWeightGrams: 2999 }],
    })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({
      items: [{
        ...item, quantity: 1, totalWeightGrams: 250,
        containerCapacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 100, maxWeightGrams: 5000 },
      }],
    })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({ prototypeId: 'tiamud:tonic' })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({
      items: [{ ...item, description: 'Prose is deliberately absent from this page.' }],
    })))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify(envelope({
      capacity: {
        usedSlots: 1, maxSlots: 8, usedWeightGrams: 3000, maxWeightGrams: 5000,
        durableContainerId: 'tiamud:satchel',
      },
    })))).toBeUndefined();
  });

  it('parses exact paginated sealed-reward odds without internal authoring ids', () => {
    const data = {
      selector: 'o6666666666666666',
      page: 1,
      totalPages: 2,
      guaranteed: [{ label: 'three sparks', quantity: 3 }],
      weighted: [{
        poolLabel: 'storm draw',
        rolls: 1,
        outcomeLabel: 'nothing',
        minQuantity: 0,
        maxQuantity: 0,
        probabilityNumerator: 1,
        probabilityDenominator: 4,
      }],
      affixes: [{
        parentOutcomeLabel: 'iron blade',
        affixLabel: 'charged',
        position: 'prefix',
        attackBonusDelta: 2,
        healAmountDelta: 0,
        conditionalProbabilityNumerator: 1,
        conditionalProbabilityDenominator: 5,
      }],
    };
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'reward.odds', seq: 30, data,
    }))).toMatchObject({
      type: 'reward.odds',
      data: {
        selector: data.selector,
        weighted: [{ probabilityNumerator: 1, probabilityDenominator: 4 }],
        affixes: [{ conditionalProbabilityNumerator: 1, conditionalProbabilityDenominator: 5 }],
      },
    });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'reward.odds', seq: 31,
      data: { ...data, page: 3 },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'reward.odds', seq: 32,
      data: { ...data, weighted: [{ ...data.weighted[0], lootTableId: 'secret' }] },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'reward.odds', seq: 33,
      data: { ...data, affixes: Array.from({ length: 20 }, () => data.affixes[0]) },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'reward.odds', seq: 34,
      data: {
        ...data,
        affixes: [{
          ...data.affixes[0],
          conditionalProbabilityNumerator: 6,
          conditionalProbabilityDenominator: 5,
        }],
      },
    }))).toBeUndefined();
  });

  it('projects only the public effective item affix shape', () => {
    const affix = {
      label: 'charged',
      position: 'prefix',
      attackBonusDelta: 2,
      healAmountDelta: 0,
    };
    const state = validPlayerState();
    const stateWithAffix = {
      ...state,
      inventory: [{ ...state.inventory[0], label: 'charged blue lantern', affix }],
    };
    const parsed = parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 35, data: stateWithAffix,
    }));
    expect(parsed).toMatchObject({
      type: 'player.snapshot',
      data: { inventory: [{ label: 'charged blue lantern', affix }] },
    });
    for (const invalidAffix of [
      { ...affix, position: 'middle' },
      { ...affix, attackBonusDelta: 0 },
      { ...affix, affixId: 'secret:charged' },
    ]) {
      expect(parseServerMessage(JSON.stringify({
        v: 1,
        type: 'player.snapshot',
        seq: 36,
        data: {
          ...validPlayerState(),
          inventory: [{ ...validPlayerState().inventory[0], affix: invalidAffix }],
        },
      }))).toBeUndefined();
    }
  });

  it('parses private trade state, reservations, and offered container pages', () => {
    const state = validPlayerState();
    const item = state.inventory[0];
    state.inventory = [{
      ...item,
      availableQuantity: 0,
      reservedForTrade: 1,
      canGive: false,
      canTrade: false,
    }];
    state.economy = {
      ...state.economy,
      reservedCoins: 10,
      spendableCoins: 30,
    };
    const trade = {
      id: 'oaaaaaaaaaaaaaaaa',
      phase: 'negotiating',
      counterparty: 'obbbbbbbbbbbbbbbb',
      expiresInTicks: 80,
      stateRevision: 4,
      ownConfirmed: false,
      counterpartyConfirmed: true,
      ownOffer: {
        coins: 10,
        lines: [{ target: item.id, label: item.label, quantity: 1, container: false }],
      },
      counterpartyOffer: { coins: 5, lines: [] },
      canAccept: false,
      canConfirm: true,
    };
    const parsed = parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 31, data: { ...state, trade },
    }));
    expect(parsed).toMatchObject({
      type: 'player.snapshot',
      data: {
        economy: { coins: 40, reservedCoins: 10, spendableCoins: 30 },
        inventory: [{ availableQuantity: 0, reservedForTrade: 1, canTrade: false }],
        trade: { id: trade.id, stateRevision: 4, counterpartyConfirmed: true },
      },
    });

    const { description: _description, ...summary } = state.inventory[0];
    const contents = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'trade.contents',
      seq: 32,
      data: {
        tradeId: trade.id,
        container: item.id,
        items: [{ ...summary, canGive: false, canTrade: false }],
        page: 1,
        totalPages: 1,
        capacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 800, maxWeightGrams: 5_000 },
      },
    }));
    expect(contents).toMatchObject({
      type: 'trade.contents',
      data: { tradeId: trade.id, container: item.id, page: 1 },
    });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'trade.contents', seq: 33,
      data: { ...(contents?.data ?? {}), hiddenOwner: 7 },
    }))).toBeUndefined();
  });

  it('preserves a manifest-only atmospheric image reference', () => {
    const hash = 'a'.repeat(64);
    const message = parseServerMessage(
      JSON.stringify({
        v: 1,
        type: 'scene.snapshot',
        seq: 4,
        data: {
          viewId: 'demo:lantern-square',
          title: 'Lantern Square',
          prose: ['Rain has polished the old stones.'],
          image: {
            alt: 'Lantern Square after rain.',
            localizedAlt: {
              en: 'Lantern Square after rain.',
              es: 'La Plaza del Farol tras la lluvia.',
            },
            manifest: `/assets/${hash}/manifest.json`,
          },
          exits: [{ id: 'north', label: 'North', command: 'north' }],
        },
      }),
    );

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: {
        prose: ['Rain has polished the old stones.'],
        image: {
          manifest: `/assets/${hash}/manifest.json`,
          localizedAlt: { en: 'Lantern Square after rain.', es: 'La Plaza del Farol tras la lluvia.' },
        },
        exits: [{ command: 'north' }],
      },
    });
  });

  it('selects localized image alt with exact, language, English, then deterministic fallback', () => {
    const image = {
      alt: 'Fallback.',
      localizedAlt: {
        es: 'Lluvia en la plaza.',
        en: 'Rain in the square.',
      },
    };
    expect(localizedImageAlt(image, 'es-MX')).toBe('Lluvia en la plaza.');
    expect(localizedImageAlt(image, 'fr-FR')).toBe('Rain in the square.');
    expect(localizedImageAlt({ alt: 'Fallback.', localizedAlt: { pt: 'Chuva.' } }, 'fr'))
      .toBe('Chuva.');
  });

  it('rejects direct image URLs outside the content-addressed asset contract', () => {
    const message = parseServerMessage(
      JSON.stringify({
        v: 1,
        type: 'scene.snapshot',
        seq: 5,
        data: {
          title: 'Unsafe image',
          prose: ['Text remains complete.'],
          image: { alt: 'Unsafe', jpeg: 'https://example.test/tracker.jpg' },
          exits: [],
        },
      }),
    );

    expect(message).toMatchObject({ type: 'scene.snapshot', data: { image: undefined } });
  });

  it('rejects malformed or unknown messages', () => {
    expect(parseServerMessage('{')).toBeUndefined();
    expect(parseServerMessage('{"type":"world.destroyed","data":{}}')).toBeUndefined();
  });

  it('parses projected events, correlated sync halves, and unsequenced heartbeats', () => {
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 10,
      data: {
        revision: 3,
        kind: 'itemTaken',
        text: 'You take the lantern.',
        actorId: 'o1111111111111111',
        itemId: 'o2222222222222222',
      },
    }))).toMatchObject({ type: 'message.append', seq: 10, data: { kind: 'itemTaken' } });
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 11,
      data: {
        revision: 4,
        kind: 'itemGiven',
        text: 'Aria gives the lantern to Bram.',
        actorId: 'o1111111111111111',
        targetId: 'o3333333333333333',
        itemId: 'o2222222222222222',
      },
    }))).toMatchObject({ type: 'message.append', data: { kind: 'itemGiven' } });
    const validGive = {
      revision: 4,
      kind: 'itemGiven',
      text: 'Aria gives the lantern to Bram.',
      actorId: 'o1111111111111111',
      targetId: 'o3333333333333333',
      itemId: 'o2222222222222222',
    };
    for (const invalidData of [
      { ...validGive, actorId: '12' },
      { ...validGive, targetId: '12' },
      { ...validGive, itemId: 'demo:blue-lantern' },
    ]) {
      expect(parseServerMessage(JSON.stringify({
        v: 1, type: 'message.append', seq: 12, data: invalidData,
      }))).toBeUndefined();
    }
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 13,
      data: {
        revision: 4,
        kind: 'adventure',
        text: 'The weather turns foggy.',
        actorId: 'o4444444444444444',
      },
    }))).toMatchObject({ type: 'message.append', data: { kind: 'adventure' } });

    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'sync.scene',
      seq: 50,
      data: {
        syncId: 'sync-50',
        afterSeq: 41,
        scene: { title: 'Lantern Square', prose: [], exits: [], items: [] },
      },
    }))).toMatchObject({
      type: 'sync.scene',
      seq: 50,
      data: { syncId: 'sync-50', afterSeq: 41, scene: { title: 'Lantern Square' } },
    });
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'sync.player',
      seq: 51,
      data: {
        syncId: 'sync-50',
        afterSeq: 41,
        player: validPlayerState(),
        complete: true,
      },
    }))).toMatchObject({
      type: 'sync.player',
      seq: 51,
      data: { syncId: 'sync-50', afterSeq: 41, complete: true, player: { revision: 3 } },
    });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'sync.snapshot', seq: 52,
      data: { afterSeq: 41, scene: {}, player: {} },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'sync.player', seq: 52,
      data: {
        syncId: 'unsafe', afterSeq: 41, player: validPlayerState(), complete: true,
      },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'sync.player', seq: 52,
      data: {
        syncId: 'sync-50', afterSeq: 41, player: validPlayerState(), complete: false,
      },
    }))).toBeUndefined();

    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'server.heartbeat',
      data: { nonce: 2, timeoutMs: 45_000 },
    }))).toEqual({
      v: 1,
      type: 'server.heartbeat',
      id: undefined,
      data: { nonce: 2, timeoutMs: 45_000 },
    });
  });

  it('accepts exactly the four camelCase trade ViewEvent kinds', () => {
    for (const [index, kind] of [
      'tradeRequested',
      'tradeChanged',
      'tradeCompleted',
      'tradeCancelled',
    ].entries()) {
      expect(parseServerMessage(JSON.stringify({
        v: 1,
        type: 'message.append',
        seq: 60 + index,
        data: {
          revision: 20 + index,
          kind,
          text: `Perception-safe ${kind} event.`,
          actorId: 'o1111111111111111',
          targetId: 'o2222222222222222',
        },
      }))).toMatchObject({
        type: 'message.append',
        data: { kind },
      });
    }
    for (const kind of [
      'TradeRequested',
      'trade_requested',
      'tradeComplete',
      'tradeUnknown',
    ]) {
      expect(parseServerMessage(JSON.stringify({
        v: 1,
        type: 'message.append',
        seq: 70,
        data: {
          revision: 30,
          kind,
          text: 'Unknown trade event.',
          actorId: 'o1111111111111111',
        },
      }))).toBeUndefined();
    }
  });

  it('accepts bounded player projections and ignores fields it does not project', () => {
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 16,
      data: validPlayerState(),
    }))).toMatchObject({
      type: 'player.snapshot',
      data: {
        character: { name: 'Aria', health: 21 },
        inventory: [{ id: 'o1111111111111111' }],
        map: { currentRoomId: 'o3333333333333333' },
        economy: { coins: 40 },
      },
    });

    const additive = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 17,
      data: { ...validPlayerState(), characterId: 42 },
    }));
    expect(additive).toMatchObject({ type: 'player.snapshot', data: { revision: 3 } });
    if (additive?.type !== 'player.snapshot') throw new Error('expected player snapshot');
    expect('characterId' in additive.data).toBe(false);
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 18,
      data: {
        ...validPlayerState(),
        inventory: Array.from({ length: 257 }, () => validPlayerState().inventory[0]),
      },
    }))).toBeUndefined();
    for (const inventoryItem of [
      { ...validPlayerState().inventory[0], rarity: 'mythic' },
      { ...validPlayerState().inventory[0], itemLevel: 1_001 },
      { ...validPlayerState().inventory[0], weightGrams: 1_000_001 },
      { ...validPlayerState().inventory[0], transferPolicy: 'account_bound' },
      { ...validPlayerState().inventory[0], transferPolicy: 'transferable', boundToYou: true },
      { ...validPlayerState().inventory[0], canGive: undefined },
      { ...validPlayerState().inventory[0], transferPolicy: 'non_transferable', canGive: true },
    ]) {
      expect(parseServerMessage(JSON.stringify({
        v: 1,
        type: 'player.snapshot',
        seq: 19,
        data: { ...validPlayerState(), inventory: [inventoryItem] },
      }))).toBeUndefined();
    }
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 20,
      data: {
        ...validPlayerState(),
        inventoryCapacity: {
          usedSlots: 25, maxSlots: 24, usedWeightGrams: 0, maxWeightGrams: 30_000,
        },
      },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 21,
      data: {
        ...validPlayerState(),
        inventoryCapacity: {
          usedSlots: 0, maxSlots: 1_000, usedWeightGrams: 0, maxWeightGrams: 0,
        },
      },
    }))).toBeDefined();
    for (const inventoryCapacity of [
      { usedSlots: 0, maxSlots: 1_001, usedWeightGrams: 0, maxWeightGrams: 1 },
      { usedSlots: 0, maxSlots: 1, usedWeightGrams: 0, maxWeightGrams: 1_000_001 },
    ]) {
      expect(parseServerMessage(JSON.stringify({
        v: 1,
        type: 'player.snapshot',
        seq: 22,
        data: { ...validPlayerState(), inventoryCapacity },
      }))).toBeUndefined();
    }
  });

  it('parses clean v3 origins, class evolution, talents, promotions, and abilities', () => {
    const baseState = validPlayerState();
    const state = {
      ...baseState,
      character: {
        ...baseState.character,
        lineage: { id: 'embermarked', label: 'Embermarked', description: 'Fire answers their blood.' },
        background: { id: 'archive', label: 'Archive Apprentice', description: 'Trained among old records.' },
        baseClass: { id: 'arcanist', label: 'Arcanist', description: 'Shapes disciplined magic.' },
        activeClass: { id: 'spellblade', label: 'Spellblade', description: 'Steel guided by ember lore.' },
        classHistory: [
          { id: 'arcanist', label: 'Arcanist', description: 'Shapes disciplined magic.' },
          { id: 'spellblade', label: 'Spellblade', description: 'Steel guided by ember lore.' },
        ],
        progressionPoints: { talent: 2 },
        talents: [{
          id: 'ember-path', label: 'Ember Path', description: 'Control fire without being consumed.',
          pointPoolId: 'talent', availablePoints: 2,
          nodes: [{
            id: 'steady-flame', label: 'Steady Flame', description: 'Keep a spell stable under pressure.',
            rank: 1, maxRank: 3, nextCost: 2, available: true,
            requirements: [{ met: true, text: 'Level 2' }], command: 'talent learn ember-path steady-flame',
          }],
        }],
        promotions: [{
          class: { id: 'stormcaller', label: 'Stormcaller', description: 'Calls lightning from rain.' },
          available: false, requirements: [{ met: false, text: 'Reach level 5' }],
          command: 'promote stormcaller',
        }],
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
          id: 'ember-bolt',
          name: 'Ember Bolt',
          description: 'Hurls a compact flame at a visible foe.',
          kind: 'spell',
          resourceCosts: [{ resourceId: 'mana', label: 'Mana', amount: 4 }],
          cooldownRemaining: 0,
          usable: true,
          targeting: 'explicitTarget',
        }],
      },
    };

    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 19, data: state,
    }))).toMatchObject({
      data: { character: {
        lineage: { id: 'embermarked' },
        background: { id: 'archive' },
        baseClass: { id: 'arcanist' },
        activeClass: { id: 'spellblade' },
        classHistory: [{ id: 'arcanist' }, { id: 'spellblade' }],
        progressionPoints: { talent: 2 },
        talents: [{ id: 'ember-path', nodes: [{ id: 'steady-flame', rank: 1, nextCost: 2 }] }],
        promotions: [{ class: { id: 'stormcaller' }, available: false }],
        attributes: [{ id: 'agility', value: 8 }, { id: 'intellect', value: 11 }],
        resistances: [{ damageType: 'fire', percent: 15 }],
        statuses: [{ id: 'focused', kind: 'attribute_modifier', remainingTicks: 2 }],
        resources: [{ id: 'mana', current: 12, max: 20 }],
        abilities: [{
          id: 'ember-bolt', kind: 'spell', resourceCosts: [{ resourceId: 'mana', amount: 4 }],
          targeting: 'explicitTarget', usable: true,
        }],
      } },
    });

    state.character.abilities[0]!.resourceCosts[0]!.amount = -1;
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 20, data: state,
    }))).toBeUndefined();
  });

  it('matches the authored character-sheet boundary limits', () => {
    const baseState = validPlayerState();
    const maximumId = 'a'.repeat(128);
    const maximumStatId = 's'.repeat(64);
    const state = {
      ...baseState,
      character: {
        ...baseState.character,
        lineage: { id: maximumId, label: 'Boundary', description: '' },
        baseClass: { id: maximumId, label: 'Boundary', description: '' },
        activeClass: { id: maximumId, label: 'Boundary', description: '' },
        classHistory: [{ id: maximumId, label: 'Boundary', description: '' }],
        attributes: Array.from({ length: 64 }, (_, index) => ({
          id: `stat-${index}`,
          label: `Stat ${index}`,
          value: 2_000_000,
        })),
        skills: [{ id: maximumStatId, label: 'Mastery', rank: 1_000 }],
        resources: [],
        abilities: [{
          id: maximumId,
          name: 'Boundary Ability',
          description: '',
          kind: 'utility',
          resourceCosts: [],
          cooldownRemaining: 10_000,
          usable: false,
          targeting: 'self',
        }],
        resistances: [{ damageType: 'lightning', percent: 90 }],
        statuses: [{
          id: maximumId,
          label: 'Boundary Status',
          kind: 'attribute_modifier',
          magnitude: -100_000,
          remainingTicks: 10_000,
        }],
      },
    };

    const message = parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 21, data: state,
    }));
    expect(message).toMatchObject({
      type: 'player.snapshot',
      data: { character: { lineage: { id: maximumId, description: '' } } },
    });
    if (message?.type !== 'player.snapshot') throw new Error('expected bounded player snapshot');
    expect(message.data.character.attributes).toHaveLength(64);
    expect(message.data.character.attributes.at(-1)).toMatchObject({ id: 'stat-63', value: 2_000_000 });
    expect(message.data.character.skills).toEqual([{ id: maximumStatId, label: 'Mastery', rank: 1_000 }]);
    expect(message.data.character.abilities[0]).toMatchObject({ id: maximumId, cooldownRemaining: 10_000 });

    state.character.attributes.push({ id: 'too-many', label: 'Too many', value: 1 });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 22, data: state,
    }))).toBeUndefined();
  });

  it('accepts only sequenced, immutable same-origin media instructions', () => {
    const hash = 'a'.repeat(64);
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'media.play',
      seq: 11,
      data: { channel: 'ambience', url: `/assets/${hash}/audio-rain.opus`, loop: true },
    }))).toMatchObject({ type: 'media.play', data: { channel: 'ambience', loop: true } });
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'media.play',
      seq: 12,
      data: { channel: 'music', url: 'https://tracker.test/music.mp3', loop: true },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'media.stop',
      seq: 13,
      data: {},
    }))).toMatchObject({ type: 'media.stop', data: {} });
  });

  it('parses bounded public chat projections and ignores additive policy fields', () => {
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'chat.message',
      seq: 14,
      data: { channel: 'tell', sender: 'Aria', text: 'Meet at the gate.' },
    }))).toMatchObject({
      type: 'chat.message',
      data: { channel: 'tell', sender: 'Aria', text: 'Meet at the gate.' },
    });
    const additive = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'chat.message',
      seq: 15,
      data: { channel: 'tell', sender: 'Aria', text: 'x', blocked: true },
    }));
    expect(additive).toMatchObject({
      type: 'chat.message',
      data: { channel: 'tell', sender: 'Aria', text: 'x' },
    });
    if (additive?.type !== 'chat.message') throw new Error('expected chat message');
    expect('blocked' in additive.data).toBe(false);

    for (const channel of ['announcement', 'party', 'general', 'newbie', 'staff'] as const) {
      expect(parseServerMessage(JSON.stringify({
        v: 1,
        type: 'chat.message',
        seq: 20,
        data: { channel, sender: 'Aria', text: 'The lantern is lit.' },
      }))).toMatchObject({
        type: 'chat.message',
        data: { channel, sender: 'Aria', text: 'The lantern is lit.' },
      });
    }
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'chat.message',
      seq: 21,
      data: { channel: 'trade', sender: 'Aria', text: 'Lantern for sale.' },
    }))).toBeUndefined();
  });

  it('ignores additive server fields in scene, player, message, and split sync envelopes', () => {
    const scene = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 30,
      rollout: 'next',
      data: {
        title: 'Lantern Square',
        prose: [],
        exits: [],
        items: [],
        futureLightingModel: { exposure: 0.8 },
      },
    }));
    expect(scene).toMatchObject({ type: 'scene.snapshot', seq: 30, data: { title: 'Lantern Square' } });
    if (scene?.type !== 'scene.snapshot') throw new Error('expected scene snapshot');
    expect('rollout' in scene).toBe(false);
    expect('futureLightingModel' in scene.data).toBe(false);

    const player = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 31,
      data: {
        ...validPlayerState(),
        futureProgression: { renown: 4 },
        character: { ...validPlayerState().character, futureAffinity: 'storm' },
      },
    }));
    expect(player).toMatchObject({ type: 'player.snapshot', data: { character: { name: 'Aria' } } });
    if (player?.type !== 'player.snapshot') throw new Error('expected player snapshot');
    expect('futureProgression' in player.data).toBe(false);
    expect('futureAffinity' in player.data.character).toBe(false);

    const message = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 32,
      traceContext: 'ignored',
      data: {
        revision: 4,
        kind: 'gameplay',
        text: 'The rain wolf circles.',
        actorId: 'o7777777777777777',
        futurePresentation: { emphasis: 'ominous' },
      },
    }));
    expect(message).toMatchObject({ type: 'message.append', data: { text: 'The rain wolf circles.' } });
    if (message?.type !== 'message.append') throw new Error('expected appended message');
    expect('traceContext' in message).toBe(false);
    expect('futurePresentation' in message.data).toBe(false);

    const syncScene = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'sync.scene',
      seq: 33,
      data: {
        syncId: 'sync-33',
        afterSeq: 29,
        scene: { title: 'Lantern Square', futureLightingModel: 'v2' },
        futureSyncWindow: 2,
      },
    }));
    const syncPlayer = parseServerMessage(JSON.stringify({
      v: 1,
      type: 'sync.player',
      seq: 34,
      data: {
        syncId: 'sync-33',
        afterSeq: 29,
        player: { ...validPlayerState(), futureProgression: true },
        complete: true,
        futureSyncWindow: 2,
      },
    }));
    expect(syncScene).toMatchObject({
      type: 'sync.scene', data: { syncId: 'sync-33', afterSeq: 29, scene: { title: 'Lantern Square' } },
    });
    expect(syncPlayer).toMatchObject({
      type: 'sync.player', data: { syncId: 'sync-33', afterSeq: 29, complete: true },
    });
    if (syncScene?.type !== 'sync.scene' || syncPlayer?.type !== 'sync.player') {
      throw new Error('expected split sync messages');
    }
    expect('futureSyncWindow' in syncScene.data).toBe(false);
    expect('futureLightingModel' in syncScene.data.scene).toBe(false);
    expect('futureSyncWindow' in syncPlayer.data).toBe(false);
    expect('futureProgression' in syncPlayer.data.player).toBe(false);
  });

  it('still rejects malformed required known fields after ignoring additions', () => {
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'scene.snapshot', seq: 40, data: { title: 42, futureField: true },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'player.snapshot', seq: 41,
      data: { ...validPlayerState(), revision: '3', futureField: true },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'message.append', seq: 42,
      data: {
        revision: 4, kind: 'futureKind', text: 'Still malformed.', actorId: 'opaque', futureField: true,
      },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'sync.player', seq: 43,
      data: {
        syncId: 'sync-43', afterSeq: 39, player: validPlayerState(), complete: false, futureField: true,
      },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'scene.patch', seq: 44,
      data: { viewId: 'o9999999999999999', revision: 9, futureField: true },
    }))).toBeUndefined();
  });

  it('rejects invalid versions and missing sequences while ignoring additive envelope fields', () => {
    const data = { title: 'Lantern Square', prose: [], exits: [], items: [] };
    expect(parseServerMessage(JSON.stringify({ v: 2, type: 'scene.snapshot', seq: 1, data }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({ v: 1, type: 'scene.snapshot', data }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 1,
      data,
      unexpected: true,
    }))).toMatchObject({ type: 'scene.snapshot', seq: 1, data: { title: 'Lantern Square' } });
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'action.result',
      seq: 2,
      data: { requestId: '', ok: true },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'server.heartbeat',
      seq: 3,
      data: { nonce: 2, timeoutMs: 45_000 },
    }))).toBeUndefined();
  });
});

describe('websocket URL', () => {
  it('uses the Tiamud WebSocket subprotocol', () => {
    expect(WEBSOCKET_SUBPROTOCOL).toBe('tiamud.v1');
  });

  it('uses secure WebSockets on HTTPS', () => {
    expect(websocketUrl({ protocol: 'https:', host: 'mud.example' })).toBe(
      'wss://mud.example/ws',
    );
  });

  it('uses local WebSockets on HTTP', () => {
    expect(websocketUrl({ protocol: 'http:', host: 'localhost:5173' })).toBe(
      'ws://localhost:5173/ws',
    );
  });
});
