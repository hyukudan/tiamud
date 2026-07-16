import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WEBSOCKET_SUBPROTOCOL } from './brand';
import {
  SERVER_SILENCE_TIMEOUT_MS,
  SYNC_RESPONSE_TIMEOUT_MS,
  TiamudConnection,
} from './socket';
import type { ConnectionState, ServerMessage } from './protocol';

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
  },
  inventory: [],
  equipment: [],
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
  economy: { coins: 40, commodities: [], shopAvailable: false, shop: [] },
  social: { online: [{ id: 'o2222222222222222', name: 'Aria', selfCharacter: true }] },
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

  it('sends ability use as a structured action without changing legacy perform', () => {
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

    expect(connection.perform('item.take', 'o8888888888888888', 'legacy-1')).toBe(true);
    expect(lastSent(socket)).toMatchObject({
      data: { requestId: 'legacy-1', action: 'item.take', target: 'o8888888888888888' },
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

  it('requests a sync on a sequence gap and suppresses stale scene snapshots', () => {
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
      type: 'sync.snapshot',
      seq: 6,
      data: {
        afterSeq: 1,
        scene: { title: 'Lantern Square', prose: [], exits: [], items: [] },
        player: validPlayerState(),
      },
    }));

    expect(received.map((message) => message.type)).toEqual([
      'server.welcome',
      'action.result',
      'sync.snapshot',
    ]);
    expect(received.at(-1)).toMatchObject({ data: { scene: { title: 'Lantern Square' } } });
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

  it('cancels the sync timeout after an authoritative sync snapshot', () => {
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
    socket.receive(JSON.stringify({
      v: 1,
      type: 'sync.snapshot',
      seq: 4,
      data: {
        afterSeq: 1,
        scene: { title: 'Lantern Square', prose: [], exits: [], items: [] },
        player: validPlayerState(),
      },
    }));

    vi.advanceTimersByTime(SYNC_RESPONSE_TIMEOUT_MS);

    expect(socket.closeCalls).not.toContainEqual({
      code: 4002,
      reason: 'State synchronization timeout',
    });
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
