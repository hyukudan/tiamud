export const MAX_SCROLLBACK_ENTRIES = 500;
export const SCROLL_BOTTOM_TOLERANCE_PX = 32;

export function boundedScrollback<T>(entries: readonly T[]): T[] {
  return entries.slice(-MAX_SCROLLBACK_ENTRIES);
}

export function isScrollAnchored(
  position: Pick<HTMLElement, 'scrollHeight' | 'clientHeight' | 'scrollTop'>,
  tolerance = SCROLL_BOTTOM_TOLERANCE_PX,
): boolean {
  return position.scrollHeight - position.clientHeight - position.scrollTop <= tolerance;
}
