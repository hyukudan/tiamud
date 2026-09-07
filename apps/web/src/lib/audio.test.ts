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

describe('overlapping audio requests', () => {
  it('keeps a replacement controllable after an older play promise rejects', async () => {
    let rejectFirst!: (error: Error) => void;
    const first = media();
    first.play = () => new Promise((_, reject) => { rejectFirst = reject; });
    const second = media();
    const queue = [first, second];
    const controller = new AudioController(() => queue.shift()!, { ...DEFAULT_PREFERENCES, audio: true });
    controller.unlockFromGesture();
    const pending = controller.play('music', `/assets/${HASH}/audio-first.ogg`);
    expect(await controller.play('music', `/assets/${HASH}/audio-second.ogg`)).toBe(true);
    rejectFirst(new Error('interrupted'));
    expect(await pending).toBe(false);
    controller.updatePreferences({ ...DEFAULT_PREFERENCES, audio: true, musicVolume: 25 });
    expect(second.volume).toBe(0.25);
    controller.stop();
    expect(second.pause).toHaveBeenCalledOnce();
  });

  it('does not report playback success after stop while play is pending', async () => {
    let finish!: () => void;
    const first = media();
    first.play = () => new Promise((resolve) => { finish = resolve; });
    const controller = new AudioController(() => first, { ...DEFAULT_PREFERENCES, audio: true });
    controller.unlockFromGesture();
    const pending = controller.play('music', `/assets/${HASH}/audio-first.ogg`);
    controller.stop();
    finish();
    expect(await pending).toBe(false);
    expect(first.pause).toHaveBeenCalled();
  });
});
