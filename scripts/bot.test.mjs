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
      inventory: [sceneItem({ actions: [{ label: 'Drop', command: `drop ${OPAQUE_ITEM}` }] })],
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
      economy: { coins: 0, commodities: [], shopAvailable: false, shop: [] },
      social: { online: [] },
    },
  };
}

test('strict bot accepts the complete bounded phase-3B scene item contract', () => {
  assert.equal(parseServerMessage(JSON.stringify(sceneEnvelope())).data.items[0].rarity, 'uncommon');

  for (const invalid of [
    sceneItem({ rarity: 'mythic' }),
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
    'announcement', 'say', 'tell', 'party', 'general', 'newbie', 'staff',
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
        actions: [],
      }],
    }),
  ];
  for (const invalid of invalidPatches) {
    assert.throws(() => parseServerMessage(JSON.stringify(invalid)));
  }
});

test('strict bot validates paginated container summaries without durable ids or prose trees', () => {
  const data = {
    container: OPAQUE_EQUIPMENT,
    items: [sceneItem({
      description: undefined,
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
    prototypeId: 'tiamud:archive-satchel',
  }), /unknown keys: prototypeId/);
});

test('strict bot accepts the current v3 character projection and fails closed', () => {
  const parsed = parseServerMessage(JSON.stringify(playerSnapshot()));
  assert.equal(parsed.data.character.lineage.id, 'tiamud:human');

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
