import { motionReduced, type ClientPreferences } from './preferences';

export type AudioChannel = 'music' | 'ambience' | 'effects';

export interface AudioLike {
  src: string;
  loop: boolean;
  volume: number;
  preload: string;
  play(): Promise<void>;
  pause(): void;
  /** Optional completion hook (HTMLMediaElement provides it). */
  onended?: ((this: never, event: Event) => unknown) | (() => unknown) | null;
}
export type AudioFactory = (url: string) => AudioLike;

export interface FadeTimers {
  setInterval(callback: () => void, delay: number): ReturnType<typeof setInterval>;
  clearInterval(handle: ReturnType<typeof setInterval>): void;
}

export interface AudioControllerOptions {
  /** Crossfade length for music/ambience changes; clamped to [0, MAX_CROSSFADE_MS]. */
  crossfadeMs?: number;
  /** Concurrent one-shot effects; clamped to [1, MAX_EFFECT_VOICES]. */
  maxEffects?: number;
  timers?: FadeTimers;
  /** The operating-system reduced-motion signal; preferences are read separately. */
  systemReducedMotion?: () => boolean;
}

export const DEFAULT_CROSSFADE_MS = 600;
export const MAX_CROSSFADE_MS = 2_000;
export const DEFAULT_EFFECT_VOICES = 4;
export const MAX_EFFECT_VOICES = 8;
/** Fade step; a 600 ms crossfade is 12 volume writes, never an unbounded loop. */
export const FADE_STEP_MS = 50;

export const safeAudioUrl = (url: string): boolean =>
  /^\/assets\/[a-f0-9]{64}\/audio-[a-z0-9-]+\.(?:opus|ogg|mp3)$/.test(url);

const defaultFadeTimers: FadeTimers = {
  setInterval: (callback, delay) => setInterval(callback, delay),
  clearInterval: (handle) => clearInterval(handle),
};

type LoopChannel = Exclude<AudioChannel, 'effects'>;

interface Ramp {
  handle: ReturnType<typeof setInterval>;
}

const clampInteger = (value: number | undefined, fallback: number, min: number, max: number): number =>
  value === undefined || !Number.isFinite(value)
    ? fallback
    : Math.min(max, Math.max(min, Math.floor(value)));

/**
 * Opt-in audio boundary. Merely constructing the app never creates or plays
 * media; the controller must be unlocked by the user's enable-audio gesture.
 *
 * Music and ambience each hold one current track; a change fades the old
 * track out while the new one fades in (instant when motion is reduced or
 * the crossfade is zero). At most one outgoing track per channel exists, so a
 * burst of changes never accumulates media. Effects share a small fixed pool;
 * a new effect beyond the pool evicts the oldest. Audio carries no gameplay
 * facts, so any failure simply leaves the channel silent.
 */
export class AudioController {
  private unlocked = false;
  private preferences: ClientPreferences;
  private readonly current = new Map<LoopChannel, AudioLike>();
  private readonly outgoing = new Map<LoopChannel, AudioLike>();
  private readonly effects: AudioLike[] = [];
  private readonly ramps = new Map<AudioLike, Ramp>();
  private readonly crossfadeMs: number;
  private readonly maxEffects: number;
  private readonly timers: FadeTimers;
  private readonly systemReducedMotion: () => boolean;

  constructor(
    private readonly factory: AudioFactory,
    preferences: ClientPreferences,
    options: AudioControllerOptions = {},
  ) {
    this.preferences = preferences;
    this.crossfadeMs = clampInteger(options.crossfadeMs, DEFAULT_CROSSFADE_MS, 0, MAX_CROSSFADE_MS);
    this.maxEffects = clampInteger(options.maxEffects, DEFAULT_EFFECT_VOICES, 1, MAX_EFFECT_VOICES);
    this.timers = options.timers ?? defaultFadeTimers;
    this.systemReducedMotion = options.systemReducedMotion ?? (() => false);
  }

  unlockFromGesture(): void {
    this.unlocked = true;
  }

  /** Number of media elements currently owned (for bounds checks). */
  get activeMediaCount(): number {
    return this.current.size + this.outgoing.size + this.effects.length;
  }

  updatePreferences(preferences: ClientPreferences): void {
    this.preferences = preferences;
    if (!preferences.audio) {
      this.stop();
      return;
    }
    // Volume changes apply at once; a ramp in progress reads the new target
    // on its next step.
    for (const [channel, media] of this.current) {
      if (!this.ramps.has(media)) media.volume = this.volume(channel);
    }
    for (const media of this.effects) media.volume = this.volume('effects');
  }

  async play(channel: AudioChannel, url: string, loop = channel !== 'effects'): Promise<boolean> {
    if (!this.unlocked || !this.preferences.audio || !safeAudioUrl(url)) return false;
    return channel === 'effects'
      ? this.playEffect(url, loop)
      : this.playLoop(channel, url, loop);
  }

  stop(channel?: AudioChannel): void {
    if (channel === 'effects' || !channel) {
      for (const media of this.effects.splice(0)) this.release(media);
    }
    for (const loopChannel of ['music', 'ambience'] as const) {
      if (channel && channel !== loopChannel) continue;
      const current = this.current.get(loopChannel);
      if (current) this.release(current);
      this.current.delete(loopChannel);
      const outgoing = this.outgoing.get(loopChannel);
      if (outgoing) this.release(outgoing);
      this.outgoing.delete(loopChannel);
    }
  }

  private async playLoop(channel: LoopChannel, url: string, loop: boolean): Promise<boolean> {
    const fade = this.fadeDuration();
    const previous = this.current.get(channel);
    this.current.delete(channel);
    // Only one track may fade out per channel; an older one stops at once.
    const stale = this.outgoing.get(channel);
    if (stale) this.release(stale);
    this.outgoing.delete(channel);
    if (previous) {
      if (fade > 0) {
        this.outgoing.set(channel, previous);
        this.ramp(previous, () => 0, fade, () => {
          if (this.outgoing.get(channel) === previous) this.outgoing.delete(channel);
          this.release(previous);
        });
      } else {
        this.release(previous);
      }
    }

    const media = this.prepare(url, loop);
    media.volume = fade > 0 ? 0 : this.volume(channel);
    this.current.set(channel, media);
    try {
      await media.play();
    } catch {
      if (this.current.get(channel) === media) this.current.delete(channel);
      this.release(media);
      return false;
    }
    if (this.current.get(channel) !== media) {
      this.release(media);
      return false;
    }
    if (fade > 0) this.ramp(media, () => this.volume(channel), fade);
    return true;
  }

  private async playEffect(url: string, loop: boolean): Promise<boolean> {
    while (this.effects.length >= this.maxEffects) {
      const oldest = this.effects.shift();
      if (oldest) this.release(oldest);
    }
    const media = this.prepare(url, loop);
    media.volume = this.volume('effects');
    this.effects.push(media);
    media.onended = () => this.removeEffect(media);
    try {
      await media.play();
    } catch {
      this.removeEffect(media);
      return false;
    }
    if (!this.effects.includes(media)) {
      this.release(media);
      return false;
    }
    return true;
  }

  private removeEffect(media: AudioLike): void {
    const index = this.effects.indexOf(media);
    if (index >= 0) this.effects.splice(index, 1);
    this.release(media);
  }

  private prepare(url: string, loop: boolean): AudioLike {
    const media = this.factory(url);
    media.src = url;
    media.loop = loop;
    media.preload = 'none';
    return media;
  }

  private release(media: AudioLike): void {
    this.cancelRamp(media);
    media.onended = null;
    media.pause();
  }

  private fadeDuration(): number {
    return motionReduced(this.preferences, this.systemReducedMotion()) ? 0 : this.crossfadeMs;
  }

  private ramp(media: AudioLike, target: () => number, durationMs: number, done?: () => void): void {
    this.cancelRamp(media);
    const steps = Math.max(1, Math.ceil(durationMs / FADE_STEP_MS));
    const start = media.volume;
    let step = 0;
    const handle = this.timers.setInterval(() => {
      step += 1;
      const end = target();
      const progress = Math.min(1, step / steps);
      media.volume = clampVolume(start + (end - start) * progress);
      if (progress >= 1) {
        this.cancelRamp(media);
        done?.();
      }
    }, FADE_STEP_MS);
    this.ramps.set(media, { handle });
  }

  private cancelRamp(media: AudioLike): void {
    const ramp = this.ramps.get(media);
    if (!ramp) return;
    this.timers.clearInterval(ramp.handle);
    this.ramps.delete(media);
  }

  private volume(channel: AudioChannel): number {
    const value = channel === 'music'
      ? this.preferences.musicVolume
      : channel === 'ambience'
        ? this.preferences.ambienceVolume
        : this.preferences.effectsVolume;
    return clampVolume(value / 100);
  }
}

function clampVolume(value: number): number {
  return Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
}
