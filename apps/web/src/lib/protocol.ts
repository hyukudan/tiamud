export const PROTOCOL_VERSION = 1 as const;

export type ConnectionState = 'connecting' | 'connected' | 'disconnected';

export interface Envelope<TType extends string, TData> {
  v: number;
  type: TType;
  seq?: number;
  id?: string;
  data: TData;
}

export interface ServerWelcomeData {
  serverName?: string;
  motd?: string;
  sessionId?: string;
}

export interface SceneImage {
  alt: string;
  localizedAlt?: Record<string, string>;
  manifest?: string;
  avif?: string;
  avifSrcset?: string;
  webp?: string;
  webpSrcset?: string;
  jpeg?: string;
  jpegSrcset?: string;
  png?: string;
  pngSrcset?: string;
  width?: number;
  height?: number;
}

export function localizedImageAlt(image: SceneImage | undefined, locale: string): string | undefined {
  if (!image) return undefined;
  const entries = Object.entries(image.localizedAlt ?? {});
  const normalized = locale.trim().toLowerCase();
  const exact = entries.find(([key]) => key.toLowerCase() === normalized)?.[1];
  const language = normalized.split('-', 1)[0];
  const base = entries.find(([key]) => key.toLowerCase() === language)?.[1];
  const english = entries.find(([key]) => key.toLowerCase() === 'en')?.[1];
  return exact ?? base ?? english ?? entries.toSorted(([left], [right]) => left.localeCompare(right))[0]?.[1] ?? image.alt;
}

export interface SceneExit {
  id: string;
  label: string;
  command: string;
}

export interface SceneAction {
  label: string;
  command: string;
}

export interface SceneItem {
  id: string;
  label: string;
  description: string;
  actions: SceneAction[];
}

export interface SceneNpc {
  id: string;
  label: string;
  description: string;
  health: number;
  maxHealth: number;
  actions: SceneAction[];
}

export interface SceneWeather {
  condition: string;
  day: number;
  hour: number;
  nextChangeInTicks: number;
}

export interface SceneSnapshot {
  viewId?: string;
  revision?: number;
  title: string;
  prose: string[];
  image?: SceneImage;
  exits: SceneExit[];
  items: SceneItem[];
  npcs: SceneNpc[];
  weather: SceneWeather;
}

export interface ActionResultData {
  requestId?: string;
  ok: boolean;
  message?: string;
}

export interface ServerNoticeData {
  level?: 'info' | 'warning' | 'error';
  code?: string;
  message: string;
}

export interface MessageAppendData {
  revision: number;
  kind: 'characterMoved' | 'itemTaken' | 'itemDropped' | 'gameplay' | 'adventure';
  text: string;
  actorId: string;
  targetId?: string;
  itemId?: string;
}

export interface SyncSnapshotData {
  afterSeq: number;
  scene: SceneSnapshot;
  player: PlayerStateSnapshot;
}

export interface ScenePatchData {
  viewId: string;
  revision: number;
  title?: string;
  prose?: string[];
  image?: SceneImage | null;
  exits?: SceneExit[];
  weather?: SceneWeather;
}

interface EntityUpdateBase {
  viewId: string;
  revision: number;
  id: string;
}

export type EntityUpsertData =
  | (EntityUpdateBase & { kind: 'item'; entity: SceneItem })
  | (EntityUpdateBase & { kind: 'npc'; entity: SceneNpc });

export interface EntityRemoveData {
  viewId: string;
  revision: number;
  kind: 'item' | 'npc';
  id: string;
}

export interface VitalsUpdateData {
  revision: number;
  character: PlayerCharacterSnapshot;
}

export interface InventoryPatchData {
  revision: number;
  inventory: PlayerItemSnapshot[];
  equipment: EquippedItemSnapshot[];
}

export interface MapSnapshotData {
  revision: number;
  map: PlayerStateSnapshot['map'];
}

export interface ServerHeartbeatData {
  nonce: number;
  timeoutMs: number;
}

export interface MediaPlayData {
  channel: 'music' | 'ambience' | 'effects';
  url: string;
  loop: boolean;
}

export interface MediaStopData {
  channel?: 'music' | 'ambience' | 'effects';
}

export interface ChatMessageData {
  channel: 'say' | 'tell';
  sender: string;
  text: string;
}

export interface PlayerCharacterSnapshot {
  name: string;
  level: number;
  experience: number;
  nextLevelExperience: number;
  health: number;
  maxHealth: number;
  strength: number;
  defence: number;
  skills: Array<{ id: string; label: string; rank: number }>;
  effects: Array<{
    kind: 'regeneration' | 'strength';
    magnitude: number;
    remainingTicks: number;
  }>;
  recoveryInTicks?: number;
}

export interface PlayerItemSnapshot {
  id: string;
  label: string;
  description: string;
  actions: SceneAction[];
}

export interface EquippedItemSnapshot {
  id: string;
  slot: 'weapon' | 'body' | 'trinket';
  label: string;
  actions: SceneAction[];
}

export interface PlayerStateSnapshot {
  revision: number;
  character: PlayerCharacterSnapshot;
  inventory: PlayerItemSnapshot[];
  equipment: EquippedItemSnapshot[];
  map: {
    currentRoomId: string;
    rooms: Array<{ id: string; title: string; current: boolean }>;
    paths: Array<{ from: string; direction: string; to?: string }>;
  };
  quests: Array<{
    id: string;
    label: string;
    summary: string;
    status: 'available' | 'active' | 'readyToClaim' | 'completed';
    current: number;
    target: number;
    actions: SceneAction[];
  }>;
  party?: {
    leader: boolean;
    members: Array<{ id: string; name: string; leader: boolean; online: boolean }>;
    actions: SceneAction[];
  };
  partyInvites: Array<{
    leaderName: string;
    expiresInTicks: number;
    actions: SceneAction[];
  }>;
  economy: {
    coins: number;
    commodities: Array<{ id: string; label: string; quantity: number }>;
    shopAvailable: boolean;
    shop: Array<{
      id: string;
      label: string;
      buyPrice: number;
      sellPrice: number;
      stock: number;
      owned: number;
      actions: SceneAction[];
    }>;
  };
  social: {
    online: Array<{ id: string; name: string; selfCharacter: boolean }>;
  };
}

export type ServerMessage =
  | Envelope<'server.welcome', ServerWelcomeData>
  | Envelope<'scene.snapshot', SceneSnapshot>
  | Envelope<'scene.patch', ScenePatchData>
  | Envelope<'entity.upsert', EntityUpsertData>
  | Envelope<'entity.remove', EntityRemoveData>
  | Envelope<'player.snapshot', PlayerStateSnapshot>
  | Envelope<'vitals.update', VitalsUpdateData>
  | Envelope<'inventory.patch', InventoryPatchData>
  | Envelope<'map.snapshot', MapSnapshotData>
  | Envelope<'action.result', ActionResultData>
  | Envelope<'server.notice', ServerNoticeData>
  | Envelope<'message.append', MessageAppendData>
  | Envelope<'sync.snapshot', SyncSnapshotData>
  | Envelope<'server.heartbeat', ServerHeartbeatData>
  | Envelope<'media.play', MediaPlayData>
  | Envelope<'media.stop', MediaStopData>
  | Envelope<'chat.message', ChatMessageData>;

export type ClientHello = Envelope<
  'client.hello',
  {
    client: 'web';
    capabilities: {
      images: readonly ('avif' | 'webp' | 'jpeg')[];
      reducedMotion: boolean;
      audio: boolean;
    };
    auth: {
      token: string;
      characterId: number;
    };
  }
>;

export type CommandSubmit = Envelope<
  'command.submit',
  {
    requestId: string;
    command: string;
  }
>;

export type ActionPerform = Envelope<
  'action.perform',
  {
    requestId: string;
    action: string;
    target?: string;
  }
>;

export type SyncRequest = Envelope<'sync.request', { afterSeq: number }>;

export type ClientHeartbeat = Envelope<'client.heartbeat', { nonce: number }>;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const stringOrUndefined = (value: unknown): string | undefined =>
  typeof value === 'string' && value.length > 0 ? value : undefined;

const numberOrUndefined = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined;

const positiveIntegerOrUndefined = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined;

const nonNegativeIntegerOrUndefined = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : undefined;

const hasOnlyKeys = (value: Record<string, unknown>, allowed: readonly string[]): boolean => {
  const keys = new Set(allowed);
  return Object.keys(value).every((key) => keys.has(key));
};

const boundedString = (value: unknown, maximum: number): string | undefined => {
  const text = stringOrUndefined(value);
  return text && text.length <= maximum ? text : undefined;
};

const opaqueId = (value: unknown): string | undefined => {
  const id = stringOrUndefined(value);
  return id && /^o[0-9a-f]{16}$/.test(id) ? id : undefined;
};

const contentId = (value: unknown): string | undefined => {
  const id = stringOrUndefined(value);
  return id && /^[a-z0-9][a-z0-9-]{0,63}$/.test(id) ? id : undefined;
};

const parseActions = (value: unknown): SceneAction[] | undefined => {
  if (!Array.isArray(value) || value.length > 8) return undefined;
  const actions: SceneAction[] = [];
  for (const entry of value) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['label', 'command'])) return undefined;
    const label = boundedString(entry.label, 128);
    const command = boundedString(entry.command, 1024);
    if (!label || !command) return undefined;
    actions.push({ label, command });
  }
  return actions;
};

export function parsePlayerState(value: unknown): PlayerStateSnapshot | undefined {
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, [
      'revision', 'character', 'inventory', 'equipment', 'map', 'quests', 'party',
      'partyInvites', 'economy', 'social',
    ])
  ) return undefined;

  const revision = positiveIntegerOrUndefined(value.revision);
  const character = value.character;
  if (
    revision === undefined ||
    !isRecord(character) ||
    !hasOnlyKeys(character, [
      'name', 'level', 'experience', 'nextLevelExperience', 'health', 'maxHealth',
      'strength', 'defence', 'skills', 'effects', 'recoveryInTicks',
    ])
  ) return undefined;
  const name = boundedString(character.name, 64);
  const level = positiveIntegerOrUndefined(character.level);
  const experience = nonNegativeIntegerOrUndefined(character.experience);
  const nextLevelExperience = positiveIntegerOrUndefined(character.nextLevelExperience);
  const health = nonNegativeIntegerOrUndefined(character.health);
  const maxHealth = positiveIntegerOrUndefined(character.maxHealth);
  const strength = positiveIntegerOrUndefined(character.strength);
  const defence = nonNegativeIntegerOrUndefined(character.defence);
  const recovery = character.recoveryInTicks === null || character.recoveryInTicks === undefined
    ? undefined
    : nonNegativeIntegerOrUndefined(character.recoveryInTicks);
  if (
    !name || level === undefined || experience === undefined || nextLevelExperience === undefined ||
    health === undefined || maxHealth === undefined || health > maxHealth || strength === undefined ||
    defence === undefined ||
    (character.recoveryInTicks !== null && character.recoveryInTicks !== undefined && recovery === undefined) ||
    !Array.isArray(character.effects) || character.effects.length > 32
  ) return undefined;
  const effects: PlayerCharacterSnapshot['effects'] = [];
  for (const effect of character.effects) {
    if (!isRecord(effect) || !hasOnlyKeys(effect, ['kind', 'magnitude', 'remainingTicks'])) return undefined;
    const kind = effect.kind;
    const magnitude = positiveIntegerOrUndefined(effect.magnitude);
    const remainingTicks = positiveIntegerOrUndefined(effect.remainingTicks);
    if (
      (kind !== 'regeneration' && kind !== 'strength') ||
      magnitude === undefined || remainingTicks === undefined
    ) return undefined;
    effects.push({ kind, magnitude, remainingTicks });
  }
  const skills: PlayerCharacterSnapshot['skills'] = [];
  if (character.skills !== undefined) {
    if (!Array.isArray(character.skills) || character.skills.length > 32) return undefined;
    for (const skill of character.skills) {
      if (!isRecord(skill) || !hasOnlyKeys(skill, ['id', 'label', 'rank'])) return undefined;
      const id = contentId(skill.id);
      const label = boundedString(skill.label, 128);
      const rank = nonNegativeIntegerOrUndefined(skill.rank);
      if (!id || !label || rank === undefined || rank > 100) return undefined;
      skills.push({ id, label, rank });
    }
  }

  if (!Array.isArray(value.inventory) || value.inventory.length > 256) return undefined;
  const inventory: PlayerItemSnapshot[] = [];
  for (const entry of value.inventory) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'label', 'description', 'actions'])) return undefined;
    const id = opaqueId(entry.id);
    const label = boundedString(entry.label, 256);
    const description = typeof entry.description === 'string' && entry.description.length <= 4096
      ? entry.description
      : undefined;
    const actions = parseActions(entry.actions);
    if (!id || !label || description === undefined || !actions) return undefined;
    inventory.push({ id, label, description, actions });
  }

  if (!Array.isArray(value.equipment) || value.equipment.length > 3) return undefined;
  const equipment: EquippedItemSnapshot[] = [];
  for (const entry of value.equipment) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'slot', 'label', 'actions'])) return undefined;
    const id = opaqueId(entry.id);
    const slot = entry.slot;
    const label = boundedString(entry.label, 256);
    const actions = parseActions(entry.actions);
    if (!id || (slot !== 'weapon' && slot !== 'body' && slot !== 'trinket') || !label || !actions) {
      return undefined;
    }
    equipment.push({ id, slot, label, actions });
  }

  const map = value.map;
  if (
    !isRecord(map) || !hasOnlyKeys(map, ['currentRoomId', 'rooms', 'paths']) ||
    !Array.isArray(map.rooms) || map.rooms.length > 512 ||
    !Array.isArray(map.paths) || map.paths.length > 3072
  ) return undefined;
  const currentRoomId = opaqueId(map.currentRoomId);
  if (!currentRoomId) return undefined;
  const rooms: PlayerStateSnapshot['map']['rooms'] = [];
  for (const entry of map.rooms) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'title', 'current'])) return undefined;
    const id = opaqueId(entry.id);
    const title = boundedString(entry.title, 256);
    if (!id || !title || typeof entry.current !== 'boolean') return undefined;
    rooms.push({ id, title, current: entry.current });
  }
  if (!rooms.some((room) => room.id === currentRoomId && room.current)) return undefined;
  const paths: PlayerStateSnapshot['map']['paths'] = [];
  const roomIds = new Set(rooms.map((room) => room.id));
  for (const entry of map.paths) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['from', 'direction', 'to'])) return undefined;
    const from = opaqueId(entry.from);
    const direction = boundedString(entry.direction, 16);
    const to = entry.to === undefined ? undefined : opaqueId(entry.to);
    if (!from || !roomIds.has(from) || !direction || (entry.to !== undefined && !to) || (to && !roomIds.has(to))) {
      return undefined;
    }
    paths.push({ from, direction, ...(to ? { to } : {}) });
  }

  if (!Array.isArray(value.quests) || value.quests.length > 32) return undefined;
  const quests: PlayerStateSnapshot['quests'] = [];
  for (const entry of value.quests) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'label', 'summary', 'status', 'current', 'target', 'actions'])) {
      return undefined;
    }
    const id = contentId(entry.id);
    const label = boundedString(entry.label, 128);
    const summary = boundedString(entry.summary, 1024);
    const status = entry.status;
    const current = nonNegativeIntegerOrUndefined(entry.current);
    const target = positiveIntegerOrUndefined(entry.target);
    const actions = parseActions(entry.actions);
    if (
      !id || !label || !summary ||
      (status !== 'available' && status !== 'active' && status !== 'readyToClaim' && status !== 'completed') ||
      current === undefined || target === undefined || current > target || !actions
    ) return undefined;
    quests.push({ id, label, summary, status, current, target, actions });
  }

  let party: PlayerStateSnapshot['party'];
  if (value.party !== undefined && value.party !== null) {
    if (!isRecord(value.party) || !hasOnlyKeys(value.party, ['leader', 'members', 'actions']) ||
      typeof value.party.leader !== 'boolean' || !Array.isArray(value.party.members) ||
      value.party.members.length === 0 || value.party.members.length > 6) return undefined;
    const members: NonNullable<PlayerStateSnapshot['party']>['members'] = [];
    for (const entry of value.party.members) {
      if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'name', 'leader', 'online'])) return undefined;
      const id = opaqueId(entry.id);
      const memberName = boundedString(entry.name, 64);
      if (!id || !memberName || typeof entry.leader !== 'boolean' || typeof entry.online !== 'boolean') {
        return undefined;
      }
      members.push({ id, name: memberName, leader: entry.leader, online: entry.online });
    }
    const actions = parseActions(value.party.actions);
    if (!actions || members.filter((member) => member.leader).length !== 1) return undefined;
    party = { leader: value.party.leader, members, actions };
  }

  if (!Array.isArray(value.partyInvites) || value.partyInvites.length > 12) return undefined;
  const partyInvites: PlayerStateSnapshot['partyInvites'] = [];
  for (const entry of value.partyInvites) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['leaderName', 'expiresInTicks', 'actions'])) return undefined;
    const leaderName = boundedString(entry.leaderName, 64);
    const expiresInTicks = nonNegativeIntegerOrUndefined(entry.expiresInTicks);
    const actions = parseActions(entry.actions);
    if (!leaderName || expiresInTicks === undefined || !actions) return undefined;
    partyInvites.push({ leaderName, expiresInTicks, actions });
  }

  const economy = value.economy;
  if (
    !isRecord(economy) || !hasOnlyKeys(economy, ['coins', 'commodities', 'shopAvailable', 'shop']) ||
    nonNegativeIntegerOrUndefined(economy.coins) === undefined ||
    typeof economy.shopAvailable !== 'boolean' ||
    !Array.isArray(economy.commodities) || economy.commodities.length > 32 ||
    !Array.isArray(economy.shop) || economy.shop.length > 32
  ) return undefined;
  const commodities: PlayerStateSnapshot['economy']['commodities'] = [];
  for (const entry of economy.commodities) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'label', 'quantity'])) return undefined;
    const id = contentId(entry.id);
    const label = boundedString(entry.label, 128);
    const quantity = positiveIntegerOrUndefined(entry.quantity);
    if (!id || !label || quantity === undefined || quantity > 99) return undefined;
    commodities.push({ id, label, quantity });
  }
  const shop: PlayerStateSnapshot['economy']['shop'] = [];
  for (const entry of economy.shop) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'label', 'buyPrice', 'sellPrice', 'stock', 'owned', 'actions'])) {
      return undefined;
    }
    const id = contentId(entry.id);
    const label = boundedString(entry.label, 128);
    const buyPrice = positiveIntegerOrUndefined(entry.buyPrice);
    const sellPrice = positiveIntegerOrUndefined(entry.sellPrice);
    const stock = nonNegativeIntegerOrUndefined(entry.stock);
    const owned = nonNegativeIntegerOrUndefined(entry.owned);
    const actions = parseActions(entry.actions);
    if (!id || !label || buyPrice === undefined || sellPrice === undefined || stock === undefined ||
      stock > 100 || owned === undefined || owned > 99 || !actions) return undefined;
    shop.push({ id, label, buyPrice, sellPrice, stock, owned, actions });
  }
  if (!economy.shopAvailable && shop.length > 0) return undefined;

  const social = value.social;
  if (!isRecord(social) || !hasOnlyKeys(social, ['online']) || !Array.isArray(social.online) || social.online.length > 256) {
    return undefined;
  }
  const online: PlayerStateSnapshot['social']['online'] = [];
  for (const entry of social.online) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['id', 'name', 'selfCharacter'])) return undefined;
    const id = opaqueId(entry.id);
    const onlineName = boundedString(entry.name, 64);
    if (!id || !onlineName || typeof entry.selfCharacter !== 'boolean') return undefined;
    online.push({ id, name: onlineName, selfCharacter: entry.selfCharacter });
  }

  return {
    revision,
    character: {
      name, level, experience, nextLevelExperience, health, maxHealth, strength, defence, skills, effects,
      ...(recovery === undefined ? {} : { recoveryInTicks: recovery }),
    },
    inventory,
    equipment,
    map: { currentRoomId, rooms, paths },
    quests,
    ...(party ? { party } : {}),
    partyInvites,
    economy: {
      coins: economy.coins as number,
      commodities,
      shopAvailable: economy.shopAvailable,
      shop,
    },
    social: { online },
  };
}

const PLAYER_VALIDATION_SHELL = {
  revision: 1,
  character: {
    name: 'validation', level: 1, experience: 0, nextLevelExperience: 100,
    health: 1, maxHealth: 1, strength: 1, defence: 0, effects: [], recoveryInTicks: null,
  },
  inventory: [],
  equipment: [],
  map: {
    currentRoomId: 'o0000000000000000',
    rooms: [{ id: 'o0000000000000000', title: 'validation', current: true }],
    paths: [],
  },
  quests: [],
  partyInvites: [],
  economy: { coins: 0, commodities: [], shopAvailable: false, shop: [] },
  social: { online: [] },
} as const;

const validatePlayerFragment = (
  revision: number,
  fragment: Partial<Record<'character' | 'inventory' | 'equipment' | 'map', unknown>>,
): PlayerStateSnapshot | undefined => parsePlayerState({
  ...PLAYER_VALIDATION_SHELL,
  revision,
  ...fragment,
});

const assetUrlOrUndefined = (value: unknown): string | undefined => {
  const url = stringOrUndefined(value);
  return url && /^\/assets\/[a-f0-9]{64}\/scene-(?:640|1280)\.(?:avif|webp|jpg|png)$/.test(url)
    ? url
    : undefined;
};

const toProse = (value: unknown): string[] => {
  if (typeof value === 'string') {
    return value.trim() ? [value] : [];
  }

  if (!Array.isArray(value)) {
    return [];
  }

  return value.flatMap((entry) => {
    if (typeof entry === 'string') {
      return entry.trim() ? [entry] : [];
    }

    if (isRecord(entry) && typeof entry.text === 'string' && entry.text.trim()) {
      return [entry.text];
    }

    return [];
  });
};

const toImage = (data: Record<string, unknown>): SceneImage | undefined => {
  const direct = isRecord(data.image) ? data.image : undefined;
  const presentation = isRecord(data.presentation) ? data.presentation : undefined;
  const backdrop = presentation && isRecord(presentation.backdrop) ? presentation.backdrop : undefined;
  const visual = isRecord(data.visual) ? data.visual : undefined;
  const source = direct ?? backdrop ?? visual;

  if (!source) {
    return undefined;
  }

  const variants = Array.isArray(source.variants) ? source.variants : [];
  const variantUrl = (formats: string[]): string | undefined => {
    const variant = variants.find(
      (candidate) =>
        isRecord(candidate) &&
        typeof candidate.format === 'string' &&
        formats.includes(candidate.format.toLowerCase()),
    );
    return isRecord(variant) ? assetUrlOrUndefined(variant.url) : undefined;
  };

  const localizedEntries = isRecord(source.localizedAlt)
    ? Object.entries(source.localizedAlt).filter(
        ([locale, text]) =>
          /^[a-z0-9]+(?:-[a-z0-9]+)*$/i.test(locale) &&
          locale.length <= 35 &&
          typeof text === 'string' &&
          text.trim().length > 0 &&
          text.length <= 512 &&
          !/[\u0000-\u001f\u007f]/.test(text),
      ) as Array<[string, string]>
    : [];
  const localizedAlt = localizedEntries.length > 0 && localizedEntries.length <= 16
    ? Object.fromEntries(localizedEntries)
    : undefined;
  const image: SceneImage = {
    alt: stringOrUndefined(source.alt) ?? stringOrUndefined(data.alt) ?? '',
    localizedAlt,
    manifest: stringOrUndefined(source.manifest),
    avif: assetUrlOrUndefined(source.avif) ?? variantUrl(['avif']),
    webp: assetUrlOrUndefined(source.webp) ?? variantUrl(['webp']),
    jpeg:
      assetUrlOrUndefined(source.jpeg) ??
      assetUrlOrUndefined(source.jpg) ??
      variantUrl(['jpeg', 'jpg']),
    png: assetUrlOrUndefined(source.png) ?? variantUrl(['png']),
    width: numberOrUndefined(source.width),
    height: numberOrUndefined(source.height),
  };

  return image.avif || image.webp || image.jpeg || image.png || image.manifest ? image : undefined;
};

const toExits = (value: unknown): SceneExit[] => {
  if (!Array.isArray(value)) {
    return [];
  }

  return value.flatMap((entry, index) => {
    if (typeof entry === 'string' && entry.trim()) {
      return [{ id: entry, label: entry, command: entry }];
    }

    if (!isRecord(entry)) {
      return [];
    }

    const label = stringOrUndefined(entry.label) ?? stringOrUndefined(entry.name);
    if (!label) {
      return [];
    }

    return [
      {
        id: stringOrUndefined(entry.id) ?? `exit-${index}`,
        label,
        command: stringOrUndefined(entry.command) ?? label,
      },
    ];
  });
};

const toActions = (value: unknown): SceneAction[] => {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    if (!isRecord(entry)) return [];
    const label = stringOrUndefined(entry.label);
    const command = stringOrUndefined(entry.command);
    return label && command ? [{ label, command }] : [];
  });
};

const toItems = (value: unknown): SceneItem[] => {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry, index) => {
    if (!isRecord(entry)) return [];
    const label = stringOrUndefined(entry.label) ?? stringOrUndefined(entry.name);
    if (!label) return [];
    const id = stringOrUndefined(entry.id) ?? `item-${index}`;
    const suppliedActions = toActions(entry.actions);
    return [{
      id,
      label,
      description: stringOrUndefined(entry.description) ?? '',
      actions: suppliedActions.length > 0
        ? suppliedActions
        : [
            { label: 'Examine', command: `examine ${id}` },
            { label: 'Take', command: `take ${id}` },
          ],
    }];
  });
};

const toNpcs = (value: unknown): SceneNpc[] => {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry, index) => {
    if (!isRecord(entry)) return [];
    const label = stringOrUndefined(entry.label) ?? stringOrUndefined(entry.name);
    if (!label) return [];
    const id = stringOrUndefined(entry.id) ?? `npc-${index}`;
    const maxHealth = Math.max(1, Math.trunc(numberOrUndefined(entry.maxHealth) ?? 1));
    const health = Math.min(maxHealth, Math.max(0, Math.trunc(numberOrUndefined(entry.health) ?? maxHealth)));
    const suppliedActions = toActions(entry.actions);
    return [{
      id,
      label,
      description: stringOrUndefined(entry.description) ?? '',
      health,
      maxHealth,
      actions: suppliedActions.length > 0
        ? suppliedActions
        : [{ label: 'Attack', command: `attack ${id}` }],
    }];
  });
};

const toWeather = (value: unknown): SceneWeather => {
  if (!isRecord(value)) {
    return { condition: 'unknown', day: 0, hour: 0, nextChangeInTicks: 0 };
  }
  return {
    condition: boundedString(value.condition, 48) ?? 'unknown',
    day: nonNegativeIntegerOrUndefined(value.day) ?? 0,
    hour: Math.min(23, nonNegativeIntegerOrUndefined(value.hour) ?? 0),
    nextChangeInTicks: nonNegativeIntegerOrUndefined(value.nextChangeInTicks) ?? 0,
  };
};

const normaliseScene = (data: Record<string, unknown>): SceneSnapshot => ({
  viewId: stringOrUndefined(data.viewId),
  revision: numberOrUndefined(data.revision),
  title: stringOrUndefined(data.title) ?? 'An unknown place',
  prose: toProse(data.prose ?? data.description),
  image: toImage(data),
  exits: toExits(data.exits),
  items: toItems(data.items),
  npcs: toNpcs(data.npcs),
  weather: toWeather(data.weather),
});

const parseScenePatch = (data: Record<string, unknown>): ScenePatchData | undefined => {
  if (!hasOnlyKeys(data, ['viewId', 'revision', 'title', 'prose', 'image', 'exits', 'weather'])) {
    return undefined;
  }
  const viewId = opaqueId(data.viewId);
  const revision = positiveIntegerOrUndefined(data.revision);
  if (!viewId || revision === undefined || Object.keys(data).length <= 2) return undefined;
  const patch: ScenePatchData = { viewId, revision };
  if (data.title !== undefined) {
    const title = boundedString(data.title, 256);
    if (!title) return undefined;
    patch.title = title;
  }
  if (data.prose !== undefined) {
    if (!Array.isArray(data.prose) || data.prose.length > 32 || data.prose.some(
      (paragraph) => typeof paragraph !== 'string' || paragraph.length > 4096,
    )) return undefined;
    patch.prose = toProse(data.prose);
  }
  if (data.image !== undefined) {
    if (data.image === null) patch.image = null;
    else {
      const image = toImage(data);
      if (!image) return undefined;
      patch.image = image;
    }
  }
  if (data.exits !== undefined) {
    if (!Array.isArray(data.exits) || data.exits.length > 32) return undefined;
    patch.exits = toExits(data.exits);
  }
  if (data.weather !== undefined) {
    if (!isRecord(data.weather)) return undefined;
    patch.weather = toWeather(data.weather);
  }
  return patch;
};

const parseEntityUpsert = (data: Record<string, unknown>): EntityUpsertData | undefined => {
  if (!hasOnlyKeys(data, ['viewId', 'revision', 'kind', 'id', 'entity']) || !isRecord(data.entity)) {
    return undefined;
  }
  const viewId = opaqueId(data.viewId);
  const revision = positiveIntegerOrUndefined(data.revision);
  const id = opaqueId(data.id);
  if (!viewId || revision === undefined || !id) return undefined;
  if (data.kind === 'item') {
    const [entity] = toItems([data.entity]);
    return entity?.id === id ? { viewId, revision, kind: 'item', id, entity } : undefined;
  }
  if (data.kind === 'npc') {
    const [entity] = toNpcs([data.entity]);
    return entity?.id === id ? { viewId, revision, kind: 'npc', id, entity } : undefined;
  }
  return undefined;
};

export function parseServerMessage(raw: string): ServerMessage | undefined {
  let candidate: unknown;

  try {
    candidate = JSON.parse(raw);
  } catch {
    return undefined;
  }

  if (
    !isRecord(candidate) ||
    !hasOnlyKeys(candidate, ['v', 'type', 'seq', 'id', 'data']) ||
    candidate.v !== PROTOCOL_VERSION ||
    typeof candidate.type !== 'string' ||
    !isRecord(candidate.data) ||
    (candidate.id !== undefined && typeof candidate.id !== 'string')
  ) {
    return undefined;
  }

  const sequence = positiveIntegerOrUndefined(candidate.seq);
  const base = {
    v: PROTOCOL_VERSION,
    seq: sequence,
    id: stringOrUndefined(candidate.id),
  };

  if (candidate.type !== 'server.heartbeat' && sequence === undefined) {
    return undefined;
  }

  switch (candidate.type) {
    case 'server.welcome': {
      if (
        (candidate.data.serverName !== undefined && typeof candidate.data.serverName !== 'string') ||
        (candidate.data.motd !== undefined && typeof candidate.data.motd !== 'string') ||
        (candidate.data.sessionId !== undefined && typeof candidate.data.sessionId !== 'string')
      ) {
        return undefined;
      }
      return {
        ...base,
        type: candidate.type,
        data: {
          serverName: stringOrUndefined(candidate.data.serverName),
          motd: stringOrUndefined(candidate.data.motd),
          sessionId: stringOrUndefined(candidate.data.sessionId),
        },
      };
    }
    case 'scene.snapshot': {
      if (typeof candidate.data.title !== 'string') {
        return undefined;
      }
      return { ...base, type: candidate.type, data: normaliseScene(candidate.data) };
    }
    case 'scene.patch': {
      const patch = parseScenePatch(candidate.data);
      return patch ? { ...base, type: candidate.type, data: patch } : undefined;
    }
    case 'entity.upsert': {
      const upsert = parseEntityUpsert(candidate.data);
      return upsert ? { ...base, type: candidate.type, data: upsert } : undefined;
    }
    case 'entity.remove': {
      if (!hasOnlyKeys(candidate.data, ['viewId', 'revision', 'kind', 'id'])) return undefined;
      const viewId = opaqueId(candidate.data.viewId);
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      const id = opaqueId(candidate.data.id);
      const kind = candidate.data.kind;
      if (!viewId || revision === undefined || !id || (kind !== 'item' && kind !== 'npc')) {
        return undefined;
      }
      return { ...base, type: candidate.type, data: { viewId, revision, kind, id } };
    }
    case 'player.snapshot': {
      const player = parsePlayerState(candidate.data);
      return player ? { ...base, type: candidate.type, data: player } : undefined;
    }
    case 'vitals.update': {
      if (!hasOnlyKeys(candidate.data, ['revision', 'character'])) return undefined;
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      if (revision === undefined) return undefined;
      const validated = validatePlayerFragment(revision, { character: candidate.data.character });
      return validated
        ? { ...base, type: candidate.type, data: { revision, character: validated.character } }
        : undefined;
    }
    case 'inventory.patch': {
      if (!hasOnlyKeys(candidate.data, ['revision', 'inventory', 'equipment'])) return undefined;
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      if (revision === undefined) return undefined;
      const validated = validatePlayerFragment(revision, {
        inventory: candidate.data.inventory,
        equipment: candidate.data.equipment,
      });
      return validated
        ? {
            ...base,
            type: candidate.type,
            data: { revision, inventory: validated.inventory, equipment: validated.equipment },
          }
        : undefined;
    }
    case 'map.snapshot': {
      if (!hasOnlyKeys(candidate.data, ['revision', 'map'])) return undefined;
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      if (revision === undefined) return undefined;
      const validated = validatePlayerFragment(revision, { map: candidate.data.map });
      return validated
        ? { ...base, type: candidate.type, data: { revision, map: validated.map } }
        : undefined;
    }
    case 'action.result': {
      const requestId = stringOrUndefined(candidate.data.requestId);
      if (
        !requestId ||
        typeof candidate.data.ok !== 'boolean' ||
        (candidate.data.message !== undefined &&
          candidate.data.message !== null &&
          typeof candidate.data.message !== 'string')
      ) {
        return undefined;
      }
      return {
        ...base,
        type: candidate.type,
        data: {
          requestId,
          ok: candidate.data.ok,
          message: stringOrUndefined(candidate.data.message),
        },
      };
    }
    case 'server.notice': {
      const message = stringOrUndefined(candidate.data.message);
      const code = stringOrUndefined(candidate.data.code);
      const level = candidate.data.level;
      if (!message || (level !== 'info' && level !== 'warning' && level !== 'error')) {
        return undefined;
      }

      return {
        ...base,
        type: candidate.type,
        data: {
          level,
          code,
          message,
        },
      };
    }
    case 'message.append': {
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      const kind = candidate.data.kind;
      const text = stringOrUndefined(candidate.data.text);
      const actorId = stringOrUndefined(candidate.data.actorId);
      const targetId = candidate.data.targetId;
      const itemId = candidate.data.itemId;
      if (
        revision === undefined ||
        (kind !== 'characterMoved' &&
          kind !== 'itemTaken' &&
          kind !== 'itemDropped' &&
          kind !== 'gameplay' &&
          kind !== 'adventure') ||
        !text ||
        !actorId ||
        (targetId !== undefined && typeof targetId !== 'string') ||
        (itemId !== undefined && typeof itemId !== 'string')
      ) {
        return undefined;
      }
      return {
        ...base,
        type: candidate.type,
        data: {
          revision,
          kind,
          text,
          actorId,
          targetId: stringOrUndefined(targetId),
          itemId: stringOrUndefined(itemId),
        },
      };
    }
    case 'sync.snapshot': {
      const afterSeq = nonNegativeIntegerOrUndefined(candidate.data.afterSeq);
      const player = parsePlayerState(candidate.data.player);
      if (
        !hasOnlyKeys(candidate.data, ['afterSeq', 'scene', 'player']) ||
        afterSeq === undefined ||
        !isRecord(candidate.data.scene) ||
        typeof candidate.data.scene.title !== 'string' ||
        !player
      ) {
        return undefined;
      }
      return {
        ...base,
        type: candidate.type,
        data: { afterSeq, scene: normaliseScene(candidate.data.scene), player },
      };
    }
    case 'media.play': {
      const channel = candidate.data.channel;
      const url = stringOrUndefined(candidate.data.url);
      if (
        !hasOnlyKeys(candidate.data, ['channel', 'url', 'loop']) ||
        (channel !== 'music' && channel !== 'ambience' && channel !== 'effects') ||
        !url ||
        !/^\/assets\/[a-f0-9]{64}\/audio-[a-z0-9-]+\.(?:opus|ogg|mp3)$/.test(url) ||
        typeof candidate.data.loop !== 'boolean'
      ) {
        return undefined;
      }
      return { ...base, type: candidate.type, data: { channel, url, loop: candidate.data.loop } };
    }
    case 'media.stop': {
      const channel = candidate.data.channel;
      if (
        !hasOnlyKeys(candidate.data, ['channel']) ||
        (channel !== undefined &&
          channel !== 'music' &&
          channel !== 'ambience' &&
          channel !== 'effects')
      ) {
        return undefined;
      }
      return { ...base, type: candidate.type, data: { channel } };
    }
    case 'chat.message': {
      const channel = candidate.data.channel;
      const sender = stringOrUndefined(candidate.data.sender);
      const text = stringOrUndefined(candidate.data.text);
      if (
        !hasOnlyKeys(candidate.data, ['channel', 'sender', 'text']) ||
        (channel !== 'say' && channel !== 'tell') ||
        !sender ||
        !text ||
        sender.length > 64 ||
        text.length > 512
      ) {
        return undefined;
      }
      return { ...base, type: candidate.type, data: { channel, sender, text } };
    }
    case 'server.heartbeat': {
      const nonce = positiveIntegerOrUndefined(candidate.data.nonce);
      const timeoutMs = positiveIntegerOrUndefined(candidate.data.timeoutMs);
      if (candidate.seq !== undefined || nonce === undefined || timeoutMs === undefined) {
        return undefined;
      }
      return {
        v: PROTOCOL_VERSION,
        type: candidate.type,
        id: base.id,
        data: { nonce, timeoutMs },
      };
    }
    default:
      return undefined;
  }
}

export function createClientHello(
  reducedMotion: boolean,
  auth: { token: string; characterId: number },
  images = true,
  audio = false,
): ClientHello {
  return {
    v: PROTOCOL_VERSION,
    type: 'client.hello',
    data: {
      client: 'web',
      capabilities: {
        images: images ? ['avif', 'webp', 'jpeg'] : [],
        reducedMotion,
        audio,
      },
      auth,
    },
  };
}

export function createCommandSubmit(command: string, requestId: string): CommandSubmit {
  return {
    v: PROTOCOL_VERSION,
    type: 'command.submit',
    data: { requestId, command },
  };
}

export function createActionPerform(
  action: string,
  target: string | undefined,
  requestId: string,
): ActionPerform {
  return {
    v: PROTOCOL_VERSION,
    type: 'action.perform',
    data: { requestId, action, ...(target === undefined ? {} : { target }) },
  };
}

export function createSyncRequest(afterSeq: number): SyncRequest {
  return {
    v: PROTOCOL_VERSION,
    type: 'sync.request',
    data: { afterSeq },
  };
}

export function createHeartbeatAck(nonce: number): ClientHeartbeat {
  return {
    v: PROTOCOL_VERSION,
    type: 'client.heartbeat',
    data: { nonce },
  };
}
