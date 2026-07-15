import {
  createClientHello,
  createCommandSubmit,
  parseServerMessage,
  type ConnectionState,
  type ServerMessage,
} from './protocol';

export interface ConnectionCallbacks {
  onMessage(message: ServerMessage): void;
  onStateChange(state: ConnectionState): void;
  onProtocolError?(): void;
}

type LocationLike = Pick<Location, 'protocol' | 'host'>;

export function websocketUrl(location: LocationLike): string {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${location.host}/ws`;
}

export class MudConnection {
  readonly #callbacks: ConnectionCallbacks;
  readonly #url: string;
  #socket?: WebSocket;
  #reconnectTimer?: ReturnType<typeof setTimeout>;
  #reconnectAttempts = 0;
  #closedByClient = false;

  constructor(url: string, callbacks: ConnectionCallbacks) {
    this.#url = url;
    this.#callbacks = callbacks;
  }

  connect(): void {
    if (this.#socket?.readyState === WebSocket.OPEN || this.#socket?.readyState === WebSocket.CONNECTING) {
      return;
    }

    this.#closedByClient = false;
    this.#callbacks.onStateChange('connecting');

    const socket = new WebSocket(this.#url, ['mud.v1']);
    this.#socket = socket;

    socket.addEventListener('open', () => {
      this.#reconnectAttempts = 0;
      this.#callbacks.onStateChange('connected');
      const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      socket.send(JSON.stringify(createClientHello(reducedMotion)));
    });

    socket.addEventListener('message', (event: MessageEvent<unknown>) => {
      if (typeof event.data !== 'string') {
        this.#callbacks.onProtocolError?.();
        return;
      }

      const message = parseServerMessage(event.data);
      if (message) {
        this.#callbacks.onMessage(message);
      } else {
        this.#callbacks.onProtocolError?.();
      }
    });

    socket.addEventListener('close', () => {
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

  close(): void {
    this.#closedByClient = true;
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#socket?.close(1000, 'Client closed');
    this.#socket = undefined;
  }

  #scheduleReconnect(): void {
    this.#reconnectAttempts += 1;
    const delay = Math.min(10_000, 500 * 2 ** (this.#reconnectAttempts - 1));
    this.#reconnectTimer = setTimeout(() => this.connect(), delay);
  }
}
