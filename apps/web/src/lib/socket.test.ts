import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WEBSOCKET_SUBPROTOCOL } from './brand';
import {
  SERVER_SILENCE_TIMEOUT_MS,
  SYNC_RESPONSE_TIMEOUT_MS,
  TiamudConnection,
} from './socket';
import type { ConnectionState, ServerMessage, SyncRecoveryData } from './protocol';

type SocketListener = (event: unknown) => void;

class TestWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: TestWebSocket[] = [];

  readonly sent: string[] = [];
  readonly closeCalls: Array<{ code?: number; reason?: string }> = [];
  readonly listeners = new Map<string, Set<SocketListener>>();
  readyState = TestWebSocket.CONNECTING;

  constructor(
    readonly url: string,
    readonly protocols?: string | string[],
  ) {
    TestWebSocket.instances.push(this);
  }

  addEventListener(name: string, callback: SocketListener): void {
    const listeners = this.listeners.get(name) ?? new Set<SocketListener>();
    listeners.add(callback);
    this.listeners.set(name, listeners);
  }

  send(message: string): void {
    this.sent.push(message);
  }

  close(code?: number, reason?: string): void {
    this.closeCalls.push({ code, reason });
    this.readyState = TestWebSocket.CLOSED;
    this.emit('close', {});
  }

  open(): void {
    this.readyState = TestWebSocket.OPEN;
    this.emit('open', {});
  }

  receive(message: unknown): void {
    this.emit('message', { data: message });
  }

  private emit(name: string, event: unknown): void {
    for (const callback of this.listeners.get(name) ?? []) {
      callback(event);
    }
  }
}

function lastSent(socket: TestWebSocket): Record<string, unknown> {
  return JSON.parse(socket.sent.at(-1) ?? '{}') as Record<string, unknown>;
}

const validPlayerState = () => ({
  revision: 1,
  character: {
    name: 'Aria', level: 1, experience: 0, nextLevelExperience: 100,
    health: 20, maxHealth: 20, strength: 5, defence: 2, effects: [], recoveryInTicks: null,
    lineage: { id: 'mortal', label: 'Mortal', description: 'Adaptable people.' },
    background: null,
    baseClass: { id: 'wanderer', label: 'Wanderer', description: 'A flexible explorer.' },
    activeClass: { id: 'wanderer', label: 'Wanderer', description: 'A flexible explorer.' },
    classHistory: [{ id: 'wanderer', label: 'Wanderer', description: 'A flexible explorer.' }],
    progressionPoints: {}, talents: [], promotions: [],
  },
  inventory: [],
  equipment: [],
  inventoryCapacity: {
    usedSlots: 0, maxSlots: 24, usedWeightGrams: 0, maxWeightGrams: 30000,
  },
  map: {
    currentRoomId: 'o1111111111111111',
    rooms: [{ id: 'o1111111111111111', title: 'Lantern Square', current: true }],
    paths: [],
  },
  quests: [{
    id: 'first-hunt', label: 'First Hunt', summary: 'Defeat one hostile creature.',
    status: 'available', current: 0, target: 1,
    actions: [{ label: 'Accept', command: 'quest accept first-hunt' }],
  }],
  partyInvites: [],
  economy: {
    coins: 40, reservedCoins: 0, spendableCoins: 40,
    commodities: [], shopAvailable: false, shop: [],
  },
  social: {
    online: [{ id: 'o2222222222222222', name: 'Aria', selfCharacter: true, nearby: true }],
  },
});

beforeEach(() => {
  TestWebSocket.instances = [];
  vi.stubGlobal('WebSocket', TestWebSocket);
  vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('TiamudConnection', () => {
  it('offers only the versioned Tiamud protocol during the WebSocket handshake', () => {
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'token', characterId: 7 },
      { onMessage: () => undefined, onStateChange: () => undefined },
    );
    connection.connect();

    expect(TestWebSocket.instances[0]?.protocols).toEqual([WEBSOCKET_SUBPROTOCOL]);
    expect(TestWebSocket.instances[0]?.protocols).toEqual(['tiamud.v1']);
    connection.close();
  });

  it('sends ability and progression choices as native structured actions', () => {
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'token', characterId: 7 },
      { onMessage: () => undefined, onStateChange: () => undefined },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();

    expect(connection.performAbility('ember-bolt', 'o9999999999999999', 'ability-1')).toBe(true);
    expect(lastSent(socket)).toEqual({
      v: 1,
      type: 'action.perform',
      data: {
        requestId: 'ability-1',
        action: 'combat.ability',
        abilityId: 'ember-bolt',
        target: 'o9999999999999999',
      },
    });

    expect(connection.perform('item.take', { target: 'o8888888888888888' }, 'legacy-1')).toBe(true);
    expect(lastSent(socket)).toMatchObject({
      data: { requestId: 'legacy-1', action: 'item.take', target: 'o8888888888888888' },
    });
    expect(connection.perform(
      'item.give',
      { target: 'o8888888888888888', recipient: 'o7777777777777777', quantity: 2 },
      'give-1',
    )).toBe(true);
    expect(lastSent(socket)).toMatchObject({
      data: {
        requestId: 'give-1', action: 'item.give', target: 'o8888888888888888',
        recipient: 'o7777777777777777', quantity: 2,
      },
    });
    expect(connection.perform(
      'social.channel.say',
      { channel: 'general', message: 'Lanterns are lit.' },
      'channel-1',
    )).toBe(true);
    expect(lastSent(socket)).toMatchObject({
      data: {
        requestId: 'channel-1', action: 'social.channel.say',
        channel: 'general', message: 'Lanterns are lit.',
      },
    });
    expect(connection.promote('stormcaller', 'promote-1')).toBe(true);
    expect(lastSent(socket)).toMatchObject({
      data: { requestId: 'promote-1', action: 'progression.promote', classId: 'stormcaller' },
    });
    expect(connection.acquireTalent('ember-path', 'steady-flame', 'talent-1')).toBe(true);
    expect(lastSent(socket)).toMatchObject({
      data: {
        requestId: 'talent-1', action: 'progression.talent',
        graphId: 'ember-path', nodeId: 'steady-flame',
      },
    });
    connection.close();
  });

  it('authenticates every socket hello with the selected character', () => {
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42, expiresAt: Date.now() + 60_000 },
      { onMessage: () => undefined, onStateChange: () => undefined },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();

    expect(JSON.parse(socket.sent[0] ?? '{}')).toMatchObject({
      type: 'client.hello',
      data: { auth: { token: 'secret', characterId: 42 } },
    });
    expect(JSON.parse(socket.sent[0] ?? '{}').data.auth).toEqual({
      token: 'secret',
      characterId: 42,
    });
    connection.close();
  });

  it('reconnects immediately and announces changed presentation capabilities', () => {
    const states: ConnectionState[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      { onMessage: () => undefined, onStateChange: (state) => states.push(state) },
      { images: true, reducedMotion: false, audio: false },
    );
    connection.connect();
    const first = TestWebSocket.instances[0]!;
    first.open();

    expect(connection.updateCapabilities({
      images: false,
      reducedMotion: true,
      audio: true,
    })).toBe(true);
    expect(first.closeCalls).toContainEqual({
      code: 4001,
      reason: 'Client capabilities changed',
    });
    expect(TestWebSocket.instances).toHaveLength(2);

    const second = TestWebSocket.instances[1]!;
    second.open();
    expect(JSON.parse(second.sent[0] ?? '{}')).toMatchObject({
      type: 'client.hello',
      data: {
        capabilities: { images: [], reducedMotion: true, audio: true },
      },
    });
    expect(states.slice(-2)).toEqual(['connecting', 'connected']);
    expect(connection.updateCapabilities({
      images: false,
      reducedMotion: true,
      audio: true,
    })).toBe(false);
    connection.close();
  });

  it('acknowledges server heartbeats without exposing them as game messages', () => {
    const received: ServerMessage[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      { onMessage: (message) => received.push(message), onStateChange: () => undefined },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.heartbeat',
      data: { nonce: 17, timeoutMs: 45_000 },
    }));

    expect(lastSent(socket)).toEqual({
      v: 1,
      type: 'client.heartbeat',
      data: { nonce: 17 },
    });
    expect(received).toEqual([]);
    connection.close();
  });

  it('requests a sync on a sequence gap and atomically publishes only a matching pair', () => {
    const received: ServerMessage[] = [];
    const recoveries: SyncRecoveryData[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      {
        onMessage: (message) => received.push(message),
        onSync: (recovery) => recoveries.push(recovery),
        onStateChange: () => undefined,
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.welcome',
      seq: 1,
      data: { serverName: 'Tiamud' },
    }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'action.result',
      seq: 3,
      data: { requestId: 'request-1', ok: true, message: null },
    }));

    expect(lastSent(socket)).toEqual({
      v: 1,
      type: 'sync.request',
      data: { afterSeq: 1 },
    });

    socket.receive(JSON.stringify({
      v: 1,
      type: 'scene.snapshot',
      seq: 4,
      data: { title: 'Stale road', prose: [], exits: [], items: [] },
    }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'player.snapshot',
      seq: 5,
      data: validPlayerState(),
    }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'sync.scene',
      seq: 6,
      data: {
        syncId: 'sync-6',
        afterSeq: 1,
        scene: { title: 'Lantern Square', prose: [], exits: [], items: [] },
      },
    }));
    expect(recoveries).toEqual([]);
    socket.receive(JSON.stringify({
      v: 1,
      type: 'sync.player',
      seq: 7,
      data: {
        syncId: 'sync-6',
        afterSeq: 1,
        player: validPlayerState(),
        complete: true,
      },
    }));

    expect(received.map((message) => message.type)).toEqual([
      'server.welcome',
      'action.result',
    ]);
    expect(recoveries).toHaveLength(1);
    expect(recoveries[0]).toMatchObject({
      syncId: 'sync-6',
      afterSeq: 1,
      scene: { title: 'Lantern Square' },
      player: { revision: 1 },
    });
    socket.receive(JSON.stringify({
      v: 1, type: 'server.notice', seq: 8,
      data: { level: 'info', message: 'Recovery complete.' },
    }));
    expect(received.map((message) => message.type)).toEqual([
      'server.welcome',
      'action.result',
      'server.notice',
    ]);
    connection.close();
  });

  it('reconnects instead of suppressing snapshots forever when sync is unanswered', () => {
    vi.useFakeTimers();
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      { onMessage: () => undefined, onStateChange: () => undefined },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({ v: 1, type: 'server.welcome', seq: 1, data: {} }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'action.result',
      seq: 3,
      data: { requestId: 'request-1', ok: true, message: null },
    }));

    vi.advanceTimersByTime(SYNC_RESPONSE_TIMEOUT_MS);

    expect(socket.closeCalls).toContainEqual({
      code: 4002,
      reason: 'State synchronization timeout',
    });
    connection.close();
  });

  it('keeps the timeout until both correlated halves arrive, then clears it', () => {
    vi.useFakeTimers();
    const recoveries: SyncRecoveryData[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      {
        onMessage: () => undefined,
        onSync: (recovery) => recoveries.push(recovery),
        onStateChange: () => undefined,
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({ v: 1, type: 'server.welcome', seq: 1, data: {} }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'action.result',
      seq: 3,
      data: { requestId: 'request-1', ok: true, message: null },
    }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'sync.scene',
      seq: 4,
      data: {
        syncId: 'sync-4',
        afterSeq: 1,
        scene: { title: 'Lantern Square', prose: [], exits: [], items: [] },
      },
    }));
    vi.advanceTimersByTime(SYNC_RESPONSE_TIMEOUT_MS - 1);
    expect(recoveries).toEqual([]);
    expect(socket.closeCalls).not.toContainEqual({
      code: 4002,
      reason: 'State synchronization timeout',
    });
    socket.receive(JSON.stringify({
      v: 1,
      type: 'sync.player',
      seq: 5,
      data: {
        syncId: 'sync-4',
        afterSeq: 1,
        player: validPlayerState(),
        complete: true,
      },
    }));

    vi.advanceTimersByTime(SYNC_RESPONSE_TIMEOUT_MS);

    expect(recoveries).toHaveLength(1);
    expect(socket.closeCalls).not.toContainEqual({
      code: 4002,
      reason: 'State synchronization timeout',
    });
    connection.close();
  });

  it('ignores duplicate, stale and mismatched recovery halves', () => {
    const recoveries: SyncRecoveryData[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      {
        onMessage: () => undefined,
        onSync: (recovery) => recoveries.push(recovery),
        onStateChange: () => undefined,
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({ v: 1, type: 'server.welcome', seq: 1, data: {} }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'action.result',
      seq: 3,
      data: { requestId: 'request-1', ok: true, message: null },
    }));

    const scene = { title: 'Lantern Square', prose: [], exits: [], items: [] };
    socket.receive(JSON.stringify({
      v: 1, type: 'sync.scene', seq: 4,
      data: { syncId: 'sync-4', afterSeq: 1, scene },
    }));
    socket.receive(JSON.stringify({
      v: 1, type: 'sync.scene', seq: 5,
      data: { syncId: 'sync-4', afterSeq: 1, scene: { ...scene, title: 'Duplicate' } },
    }));
    socket.receive(JSON.stringify({
      v: 1, type: 'sync.player', seq: 6,
      data: { syncId: 'sync-6', afterSeq: 1, player: validPlayerState(), complete: true },
    }));
    socket.receive(JSON.stringify({
      v: 1, type: 'sync.player', seq: 7,
      data: { syncId: 'sync-4', afterSeq: 0, player: validPlayerState(), complete: true },
    }));
    expect(recoveries).toEqual([]);
    socket.receive(JSON.stringify({
      v: 1, type: 'sync.player', seq: 8,
      data: { syncId: 'sync-4', afterSeq: 1, player: validPlayerState(), complete: true },
    }));
    expect(recoveries).toHaveLength(1);
    expect(recoveries[0]?.scene.title).toBe('Lantern Square');

    socket.receive(JSON.stringify({
      v: 1, type: 'sync.player', seq: 8,
      data: { syncId: 'sync-4', afterSeq: 1, player: validPlayerState(), complete: true },
    }));
    socket.receive(JSON.stringify({
      v: 1, type: 'sync.scene', seq: 9,
      data: { syncId: 'sync-9', afterSeq: 1, scene },
    }));
    expect(recoveries).toHaveLength(1);
    connection.close();
  });

  it('ignores replayed sequence numbers', () => {
    const received: ServerMessage[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      { onMessage: (message) => received.push(message), onStateChange: () => undefined },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({ v: 1, type: 'server.welcome', seq: 1, data: {} }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.notice',
      seq: 1,
      data: { level: 'warning', message: 'replayed' },
    }));

    expect(received).toHaveLength(1);
    connection.close();
  });

  it('closes a silent server connection so the reconnect path can run', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-15T00:00:00Z'));
    const states: ConnectionState[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'secret', characterId: 42 },
      { onMessage: () => undefined, onStateChange: (state) => states.push(state) },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();

    vi.advanceTimersByTime(SERVER_SILENCE_TIMEOUT_MS);

    expect(socket.closeCalls).toContainEqual({ code: 4000, reason: 'Server heartbeat timeout' });
    expect(states.at(-1)).toBe('disconnected');
    connection.close();
  });

  it('stops reconnecting and reports an authenticated entry rejection', () => {
    vi.useFakeTimers();
    const rejections: string[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'expired', characterId: 42 },
      {
        onMessage: () => undefined,
        onStateChange: () => undefined,
        onAccessRejected: (message) => rejections.push(message),
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.notice',
      seq: 1,
      data: {
        level: 'error',
        code: 'access.session_expired',
        message: 'Your session has expired.',
      },
    }));

    vi.advanceTimersByTime(60_000);

    expect(rejections).toEqual(['Your session has expired.']);
    expect(socket.closeCalls).toContainEqual({ code: 4003, reason: 'Access rejected' });
    expect(TestWebSocket.instances).toHaveLength(1);
  });

  it('stops retrying a transient entry rejection without invalidating credentials', () => {
    vi.useFakeTimers();
    const rejections: Array<{ message: string; clearCredentials: boolean }> = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'active', characterId: 42 },
      {
        onMessage: () => undefined,
        onStateChange: () => undefined,
        onAccessRejected: (message, clearCredentials) =>
          rejections.push({ message, clearCredentials }),
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.notice',
      seq: 1,
      data: {
        level: 'error',
        code: 'access.session_limit',
        message: 'This account has reached its concurrent session limit.',
      },
    }));

    vi.advanceTimersByTime(60_000);

    expect(rejections).toEqual([{
      message: 'This account has reached its concurrent session limit.',
      clearCredentials: false,
    }]);
    expect(TestWebSocket.instances).toHaveLength(1);
  });

  it('does not confuse an operational error after welcome with access rejection', () => {
    const rejections: string[] = [];
    const received: ServerMessage[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      { token: 'active', characterId: 42 },
      {
        onMessage: (message) => received.push(message),
        onStateChange: () => undefined,
        onAccessRejected: (message) => rejections.push(message),
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.welcome',
      seq: 1,
      data: { serverName: 'Tiamud' },
    }));
    socket.receive(JSON.stringify({
      v: 1,
      type: 'server.notice',
      seq: 2,
      data: { level: 'error', message: 'A command failed.' },
    }));

    expect(rejections).toEqual([]);
    expect(received.map((message) => message.type)).toEqual([
      'server.welcome',
      'server.notice',
    ]);
    expect(socket.closeCalls).toEqual([]);
    connection.close();
  });

  it('expires a live client credential without entering the reconnect loop', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-16T00:00:00Z'));
    const rejections: string[] = [];
    const connection = new TiamudConnection(
      'wss://tiamud.test/ws',
      {
        token: 'expiring',
        characterId: 42,
        expiresAt: Date.now() + 1_000,
      },
      {
        onMessage: () => undefined,
        onStateChange: () => undefined,
        onAccessRejected: (message) => rejections.push(message),
      },
    );
    connection.connect();
    const socket = TestWebSocket.instances[0]!;
    socket.open();

    vi.advanceTimersByTime(1_000);

    expect(rejections).toEqual(['Your session has expired. Sign in again.']);
    expect(TestWebSocket.instances).toHaveLength(1);
  });
});
