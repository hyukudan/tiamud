export const PROTOCOL_VERSION = 1 as const;

export type ConnectionState = 'connecting' | 'connected' | 'disconnected';

export interface Envelope<TType extends string, TData> {
  v: number;
  type: TType;
  seq?: number;
  id?: string;
  data: TData;
}

export interface ServerWelcomeData {
  serverName?: string;
  motd?: string;
  sessionId?: string;
}

export interface SceneImage {
  alt: string;
  avif?: string;
  webp?: string;
  jpeg?: string;
  width?: number;
  height?: number;
}

export interface SceneExit {
  id: string;
  label: string;
  command: string;
}

export interface SceneSnapshot {
  viewId?: string;
  revision?: number;
  title: string;
  prose: string[];
  image?: SceneImage;
  exits: SceneExit[];
}

export interface ActionResultData {
  requestId?: string;
  ok: boolean;
  message?: string;
}

export interface ServerNoticeData {
  level?: 'info' | 'warning' | 'error';
  message: string;
}

export type ServerMessage =
  | Envelope<'server.welcome', ServerWelcomeData>
  | Envelope<'scene.snapshot', SceneSnapshot>
  | Envelope<'action.result', ActionResultData>
  | Envelope<'server.notice', ServerNoticeData>;

export type ClientHello = Envelope<
  'client.hello',
  {
    client: 'web';
    capabilities: {
      images: readonly ['avif', 'webp', 'jpeg'];
      reducedMotion: boolean;
    };
  }
>;

export type CommandSubmit = Envelope<
  'command.submit',
  {
    requestId: string;
    command: string;
  }
>;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const stringOrUndefined = (value: unknown): string | undefined =>
  typeof value === 'string' && value.length > 0 ? value : undefined;

const numberOrUndefined = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined;

const toProse = (value: unknown): string[] => {
  if (typeof value === 'string') {
    return value.trim() ? [value] : [];
  }

  if (!Array.isArray(value)) {
    return [];
  }

  return value.flatMap((entry) => {
    if (typeof entry === 'string') {
      return entry.trim() ? [entry] : [];
    }

    if (isRecord(entry) && typeof entry.text === 'string' && entry.text.trim()) {
      return [entry.text];
    }

    return [];
  });
};

const toImage = (data: Record<string, unknown>): SceneImage | undefined => {
  const direct = isRecord(data.image) ? data.image : undefined;
  const presentation = isRecord(data.presentation) ? data.presentation : undefined;
  const backdrop = presentation && isRecord(presentation.backdrop) ? presentation.backdrop : undefined;
  const visual = isRecord(data.visual) ? data.visual : undefined;
  const source = direct ?? backdrop ?? visual;

  if (!source) {
    return undefined;
  }

  const variants = Array.isArray(source.variants) ? source.variants : [];
  const variantUrl = (formats: string[]): string | undefined => {
    const variant = variants.find(
      (candidate) =>
        isRecord(candidate) &&
        typeof candidate.format === 'string' &&
        formats.includes(candidate.format.toLowerCase()),
    );
    return isRecord(variant) ? stringOrUndefined(variant.url) : undefined;
  };

  const image: SceneImage = {
    alt: stringOrUndefined(source.alt) ?? stringOrUndefined(data.alt) ?? '',
    avif: stringOrUndefined(source.avif) ?? variantUrl(['avif']),
    webp: stringOrUndefined(source.webp) ?? variantUrl(['webp']),
    jpeg:
      stringOrUndefined(source.jpeg) ??
      stringOrUndefined(source.jpg) ??
      variantUrl(['jpeg', 'jpg']),
    width: numberOrUndefined(source.width),
    height: numberOrUndefined(source.height),
  };

  return image.avif || image.webp || image.jpeg ? image : undefined;
};

const toExits = (value: unknown): SceneExit[] => {
  if (!Array.isArray(value)) {
    return [];
  }

  return value.flatMap((entry, index) => {
    if (typeof entry === 'string' && entry.trim()) {
      return [{ id: entry, label: entry, command: entry }];
    }

    if (!isRecord(entry)) {
      return [];
    }

    const label = stringOrUndefined(entry.label) ?? stringOrUndefined(entry.name);
    if (!label) {
      return [];
    }

    return [
      {
        id: stringOrUndefined(entry.id) ?? `exit-${index}`,
        label,
        command: stringOrUndefined(entry.command) ?? label,
      },
    ];
  });
};

const normaliseScene = (data: Record<string, unknown>): SceneSnapshot => ({
  viewId: stringOrUndefined(data.viewId),
  revision: numberOrUndefined(data.revision),
  title: stringOrUndefined(data.title) ?? 'An unknown place',
  prose: toProse(data.prose ?? data.description),
  image: toImage(data),
  exits: toExits(data.exits),
});

export function parseServerMessage(raw: string): ServerMessage | undefined {
  let candidate: unknown;

  try {
    candidate = JSON.parse(raw);
  } catch {
    return undefined;
  }

  if (!isRecord(candidate) || typeof candidate.type !== 'string' || !isRecord(candidate.data)) {
    return undefined;
  }

  const base = {
    v: numberOrUndefined(candidate.v) ?? PROTOCOL_VERSION,
    seq: numberOrUndefined(candidate.seq),
    id: stringOrUndefined(candidate.id),
  };

  switch (candidate.type) {
    case 'server.welcome':
      return {
        ...base,
        type: candidate.type,
        data: {
          serverName: stringOrUndefined(candidate.data.serverName),
          motd: stringOrUndefined(candidate.data.motd),
          sessionId: stringOrUndefined(candidate.data.sessionId),
        },
      };
    case 'scene.snapshot':
      return { ...base, type: candidate.type, data: normaliseScene(candidate.data) };
    case 'action.result':
      return {
        ...base,
        type: candidate.type,
        data: {
          requestId: stringOrUndefined(candidate.data.requestId),
          ok: candidate.data.ok !== false,
          message: stringOrUndefined(candidate.data.message),
        },
      };
    case 'server.notice': {
      const message = stringOrUndefined(candidate.data.message);
      if (!message) {
        return undefined;
      }

      const level = candidate.data.level;
      return {
        ...base,
        type: candidate.type,
        data: {
          level: level === 'warning' || level === 'error' ? level : 'info',
          message,
        },
      };
    }
    default:
      return undefined;
  }
}

export function createClientHello(reducedMotion: boolean): ClientHello {
  return {
    v: PROTOCOL_VERSION,
    type: 'client.hello',
    data: {
      client: 'web',
      capabilities: {
        images: ['avif', 'webp', 'jpeg'],
        reducedMotion,
      },
    },
  };
}

export function createCommandSubmit(command: string, requestId: string): CommandSubmit {
  return {
    v: PROTOCOL_VERSION,
    type: 'command.submit',
    data: { requestId, command },
  };
}
