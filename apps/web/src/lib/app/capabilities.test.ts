import { describe, expect, it } from 'vitest';
import { capabilitiesFor } from './capabilities';
import { DEFAULT_PREFERENCES } from '../preferences';

describe('capabilitiesFor', () => {
  it('enables images when multimedia is allowed and disables when images are off', () => {
    expect(capabilitiesFor(DEFAULT_PREFERENCES, false).images).toBe(true);
    expect(capabilitiesFor({ ...DEFAULT_PREFERENCES, images: false }, false).images).toBe(false);
  });

  it('disables images when save-data or screen reader suppress multimedia', () => {
    expect(capabilitiesFor({ ...DEFAULT_PREFERENCES, saveData: true }, false).images).toBe(false);
    expect(capabilitiesFor({ ...DEFAULT_PREFERENCES, screenReader: true }, false).images).toBe(false);
  });

  it('reports reduced motion when the preference requests it', () => {
    expect(
      capabilitiesFor({ ...DEFAULT_PREFERENCES, reduceMotion: true }, false).reducedMotion,
    ).toBe(true);
  });

  it('reports reduced motion when the system prefers it even if the preference does not', () => {
    expect(capabilitiesFor(DEFAULT_PREFERENCES, true).reducedMotion).toBe(true);
    expect(capabilitiesFor(DEFAULT_PREFERENCES, false).reducedMotion).toBe(false);
  });

  it('reports reduced motion when the screen reader is active', () => {
    expect(
      capabilitiesFor({ ...DEFAULT_PREFERENCES, screenReader: true }, false).reducedMotion,
    ).toBe(true);
  });

  it('mirrors the audio preference flag', () => {
    expect(capabilitiesFor(DEFAULT_PREFERENCES, false).audio).toBe(false);
    expect(capabilitiesFor({ ...DEFAULT_PREFERENCES, audio: true }, false).audio).toBe(true);
  });
});
