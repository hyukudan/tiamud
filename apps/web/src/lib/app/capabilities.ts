import {
  motionReduced,
  multimediaAllowed,
  type ClientPreferences,
} from '../preferences';
import type { BrowserCapabilities } from '../socket';

export function capabilitiesFor(
  value: ClientPreferences,
  systemReducedMotion: boolean,
): BrowserCapabilities {
  return {
    images: multimediaAllowed(value),
    reducedMotion: motionReduced(value, systemReducedMotion),
    audio: value.audio,
  };
}
