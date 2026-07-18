export function clampItemQuantity(raw: number, maximum: number): number {
  return Math.max(1, Math.min(256, maximum, Math.trunc(raw) || 1));
}

export function clampTradeQuantity(raw: number, maximum: number): number {
  return Math.max(1, Math.min(128, maximum, Math.trunc(raw) || 1));
}
