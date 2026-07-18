import { isRecord } from './guards';

export interface Account {
  id: number;
  name: string;
}

export interface Character {
  id: number;
  accountId?: number;
  name: string;
  roomId?: string;
}

export interface CharacterOption {
  id: string;
  label: string;
  description: string;
}

export interface CharacterCreationCatalog {
  defaultLineageId: string;
  defaultBackgroundId?: string;
  defaultBaseClassId: string;
  lineages: CharacterOption[];
  backgrounds: CharacterOption[];
  baseClasses: CharacterOption[];
}

export interface CharacterCreationSelection {
  lineageId?: string;
  /** Omitted means world default; null explicitly selects no background. */
  backgroundId?: string | null;
  baseClassId?: string;
}

export interface AuthenticatedSession {
  account: Account;
  token: string;
  expiresAt: number;
}

export interface CharacterSelection extends AuthenticatedSession {
  characterId: number;
  characterName: string;
}

export interface StoredAccess extends AuthenticatedSession {
  characterId?: number;
}

export const ACCESS_STORAGE_KEY = 'tiamud.access.v1';

type FetchLike = typeof fetch;
type StorageLike = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function parseAccount(value: unknown): Account {
  if (!isRecord(value) || typeof value.id !== 'number' || typeof value.name !== 'string') {
    throw new Error('The server returned an invalid account.');
  }
  return { id: value.id, name: value.name };
}

function parseSession(value: unknown): AuthenticatedSession {
  if (
    !isRecord(value) ||
    typeof value.token !== 'string' ||
    typeof value.expiresAt !== 'number'
  ) {
    throw new Error('The server returned an invalid session.');
  }
  return { account: parseAccount(value.account), token: value.token, expiresAt: value.expiresAt };
}

function parseCharacter(value: unknown): Character {
  if (!isRecord(value) || typeof value.id !== 'number' || typeof value.name !== 'string') {
    throw new Error('The server returned an invalid character.');
  }
  const accountId = value.accountId ?? value.account_id;
  const roomId = value.roomId ?? value.room_id;
  return {
    id: value.id,
    accountId: typeof accountId === 'number' ? accountId : undefined,
    name: value.name,
    roomId: typeof roomId === 'string' ? roomId : undefined,
  };
}

const boundedText = (value: unknown, maximum: number): string | undefined =>
  typeof value === 'string' && value.length > 0 && value.length <= maximum ? value : undefined;

const boundedContentId = (value: unknown): string | undefined => {
  const id = boundedText(value, 128);
  return id && /^[a-z0-9](?:[a-z0-9_.:-]{0,126}[a-z0-9])?$/.test(id) ? id : undefined;
};

function parseCharacterOption(value: unknown): CharacterOption {
  if (!isRecord(value)) throw new Error('The server returned invalid character options.');
  const id = boundedContentId(value.id);
  const label = boundedText(value.label, 128);
  const description = typeof value.description === 'string' && value.description.length <= 2048
    ? value.description
    : undefined;
  if (!id || !label || description === undefined) {
    throw new Error('The server returned invalid character options.');
  }
  return { id, label, description };
}

function parseOptionList(value: unknown, maximum: number): CharacterOption[] {
  if (!Array.isArray(value) || value.length > maximum) {
    throw new Error('The server returned invalid character options.');
  }
  const options = value.map(parseCharacterOption);
  if (new Set(options.map((option) => option.id)).size !== options.length) {
    throw new Error('The server returned invalid character options.');
  }
  return options;
}

async function requestJson(
  path: string,
  init: RequestInit,
  fetcher: FetchLike,
): Promise<unknown> {
  const response = await fetcher(path, init);
  if (!response.ok) {
    let message = `The request failed (${response.status}).`;
    try {
      const body: unknown = await response.json();
      if (isRecord(body) && typeof body.error === 'string') {
        message = body.error;
      }
    } catch {
      // The status is still useful if an intermediary returned non-JSON.
    }
    throw new Error(message);
  }
  if (response.status === 204) {
    return undefined;
  }
  return response.json();
}

export async function authenticate(
  mode: 'login' | 'register',
  name: string,
  password: string,
  fetcher: FetchLike = fetch,
): Promise<AuthenticatedSession> {
  const body = await requestJson(
    `/api/auth/${mode}`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name, password }),
    },
    fetcher,
  );
  return parseSession(body);
}

export async function loadCharacters(
  token: string,
  fetcher: FetchLike = fetch,
): Promise<Character[]> {
  const body = await requestJson(
    '/api/characters',
    { headers: { authorization: `Bearer ${token}` } },
    fetcher,
  );
  if (!isRecord(body) || !Array.isArray(body.characters)) {
    throw new Error('The server returned an invalid character roster.');
  }
  return body.characters.map(parseCharacter);
}

export async function loadCharacterOptions(
  fetcher: FetchLike = fetch,
): Promise<CharacterCreationCatalog> {
  const body = await requestJson('/api/character-options', {}, fetcher);
  if (!isRecord(body)) throw new Error('The server returned invalid character options.');

  const defaultLineageId = boundedContentId(body.defaultLineageId);
  const defaultBackgroundId = body.defaultBackgroundId === null || body.defaultBackgroundId === undefined
    ? undefined
    : boundedContentId(body.defaultBackgroundId);
  const defaultBaseClassId = boundedContentId(body.defaultBaseClassId);
  const lineages = parseOptionList(body.lineages, 16);
  const backgrounds = parseOptionList(body.backgrounds, 16);
  const baseClasses = parseOptionList(body.baseClasses, 32);
  if (
    !defaultLineageId || !lineages.some((option) => option.id === defaultLineageId) ||
    !defaultBaseClassId || !baseClasses.some((option) => option.id === defaultBaseClassId) ||
    (body.defaultBackgroundId !== null && body.defaultBackgroundId !== undefined &&
      (!defaultBackgroundId || !backgrounds.some((option) => option.id === defaultBackgroundId)))
  ) {
    throw new Error('The server returned invalid character options.');
  }
  return {
    defaultLineageId,
    ...(defaultBackgroundId ? { defaultBackgroundId } : {}),
    defaultBaseClassId,
    lineages,
    backgrounds,
    baseClasses,
  };
}

export async function createCharacter(
  token: string,
  name: string,
  selection: CharacterCreationSelection = {},
  fetcher: FetchLike = fetch,
): Promise<Character> {
  const body = await requestJson(
    '/api/characters',
    {
      method: 'POST',
      headers: {
        authorization: `Bearer ${token}`,
        'content-type': 'application/json',
      },
      body: JSON.stringify({ name, ...selection }),
    },
    fetcher,
  );
  if (!isRecord(body)) {
    throw new Error('The server returned an invalid character.');
  }
  return parseCharacter(body.character);
}

export async function logout(token: string, fetcher: FetchLike = fetch): Promise<void> {
  await requestJson(
    '/api/auth/logout',
    { method: 'POST', headers: { authorization: `Bearer ${token}` } },
    fetcher,
  );
}

export function sessionHasExpired(session: Pick<AuthenticatedSession, 'expiresAt'>): boolean {
  const expiryMs = session.expiresAt > 10_000_000_000 ? session.expiresAt : session.expiresAt * 1000;
  return !Number.isFinite(expiryMs) || Date.now() >= expiryMs;
}

// SECURITY NOTE: `readStoredAccess`/`writeStoredAccess` persist the access
// token via the injected `StorageLike` (localStorage in the browser). Any
// script running in the page can read localStorage, so the token is exposed to
// XSS token theft. The mitigation is to move the token into an HttpOnly, Secure,
// SameSite cookie, which requires a backend/protocol change and is intentionally
// deferred to a future ADR. This comment is a deliberate marker of that accepted
// trade-off, not a bug to silently fix in this pass.
export function readStoredAccess(storage: StorageLike): StoredAccess | undefined {
  let candidate: unknown;
  try {
    const raw = storage.getItem(ACCESS_STORAGE_KEY);
    if (!raw) return undefined;
    candidate = JSON.parse(raw);
    const session = parseSession(candidate);
    if (sessionHasExpired(session)) {
      storage.removeItem(ACCESS_STORAGE_KEY);
      return undefined;
    }
    const characterId = isRecord(candidate) ? candidate.characterId : undefined;
    return {
      ...session,
      characterId: typeof characterId === 'number' ? characterId : undefined,
    };
  } catch {
    storage.removeItem(ACCESS_STORAGE_KEY);
    return undefined;
  }
}

export function writeStoredAccess(storage: StorageLike, access: StoredAccess): void {
  storage.setItem(ACCESS_STORAGE_KEY, JSON.stringify(access));
}

export function clearStoredAccess(storage: StorageLike): void {
  storage.removeItem(ACCESS_STORAGE_KEY);
}
