interface CryptoLike {
  randomUUID?: () => string;
  getRandomValues?: <T extends ArrayBufferView>(values: T) => T;
}

let fallbackCounter = 0;
const fallbackProcessNonce = Math.floor(Math.random() * Number.MAX_SAFE_INTEGER).toString(36);

function bytesAsUuid(bytes: Uint8Array): string {
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40;
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80;
  const hex = [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/**
 * Produce a bounded, protocol-safe idempotency key on old and restricted
 * browser contexts as well as browsers with `crypto.randomUUID()`.
 */
export function createRequestId(
  cryptoLike: CryptoLike | undefined = globalThis.crypto as unknown as CryptoLike,
): string {
  if (typeof cryptoLike?.randomUUID === 'function') {
    try {
      return cryptoLike.randomUUID();
    } catch {
      // A restricted context can expose the method but reject the operation.
    }
  }

  if (typeof cryptoLike?.getRandomValues === 'function') {
    try {
      return bytesAsUuid(cryptoLike.getRandomValues(new Uint8Array(16)));
    } catch {
      // Continue with the process-local monotonic fallback.
    }
  }

  fallbackCounter = (fallbackCounter + 1) % Number.MAX_SAFE_INTEGER;
  return `req-${Date.now().toString(36)}-${fallbackProcessNonce}-${fallbackCounter.toString(36)}`;
}
