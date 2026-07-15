export interface ClientPreferences {
  images: boolean;
  audio: boolean;
  reduceMotion: boolean;
  saveData: boolean;
  highContrast: boolean;
  screenReader: boolean;
  fontScale: 100 | 112 | 125;
  musicVolume: number;
  ambienceVolume: number;
  effectsVolume: number;
}

export const DEFAULT_PREFERENCES: ClientPreferences = {
  images: true,
  audio: false,
  reduceMotion: false,
  saveData: false,
  highContrast: false,
  screenReader: false,
  fontScale: 100,
  musicVolume: 50,
  ambienceVolume: 50,
  effectsVolume: 70,
};

const STORAGE_KEY = 'tiamud.preferences.v1';

const isBoolean = (value: unknown): value is boolean => typeof value === 'boolean';
const isVolume = (value: unknown): value is number =>
  typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100;

export function loadPreferences(
  storage: Pick<Storage, 'getItem'> | undefined,
  saveData = false,
): ClientPreferences {
  let stored: Record<string, unknown> = {};
  try {
    const value = storage?.getItem(STORAGE_KEY);
    const parsed: unknown = value ? JSON.parse(value) : {};
    if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
      stored = parsed as Record<string, unknown>;
    }
  } catch {
    // Corrupt or unavailable storage falls back to safe defaults.
  }

  const fontScale = stored.fontScale;
  return {
    images: isBoolean(stored.images) ? stored.images : DEFAULT_PREFERENCES.images,
    audio: isBoolean(stored.audio) ? stored.audio : DEFAULT_PREFERENCES.audio,
    reduceMotion: isBoolean(stored.reduceMotion)
      ? stored.reduceMotion
      : DEFAULT_PREFERENCES.reduceMotion,
    saveData: saveData || (isBoolean(stored.saveData) ? stored.saveData : false),
    highContrast: isBoolean(stored.highContrast) ? stored.highContrast : false,
    screenReader: isBoolean(stored.screenReader) ? stored.screenReader : false,
    fontScale: fontScale === 112 || fontScale === 125 ? fontScale : 100,
    musicVolume: isVolume(stored.musicVolume)
      ? stored.musicVolume
      : DEFAULT_PREFERENCES.musicVolume,
    ambienceVolume: isVolume(stored.ambienceVolume)
      ? stored.ambienceVolume
      : DEFAULT_PREFERENCES.ambienceVolume,
    effectsVolume: isVolume(stored.effectsVolume)
      ? stored.effectsVolume
      : DEFAULT_PREFERENCES.effectsVolume,
  };
}

export function savePreferences(
  storage: Pick<Storage, 'setItem'> | undefined,
  preferences: ClientPreferences,
): void {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // Preferences remain effective for this session when storage is blocked.
  }
}

export function multimediaAllowed(preferences: ClientPreferences): boolean {
  return preferences.images && !preferences.saveData && !preferences.screenReader;
}

export function motionReduced(preferences: ClientPreferences, systemReduced: boolean): boolean {
  return preferences.reduceMotion || preferences.screenReader || systemReduced;
}
