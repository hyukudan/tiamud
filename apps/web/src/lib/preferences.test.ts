import { describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_PREFERENCES,
  loadPreferences,
  motionReduced,
  multimediaAllowed,
  savePreferences,
} from './preferences';

describe('client preferences', () => {
  it('uses safe defaults and honors Save-Data before media is requested', () => {
    expect(loadPreferences(undefined)).toEqual(DEFAULT_PREFERENCES);
    const saveData = loadPreferences(undefined, true);
    expect(saveData.saveData).toBe(true);
    expect(multimediaAllowed(saveData)).toBe(false);
  });

  it('rejects malformed stored values and bounds volume controls', () => {
    const storage = {
      getItem: () => JSON.stringify({ images: false, audio: true, fontScale: 999, effectsVolume: 101 }),
    };
    const preferences = loadPreferences(storage);
    expect(preferences.images).toBe(false);
    expect(preferences.audio).toBe(true);
    expect(preferences.fontScale).toBe(100);
    expect(preferences.effectsVolume).toBe(DEFAULT_PREFERENCES.effectsVolume);
  });

  it('persists settings without making unavailable storage fatal', () => {
    const setItem = vi.fn();
    savePreferences({ setItem }, DEFAULT_PREFERENCES);
    expect(setItem).toHaveBeenCalledWith(
      'tiamud.preferences.v1',
      JSON.stringify(DEFAULT_PREFERENCES),
    );
    expect(() => savePreferences({ setItem: () => { throw new Error('blocked'); } }, DEFAULT_PREFERENCES)).not.toThrow();
  });

  it('reduces motion for either manual, reader, or system preference', () => {
    expect(motionReduced(DEFAULT_PREFERENCES, true)).toBe(true);
    expect(motionReduced({ ...DEFAULT_PREFERENCES, reduceMotion: true }, false)).toBe(true);
    expect(motionReduced({ ...DEFAULT_PREFERENCES, screenReader: true }, false)).toBe(true);
  });
});
