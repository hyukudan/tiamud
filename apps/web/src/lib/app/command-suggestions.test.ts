import { describe, expect, it } from 'vitest';
import { computeCommandSuggestions } from './command-suggestions';

const BASE = [
  'look',
  'inventory',
  'loot',
  'north',
  'east',
  'south',
  'west',
  'up',
  'down',
  'say ',
  'tell ',
  'who',
  'help',
  'ignore ',
  'unignore ',
  'block ',
  'unblock ',
  'report ',
];

describe('computeCommandSuggestions', () => {
  it('returns the base command set when no exits or history are supplied', () => {
    expect(computeCommandSuggestions([], [])).toEqual(BASE);
  });

  it('appends exit commands after the base set', () => {
    expect(computeCommandSuggestions(['enter gate'], [])).toEqual([...BASE, 'enter gate']);
  });

  it('appends remembered commands in reverse (most-recent-first) order', () => {
    expect(computeCommandSuggestions([], ['first', 'second', 'third'])).toEqual([
      ...BASE,
      'third',
      'second',
      'first',
    ]);
  });

  it('deduplicates while preserving first occurrence, so base entries win over history', () => {
    expect(computeCommandSuggestions([], ['look', 'brand-new'])).toEqual([...BASE, 'brand-new']);
  });

  it('caps the result at 24 entries', () => {
    const remembered = Array.from({ length: 40 }, (_, index) => `cmd-${index}`);
    const result = computeCommandSuggestions([], remembered);
    expect(result).toHaveLength(24);
    expect(result.slice(0, BASE.length)).toEqual(BASE);
    // Most-recent history entry (last element) leads the post-base slots.
    expect(result[BASE.length]).toBe('cmd-39');
  });

  it('does not mutate the remembered commands input', () => {
    const remembered = ['a', 'b'];
    computeCommandSuggestions([], remembered);
    expect(remembered).toEqual(['a', 'b']);
  });
});
