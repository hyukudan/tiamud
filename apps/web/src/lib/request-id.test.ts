import { describe, expect, it, vi } from 'vitest';
import { createRequestId } from './request-id';

describe('request IDs', () => {
  it('uses randomUUID when the browser provides it', () => {
    const randomUUID = vi.fn(() => 'c9dd1806-8f8b-4ab0-908d-0a6a90b11942');
    expect(createRequestId({ randomUUID })).toBe('c9dd1806-8f8b-4ab0-908d-0a6a90b11942');
    expect(randomUUID).toHaveBeenCalledOnce();
  });

  it('uses random bytes when randomUUID is unavailable', () => {
    const getRandomValues = <T extends ArrayBufferView>(values: T): T => {
      new Uint8Array(values.buffer, values.byteOffset, values.byteLength)
        .forEach((_, index, bytes) => { bytes[index] = index; });
      return values;
    };
    expect(createRequestId({ getRandomValues }))
      .toBe('00010203-0405-4607-8809-0a0b0c0d0e0f');
  });

  it('remains unique and protocol-safe without either crypto API', () => {
    const first = createRequestId({});
    const second = createRequestId({});
    expect(first).not.toBe(second);
    expect(first).toMatch(/^[a-zA-Z0-9-]+$/);
    expect(first.length).toBeLessThanOrEqual(128);
  });
});
