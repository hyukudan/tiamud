import type { ClientPreferences } from './preferences';

export type AudioChannel = 'music' | 'ambience' | 'effects';

export interface AudioLike {
  src: string;
  loop: boolean;
  volume: number;
  preload: string;
  play(): Promise<void>;
  pause(): void;
}
export type AudioFactory = (url: string) => AudioLike;

export const safeAudioUrl = (url: string): boolean =>
  /^\/assets\/[a-f0-9]{64}\/audio-[a-z0-9-]+\.(?:opus|ogg|mp3)$/.test(url);

/**
 * Opt-in audio boundary. Merely constructing the app never creates or plays
 * media; the controller must be unlocked by the user's enable-audio gesture.
 */
export class AudioController {
  private unlocked = false;
  private preferences: ClientPreferences;
  private readonly playing = new Map<AudioChannel, AudioLike>();

  constructor(private readonly factory: AudioFactory, preferences: ClientPreferences) {
    this.preferences = preferences;
  }

  unlockFromGesture(): void {
    this.unlocked = true;
  }

  updatePreferences(preferences: ClientPreferences): void {
    this.preferences = preferences;
    if (!preferences.audio) {
      this.stop();
      return;
    }
    for (const [channel, media] of this.playing) {
      media.volume = this.volume(channel);
    }
  }

  async play(channel: AudioChannel, url: string, loop = channel !== 'effects'): Promise<boolean> {
    if (!this.unlocked || !this.preferences.audio || !safeAudioUrl(url)) return false;
    this.stop(channel);
    const media = this.factory(url);
    media.src = url;
    media.loop = loop;
    media.preload = 'none';
    media.volume = this.volume(channel);
    this.playing.set(channel, media);
    try {
      await media.play();
      return true;
    } catch {
      this.playing.delete(channel);
      media.pause();
      return false;
    }
  }

  stop(channel?: AudioChannel): void {
    if (channel) {
      this.playing.get(channel)?.pause();
      this.playing.delete(channel);
      return;
    }
    for (const media of this.playing.values()) media.pause();
    this.playing.clear();
  }

  private volume(channel: AudioChannel): number {
    const value = channel === 'music'
      ? this.preferences.musicVolume
      : channel === 'ambience'
        ? this.preferences.ambienceVolume
        : this.preferences.effectsVolume;
    return Math.min(1, Math.max(0, value / 100));
  }
}
