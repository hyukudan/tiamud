import { afterEach, describe, expect, it, vi } from 'vitest';
import { browserStorage } from './storage';
import { clearStoredAccess, readStoredAccess, writeStoredAccess } from './auth';

afterEach(() => vi.unstubAllGlobals());

describe('unavailable browser persistence', () => {
  it('survives a throwing localStorage getter and continues without storage', () => {
    vi.stubGlobal('window', Object.defineProperty({}, 'localStorage', {
      get() { throw new Error('SecurityError'); },
    }));
    expect(browserStorage()).toBeUndefined();
    expect(readStoredAccess(browserStorage())).toBeUndefined();
  });

  it('does not interrupt live access or sign-out when storage operations throw', () => {
    const fail = () => { throw new Error('QuotaExceededError'); };
    const storage = { getItem: fail, setItem: fail, removeItem: fail };
    expect(readStoredAccess(storage)).toBeUndefined();
    expect(() => writeStoredAccess(storage, {
      account: { id: 1, name: 'Aria' }, token: 'test', expiresAt: Date.now() + 60_000,
    })).not.toThrow();
    expect(() => clearStoredAccess(storage)).not.toThrow();
  });
});
