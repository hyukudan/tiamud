import {
  createActionPerform,
  createClientHello,
  createCommandSubmit,
  createHeartbeatAck,
  createSyncRequest,
  parseServerMessage,
  type ConnectionState,
  type ServerMessage,
} from './protocol';
import { WEBSOCKET_SUBPROTOCOL } from './brand';

export interface ConnectionCallbacks {
  onMessage(message: ServerMessage): void;
  onStateChange(state: ConnectionState): void;
  onProtocolError?(): void;
}

export interface ConnectionAuth {
  token: string;
  characterId: number;
}

export interface BrowserCapabilities {
  images: boolean;
  reducedMotion: boolean;
  audio: boolean;
}

type LocationLike = Pick<Location, 'protocol' | 'host'>;

export const SERVER_SILENCE_TIMEOUT_MS = 45_000;
const WATCHDOG_INTERVAL_MS = 5_000;

export function websocketUrl(location: LocationLike): string {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${location.host}/ws`;
}

export class TiamudConnection {
  readonly #callbacks: ConnectionCallbacks;
  readonly #url: string;
  readonly #auth: ConnectionAuth;
  readonly #capabilities?: BrowserCapabilities;
  #socket?: WebSocket;
  #reconnectTimer?: ReturnType<typeof setTimeout>;
  #reconnectAttempts = 0;
  #closedByClient = false;
  #lastSequence?: number;
  #syncPending = false;
  #lastServerActivity = 0;
  #watchdogTimer?: ReturnType<typeof setInterval>;

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
    if (this.#socket?.readyState === WebSocket.OPEN || this.#socket?.readyState === WebSocket.CONNECTING) {
      return;
    }

    this.#closedByClient = false;
    this.#callbacks.onStateChange('connecting');

    const socket = new WebSocket(this.#url, [WEBSOCKET_SUBPROTOCOL]);
    this.#socket = socket;

    socket.addEventListener('open', () => {
      this.#reconnectAttempts = 0;
      this.#lastSequence = undefined;
      this.#syncPending = false;
      this.#lastServerActivity = Date.now();
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
        this.#handleSequenced(message, socket);
      } else {
        this.#callbacks.onProtocolError?.();
      }
    });

    socket.addEventListener('close', () => {
      this.#stopWatchdog();
      if (this.#socket === socket) {
        this.#socket = undefined;
      }
      this.#callbacks.onStateChange('disconnected');
      if (!this.#closedByClient) {
        this.#scheduleReconnect();
      }
    });
  }

  submit(command: string, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) {
      return false;
    }

    this.#socket.send(JSON.stringify(createCommandSubmit(command, requestId)));
    return true;
  }

  perform(action: string, target: string | undefined, requestId: string): boolean {
    if (this.#socket?.readyState !== WebSocket.OPEN) {
      return false;
    }

    this.#socket.send(JSON.stringify(createActionPerform(action, target, requestId)));
    return true;
  }

  close(): void {
    this.#closedByClient = true;
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#stopWatchdog();
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

    if (message.type === 'sync.snapshot') {
      if (this.#lastSequence !== undefined && sequence <= this.#lastSequence) {
        return;
      }
      this.#lastSequence = sequence;
      this.#syncPending = false;
      this.#callbacks.onMessage(message);
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
    socket.send(JSON.stringify(createSyncRequest(afterSeq)));
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
}
