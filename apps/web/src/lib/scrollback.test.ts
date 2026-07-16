import { describe, expect, it } from 'vitest';
import {
  MAX_SCROLLBACK_ENTRIES,
  boundedScrollback,
  isScrollAnchored,
} from './scrollback';

describe('console scrollback', () => {
  it('retains the newest 500 entries', () => {
    const values = Array.from({ length: 650 }, (_, index) => index);
    const retained = boundedScrollback(values);
    expect(retained).toHaveLength(MAX_SCROLLBACK_ENTRIES);
    expect(retained[0]).toBe(150);
    expect(retained.at(-1)).toBe(649);
  });

  it('only considers a reader anchored when they remain near the bottom', () => {
    expect(isScrollAnchored({ scrollHeight: 1000, clientHeight: 300, scrollTop: 680 })).toBe(true);
    expect(isScrollAnchored({ scrollHeight: 1000, clientHeight: 300, scrollTop: 500 })).toBe(false);
  });
});
