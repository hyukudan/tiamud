import assert from 'node:assert/strict';
import test from 'node:test';

import { parseServerMessage } from './bot.mjs';

const OPAQUE_ITEM = 'o0000000000000001';
const OPAQUE_EQUIPMENT = 'o0000000000000002';
const OPAQUE_ACTOR = 'o0000000000000003';
const OPAQUE_TARGET = 'o0000000000000004';
const ASSET_HASH = 'a'.repeat(64);
const AUDIO_URL = `/assets/${ASSET_HASH}/audio-bell.ogg`;

function envelope(type, data, seq = 1) {
  return { v: 1, type, seq, data };
}

function parseEnvelope(type, data, seq = 1) {
  return parseServerMessage(JSON.stringify(envelope(type, data, seq)));
}

function sceneItem(overrides = {}) {
  return {
    id: OPAQUE_ITEM,
    label: 'blue lantern',
    description: 'A reusable prototype projected as one concrete instance.',
    rarity: 'uncommon',
    itemLevel: 3,
    weightGrams: 800,
    quantity: 1,
    stackSlots: 1,
    totalWeightGrams: 800,
    transferPolicy: 'transferable',
    boundToYou: false,
    actions: [{ label: 'Take', command: `take ${OPAQUE_ITEM}` }],
    ...overrides,
  };
}

function sceneEnvelope(item = sceneItem()) {
  return {
    v: 1,
    type: 'scene.snapshot',
    seq: 1,
    data: {
      viewId: 'o0000000000000010',
      revision: 1,
      title: 'Lantern Square',
      prose: ['A bounded scene.'],
      exits: [],
      items: [item],
      npcs: [],
      weather: { condition: 'clear', day: 1, hour: 0, nextChangeInTicks: 1 },
    },
  };
}

function entityPresentation(overrides = {}) {
  return {
    id: OPAQUE_ITEM,
    alt: 'A blue lantern sprite.',
    decorative: false,
    footprint: { width: 0.2, height: 0.3 },
    preferredAnchor: { x: 0.5, y: 0.8 },
    priority: 10,
    layer: 'foreground',
    poses: {
      idle: {
        webp: `/assets/${ASSET_HASH}/sprite-idle-320.webp`,
        png: `/assets/${ASSET_HASH}/sprite-idle-320.png`,
      },
      hit: { webp: `/assets/${ASSET_HASH}/sprite-hit.webp` },
    },
    ...overrides,
  };
}

function inventoryPatch(overrides = {}) {
  return {
    v: 1,
    type: 'inventory.patch',
    seq: 2,
    data: {
      revision: 2,
      inventory: [sceneItem({
        canGive: true,
        availableQuantity: 1,
        reservedForTrade: 0,
        canTrade: true,
        actions: [{ label: 'Drop', command: `drop ${OPAQUE_ITEM}` }],
      })],
      equipment: [{
        id: OPAQUE_EQUIPMENT,
        slot: 'weapon',
        label: 'iron sword',
        rarity: 'common',
        itemLevel: 2,
        weightGrams: 2_200,
        quantity: 1,
        stackSlots: 1,
        totalWeightGrams: 2_200,
        transferPolicy: 'transferable',
        boundToYou: false,
        actions: [{ label: 'Unequip', command: 'unequip weapon' }],
      }],
      inventoryCapacity: {
        usedSlots: 1,
        maxSlots: 24,
        usedWeightGrams: 3_000,
        maxWeightGrams: 30_000,
      },
      ...overrides,
    },
  };
}

function playerSnapshot(characterOverrides = {}) {
  const origin = (id, label) => ({ id, label, description: '' });
  return {
    v: 1,
    type: 'player.snapshot',
    seq: 3,
    data: {
      revision: 3,
      character: {
        name: 'Load Explorer',
        lineage: origin('tiamud:human', 'Human'),
        baseClass: origin('tiamud:guardian', 'Guardian'),
        activeClass: origin('tiamud:guardian', 'Guardian'),
        classHistory: [origin('tiamud:guardian', 'Guardian')],
        level: 1,
        experience: 0,
        nextLevelExperience: 100,
        health: 20,
        maxHealth: 20,
        strength: 5,
        defence: 2,
        attributes: [],
        skills: [],
        resources: [],
        abilities: [],
        resistances: [],
        statuses: [],
        progressionPoints: {},
        talents: [],
        promotions: [],
        effects: [],
        recoveryInTicks: null,
        ...characterOverrides,
      },
      inventory: [],
      equipment: [],
      inventoryCapacity: {
        usedSlots: 0,
        maxSlots: 24,
        usedWeightGrams: 0,
        maxWeightGrams: 30_000,
      },
      map: {
        currentRoomId: 'o0000000000000020',
        rooms: [{ id: 'o0000000000000020', title: 'Lantern Square', current: true }],
        paths: [],
      },
      quests: [],
      partyInvites: [],
      economy: {
        coins: 0, reservedCoins: 0, spendableCoins: 0,
        commodities: [], shopAvailable: false, shop: [],
      },
      social: { online: [] },
    },
  };
}

test('strict bot accepts the complete bounded phase-3B scene item contract', () => {
  assert.equal(parseServerMessage(JSON.stringify(sceneEnvelope())).data.items[0].rarity, 'uncommon');

  for (const invalid of [
    sceneItem({ rarity: 'mythic' }),
    sceneItem({ transferPolicy: 'account_bound' }),
    sceneItem({ transferPolicy: 'transferable', boundToYou: true }),
    sceneItem({ itemLevel: 0 }),
    sceneItem({ itemLevel: 1_001 }),
    sceneItem({ weightGrams: -1 }),
    sceneItem({ weightGrams: 1_000_001 }),
    sceneItem({ quantity: 0 }),
    sceneItem({ quantity: 2, totalWeightGrams: 800 }),
    sceneItem({ stackSlots: 0 }),
    sceneItem({
      containerCapacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 100, maxWeightGrams: 5000 },
      totalWeightGrams: 800,
    }),
  ]) {
    assert.throws(() => parseServerMessage(JSON.stringify(sceneEnvelope(invalid))));
  }
  assert.throws(
    () => parseServerMessage(JSON.stringify(sceneEnvelope({ ...sceneItem(), prototypeId: 'secret' }))),
    /unknown keys: prototypeId/,
  );
});

test('strict bot accepts only perception-safe relative loot access', () => {
  const yours = parseServerMessage(JSON.stringify(sceneEnvelope(sceneItem({
    loot: { access: 'yours', expiresInTicks: 20 },
    actions: [
      { label: 'Take', command: `take ${OPAQUE_ITEM}` },
      { label: 'Pass', command: `loot pass ${OPAQUE_ITEM}` },
    ],
  }))));
  assert.deepEqual(yours.data.items[0].loot, { access: 'yours', expiresInTicks: 20 });

  const reserved = parseServerMessage(JSON.stringify(sceneEnvelope(sceneItem({
    loot: { access: 'reserved' },
    actions: [],
  }))));
  assert.deepEqual(reserved.data.items[0].loot, { access: 'reserved' });
  assert.equal(sceneEnvelope(sceneItem()).data.items[0].loot, undefined);

  for (const loot of [
    { access: 'yours' },
    { access: 'yours', expiresInTicks: 0 },
    { access: 'yours', expiresInTicks: 21 },
    { access: 'reserved', expiresInTicks: 4 },
    { access: 'reserved', beneficiary: 42 },
    { access: 'public' },
  ]) {
    assert.throws(() => parseServerMessage(JSON.stringify(sceneEnvelope(sceneItem({ loot })))));
  }
  assert.throws(() => parseServerMessage(JSON.stringify(sceneEnvelope(sceneItem({
    loot: { access: 'reserved' },
    actions: [{ label: 'Take', command: `take ${OPAQUE_ITEM}` }],
  })))), /unauthorized action/);
});

test('strict bot accepts opaque itemGiven projections without durable ownership', () => {
  const event = {
    revision: 8,
    kind: 'itemGiven',
    text: 'Aria gives a blue lantern to Bram.',
    actorId: OPAQUE_ACTOR,
    targetId: OPAQUE_TARGET,
    itemId: OPAQUE_ITEM,
  };
  assert.equal(parseEnvelope('message.append', event).data.kind, 'itemGiven');
  assert.throws(() => parseEnvelope('message.append', {
    ...event,
    ownerCharacterId: 42,
  }));
});

test('strict bot accepts exactly the four camelCase trade ViewEvent kinds', () => {
  for (const [index, kind] of [
    'tradeRequested',
    'tradeChanged',
    'tradeCompleted',
    'tradeCancelled',
  ].entries()) {
    assert.equal(parseEnvelope('message.append', {
      revision: 20 + index,
      kind,
      text: `Perception-safe ${kind} event.`,
      actorId: OPAQUE_ACTOR,
      targetId: OPAQUE_TARGET,
    }).data.kind, kind);
  }
  for (const kind of [
    'TradeRequested',
    'trade_requested',
    'tradeComplete',
    'tradeUnknown',
  ]) {
    assert.throws(() => parseEnvelope('message.append', {
      revision: 30,
      kind,
      text: 'Unknown trade event.',
      actorId: OPAQUE_ACTOR,
    }), /Unknown message\.append kind/);
  }
});

test('strict bot accepts bounded entity presentation and scene audio without weakening secrecy', () => {
  const valid = sceneEnvelope();
  valid.data.presentation = { entities: [entityPresentation()] };
  valid.data.audio = { channel: 'ambience', url: AUDIO_URL, loop: true };
  assert.equal(
    parseServerMessage(JSON.stringify(valid)).data.presentation.entities[0].id,
    OPAQUE_ITEM,
  );

  const invalidCases = [
    { entities: [entityPresentation({ id: 'o0000000000000099' })] },
    { entities: [entityPresentation({ footprint: { width: 0.61, height: 0.3 } })] },
    { entities: [entityPresentation({ poses: { idle: { webp: '/remote/sprite.webp' } } })] },
    { entities: [entityPresentation({ prototypeId: 'tiamud:lantern' })] },
  ];
  for (const presentation of invalidCases) {
    const candidate = sceneEnvelope();
    candidate.data.presentation = presentation;
    assert.throws(() => parseServerMessage(JSON.stringify(candidate)));
  }

  const tooMany = sceneEnvelope();
  tooMany.data.presentation = {
    entities: Array.from({ length: 13 }, (_, index) => entityPresentation({
      id: `o${(index + 1).toString(16).padStart(16, '0')}`,
    })),
  };
  assert.throws(() => parseServerMessage(JSON.stringify(tooMany)), /12-entry bound/);

  const badAudio = sceneEnvelope();
  badAudio.data.audio = { channel: 'voice', url: AUDIO_URL, loop: true };
  assert.throws(() => parseServerMessage(JSON.stringify(badAudio)), /channel is invalid/);
});

test('strict bot accepts current media frames and rejects remote, oversized, or extended forms', () => {
  assert.doesNotThrow(() => parseEnvelope(
    'media.play',
    { channel: 'effects', url: AUDIO_URL, loop: false },
  ));
  assert.doesNotThrow(() => parseEnvelope('media.stop', { channel: 'effects' }));
  assert.doesNotThrow(() => parseEnvelope('media.stop', {}));

  for (const data of [
    { channel: 'voice', url: AUDIO_URL, loop: false },
    { channel: 'effects', url: 'https://example.test/audio.ogg', loop: false },
    { channel: 'effects', url: `/assets/${ASSET_HASH}/audio-${'a'.repeat(130)}.ogg`, loop: false },
    { channel: 'effects', url: AUDIO_URL, loop: false, autoplay: true },
  ]) {
    assert.throws(() => parseEnvelope('media.play', data));
  }
});

test('strict bot accepts bounded combat cues and rejects unrelated or extended cues', () => {
  const event = {
    revision: 2,
    kind: 'gameplay',
    text: 'The blow lands.',
    actorId: OPAQUE_ACTOR,
    targetId: OPAQUE_TARGET,
    cue: {
      kind: 'hit',
      entityId: OPAQUE_TARGET,
      pose: 'hit',
      intensity: 2,
      durationMs: 320,
    },
  };
  assert.equal(parseEnvelope('message.append', event).data.cue.kind, 'hit');

  for (const cue of [
    { ...event.cue, entityId: OPAQUE_ITEM },
    { ...event.cue, durationMs: 801 },
    { ...event.cue, stableId: 'tiamud:npc' },
  ]) {
    assert.throws(() => parseEnvelope('message.append', { ...event, cue }));
  }
});

test('strict bot accepts every social scope and bounded access notice codes', () => {
  for (const channel of [
    'announcement', 'say', 'tell', 'telepathy', 'emote', 'party', 'general', 'newbie', 'staff',
  ]) {
    assert.equal(
      parseEnvelope('chat.message', { channel, sender: 'Tiamud', text: 'Ready.' }).data.channel,
      channel,
    );
  }
  assert.throws(() => parseEnvelope(
    'chat.message',
    { channel: 'general', sender: 'Tiamud', text: 'x'.repeat(513) },
  ));
  assert.throws(() => parseEnvelope(
    'chat.message',
    { channel: 'general', sender: 'Tiamud', text: 'Ready.', durableId: 1 },
  ));

  assert.equal(parseEnvelope('server.notice', {
    level: 'error',
    code: 'access.session_invalid',
    message: 'The session is invalid.',
  }).data.code, 'access.session_invalid');
  assert.throws(() => parseEnvelope('server.notice', {
    level: 'error', code: 'access session', message: 'Invalid.',
  }));
  assert.throws(() => parseEnvelope('server.notice', {
    level: 'error', code: 'access.invalid', message: 'Invalid.', internal: true,
  }));
});

test('strict bot validates both correlated sync recovery halves', () => {
  const scene = sceneEnvelope().data;
  scene.presentation = { entities: [entityPresentation()] };
  const player = playerSnapshot().data;
  assert.equal(parseEnvelope('sync.scene', {
    syncId: 'sync-17', afterSeq: 12, scene,
  }).data.syncId, 'sync-17');
  assert.equal(parseEnvelope('sync.player', {
    syncId: 'sync-17', afterSeq: 12, player, complete: true,
  }).data.complete, true);

  assert.throws(() => parseEnvelope('sync.scene', {
    syncId: 'sync-0', afterSeq: 12, scene,
  }));
  assert.throws(() => parseEnvelope('sync.player', {
    syncId: 'sync-17', afterSeq: 12, player, complete: false,
  }));
  assert.throws(() => parseEnvelope('sync.player', {
    syncId: 'sync-17', afterSeq: 12, player, complete: true, durableRevision: 2,
  }));
});

test('strict bot validates inventory, equipment, and capacity as one coherent projection', () => {
  assert.equal(
    parseServerMessage(JSON.stringify(inventoryPatch())).data.inventoryCapacity.usedWeightGrams,
    3_000,
  );

  const emptyAtZeroWeight = inventoryPatch({
    inventory: [],
    equipment: [],
    inventoryCapacity: {
      usedSlots: 0,
      maxSlots: 1_000,
      usedWeightGrams: 0,
      maxWeightGrams: 0,
    },
  });
  assert.doesNotThrow(() => parseServerMessage(JSON.stringify(emptyAtZeroWeight)));

  const invalidPatches = [
    inventoryPatch({
      inventoryCapacity: {
        usedSlots: 1,
        maxSlots: 24,
        usedWeightGrams: 2_999,
        maxWeightGrams: 30_000,
      },
    }),
    inventoryPatch({
      inventoryCapacity: {
        usedSlots: 0,
        maxSlots: 24,
        usedWeightGrams: 3_000,
        maxWeightGrams: 30_000,
      },
    }),
    inventoryPatch({
      inventoryCapacity: {
        usedSlots: 1,
        maxSlots: 1_001,
        usedWeightGrams: 3_000,
        maxWeightGrams: 30_000,
      },
    }),
    inventoryPatch({
      equipment: [{
        id: OPAQUE_ITEM,
        slot: 'weapon',
        label: 'duplicate instance',
        rarity: 'rare',
        itemLevel: 1,
        weightGrams: 2_200,
        quantity: 1,
        stackSlots: 1,
        totalWeightGrams: 2_200,
        transferPolicy: 'transferable',
        boundToYou: false,
        actions: [],
      }],
    }),
  ];
  for (const invalid of invalidPatches) {
    assert.throws(() => parseServerMessage(JSON.stringify(invalid)));
  }
  assert.throws(() => parseServerMessage(JSON.stringify(inventoryPatch({
    inventory: [sceneItem({ canGive: undefined })],
  }))));
  assert.throws(() => parseServerMessage(JSON.stringify(inventoryPatch({
    inventory: [sceneItem({ transferPolicy: 'non_transferable', canGive: true })],
  }))));
});

test('strict bot validates paginated container summaries without durable ids or prose trees', () => {
  const data = {
    container: OPAQUE_EQUIPMENT,
    items: [sceneItem({
      description: undefined,
      canGive: false,
      availableQuantity: 3,
      reservedForTrade: 0,
      canTrade: false,
      quantity: 3,
      totalWeightGrams: 2_400,
      actions: [{
        label: 'Get',
        command: `get 3 ${OPAQUE_ITEM} from ${OPAQUE_EQUIPMENT}`,
      }],
    })],
    page: 1,
    totalPages: 2,
    capacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 2_400, maxWeightGrams: 5_000 },
  };
  assert.equal(parseEnvelope('container.contents', data).data.items[0].quantity, 3);
  assert.throws(() => parseEnvelope('container.contents', { ...data, page: 3 }));
  assert.throws(() => parseEnvelope('container.contents', {
    ...data,
    items: Array.from({ length: 21 }, () => data.items[0]),
  }));
  assert.throws(() => parseEnvelope('container.contents', {
    ...data,
    items: [{ ...data.items[0], canGive: true }],
  }));
  assert.throws(() => parseEnvelope('container.contents', {
    ...data,
    prototypeId: 'tiamud:archive-satchel',
  }), /unknown keys: prototypeId/);
});

test('strict bot validates exact sealed-reward odds without authoring ids', () => {
  const data = {
    selector: OPAQUE_ITEM,
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
  const parsed = parseEnvelope('reward.odds', data);
  assert.equal(parsed.data.weighted[0].probabilityDenominator, 4);
  assert.equal(parsed.data.affixes[0].conditionalProbabilityDenominator, 5);
  assert.throws(() => parseEnvelope('reward.odds', { ...data, page: 3 }));
  assert.throws(() => parseEnvelope('reward.odds', {
    ...data,
    weighted: [{ ...data.weighted[0], lootTableId: 'secret' }],
  }), /unknown keys: lootTableId/);
  assert.throws(() => parseEnvelope('reward.odds', {
    ...data,
    affixes: Array.from({ length: 20 }, () => data.affixes[0]),
  }));
  assert.throws(() => parseEnvelope('reward.odds', {
    ...data,
    affixes: [{
      ...data.affixes[0],
      conditionalProbabilityNumerator: 6,
      conditionalProbabilityDenominator: 5,
    }],
  }));
});

test('strict bot accepts only the public item-affix projection', () => {
  const affix = {
    label: 'charged',
    position: 'prefix',
    attackBonusDelta: 2,
    healAmountDelta: 0,
  };
  const parsed = parseServerMessage(JSON.stringify(sceneEnvelope(sceneItem({
    label: 'charged blue lantern',
    affix,
  }))));
  assert.deepEqual(parsed.data.items[0].affix, affix);
  for (const invalid of [
    { ...affix, position: 'middle' },
    { ...affix, attackBonusDelta: 0 },
    { ...affix, affixId: 'secret:charged' },
  ]) {
    assert.throws(() => parseServerMessage(
      JSON.stringify(sceneEnvelope(sceneItem({ affix: invalid }))),
    ));
  }
});

test('strict bot validates private confirmed-trade state and paginated offered contents', () => {
  const player = playerSnapshot();
  player.data.economy = {
    ...player.data.economy,
    coins: 20,
    reservedCoins: 4,
    spendableCoins: 16,
  };
  player.data.trade = {
    id: 'o0000000000000030',
    phase: 'negotiating',
    counterparty: OPAQUE_TARGET,
    expiresInTicks: 90,
    stateRevision: 3,
    ownConfirmed: false,
    counterpartyConfirmed: true,
    ownOffer: { coins: 4, lines: [] },
    counterpartyOffer: {
      coins: 0,
      lines: [{ target: OPAQUE_ITEM, label: 'blue lantern', quantity: 1, container: false }],
    },
    canAccept: false,
    canConfirm: true,
  };
  assert.equal(parseServerMessage(JSON.stringify(player)).data.trade.stateRevision, 3);
  assert.throws(() => parseServerMessage(JSON.stringify({
    ...player,
    data: { ...player.data, trade: { ...player.data.trade, basisSha256: 'secret' } },
  })), /unknown keys: basisSha256/);
  assert.throws(() => parseServerMessage(JSON.stringify({
    ...player,
    data: {
      ...player.data,
      economy: { ...player.data.economy, spendableCoins: 17 },
    },
  })));

  const item = sceneItem({
    description: undefined,
    canGive: false,
    availableQuantity: 0,
    reservedForTrade: 1,
    canTrade: false,
    actions: [],
  });
  const contents = {
    tradeId: player.data.trade.id,
    container: OPAQUE_EQUIPMENT,
    items: [item],
    page: 1,
    totalPages: 1,
    capacity: { usedSlots: 1, maxSlots: 8, usedWeightGrams: 800, maxWeightGrams: 5_000 },
  };
  assert.equal(parseEnvelope('trade.contents', contents).data.tradeId, player.data.trade.id);
  assert.throws(() => parseEnvelope('trade.contents', { ...contents, ownerId: 42 }));
});

test('strict bot accepts the current v3 character projection and fails closed', () => {
  const telepathy = {
    id: 'tiamud:mind-thread',
    name: 'Mind Thread',
    description: 'Carries a private thought.',
    kind: 'utility',
    resourceCosts: [{ resourceId: 'mana', label: 'Mana', amount: 2 }],
    cooldownRemaining: 0,
    usable: true,
    purpose: 'telepathy',
    targeting: 'optionalTarget',
  };
  const parsed = parseServerMessage(JSON.stringify(playerSnapshot({
    resources: [{ id: 'mana', label: 'Mana', current: 4, max: 8 }],
    abilities: [telepathy],
  })));
  assert.equal(parsed.data.character.lineage.id, 'tiamud:human');
  assert.equal(parsed.data.character.abilities[0].purpose, 'telepathy');

  assert.throws(
    () => parseServerMessage(JSON.stringify(playerSnapshot({
      resources: [{ id: 'mana', label: 'Mana', current: 4, max: 8 }],
      abilities: [{ ...telepathy, purpose: 'mail' }],
    }))),
    /purpose is invalid/,
  );

  assert.throws(
    () => parseServerMessage(JSON.stringify(playerSnapshot({ durableCharacterId: 42 }))),
    /unknown keys: durableCharacterId/,
  );
  assert.throws(
    () => parseServerMessage(JSON.stringify(playerSnapshot({ level: 101 }))),
    /level exceeds 100/,
  );
  assert.throws(
    () => parseServerMessage(JSON.stringify(playerSnapshot({
      statuses: [{
        id: 'tiamud:burning',
        label: 'Burning',
        kind: 'damage_over_time',
        magnitude: 1,
        remainingTicks: 10_001,
      }],
    }))),
    /duration exceeds 10000/,
  );
});
