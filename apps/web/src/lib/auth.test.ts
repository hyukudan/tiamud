import { describe, expect, it, vi } from 'vitest';
import {
  ACCESS_STORAGE_KEY,
  authenticate,
  createCharacter,
  loadArchetypes,
  loadCharacters,
  readStoredAccess,
  writeStoredAccess,
} from './auth';

const jsonResponse = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });

describe('authentication API', () => {
  it('registers with JSON and parses the bearer session', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      jsonResponse({
        account: { id: 4, name: 'Aster' },
        token: 'a'.repeat(64),
        expiresAt: 2_000_000_000,
      }, 201),
    );

    await expect(authenticate('register', 'Aster', 'long password', fetcher)).resolves.toMatchObject({
      account: { id: 4, name: 'Aster' },
      token: 'a'.repeat(64),
    });
    expect(fetcher).toHaveBeenCalledWith('/api/auth/register', expect.objectContaining({
      method: 'POST',
      body: JSON.stringify({ name: 'Aster', password: 'long password' }),
    }));
  });

  it('uses the bearer token for roster and character creation', async () => {
    const fetcher = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse({ characters: [{
        id: 8, account_id: 4, name: 'Lyra', room_id: 'square',
        archetype: { id: 'warden', label: 'Warden' },
      }] }))
      .mockResolvedValueOnce(jsonResponse({
        archetypes: [{ id: 'warden', label: 'Warden', description: 'A resilient frontier guardian.' }],
      }))
      .mockResolvedValueOnce(jsonResponse({ character: {
        id: 9, account_id: 4, name: 'Orin', room_id: 'square',
        archetype: { id: 'warden', label: 'Warden' },
      } }, 201));

    await expect(loadCharacters('token', fetcher)).resolves.toEqual([
      {
        id: 8, accountId: 4, name: 'Lyra', roomId: 'square',
        archetype: { id: 'warden', label: 'Warden' },
      },
    ]);
    await expect(loadArchetypes(fetcher)).resolves.toEqual([
      { id: 'warden', label: 'Warden', description: 'A resilient frontier guardian.' },
    ]);
    await expect(createCharacter('token', 'Orin', 'warden', fetcher)).resolves.toMatchObject({
      id: 9, name: 'Orin', archetype: { id: 'warden' },
    });
    expect(fetcher.mock.calls[0]?.[1]?.headers).toEqual({ authorization: 'Bearer token' });
    expect(fetcher.mock.calls[2]?.[1]?.headers).toMatchObject({ authorization: 'Bearer token' });
    expect(fetcher.mock.calls[2]?.[1]?.body).toBe(JSON.stringify({ name: 'Orin', archetypeId: 'warden' }));
  });

  it('falls back cleanly when an older server has no archetype catalog', async () => {
    const catalogFetcher = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse({}, 404));
    await expect(loadArchetypes(catalogFetcher)).resolves.toEqual([]);

    const createFetcher = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse({
      character: { id: 9, name: 'Orin' },
    }, 201));
    await expect(createCharacter('token', 'Orin', createFetcher)).resolves.toMatchObject({ id: 9, name: 'Orin' });
    expect(createFetcher.mock.calls[0]?.[1]?.body).toBe(JSON.stringify({ name: 'Orin' }));
  });

  it('accepts the authored archetype id and description boundaries', async () => {
    const id = 'a'.repeat(128);
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse({
      archetypes: [{ id, label: 'Boundary', description: '' }],
    }));
    await expect(loadArchetypes(fetcher)).resolves.toEqual([
      { id, label: 'Boundary', description: '' },
    ]);
    expect(fetcher).toHaveBeenCalledWith('/api/archetypes', {});
  });

  it('surfaces the public API error message', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse({ error: 'Invalid credentials.' }, 401));
    await expect(authenticate('login', 'Aster', 'wrong password', fetcher)).rejects.toThrow('Invalid credentials.');
  });
});

describe('stored access', () => {
  it('round-trips a valid session and selected character', () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    };
    const access = {
      account: { id: 4, name: 'Aster' },
      token: 'a'.repeat(64),
      expiresAt: Math.floor(Date.now() / 1000) + 3600,
      characterId: 8,
    };

    writeStoredAccess(storage, access);
    expect(readStoredAccess(storage)).toEqual(access);
    expect(values.has(ACCESS_STORAGE_KEY)).toBe(true);
  });

  it('removes expired or malformed credentials', () => {
    const values = new Map([[ACCESS_STORAGE_KEY, JSON.stringify({
      account: { id: 4, name: 'Aster' }, token: 'token', expiresAt: 1,
    })]]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    };

    expect(readStoredAccess(storage)).toBeUndefined();
    expect(values.has(ACCESS_STORAGE_KEY)).toBe(false);
  });
});
