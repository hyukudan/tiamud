import { describe, expect, it, vi } from 'vitest';
import { AudioController, safeAudioUrl, type AudioLike } from './audio';
import { DEFAULT_PREFERENCES } from './preferences';

const HASH = 'a'.repeat(64);

function media(): AudioLike {
  return {
    src: '',
    loop: false,
    volume: 1,
    preload: '',
    play: vi.fn().mockResolvedValue(undefined),
    pause: vi.fn(),
  };
}

describe('opt-in audio', () => {
  it('accepts only immutable same-origin audio assets', () => {
    expect(safeAudioUrl(`/assets/${HASH}/audio-rain.opus`)).toBe(true);
    expect(safeAudioUrl('https://tracker.test/audio.mp3')).toBe(false);
    expect(safeAudioUrl(`/assets/${HASH}/../audio-rain.ogg`)).toBe(false);
  });

  it('requires both preference and a gesture, then applies separate volumes', async () => {
    const created: AudioLike[] = [];
    const controller = new AudioController((url) => {
      const next = media();
      next.src = url;
      created.push(next);
      return next;
    }, DEFAULT_PREFERENCES);
    const url = `/assets/${HASH}/audio-rain.ogg`;
    expect(await controller.play('ambience', url)).toBe(false);
    controller.unlockFromGesture();
    controller.updatePreferences({ ...DEFAULT_PREFERENCES, audio: true, ambienceVolume: 35 });
    expect(await controller.play('ambience', url)).toBe(true);
    expect(created[0].volume).toBe(0.35);
    controller.updatePreferences({ ...DEFAULT_PREFERENCES, audio: false });
    expect(created[0].pause).toHaveBeenCalled();
  });

  it('treats autoplay rejection as an optional-media failure', async () => {
    const failed = media();
    failed.play = vi.fn().mockRejectedValue(new Error('blocked'));
    const controller = new AudioController(() => failed, { ...DEFAULT_PREFERENCES, audio: true });
    controller.unlockFromGesture();
    await expect(controller.play('effects', `/assets/${HASH}/audio-bell.mp3`)).resolves.toBe(false);
    expect(failed.pause).toHaveBeenCalled();
  });
});
