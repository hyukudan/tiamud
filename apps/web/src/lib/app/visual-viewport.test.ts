import { describe, expect, it } from 'vitest';
import {
  KEYBOARD_INSET_PROPERTY,
  keepCommandVisible,
  keyboardInset,
  type VisualViewportLike,
} from './visual-viewport';

class FakeViewport implements VisualViewportLike {
  height = 740;
  offsetTop = 0;
  readonly listeners = new Map<string, Set<() => void>>();
  addEventListener(type: string, listener: () => void): void {
    const set = this.listeners.get(type) ?? new Set();
    set.add(listener);
    this.listeners.set(type, set);
  }
  removeEventListener(type: string, listener: () => void): void {
    this.listeners.get(type)?.delete(listener);
  }
  emit(type: string): void {
    for (const listener of this.listeners.get(type) ?? []) listener();
  }
}

function element() {
  const properties = new Map<string, string>();
  return {
    properties,
    style: {
      setProperty: (name: string, value: string | null) => void properties.set(name, value ?? ''),
      removeProperty: (name: string) => {
        const previous = properties.get(name) ?? '';
        properties.delete(name);
        return previous;
      },
    },
  };
}

describe('visual viewport keyboard inset', () => {
  it('measures only the covered height and bounds noise and hostile values', () => {
    expect(keyboardInset(740, { height: 740, offsetTop: 0 })).toBe(0);
    expect(keyboardInset(740, { height: 440, offsetTop: 0 })).toBe(300);
    expect(keyboardInset(740, { height: 440, offsetTop: 100 })).toBe(200);
    expect(keyboardInset(740, { height: 739.6, offsetTop: 0 })).toBe(0);
    expect(keyboardInset(740, { height: Number.NaN, offsetTop: 0 })).toBe(0);
    expect(keyboardInset(740, { height: -5_000, offsetTop: 0 })).toBe(740);
    expect(keyboardInset(740, { height: 900, offsetTop: 0 })).toBe(0);
  });

  it('lifts the command bar while the keyboard is open and cleans up', () => {
    const viewport = new FakeViewport();
    const form = element();
    const view = { innerHeight: 740, visualViewport: viewport };
    const stop = keepCommandVisible(view, form);
    expect(form.properties.has(KEYBOARD_INSET_PROPERTY)).toBe(false);

    viewport.height = 420;
    viewport.emit('resize');
    expect(form.properties.get(KEYBOARD_INSET_PROPERTY)).toBe('320px');

    viewport.offsetTop = 20;
    viewport.emit('scroll');
    expect(form.properties.get(KEYBOARD_INSET_PROPERTY)).toBe('300px');

    viewport.height = 740;
    viewport.offsetTop = 0;
    viewport.emit('resize');
    expect(form.properties.has(KEYBOARD_INSET_PROPERTY)).toBe(false);

    viewport.height = 400;
    viewport.emit('resize');
    stop();
    expect(form.properties.has(KEYBOARD_INSET_PROPERTY)).toBe(false);
    expect(viewport.listeners.get('resize')?.size).toBe(0);
    expect(viewport.listeners.get('scroll')?.size).toBe(0);
  });

  it('is a no-op without visualViewport support', () => {
    const form = element();
    const stop = keepCommandVisible({ innerHeight: 740, visualViewport: null }, form);
    stop();
    expect(form.properties.size).toBe(0);
  });
});
