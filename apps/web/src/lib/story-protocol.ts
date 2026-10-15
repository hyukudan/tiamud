// Strict runtime validation of the `story.*` messages (ADR 0029). The wire
// shapes come from `crates/protocol`; every inbound frame is still checked
// here before the session applies it.
import type {
  MediaChannel,
  StoryEnding,
  StoryEntered,
  StoryError,
  StoryErrorCode,
  StoryItem,
  StoryLeft,
  StoryRestored,
  StorySaved,
  StorySlotSummary,
  StorySlots,
  StoryView,
} from './generated/protocol';
import { isRecord } from './guards';

export const MAX_STORY_ITEMS_PER_PART = 1_024;
export const MAX_STORY_PARTS = 64;
const MAX_STORY_TEXT = 4_096;
const MAX_STORY_LABEL = 1_024;
const MAX_U32 = 0xffff_ffff;
const MAX_SCORE = 1_000_000_000;

export const STORY_CHOICE_ID = /^c[0-9a-f]{16}$/;
const STORY_ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const SAFE_AUDIO_URL = /^\/assets\/[a-f0-9]{64}\/audio-[a-z0-9-]+\.(?:opus|ogg|mp3)$/;

const STORY_ERROR_CODES: readonly StoryErrorCode[] = [
  'disabled', 'unknown_story', 'already_in_story', 'capacity', 'not_in_story',
  'stale_view', 'unknown_choice', 'ended', 'nothing_to_undo', 'busy', 'internal',
  'slots_unavailable', 'invalid_slot', 'slot_empty', 'slot_corrupt', 'slot_incompatible',
  'slot_capacity', 'rate_limited',
];

/** Highest save slot; `0` is the autosave. At most ten slots per story. */
const MAX_SLOT = 9;
/** Unix seconds bound accepted for `updatedAt` (year 9999). */
const MAX_UNIX_SECONDS = 253_402_300_799;

const exactKeys = (
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[] = [],
): boolean => {
  const allowed = new Set([...required, ...optional]);
  return required.every((key) => key in value) &&
    Object.keys(value).every((key) => allowed.has(key));
};

const text = (value: unknown, maximum: number): string | undefined =>
  typeof value === 'string' && value.length > 0 && value.length <= maximum ? value : undefined;

const u32 = (value: unknown, minimum = 0): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= minimum && value <= MAX_U32
    ? value
    : undefined;

const channel = (value: unknown): MediaChannel | undefined =>
  value === 'music' || value === 'effects' || value === 'ambience' ? value : undefined;

const opaqueId = (value: unknown): string | undefined =>
  typeof value === 'string' && STORY_CHOICE_ID.test(value) ? value : undefined;

function boundedArray<T>(
  value: unknown,
  maximum: number,
  parse: (item: unknown) => T | undefined,
): T[] | undefined {
  if (!Array.isArray(value) || value.length > maximum) return undefined;
  const parsed: T[] = [];
  for (const item of value) {
    const result = parse(item);
    if (result === undefined) return undefined;
    parsed.push(result);
  }
  return parsed;
}

const parseCue = (value: Record<string, unknown>): StoryItem | undefined => {
  if (!exactKeys(value, ['type', 'action', 'channel', 'loop'], ['url'])) return undefined;
  const cueChannel = channel(value.channel);
  if (!cueChannel || typeof value.loop !== 'boolean') return undefined;
  if (value.action === 'stop') {
    return value.url === undefined
      ? { type: 'cue', action: 'stop', channel: cueChannel, loop: value.loop }
      : undefined;
  }
  if (value.action !== 'play' || typeof value.url !== 'string' || !SAFE_AUDIO_URL.test(value.url)) {
    return undefined;
  }
  return { type: 'cue', action: 'play', channel: cueChannel, url: value.url, loop: value.loop };
};

const parseEnding = (value: unknown): StoryEnding | null | undefined => {
  if (value === null) return null;
  return isRecord(value) && exactKeys(value, ['kind']) &&
    (value.kind === 'victory' || value.kind === 'defeat' || value.kind === 'neutral')
    ? { kind: value.kind }
    : undefined;
};

/** Validates one item of a `story.view` part; a malformed item fails the frame. */
export const parseStoryItem = (value: unknown): StoryItem | undefined => {
  if (!isRecord(value)) return undefined;
  switch (value.type) {
    case 'scene': {
      if (!exactKeys(value, ['type', 'title', 'text', 'score', 'ending'])) return undefined;
      const title = text(value.title, MAX_STORY_LABEL);
      const prose = typeof value.text === 'string' && value.text.length <= MAX_STORY_TEXT
        ? value.text
        : undefined;
      const score = typeof value.score === 'number' && Number.isSafeInteger(value.score) &&
        Math.abs(value.score) <= MAX_SCORE ? value.score : undefined;
      const ending = parseEnding(value.ending);
      return title && prose !== undefined && score !== undefined && ending !== undefined
        ? { type: 'scene', title, text: prose, score, ending }
        : undefined;
    }
    case 'cue':
      return parseCue(value);
    case 'portrait': {
      const name = text(value.name, MAX_STORY_LABEL);
      return exactKeys(value, ['type', 'name']) && name ? { type: 'portrait', name } : undefined;
    }
    case 'block': {
      if (!exactKeys(value, ['type', 'kind', 'text'], ['speaker'])) return undefined;
      const body = text(value.text, MAX_STORY_TEXT);
      const speaker = value.speaker === undefined ? undefined : text(value.speaker, MAX_STORY_LABEL);
      if (!body || (value.speaker !== undefined && !speaker) ||
        (value.kind !== 'event' && value.kind !== 'line')) return undefined;
      return speaker
        ? { type: 'block', kind: value.kind, speaker, text: body }
        : { type: 'block', kind: value.kind, text: body };
    }
    case 'stat': {
      const label = text(value.label, MAX_STORY_LABEL);
      const rendered = text(value.value, 64);
      return exactKeys(value, ['type', 'label', 'value']) && label && rendered
        ? { type: 'stat', label, value: rendered }
        : undefined;
    }
    case 'item': {
      const name = text(value.name, MAX_STORY_LABEL);
      return exactKeys(value, ['type', 'name']) && name ? { type: 'item', name } : undefined;
    }
    case 'choice':
    case 'dialogueOption': {
      const id = opaqueId(value.id);
      const label = text(value.label, MAX_STORY_LABEL);
      return exactKeys(value, ['type', 'id', 'label']) && id && label
        ? { type: value.type, id, label }
        : undefined;
    }
    case 'verb': {
      const id = opaqueId(value.id);
      const label = text(value.label, MAX_STORY_LABEL);
      return exactKeys(value, ['type', 'id', 'label', 'needsTarget']) && id && label &&
        typeof value.needsTarget === 'boolean'
        ? { type: 'verb', id, label, needsTarget: value.needsTarget }
        : undefined;
    }
    case 'target': {
      const id = opaqueId(value.id);
      const verb = opaqueId(value.verb);
      const label = text(value.label, MAX_STORY_LABEL);
      return exactKeys(value, ['type', 'verb', 'id', 'label']) && id && verb && label
        ? { type: 'target', verb, id, label }
        : undefined;
    }
    default:
      return undefined;
  }
};

export function parseStoryView(data: Record<string, unknown>): StoryView | undefined {
  if (!exactKeys(data, ['viewRev', 'part', 'parts', 'turn', 'canUndo', 'items'])) return undefined;
  const viewRev = u32(data.viewRev, 1);
  const parts = u32(data.parts, 1);
  const part = u32(data.part);
  const turn = u32(data.turn);
  const items = boundedArray(data.items, MAX_STORY_ITEMS_PER_PART, parseStoryItem);
  if (
    viewRev === undefined || parts === undefined || parts > MAX_STORY_PARTS ||
    part === undefined || part >= parts || turn === undefined || !items ||
    typeof data.canUndo !== 'boolean'
  ) return undefined;
  return { viewRev, part, parts, turn, canUndo: data.canUndo, items };
}

export function parseStoryEntered(data: Record<string, unknown>): StoryEntered | undefined {
  if (!exactKeys(data, ['storyId', 'title'])) return undefined;
  const title = text(data.title, MAX_STORY_LABEL);
  return typeof data.storyId === 'string' && STORY_ID.test(data.storyId) && title
    ? { storyId: data.storyId, title }
    : undefined;
}

export function parseStoryLeft(data: Record<string, unknown>): StoryLeft | undefined {
  return exactKeys(data, ['reason']) && (data.reason === 'left' || data.reason === 'evicted')
    ? { reason: data.reason }
    : undefined;
}

export function parseStoryError(data: Record<string, unknown>): StoryError | undefined {
  if (!exactKeys(data, ['code', 'message'])) return undefined;
  const message = text(data.message, 512);
  const code = STORY_ERROR_CODES.find((candidate) => candidate === data.code);
  return code && message ? { code, message } : undefined;
}

const slotNumber = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= MAX_SLOT
    ? value
    : undefined;

const unixSeconds = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 &&
    value <= MAX_UNIX_SECONDS ? value : undefined;

const parseSlotSummary = (value: unknown): StorySlotSummary | undefined => {
  if (!isRecord(value) || !exactKeys(value, ['slot', 'updatedAt', 'compatible'])) return undefined;
  const slot = slotNumber(value.slot);
  const updatedAt = unixSeconds(value.updatedAt);
  return slot !== undefined && updatedAt !== undefined && typeof value.compatible === 'boolean'
    ? { slot, updatedAt, compatible: value.compatible }
    : undefined;
};

/** `story.slots`: unique slots in ascending order, at most ten. */
export function parseStorySlots(data: Record<string, unknown>): StorySlots | undefined {
  if (!exactKeys(data, ['slots', 'offerContinue']) || typeof data.offerContinue !== 'boolean') {
    return undefined;
  }
  const slots = boundedArray(data.slots, MAX_SLOT + 1, parseSlotSummary);
  if (!slots || slots.some((slot, index) => index > 0 && slot.slot <= slots[index - 1].slot)) {
    return undefined;
  }
  // An offer to continue needs a compatible autosave to continue from.
  if (data.offerContinue && !slots.some((slot) => slot.slot === 0 && slot.compatible)) {
    return undefined;
  }
  return { slots, offerContinue: data.offerContinue };
}

export function parseStorySaved(data: Record<string, unknown>): StorySaved | undefined {
  if (!exactKeys(data, ['slot', 'updatedAt'])) return undefined;
  const slot = slotNumber(data.slot);
  const updatedAt = unixSeconds(data.updatedAt);
  return slot !== undefined && slot > 0 && updatedAt !== undefined
    ? { slot, updatedAt }
    : undefined;
}

export function parseStoryRestored(data: Record<string, unknown>): StoryRestored | undefined {
  if (!exactKeys(data, ['slot'])) return undefined;
  const slot = slotNumber(data.slot);
  return slot !== undefined ? { slot } : undefined;
}

/** A complete story view assembled from every part of one `viewRev`. */
export type AssembledStoryView = Omit<StoryView, 'part' | 'parts'>;

/**
 * Collects the parts of one view. Parts of an older revision are discarded
 * when a newer revision starts; a part out of order or inconsistent with its
 * siblings restarts the buffer instead of producing a mixed view. Opaque ids
 * must be unique across the whole view, every target must name a verb of the
 * view and exactly one scene item must exist.
 */
export class StoryViewAssembler {
  #parts: StoryView[] = [];

  push(part: StoryView): AssembledStoryView | undefined {
    const first = this.#parts[0];
    if (part.part === 0 || !first || first.viewRev !== part.viewRev ||
      first.parts !== part.parts || part.part !== this.#parts.length) {
      this.#parts = [];
      if (part.part !== 0) return undefined;
    }
    this.#parts.push(part);
    if (this.#parts.length < part.parts) return undefined;
    const items = this.#parts.flatMap((next) => next.items);
    this.#parts = [];
    const ids = items.flatMap((item) => ('id' in item ? [item.id] : []));
    const verbs = new Set(items.flatMap((item) => (item.type === 'verb' ? [item.id] : [])));
    if (new Set(ids).size !== ids.length ||
      items.some((item) => item.type === 'target' && !verbs.has(item.verb)) ||
      items.filter((item) => item.type === 'scene').length !== 1) return undefined;
    return { viewRev: part.viewRev, turn: part.turn, canUndo: part.canUndo, items };
  }

  reset(): void {
    this.#parts = [];
  }
}

export interface StoryVerbPresentation {
  id: string;
  label: string;
  needsTarget: boolean;
  targets: { id: string; label: string }[];
}

/** Read model of an assembled view for rendering. */
export interface StoryPresentation {
  title: string;
  paragraphs: string[];
  score: number;
  ending?: 'victory' | 'defeat' | 'neutral';
  events: string[];
  lines: { speaker?: string; text: string }[];
  present: string[];
  stats: { label: string; value: string }[];
  inventory: string[];
  choices: { id: string; label: string }[];
  dialogue: { id: string; label: string }[];
  verbs: StoryVerbPresentation[];
}

export function presentStory(view: AssembledStoryView): StoryPresentation {
  const model: StoryPresentation = {
    title: '', paragraphs: [], score: 0, events: [], lines: [], present: [], stats: [],
    inventory: [], choices: [], dialogue: [], verbs: [],
  };
  for (const item of view.items) {
    switch (item.type) {
      case 'scene':
        model.title = item.title;
        model.paragraphs = item.text.split('\n').filter((line) => line.trim().length > 0);
        model.score = item.score;
        model.ending = item.ending?.kind;
        break;
      case 'block':
        if (item.kind === 'event') model.events.push(item.text);
        else model.lines.push(item.speaker ? { speaker: item.speaker, text: item.text } : { text: item.text });
        break;
      case 'portrait':
        model.present.push(item.name);
        break;
      case 'stat':
        model.stats.push({ label: item.label, value: item.value });
        break;
      case 'item':
        model.inventory.push(item.name);
        break;
      case 'choice':
        model.choices.push({ id: item.id, label: item.label });
        break;
      case 'dialogueOption':
        model.dialogue.push({ id: item.id, label: item.label });
        break;
      case 'verb':
        model.verbs.push({ id: item.id, label: item.label, needsTarget: item.needsTarget, targets: [] });
        break;
      case 'target':
        model.verbs.find((verb) => verb.id === item.verb)?.targets.push({ id: item.id, label: item.label });
        break;
      case 'cue':
        break;
    }
  }
  return model;
}
