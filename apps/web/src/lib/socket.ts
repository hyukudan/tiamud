import {
  createAbilityPerform,
  createActionPerform,
  createClientHello,
  createCommandSubmit,
  createHeartbeatAck,
  createPromotionPerform,
  createSyncRequest,
  createTalentPerform,
  parseServerMessage,
  type ActionPerformFields,
  type ConnectionState,
  type LiveServerMessage,
  type ServerMessage,
  type SyncRecoveryData,
} from './protocol';
import { WEBSOCKET_SUBPROTOCOL } from './brand';

export interface ConnectionCallbacks {
  onMessage(message: LiveServerMessage): void;
  onSync?(recovery: SyncRecoveryData): void;
  onStateChange(state: ConnectionState): void;
  onProtocolError?(): void;
  onAccessRejected?(message: string, clearCredentials: boolean): void;
}

interface SyncBuffer {
  syncId: string;
  afterSeq: number;
  highestSequence: number;
  scene?: SyncRecoveryData['scene'];
  sceneSequence?: number;
  player?: SyncRecoveryData['player'];
  playerSequence?: number;
}

export interface ConnectionAuth {
  token: string;
  characterId: number;
  expiresAt?: number;
}

export interface BrowserCapabilities {
  images: boolean;
  reducedMotion: boolean;
  audio: boolean;
}

type LocationLike = Pick<Location, 'protocol' | 'host'>;

export const SERVER_SILENCE_TIMEOUT_MS = 45_000;
export const SYNC_RESPONSE_TIMEOUT_MS = 10_000;
const WATCHDOG_INTERVAL_MS = 5_000;
const MAX_TIMER_DELAY_MS = 2_147_000_000;

export function websocketUrl(location: LocationLike): string {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${location.host}/ws`;
}

export class TiamudConnection {
  readonly #callbacks: ConnectionCallbacks;
  readonly #url: string;
  readonly #auth: ConnectionAuth;
  #capabilities?: BrowserCapabilities;
  #socket?: WebSocket;
  #reconnectTimer?: ReturnType<typeof setTimeout>;
  #reconnectAttempts = 0;
  #closedByClient = false;
  #lastSequence?: number;
  #syncPending = false;
  #syncAfterSeq?: number;
  #syncBuffer?: SyncBuffer;
  #syncTimer?: ReturnType<typeof setTimeout>;
  #lastServerActivity = 0;
  #watchdogTimer?: ReturnType<typeof setInterval>;
  #authExpiryTimer?: ReturnType<typeof setTimeout>;
  #welcomed = false;
  #accessRejected = false;
  #reconnectImmediately = false;

  constructor(
    url: string,
    auth: ConnectionAuth,
    callbacks: ConnectionCallbacks,
    capabilities?: BrowserCapabilities,
  ) {
    this.#url = url;
    this.#auth = auth;
    this.#callbacks = callbacks;
    this.#capabilities = capabilities;
  }

  connect(): void {
    if (this.#accessRejected) {
      return;
    }
    if (this.#socket?.readyState === WebSocket.OPEN || this.#socket?.readyState === WebSocket.CONNECTING) {
      return;
    }

    this.#closedByClient = false;
    this.#callbacks.onStateChange('connecting');

    const socket = new WebSocket(this.#url, [WEBSOCKET_SUBPROTOCOL]);
    this.#socket = socket;

    socket.addEventListener('open', () => {
      if (this.#socket !== socket) return;
      this.#reconnectAttempts = 0;
      this.#lastSequence = undefined;
      this.#clearSyncWait();
      this.#welcomed = false;
      this.#lastServerActivity = Date.now();
      if (this.#authHasExpired()) {
        this.#rejectAccess('Your session has expired. Sign in again.', socket, true);
        return;
      }
      this.#scheduleAuthExpiry(socket);
      this.#startWatchdog(socket);
      this.#callbacks.onStateChange('connected');
      const reducedMotion = this.#capabilities?.reducedMotion ??
        window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      socket.send(JSON.stringify(createClientHello(
        reducedMotion,
        this.#auth,
        this.#capabilities?.images ?? true,
        this.#capabilities?.audio ?? false,
      )));
    });

    socket.addEventListener('message', (event: MessageEvent<unknown>) => {
      if (this.#socket !== socket) return;
      if (typeof event.data !== 'string') {
        this.#callbacks.onProtocolError?.();
        return;
      }

      const message = parseServerMessage(event.data);
      if (message) {
        this.#lastServerActivity = Date.now();
        if (message.type === 'server.heartbeat') {
          if (socket.readyState === WebSocket.OPEN) {
            socket.send(JSON.stringify(createHeartbeatAck(message.data.nonce)));
          }
          return;
        }
        if (message.type === 'server.welcome') {
          this.#welcomed = true;
        } else if (
          !this.#welcomed &&
          message.type === 'server.notice' &&
          message.seq === 1 &&
          message.data.level === 'error' &&
          message.data.code?.startsWith('access.')
        ) {
          this.#rejectAccess(
            message.data.message,
            socket,
            message.data.code === 'access.session_expired' ||
              message.data.code === 'access.session_invalid',
          );
          return;
        }
        this.#handleSequenced(message, socket);
      } else {
        this.#callbacks.onProtocolError?.();
      }
    });

    socket.addEventListener('close', () => {
      if (this.#socket !== socket) return;
      this.#stopConnectionTimers();
      this.#socket = undefined;
      this.#callbacks.onStateChange('disconnected');
      if (this.#closedByClient || this.#accessRejected) return;
      if (this.#reconnectImmediately) {
        this.#reconnectImmediately = false;
        this.connect();
      } else {
        this.#scheduleReconnect();
      }
    });
  }

  updateCapabilities(capabilities: BrowserCapabilities): boolean {
    if (sameCapabilities(this.#capabilities, capabilities)) {
      return false;
    }
    this.#capabilities = { ...capabilities };
    if (this.#closedByClient || this.#accessRejected) {
      return true;
    }

    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#reconnectImmediately = true;
    const socket = this.#socket;
    if (socket?.readyState === WebSocket.OPEN || socket?.readyState === WebSocket.CONNECTING) {
      socket.close(4001, 'Client capabilities changed');
    } else if (!socket) {
      this.#reconnectImmediately = false;
      this.connect();
    }
    return true;
  }

  submit(command: string, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) {
      return false;
    }

    this.#socket.send(JSON.stringify(createCommandSubmit(command, requestId)));
    return true;
  }

  perform(action: string, fields: ActionPerformFields | undefined, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) {
      return false;
    }

    this.#socket.send(JSON.stringify(createActionPerform(action, fields, requestId)));
    return true;
  }

  performAbility(abilityId: string, target: string | undefined, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) {
      return false;
    }

    this.#socket.send(JSON.stringify(createAbilityPerform(abilityId, target, requestId)));
    return true;
  }

  promote(classId: string, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) return false;
    this.#socket.send(JSON.stringify(createPromotionPerform(classId, requestId)));
    return true;
  }

  acquireTalent(graphId: string, nodeId: string, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) return false;
    this.#socket.send(JSON.stringify(createTalentPerform(graphId, nodeId, requestId)));
    return true;
  }

  close(): void {
    this.#closedByClient = true;
    this.#reconnectImmediately = false;
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#stopConnectionTimers();
    this.#socket?.close(1000, 'Client closed');
    this.#socket = undefined;
  }

  #scheduleReconnect(): void {
    this.#reconnectAttempts += 1;
    const delay = Math.min(10_000, 500 * 2 ** (this.#reconnectAttempts - 1));
    this.#reconnectTimer = setTimeout(() => this.connect(), delay);
  }

  #handleSequenced(message: ServerMessage, socket: WebSocket): void {
    const sequence = message.seq;
    if (sequence === undefined) {
      this.#callbacks.onProtocolError?.();
      return;
    }

    if (message.type === 'sync.scene' || message.type === 'sync.player') {
      this.#handleSyncHalf(message, sequence);
      return;
    }

    if (this.#lastSequence === undefined) {
      this.#lastSequence = sequence;
      if (message.type !== 'server.welcome' || sequence !== 1) {
        this.#requestSync(socket, 0);
      }
      if (this.#syncPending && (message.type === 'scene.snapshot' || message.type === 'player.snapshot')) {
        return;
      }
      this.#callbacks.onMessage(message);
      return;
    }

    if (sequence <= this.#lastSequence) {
      return;
    }
    const expected = this.#lastSequence + 1;
    const previous = this.#lastSequence;
    this.#lastSequence = sequence;
    if (sequence !== expected) {
      this.#requestSync(socket, previous);
    }

    if (this.#syncPending && (message.type === 'scene.snapshot' || message.type === 'player.snapshot')) {
      return;
    }
    this.#callbacks.onMessage(message);
  }

  #requestSync(socket: WebSocket, afterSeq: number): void {
    if (this.#syncPending || socket.readyState !== WebSocket.OPEN) {
      return;
    }
    this.#syncPending = true;
    this.#syncAfterSeq = afterSeq;
    this.#syncBuffer = undefined;
    socket.send(JSON.stringify(createSyncRequest(afterSeq)));
    this.#syncTimer = setTimeout(() => {
      if (this.#socket === socket && this.#syncPending && socket.readyState === WebSocket.OPEN) {
        socket.close(4002, 'State synchronization timeout');
      }
    }, SYNC_RESPONSE_TIMEOUT_MS);
  }

  #handleSyncHalf(
    message: Extract<ServerMessage, { type: 'sync.scene' | 'sync.player' }>,
    sequence: number,
  ): void {
    if (!this.#syncPending || this.#syncAfterSeq === undefined ||
      message.data.afterSeq !== this.#syncAfterSeq ||
      (this.#lastSequence !== undefined && sequence <= this.#lastSequence)) return;

    let buffer = this.#syncBuffer;
    if (!buffer || buffer.syncId !== message.data.syncId) {
      if (buffer) return;
      buffer = {
        syncId: message.data.syncId,
        afterSeq: message.data.afterSeq,
        highestSequence: sequence,
      };
      this.#syncBuffer = buffer;
    } else {
      if (sequence <= buffer.highestSequence) return;
      buffer.highestSequence = sequence;
    }

    if (message.type === 'sync.scene') {
      if (buffer.scene !== undefined) return;
      buffer.scene = message.data.scene;
      buffer.sceneSequence = sequence;
    } else {
      if (buffer.player !== undefined) return;
      buffer.player = message.data.player;
      buffer.playerSequence = sequence;
    }

    if (!buffer.scene || !buffer.player || buffer.sceneSequence === undefined ||
      buffer.playerSequence === undefined || buffer.sceneSequence >= buffer.playerSequence) return;

    const recovery: SyncRecoveryData = {
      syncId: buffer.syncId,
      afterSeq: buffer.afterSeq,
      scene: buffer.scene,
      player: buffer.player,
    };
    this.#lastSequence = Math.max(this.#lastSequence ?? 0, buffer.playerSequence);
    this.#clearSyncWait();
    this.#callbacks.onSync?.(recovery);
  }

  #startWatchdog(socket: WebSocket): void {
    this.#stopWatchdog();
    this.#watchdogTimer = setInterval(() => {
      if (
        this.#socket === socket &&
        socket.readyState === WebSocket.OPEN &&
        Date.now() - this.#lastServerActivity >= SERVER_SILENCE_TIMEOUT_MS
      ) {
        socket.close(4000, 'Server heartbeat timeout');
      }
    }, WATCHDOG_INTERVAL_MS);
  }

  #stopWatchdog(): void {
    if (this.#watchdogTimer) {
      clearInterval(this.#watchdogTimer);
      this.#watchdogTimer = undefined;
    }
  }

  #clearSyncWait(): void {
    this.#syncPending = false;
    this.#syncAfterSeq = undefined;
    this.#syncBuffer = undefined;
    if (this.#syncTimer) {
      clearTimeout(this.#syncTimer);
      this.#syncTimer = undefined;
    }
  }

  #stopConnectionTimers(): void {
    this.#stopWatchdog();
    this.#clearSyncWait();
    if (this.#authExpiryTimer) {
      clearTimeout(this.#authExpiryTimer);
      this.#authExpiryTimer = undefined;
    }
  }

  #authExpiryMs(): number | undefined {
    const expiresAt = this.#auth.expiresAt;
    if (expiresAt === undefined || !Number.isFinite(expiresAt)) return undefined;
    return expiresAt > 10_000_000_000 ? expiresAt : expiresAt * 1000;
  }

  #authHasExpired(): boolean {
    const expiry = this.#authExpiryMs();
    return expiry !== undefined && Date.now() >= expiry;
  }

  #scheduleAuthExpiry(socket: WebSocket): void {
    if (this.#authExpiryTimer) clearTimeout(this.#authExpiryTimer);
    const expiry = this.#authExpiryMs();
    if (expiry === undefined) return;
    const remaining = expiry - Date.now();
    if (remaining <= 0) {
      this.#rejectAccess('Your session has expired. Sign in again.', socket, true);
      return;
    }
    this.#authExpiryTimer = setTimeout(() => {
      this.#authExpiryTimer = undefined;
      if (this.#socket !== socket) return;
      if (this.#authHasExpired()) {
        this.#rejectAccess('Your session has expired. Sign in again.', socket, true);
      } else {
        this.#scheduleAuthExpiry(socket);
      }
    }, Math.min(remaining, MAX_TIMER_DELAY_MS));
  }

  #rejectAccess(message: string, socket: WebSocket, clearCredentials: boolean): void {
    if (this.#accessRejected) return;
    this.#accessRejected = true;
    this.#closedByClient = true;
    this.#reconnectImmediately = false;
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#stopConnectionTimers();
    this.#callbacks.onAccessRejected?.(message, clearCredentials);
    if (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING) {
      socket.close(4003, 'Access rejected');
    }
  }
}

function sameCapabilities(
  left: BrowserCapabilities | undefined,
  right: BrowserCapabilities,
): boolean {
  return left?.images === right.images &&
    left.reducedMotion === right.reducedMotion &&
    left.audio === right.audio;
}
