import { describe, expect, it } from 'vitest';
import {
  createActionPerform,
  createClientHello,
  createCommandSubmit,
  createHeartbeatAck,
  createSyncRequest,
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
  },
  inventory: [{
    id: 'o1111111111111111', label: 'blue lantern', description: 'Cold light.',
    actions: [{ label: 'Drop', command: 'drop o1111111111111111' }],
  }],
  equipment: [{
    id: 'o2222222222222222', slot: 'weapon', label: 'bronze sword',
    actions: [{ label: 'Unequip', command: 'unequip weapon' }],
  }],
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
    commodities: [],
    shopAvailable: true,
    shop: [{
      id: 'tonic', label: 'healing tonic', buyPrice: 12, sellPrice: 6,
      stock: 20, owned: 0, actions: [{ label: 'Buy', command: 'buy tonic' }],
    }],
  },
  social: {
    online: [{ id: 'o4444444444444444', name: 'Aria', selfCharacter: true }],
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
    expect(createActionPerform('item.take', 'lantern', 'request-2')).toEqual({
      v: 1,
      type: 'action.perform',
      data: { requestId: 'request-2', action: 'item.take', target: 'lantern' },
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
          actions: [
            { label: 'Examine', command: 'examine tiamud:blue-lantern' },
            { label: 'Take', command: 'take tiamud:blue-lantern' },
          ],
        }],
      },
    }));

    expect(message).toMatchObject({
      type: 'scene.snapshot',
      data: { items: [{
        id: 'tiamud:blue-lantern',
        label: 'blue-glass lantern',
        actions: [{ command: 'examine tiamud:blue-lantern' }, { command: 'take tiamud:blue-lantern' }],
      }] },
    });
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
      data: { revision: 4, inventory: player.inventory, equipment: player.equipment },
    }))).toMatchObject({ type: 'inventory.patch', data: { revision: 4, inventory: [{ label: 'blue lantern' }] } });
    expect(parseServerMessage(JSON.stringify({
      v: 1, type: 'map.snapshot', seq: 14,
      data: { revision: 4, map: player.map },
    }))).toMatchObject({ type: 'map.snapshot', data: { revision: 4, map: { currentRoomId: 'o3333333333333333' } } });
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

  it('parses projected events, sync snapshots, and unsequenced heartbeats', () => {
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 10,
      data: {
        revision: 3,
        kind: 'itemTaken',
        text: 'You take the lantern.',
        actorId: 'opaque-character',
        itemId: 'opaque-item',
      },
    }))).toMatchObject({ type: 'message.append', seq: 10, data: { kind: 'itemTaken' } });
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'message.append',
      seq: 11,
      data: {
        revision: 4,
        kind: 'adventure',
        text: 'The weather turns foggy.',
        actorId: 'opaque-world',
      },
    }))).toMatchObject({ type: 'message.append', data: { kind: 'adventure' } });

    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'sync.snapshot',
      seq: 11,
      data: {
        afterSeq: 8,
        scene: { title: 'Lantern Square', prose: [], exits: [], items: [] },
        player: validPlayerState(),
      },
    }))).toMatchObject({
      type: 'sync.snapshot',
      seq: 11,
      data: { afterSeq: 8, scene: { title: 'Lantern Square' } },
    });

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

  it('accepts bounded player projections and rejects durable or policy state', () => {
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

    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 17,
      data: { ...validPlayerState(), characterId: 42 },
    }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 18,
      data: {
        ...validPlayerState(),
        inventory: Array.from({ length: 257 }, () => validPlayerState().inventory[0]),
      },
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

  it('parses bounded public chat projections without accepting policy fields', () => {
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'chat.message',
      seq: 14,
      data: { channel: 'tell', sender: 'Aria', text: 'Meet at the gate.' },
    }))).toMatchObject({
      type: 'chat.message',
      data: { channel: 'tell', sender: 'Aria', text: 'Meet at the gate.' },
    });
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'chat.message',
      seq: 15,
      data: { channel: 'tell', sender: 'Aria', text: 'x', blocked: true },
    }))).toBeUndefined();
  });

  it('rejects invalid versions, missing sequences, and unknown envelope fields', () => {
    const data = { title: 'Lantern Square', prose: [], exits: [], items: [] };
    expect(parseServerMessage(JSON.stringify({ v: 2, type: 'scene.snapshot', seq: 1, data }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({ v: 1, type: 'scene.snapshot', data }))).toBeUndefined();
    expect(parseServerMessage(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 1,
      data,
      unexpected: true,
    }))).toBeUndefined();
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
