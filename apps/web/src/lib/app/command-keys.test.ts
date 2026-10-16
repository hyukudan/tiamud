import { describe, expect, it } from 'vitest';
import { isSubmitEnter, type CommandKeyEvent } from './command-keys';

const key = (overrides: Partial<CommandKeyEvent> = {}): CommandKeyEvent => ({
  key: 'Enter',
  keyCode: 13,
  isComposing: false,
  shiftKey: false,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  defaultPrevented: false,
  ...overrides,
});

describe('isSubmitEnter', () => {
  it('accepts a plain Enter', () => {
    expect(isSubmitEnter(key())).toBe(true);
  });

  it('ignores other keys and already handled events', () => {
    expect(isSubmitEnter(key({ key: 'a', keyCode: 65 }))).toBe(false);
    expect(isSubmitEnter(key({ key: 'ArrowUp', keyCode: 38 }))).toBe(false);
    expect(isSubmitEnter(key({ defaultPrevented: true }))).toBe(false);
  });

  it('never submits while an input method composition is being confirmed', () => {
    expect(isSubmitEnter(key({ isComposing: true }))).toBe(false);
    expect(isSubmitEnter(key({ keyCode: 229 }))).toBe(false);
  });

  it('leaves modified Enter to the browser', () => {
    for (const modifier of ['shiftKey', 'altKey', 'ctrlKey', 'metaKey'] as const) {
      expect(isSubmitEnter(key({ [modifier]: true }))).toBe(false);
    }
  });
});
