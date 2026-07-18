import { describe, expect, it, vi } from 'vitest';
import {
  applyPreferenceChange,
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
    expect(multimediaAllowed({ ...DEFAULT_PREFERENCES, screenReader: true })).toBe(false);
  });
});

describe('applyPreferenceChange', () => {
  it('changes only the targeted boolean key and leaves the rest equal', () => {
    const result = applyPreferenceChange(DEFAULT_PREFERENCES, 'images', false);
    expect(result.images).toBe(false);
    expect(result.audio).toBe(DEFAULT_PREFERENCES.audio);
    expect(result.fontScale).toBe(DEFAULT_PREFERENCES.fontScale);
    expect(result.effectsVolume).toBe(DEFAULT_PREFERENCES.effectsVolume);
  });

  it('does not mutate the input object', () => {
    const value = { ...DEFAULT_PREFERENCES };
    const result = applyPreferenceChange(value, 'images', false);
    expect(value.images).toBe(true);
    expect(result).not.toBe(value);
  });

  it('changes fontScale to 125', () => {
    const result = applyPreferenceChange(DEFAULT_PREFERENCES, 'fontScale', 125);
    expect(result.fontScale).toBe(125);
    expect(result.images).toBe(DEFAULT_PREFERENCES.images);
  });

  it('changes a numeric volume', () => {
    const result = applyPreferenceChange(DEFAULT_PREFERENCES, 'musicVolume', 30);
    expect(result.musicVolume).toBe(30);
    expect(result.effectsVolume).toBe(DEFAULT_PREFERENCES.effectsVolume);
  });
});
