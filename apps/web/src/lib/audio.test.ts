import { describe, expect, it, vi } from 'vitest';
import {
  AudioController,
  DEFAULT_CROSSFADE_MS,
  FADE_STEP_MS,
  MAX_EFFECT_VOICES,
  safeAudioUrl,
  type AudioLike,
  type FadeTimers,
} from './audio';
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
    }, DEFAULT_PREFERENCES, { crossfadeMs: 0 });
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
    const controller = new AudioController(
      () => queue.shift()!,
      { ...DEFAULT_PREFERENCES, audio: true },
      { crossfadeMs: 0 },
    );
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

function manualTimers(): FadeTimers & { tick(times?: number): void; active(): number } {
  const intervals = new Map<number, () => void>();
  let next = 1;
  return {
    setInterval(callback) {
      const id = next++;
      intervals.set(id, callback);
      return id as unknown as ReturnType<typeof setInterval>;
    },
    clearInterval(handle) {
      intervals.delete(handle as unknown as number);
    },
    tick(times = 1) {
      for (let index = 0; index < times; index += 1) {
        for (const callback of [...intervals.values()]) callback();
      }
    },
    active: () => intervals.size,
  };
}

const enabled = { ...DEFAULT_PREFERENCES, audio: true, musicVolume: 80, effectsVolume: 50 };
const track = (name: string) => `/assets/${HASH}/audio-${name}.ogg`;
const FULL_FADE = Math.ceil(DEFAULT_CROSSFADE_MS / FADE_STEP_MS);

describe('music and ambience crossfade', () => {
  it('fades the old track out while the new one fades in to the preference volume', async () => {
    const timers = manualTimers();
    const created: AudioLike[] = [];
    const controller = new AudioController(() => {
      const next = media();
      created.push(next);
      return next;
    }, enabled, { timers });
    controller.unlockFromGesture();

    expect(await controller.play('music', track('dawn'))).toBe(true);
    expect(created[0].volume).toBe(0);
    timers.tick(FULL_FADE);
    expect(created[0].volume).toBeCloseTo(0.8);
    expect(timers.active()).toBe(0);

    expect(await controller.play('music', track('dusk'))).toBe(true);
    timers.tick(FULL_FADE / 2);
    expect(created[0].volume).toBeGreaterThan(0);
    expect(created[0].volume).toBeLessThan(0.8);
    expect(created[1].volume).toBeGreaterThan(0);
    expect(created[0].pause).not.toHaveBeenCalled();
    expect(controller.activeMediaCount).toBe(2);

    timers.tick(FULL_FADE);
    expect(created[0].volume).toBe(0);
    expect(created[0].pause).toHaveBeenCalled();
    expect(created[1].volume).toBeCloseTo(0.8);
    expect(controller.activeMediaCount).toBe(1);
    expect(timers.active()).toBe(0);
  });

  it('keeps at most one outgoing track per channel during rapid changes', async () => {
    const timers = manualTimers();
    const created: AudioLike[] = [];
    const controller = new AudioController(() => {
      const next = media();
      created.push(next);
      return next;
    }, enabled, { timers });
    controller.unlockFromGesture();
    for (let index = 0; index < 20; index += 1) {
      await controller.play('ambience', track(`wind-${index}`));
      expect(controller.activeMediaCount).toBeLessThanOrEqual(2);
      expect(timers.active()).toBeLessThanOrEqual(2);
    }
    for (const stale of created.slice(0, -2)) expect(stale.pause).toHaveBeenCalled();
    controller.stop();
    expect(controller.activeMediaCount).toBe(0);
    expect(timers.active()).toBe(0);
  });

  it('switches instantly when either the preference or the system reduces motion', async () => {
    for (const [preferences, system] of [
      [{ ...enabled, reduceMotion: true }, false],
      [enabled, true],
    ] as const) {
      const timers = manualTimers();
      const created: AudioLike[] = [];
      const controller = new AudioController(() => {
        const next = media();
        created.push(next);
        return next;
      }, preferences, { timers, systemReducedMotion: () => system });
      controller.unlockFromGesture();
      await controller.play('music', track('one'));
      await controller.play('music', track('two'));
      expect(created[0].pause).toHaveBeenCalled();
      expect(created[1].volume).toBeCloseTo(0.8);
      expect(timers.active()).toBe(0);
    }
  });

  it('retargets a fade-in when the volume preference changes mid-ramp', async () => {
    const timers = manualTimers();
    const music = media();
    const controller = new AudioController(() => music, enabled, { timers });
    controller.unlockFromGesture();
    await controller.play('music', track('dawn'));
    timers.tick(2);
    controller.updatePreferences({ ...enabled, musicVolume: 20 });
    timers.tick(FULL_FADE);
    expect(music.volume).toBeCloseTo(0.2);
  });

  it('disabling audio stops current, outgoing and pending fades', async () => {
    const timers = manualTimers();
    const created: AudioLike[] = [];
    const controller = new AudioController(() => {
      const next = media();
      created.push(next);
      return next;
    }, enabled, { timers });
    controller.unlockFromGesture();
    await controller.play('music', track('one'));
    await controller.play('music', track('two'));
    controller.updatePreferences({ ...enabled, audio: false });
    expect(created.every((entry) => (entry.pause as ReturnType<typeof vi.fn>).mock.calls.length > 0))
      .toBe(true);
    expect(timers.active()).toBe(0);
    expect(controller.activeMediaCount).toBe(0);
    expect(await controller.play('music', track('three'))).toBe(false);
  });

  it('bounds a hostile crossfade option', async () => {
    const timers = manualTimers();
    const music = media();
    const controller = new AudioController(() => music, enabled, {
      timers,
      crossfadeMs: Number.POSITIVE_INFINITY,
    });
    controller.unlockFromGesture();
    await controller.play('music', track('dawn'));
    timers.tick(FULL_FADE);
    expect(music.volume).toBeCloseTo(0.8);
  });
});

describe('overlapping effects', () => {
  it('lets effects overlap up to the pool size and evicts the oldest beyond it', async () => {
    const created: AudioLike[] = [];
    const controller = new AudioController(() => {
      const next = media();
      created.push(next);
      return next;
    }, enabled, { timers: manualTimers() });
    controller.unlockFromGesture();
    for (let index = 0; index < 4; index += 1) {
      expect(await controller.play('effects', track(`hit-${index}`))).toBe(true);
    }
    expect(created.every((entry) => entry.volume === 0.5)).toBe(true);
    expect(created.some((entry) => (entry.pause as ReturnType<typeof vi.fn>).mock.calls.length > 0))
      .toBe(false);
    expect(controller.activeMediaCount).toBe(4);

    await controller.play('effects', track('hit-4'));
    expect(created[0].pause).toHaveBeenCalled();
    expect(created[1].pause).not.toHaveBeenCalled();
    expect(controller.activeMediaCount).toBe(4);
  });

  it('frees a voice when an effect ends and clamps the pool option', async () => {
    const created: AudioLike[] = [];
    const controller = new AudioController(() => {
      const next = media();
      created.push(next);
      return next;
    }, enabled, { maxEffects: 1_000 });
    controller.unlockFromGesture();
    for (let index = 0; index < MAX_EFFECT_VOICES + 3; index += 1) {
      await controller.play('effects', track(`bell-${index}`));
    }
    expect(controller.activeMediaCount).toBe(MAX_EFFECT_VOICES);
    (created.at(-1)!.onended as () => void)();
    expect(controller.activeMediaCount).toBe(MAX_EFFECT_VOICES - 1);
    expect(created.at(-1)!.onended).toBeNull();
  });

  it('applies effects volume changes to every live voice and stops only effects on demand', async () => {
    const created: AudioLike[] = [];
    const controller = new AudioController(() => {
      const next = media();
      created.push(next);
      return next;
    }, enabled, { crossfadeMs: 0 });
    controller.unlockFromGesture();
    await controller.play('music', track('theme'));
    await controller.play('effects', track('a'));
    await controller.play('effects', track('b'));
    controller.updatePreferences({ ...enabled, effectsVolume: 10 });
    expect(created[1].volume).toBe(0.1);
    expect(created[2].volume).toBe(0.1);
    controller.stop('effects');
    expect(created[1].pause).toHaveBeenCalled();
    expect(created[0].pause).not.toHaveBeenCalled();
    expect(controller.activeMediaCount).toBe(1);
  });

  it('still refuses effects before the gesture or outside the asset allowlist', async () => {
    const factory = vi.fn(() => media());
    const controller = new AudioController(factory, enabled);
    expect(await controller.play('effects', track('a'))).toBe(false);
    controller.unlockFromGesture();
    expect(await controller.play('effects', 'https://tracker.test/audio-a.ogg')).toBe(false);
    expect(factory).not.toHaveBeenCalled();
  });
});
