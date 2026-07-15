import { describe, expect, it, vi } from 'vitest';
import { loadCommandHistory, MAX_STORED_COMMANDS, rememberCommand } from './history';

describe('command history', () => {
  it('loads only bounded one-line commands', () => {
    const values = [...Array.from({ length: 120 }, (_, index) => `look ${index}`), 'bad\ncommand'];
    const loaded = loadCommandHistory({ getItem: () => JSON.stringify(values) });
    expect(loaded).toHaveLength(MAX_STORED_COMMANDS);
    expect(loaded).not.toContain('bad\ncommand');
  });

  it('deduplicates recent commands and persists safely', () => {
    const setItem = vi.fn();
    const next = rememberCommand(['look', 'north'], ' look ', { setItem });
    expect(next).toEqual(['north', 'look']);
    expect(setItem).toHaveBeenCalledWith('tiamud.command-history.v1', JSON.stringify(next));
    expect(() => rememberCommand(next, 'south', { setItem: () => { throw new Error('blocked'); } })).not.toThrow();
  });
});
