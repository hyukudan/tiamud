// Strict runtime validation of the `story.*` messages (ADR 0029). The wire
// shapes come from `crates/protocol`; every inbound frame is still checked
// here before the session applies it.
import type {
  MediaChannel,
  StoryCatalog,
  StoryCatalogEntry,
  StoryDecisionMode,
  StoryEnding,
  StoryEntered,
  StoryError,
  StoryErrorCode,
  StoryImage,
  StoryInvitation,
  StoryItem,
  StoryLeft,
  StoryLobby,
  StoryPrompt,
  StoryRestored,
  StorySaved,
  StorySeat,
  StorySeatRole,
  StorySlotSummary,
  StorySlots,
  StoryTally,
  StoryTallyEntry,
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
/** Reviewed pipeline manifest of a story background or portrait. */
const SAFE_IMAGE_MANIFEST = /^\/assets\/[a-f0-9]{64}\/manifest\.json$/;
/** Maximum alternative text of one story image (UTF-8 bytes on the server). */
const MAX_STORY_IMAGE_ALT = 512;
const STORY_LOCALE = /^[a-z][a-z0-9-]{1,34}$/;
/** Maximum languages one story offers. */
const MAX_STORY_LOCALES = 16;
/** Maximum stories one server catalog lists. */
export const MAX_CATALOG_STORIES = 64;

const STORY_ERROR_CODES: readonly StoryErrorCode[] = [
  'disabled', 'unknown_story', 'already_in_story', 'capacity', 'not_in_story',
  'stale_view', 'unknown_choice', 'ended', 'nothing_to_undo', 'busy', 'internal',
  'slots_unavailable', 'invalid_slot', 'slot_empty', 'slot_corrupt', 'slot_incompatible',
  'slot_capacity', 'rate_limited', 'host_only', 'spectator', 'not_your_turn', 'unknown_code',
  'seats_full', 'not_shared', 'unknown_member', 'invalid_setting', 'no_parser',
  'invalid_command', 'unknown_locale',
];

/** Maximum UTF-8 bytes of one `story.command` line (StoryPack v3 parser). */
export const MAX_STORY_COMMAND_BYTES = 256;
const MAX_STORY_PROMPT = 64;

const parsePrompt = (value: unknown): StoryPrompt | undefined => {
  if (!isRecord(value) || !exactKeys(value, ['text', 'question'])) return undefined;
  const prompt = text(value.text, MAX_STORY_PROMPT);
  return prompt && typeof value.question === 'boolean'
    ? { text: prompt, question: value.question }
    : undefined;
};

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

/** A reviewed story image: its pipeline manifest and plain alt text. */
export const parseStoryImage = (value: unknown): StoryImage | undefined => {
  if (!isRecord(value) || !exactKeys(value, ['manifest', 'alt'])) return undefined;
  const alt = text(value.alt, MAX_STORY_IMAGE_ALT);
  return typeof value.manifest === 'string' && SAFE_IMAGE_MANIFEST.test(value.manifest) &&
    alt && alt.trim().length > 0 && !/[\u0000-\u001f\u007f]/.test(alt)
    ? { manifest: value.manifest, alt }
    : undefined;
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
      if (!exactKeys(value, ['type', 'title', 'text', 'score', 'ending'], ['prompt', 'background'])) {
        return undefined;
      }
      const title = text(value.title, MAX_STORY_LABEL);
      const prose = typeof value.text === 'string' && value.text.length <= MAX_STORY_TEXT
        ? value.text
        : undefined;
      const score = typeof value.score === 'number' && Number.isSafeInteger(value.score) &&
        Math.abs(value.score) <= MAX_SCORE ? value.score : undefined;
      const ending = parseEnding(value.ending);
      const prompt = value.prompt === undefined ? undefined : parsePrompt(value.prompt);
      const background = value.background === undefined ? undefined : parseStoryImage(value.background);
      if (!title || prose === undefined || score === undefined || ending === undefined ||
        (value.prompt !== undefined && !prompt) ||
        (value.background !== undefined && !background)) return undefined;
      const scene: StoryItem = { type: 'scene', title, text: prose, score, ending };
      if (prompt) scene.prompt = prompt;
      if (background) scene.background = background;
      return scene;
    }
    case 'cue':
      return parseCue(value);
    case 'portrait': {
      const name = text(value.name, MAX_STORY_LABEL);
      const image = value.image === undefined ? undefined : parseStoryImage(value.image);
      if (!exactKeys(value, ['type', 'name'], ['image']) || !name ||
        (value.image !== undefined && !image)) return undefined;
      return image ? { type: 'portrait', name, image } : { type: 'portrait', name };
    }
    case 'block': {
      if (!exactKeys(value, ['type', 'kind', 'text'], ['speaker', 'portrait'])) return undefined;
      const body = text(value.text, MAX_STORY_TEXT);
      const speaker = value.speaker === undefined ? undefined : text(value.speaker, MAX_STORY_LABEL);
      const portrait = value.portrait === undefined ? undefined : parseStoryImage(value.portrait);
      if (!body || (value.speaker !== undefined && !speaker) ||
        (value.kind !== 'event' && value.kind !== 'line' && value.kind !== 'command' &&
          value.kind !== 'reply')) return undefined;
      // Only a spoken dialogue line may carry its speaker's portrait.
      if (value.portrait !== undefined && (!portrait || value.kind !== 'line' || !speaker)) {
        return undefined;
      }
      const block: StoryItem = { type: 'block', kind: value.kind, text: body };
      if (speaker) block.speaker = speaker;
      if (portrait) block.portrait = portrait;
      return block;
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

/**
 * `story.entered`. `locale`/`locales` name the instance language and the
 * languages the story offers; older servers omit them (Spanish only).
 */
export function parseStoryEntered(data: Record<string, unknown>): StoryEntered | undefined {
  if (!exactKeys(data, ['storyId', 'title'], ['locale', 'locales'])) return undefined;
  const title = text(data.title, MAX_STORY_LABEL);
  const locale = data.locale === undefined ? 'es' : data.locale;
  const locales = parseLocales(data.locales === undefined ? [locale] : data.locales);
  if (typeof locale !== 'string' || !STORY_LOCALE.test(locale) || !locales ||
    !locales.includes(locale)) return undefined;
  return typeof data.storyId === 'string' && STORY_ID.test(data.storyId) && title
    ? { storyId: data.storyId, title, locale, locales }
    : undefined;
}

/** One to sixteen distinct, well-formed locale tags, in server order. */
function parseLocales(value: unknown): string[] | undefined {
  if (!Array.isArray(value) || value.length === 0 || value.length > MAX_STORY_LOCALES ||
    !value.every((entry) => typeof entry === 'string' && STORY_LOCALE.test(entry)) ||
    new Set(value).size !== value.length) return undefined;
  return value as string[];
}

const parseCatalogEntry = (value: unknown): StoryCatalogEntry | undefined => {
  if (!isRecord(value) || !exactKeys(value, ['storyId', 'title', 'locales'])) return undefined;
  const title = text(value.title, MAX_STORY_LABEL);
  const locales = parseLocales(value.locales);
  return typeof value.storyId === 'string' && STORY_ID.test(value.storyId) && title && locales
    ? { storyId: value.storyId, title, locales }
    : undefined;
};

/** `story.catalog`: one part of the server's story list (ids, titles, languages). */
export function parseStoryCatalog(data: Record<string, unknown>): StoryCatalog | undefined {
  if (!exactKeys(data, ['part', 'parts', 'stories'])) return undefined;
  const parts = u32(data.parts, 1);
  const part = u32(data.part);
  const stories = boundedArray(data.stories, MAX_CATALOG_STORIES, parseCatalogEntry);
  return parts !== undefined && parts <= MAX_STORY_PARTS && part !== undefined && part < parts &&
    stories
    ? { part, parts, stories }
    : undefined;
}

/**
 * Collects the parts of one catalog listing. A part out of order restarts
 * the buffer; the assembled list must have unique ids and at most 64 stories.
 */
export class StoryCatalogAssembler {
  #parts: StoryCatalog[] = [];

  push(part: StoryCatalog): StoryCatalogEntry[] | undefined {
    const first = this.#parts[0];
    if (part.part === 0 || !first || first.parts !== part.parts ||
      part.part !== this.#parts.length) {
      this.#parts = [];
      if (part.part !== 0) return undefined;
    }
    this.#parts.push(part);
    if (this.#parts.length < part.parts) return undefined;
    const stories = this.#parts.flatMap((next) => next.stories);
    this.#parts = [];
    const ids = new Set(stories.map((story) => story.storyId));
    return ids.size === stories.length && stories.length <= MAX_CATALOG_STORIES
      ? stories
      : undefined;
  }

  reset(): void {
    this.#parts = [];
  }
}

/**
 * The language a story picker preselects: the reader's first preferred
 * language the story offers (exact tag, then primary language), else the
 * story's default (its first locale).
 */
export function preferredStoryLocale(
  locales: readonly string[],
  preferred: readonly unknown[] = [],
): string {
  const offered = locales.map((locale) => locale.toLowerCase());
  for (const candidate of preferred.slice(0, MAX_STORY_LOCALES)) {
    if (typeof candidate !== 'string' || candidate.length > 35) continue;
    const tag = candidate.trim().toLowerCase();
    const exact = offered.indexOf(tag);
    if (exact >= 0) return locales[exact];
    const primary = offered.indexOf(tag.split('-', 1)[0]);
    if (primary >= 0) return locales[primary];
  }
  return locales[0] ?? 'en';
}

export function parseStoryLeft(data: Record<string, unknown>): StoryLeft | undefined {
  return exactKeys(data, ['reason']) && (data.reason === 'left' || data.reason === 'evicted' ||
    data.reason === 'kicked' || data.reason === 'ended')
    ? { reason: data.reason }
    : undefined;
}

// --- Shared story instances (ADR 0033) -------------------------------------

/** Absolute caps of one shared instance and the vote window bounds. */
export const MAX_STORY_SEATS = 8;
export const MAX_STORY_SPECTATORS = 32;
export const MIN_STORY_VOTE_SECONDS = 30;
export const MAX_STORY_VOTE_SECONDS = 120;
export const STORY_JOIN_CODE = /^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/;
const MAX_MEMBER_NAME = 64;

const decisionMode = (value: unknown): StoryDecisionMode | undefined =>
  value === 'host' || value === 'vote' || value === 'turns' ? value : undefined;

const seatRole = (value: unknown): StorySeatRole | undefined =>
  value === 'host' || value === 'player' || value === 'spectator' ? value : undefined;

const boundedCount = (value: unknown, maximum: number): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= maximum
    ? value
    : undefined;

const parseSeat = (value: unknown): StorySeat | undefined => {
  if (!isRecord(value) || !exactKeys(value, ['name', 'role', 'connected', 'turn'])) return undefined;
  const name = text(value.name, MAX_MEMBER_NAME);
  const role = seatRole(value.role);
  return name && (role === 'host' || role === 'player') &&
    typeof value.connected === 'boolean' && typeof value.turn === 'boolean'
    ? { name, role, connected: value.connected, turn: value.turn }
    : undefined;
};

/**
 * `story.lobby`: at most eight seats with exactly one host (the first seat),
 * at most one turn holder, and the join code only when the recipient hosts.
 */
export function parseStoryLobby(data: Record<string, unknown>): StoryLobby | undefined {
  if (!exactKeys(
    data,
    ['mode', 'voteSeconds', 'you', 'seats', 'spectators', 'maxSeats', 'maxSpectators'],
    ['code'],
  )) return undefined;
  const mode = decisionMode(data.mode);
  const you = seatRole(data.you);
  const voteSeconds = typeof data.voteSeconds === 'number' && Number.isSafeInteger(data.voteSeconds) &&
    data.voteSeconds >= MIN_STORY_VOTE_SECONDS && data.voteSeconds <= MAX_STORY_VOTE_SECONDS
    ? data.voteSeconds
    : undefined;
  const maxSeats = boundedCount(data.maxSeats, MAX_STORY_SEATS);
  const maxSpectators = boundedCount(data.maxSpectators, MAX_STORY_SPECTATORS);
  const spectators = boundedCount(data.spectators, MAX_STORY_SPECTATORS);
  const seats = boundedArray(data.seats, MAX_STORY_SEATS, parseSeat);
  if (!mode || !you || voteSeconds === undefined || maxSeats === undefined ||
    maxSpectators === undefined || spectators === undefined || !seats ||
    seats.length > maxSeats || spectators > maxSpectators) return undefined;
  if (seats.length > 0 && (seats[0].role !== 'host' ||
    seats.slice(1).some((seat) => seat.role === 'host'))) return undefined;
  if (seats.filter((seat) => seat.turn).length > (mode === 'turns' ? 1 : 0)) return undefined;
  const code = data.code;
  if (code !== undefined &&
    (you !== 'host' || typeof code !== 'string' || !STORY_JOIN_CODE.test(code))) {
    return undefined;
  }
  const lobby: StoryLobby = { mode, voteSeconds, you, seats, spectators, maxSeats, maxSpectators };
  if (typeof code === 'string') lobby.code = code;
  return lobby;
}

const parseTallyEntry = (value: unknown): StoryTallyEntry | undefined => {
  if (!isRecord(value) || !exactKeys(value, ['choiceId', 'votes'])) return undefined;
  const choiceId = opaqueId(value.choiceId);
  const votes = boundedCount(value.votes, MAX_STORY_SEATS);
  return choiceId && votes !== undefined && votes > 0 ? { choiceId, votes } : undefined;
};

/** `story.tally`: public counts only; never voter identities. */
export function parseStoryTally(data: Record<string, unknown>): StoryTally | undefined {
  if (!exactKeys(
    data,
    ['viewRev', 'mode', 'choices', 'voted', 'eligible'],
    ['remainingSeconds', 'yourVote'],
  )) return undefined;
  const viewRev = u32(data.viewRev, 1);
  const mode = decisionMode(data.mode);
  const choices = boundedArray(data.choices, MAX_STORY_SEATS, parseTallyEntry);
  const voted = boundedCount(data.voted, MAX_STORY_SEATS);
  const eligible = boundedCount(data.eligible, MAX_STORY_SEATS);
  const remaining = data.remainingSeconds === undefined
    ? undefined
    : boundedCount(data.remainingSeconds, MAX_STORY_VOTE_SECONDS);
  const yourVote = data.yourVote === undefined ? undefined : opaqueId(data.yourVote);
  if (viewRev === undefined || !mode || !choices || voted === undefined || eligible === undefined ||
    (data.remainingSeconds !== undefined && remaining === undefined) ||
    (data.yourVote !== undefined && !yourVote)) return undefined;
  const ids = choices.map((entry) => entry.choiceId);
  if (new Set(ids).size !== ids.length ||
    choices.reduce((sum, entry) => sum + entry.votes, 0) !== voted ||
    (yourVote !== undefined && !ids.includes(yourVote))) return undefined;
  const tally: StoryTally = { viewRev, mode, choices, voted, eligible };
  if (remaining !== undefined) tally.remainingSeconds = remaining;
  if (yourVote !== undefined) tally.yourVote = yourVote;
  return tally;
}

/** `story.invitation`: who invited this character, to what, with which code. */
export function parseStoryInvitation(data: Record<string, unknown>): StoryInvitation | undefined {
  if (!exactKeys(data, ['host', 'storyId', 'title', 'code'])) return undefined;
  const host = text(data.host, MAX_MEMBER_NAME);
  const title = text(data.title, MAX_STORY_LABEL);
  return host && title && typeof data.storyId === 'string' && STORY_ID.test(data.storyId) &&
    typeof data.code === 'string' && STORY_JOIN_CODE.test(data.code)
    ? { host, storyId: data.storyId, title, code: data.code }
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
  /** Menu responses and dialogue lines; `portrait` is the speaker's reviewed image. */
  lines: { speaker?: string; text: string; portrait?: StoryImage }[];
  present: string[];
  /** Characters present with their reviewed portrait, when one was sent. */
  portraits: { name: string; image?: StoryImage }[];
  /** Reviewed background of the scene, when one was sent. */
  background?: StoryImage;
  stats: { label: string; value: string }[];
  inventory: string[];
  choices: { id: string; label: string }[];
  dialogue: { id: string; label: string }[];
  verbs: StoryVerbPresentation[];
  /** Free-text prompt of a parser story; absent when no text is accepted. */
  prompt?: StoryPrompt;
  /** Echo of the command this view answers (parser stories). */
  command?: string;
  /** Parser replies to that command, in order. */
  replies: string[];
}

export function presentStory(view: AssembledStoryView): StoryPresentation {
  const model: StoryPresentation = {
    title: '', paragraphs: [], score: 0, events: [], lines: [], present: [], portraits: [], stats: [],
    inventory: [], choices: [], dialogue: [], verbs: [], replies: [],
  };
  for (const item of view.items) {
    switch (item.type) {
      case 'scene':
        model.title = item.title;
        model.paragraphs = item.text.split('\n').filter((line) => line.trim().length > 0);
        model.score = item.score;
        model.ending = item.ending?.kind;
        if (item.prompt) model.prompt = item.prompt;
        if (item.background) model.background = item.background;
        break;
      case 'block':
        if (item.kind === 'event') model.events.push(item.text);
        else if (item.kind === 'command') model.command = item.text;
        else if (item.kind === 'reply') model.replies.push(item.text);
        else {
          const line: StoryPresentation['lines'][number] = { text: item.text };
          if (item.speaker) line.speaker = item.speaker;
          if (item.portrait) line.portrait = item.portrait;
          model.lines.push(line);
        }
        break;
      case 'portrait':
        model.present.push(item.name);
        model.portraits.push(item.image ? { name: item.name, image: item.image } : { name: item.name });
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
