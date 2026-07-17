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

export type SceneEntityPose = 'idle' | 'attack' | 'hit' | 'defeated';

export interface SceneEntityVisualSource {
  /**
   * Optional manifest/AVIF fields remain reserved for additive negotiation.
   * The v1 server emits only validated direct WebP/PNG raster sources and
   * sheds optional annotations before the 16 KiB frame boundary.
   */
  manifest?: string;
  avif?: string;
  webp?: string;
  png?: string;
}

export interface SceneEntityVisual {
  alt: string;
  decorative: boolean;
  footprint: { width: number; height: number };
  preferredAnchor?: { x: number; y: number };
  priority: number;
  layer: 'midground' | 'foreground' | 'overlay';
  poses: Record<'idle', SceneEntityVisualSource> & Partial<
    Record<Exclude<SceneEntityPose, 'idle'>, SceneEntityVisualSource>
  >;
}

export interface SceneEntityPresentation extends SceneEntityVisual {
  id: string;
}

export interface ScenePresentation {
  entities: SceneEntityPresentation[];
}

export interface SceneItem {
  id: string;
  label: string;
  description: string;
  rarity: ItemRarity;
  itemLevel: number;
  weightGrams: number;
  quantity: number;
  stackSlots: number;
  totalWeightGrams: number;
  transferPolicy: ItemTransferPolicy;
  boundToYou: boolean;
  affix?: ItemAffixSnapshot;
  containerCapacity?: InventoryCapacitySnapshot;
  loot?: SceneLootAccess;
  actions: SceneAction[];
}

export type SceneLootAccess =
  | { access: 'yours'; expiresInTicks: number }
  | { access: 'reserved' };

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
  presentation?: ScenePresentation;
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
  kind:
    | 'characterMoved'
    | 'itemTaken'
    | 'itemDropped'
    | 'itemGiven'
    | 'tradeRequested'
    | 'tradeChanged'
    | 'tradeCompleted'
    | 'tradeCancelled'
    | 'gameplay'
    | 'adventure';
  text: string;
  actorId: string;
  targetId?: string;
  itemId?: string;
  cue?: MessagePresentationCue;
}

export interface SyncSceneData {
  syncId: string;
  afterSeq: number;
  scene: SceneSnapshot;
}

export interface SyncPlayerData {
  syncId: string;
  afterSeq: number;
  player: PlayerStateSnapshot;
  complete: true;
}

export interface SyncRecoveryData {
  syncId: string;
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

export type MessagePresentationCueKind = 'attack' | 'hit' | 'heal' | 'defeat';

export interface MessagePresentationCue {
  kind: MessagePresentationCueKind;
  entityId?: string;
  pose?: SceneEntityPose;
  intensity: 1 | 2 | 3;
  durationMs: number;
}

export interface VitalsUpdateData {
  revision: number;
  character: PlayerCharacterSnapshot;
}

export interface InventoryPatchData {
  revision: number;
  inventory: PlayerItemSnapshot[];
  equipment: EquippedItemSnapshot[];
  inventoryCapacity: InventoryCapacitySnapshot;
}

export interface TradeContentsData {
  tradeId: string;
  container: string;
  items: ContainerItemSnapshot[];
  page: number;
  totalPages: number;
  capacity: InventoryCapacitySnapshot;
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

export type CommunityChannel = 'general' | 'newbie' | 'staff';

export type SocialMessageChannel =
  | 'announcement'
  | 'say'
  | 'tell'
  | 'telepathy'
  | 'emote'
  | 'party'
  | CommunityChannel;

export type StaffReportResolution = 'dismissed' | 'action_taken' | 'no_action';

export type StaffProfile =
  | 'helper'
  | 'builder'
  | 'moderator'
  | 'administrator'
  | 'implementor';

export type StaffCapability =
  | 'reports.view'
  | 'reports.resolve'
  | 'moderation.mute'
  | 'moderation.ban'
  | 'staff.assign'
  | 'staff.inspect'
  | 'staff.where'
  | 'staff.goto'
  | 'staff.transfer'
  | 'staff.restore'
  | 'staff.announce'
  | 'staff.audit';

export interface SocialMessageData {
  channel: SocialMessageChannel;
  sender: string;
  text: string;
}

/** Retained for the v1 `chat.message` envelope name. */
export type ChatMessageData = SocialMessageData;

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
  lineage: {
    id: string;
    label: string;
    description: string;
  };
  background?: {
    id: string;
    label: string;
    description: string;
  };
  baseClass: {
    id: string;
    label: string;
    description: string;
  };
  activeClass: {
    id: string;
    label: string;
    description: string;
  };
  classHistory: Array<{
    id: string;
    label: string;
    description: string;
  }>;
  attributes: Array<{
    id: string;
    label: string;
    value: number;
  }>;
  resistances: Array<{
    damageType: 'physical' | 'arcane' | 'fire' | 'frost' | 'lightning';
    percent: number;
  }>;
  statuses: Array<{
    id: string;
    label: string;
    kind: 'attribute_modifier' | 'regeneration' | 'damage_over_time' | 'stun';
    magnitude: number;
    remainingTicks: number;
  }>;
  resources: Array<{
    id: string;
    label: string;
    current: number;
    max: number;
  }>;
  abilities: Array<{
    id: string;
    name: string;
    description: string;
    kind: 'martial' | 'spell' | 'utility';
    resourceCosts: Array<{
      resourceId: string;
      label: string;
      amount: number;
    }>;
    cooldownRemaining: number;
    usable: boolean;
    purpose: 'gameplay' | 'telepathy';
    targeting: 'self' | 'optionalTarget' | 'explicitTarget';
  }>;
  progressionPoints: Record<string, number>;
  talents: Array<{
    id: string;
    label: string;
    description: string;
    pointPoolId: string;
    availablePoints: number;
    nodes: Array<{
      id: string;
      label: string;
      description: string;
      rank: number;
      maxRank: number;
      nextCost?: number;
      available: boolean;
      requirements: Array<{ met: boolean; text: string }>;
      command: string;
    }>;
  }>;
  promotions: Array<{
    class: {
      id: string;
      label: string;
      description: string;
    };
    available: boolean;
    requirements: Array<{ met: boolean; text: string }>;
    command: string;
  }>;
}

export interface PlayerItemSnapshot {
  id: string;
  label: string;
  description: string;
  rarity: ItemRarity;
  itemLevel: number;
  weightGrams: number;
  quantity: number;
  availableQuantity: number;
  reservedForTrade: number;
  stackSlots: number;
  totalWeightGrams: number;
  transferPolicy: ItemTransferPolicy;
  boundToYou: boolean;
  affix?: ItemAffixSnapshot;
  canGive: boolean;
  canTrade: boolean;
  containerCapacity?: InventoryCapacitySnapshot;
  actions: SceneAction[];
}

export interface EquippedItemSnapshot {
  id: string;
  slot: 'weapon' | 'body' | 'trinket';
  label: string;
  rarity: ItemRarity;
  itemLevel: number;
  weightGrams: number;
  quantity: number;
  stackSlots: number;
  totalWeightGrams: number;
  transferPolicy: ItemTransferPolicy;
  boundToYou: boolean;
  affix?: ItemAffixSnapshot;
  containerCapacity?: InventoryCapacitySnapshot;
  actions: SceneAction[];
}

export type ItemRarity = 'common' | 'uncommon' | 'rare' | 'epic' | 'legendary';
export type ItemTransferPolicy = 'transferable' | 'character_bound' | 'non_transferable';

export interface ItemAffixSnapshot {
  label: string;
  position: 'prefix' | 'suffix';
  attackBonusDelta: number;
  healAmountDelta: number;
}

export interface InventoryCapacitySnapshot {
  usedSlots: number;
  maxSlots: number;
  usedWeightGrams: number;
  maxWeightGrams: number;
}

export type ContainerItemSnapshot = Omit<PlayerItemSnapshot, 'description'>;

export interface ContainerContentsData {
  container: string;
  items: ContainerItemSnapshot[];
  page: number;
  totalPages: number;
  capacity: InventoryCapacitySnapshot;
}

export interface RewardOddsGuaranteedRow {
  label: string;
  quantity: number;
  affixPoolLabel?: string;
}

export interface RewardOddsWeightedRow {
  poolLabel: string;
  rolls: number;
  outcomeLabel: string;
  minQuantity: number;
  maxQuantity: number;
  probabilityNumerator: number;
  probabilityDenominator: number;
  affixPoolLabel?: string;
}

export interface RewardOddsAffixRow {
  parentOutcomeLabel: string;
  affixLabel: string;
  position?: 'prefix' | 'suffix';
  attackBonusDelta: number;
  healAmountDelta: number;
  conditionalProbabilityNumerator: number;
  conditionalProbabilityDenominator: number;
}

export interface RewardOddsData {
  selector: string;
  page: number;
  totalPages: number;
  guaranteed: RewardOddsGuaranteedRow[];
  weighted: RewardOddsWeightedRow[];
  affixes: RewardOddsAffixRow[];
}

export interface PlayerStateSnapshot {
  revision: number;
  character: PlayerCharacterSnapshot;
  inventory: PlayerItemSnapshot[];
  equipment: EquippedItemSnapshot[];
  inventoryCapacity: InventoryCapacitySnapshot;
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
    reservedCoins: number;
    spendableCoins: number;
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
    online: Array<{ id: string; name: string; selfCharacter: boolean; nearby: boolean }>;
  };
  trade?: TradeSnapshot;
}

export type TradePhase = 'requested' | 'negotiating';

export interface TradeOfferLineSnapshot {
  target: string;
  label: string;
  quantity: number;
  container: boolean;
}

export interface TradeOfferSnapshot {
  coins: number;
  lines: TradeOfferLineSnapshot[];
}

export interface TradeSnapshot {
  id: string;
  phase: TradePhase;
  counterparty: string;
  expiresInTicks: number;
  stateRevision: number;
  ownConfirmed: boolean;
  counterpartyConfirmed: boolean;
  ownOffer: TradeOfferSnapshot;
  counterpartyOffer: TradeOfferSnapshot;
  canAccept: boolean;
  canConfirm: boolean;
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
  | Envelope<'container.contents', ContainerContentsData>
  | Envelope<'trade.contents', TradeContentsData>
  | Envelope<'reward.odds', RewardOddsData>
  | Envelope<'map.snapshot', MapSnapshotData>
  | Envelope<'action.result', ActionResultData>
  | Envelope<'server.notice', ServerNoticeData>
  | Envelope<'message.append', MessageAppendData>
  | Envelope<'sync.scene', SyncSceneData>
  | Envelope<'sync.player', SyncPlayerData>
  | Envelope<'server.heartbeat', ServerHeartbeatData>
  | Envelope<'media.play', MediaPlayData>
  | Envelope<'media.stop', MediaStopData>
  | Envelope<'chat.message', ChatMessageData>;

export type LiveServerMessage = Exclude<
  ServerMessage,
  Envelope<'sync.scene', SyncSceneData> | Envelope<'sync.player', SyncPlayerData>
>;

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

export interface ActionPerformFields {
  abilityId?: string;
  emote?: string;
  target?: string;
  recipient?: string;
  container?: string;
  quantity?: number;
  page?: number;
  tradeId?: string;
  coins?: number;
  stateRevision?: number;
  message?: string;
  channel?: CommunityChannel;
  reason?: string;
  reportId?: number;
  resolution?: StaffReportResolution;
  profile?: StaffProfile | null;
  loreTitle?: string | null;
  capability?: StaffCapability;
  enabled?: boolean;
}

export type ActionPerform = Envelope<
  'action.perform',
  ActionPerformFields & {
    requestId: string;
    action: string;
    classId?: string;
    graphId?: string;
    nodeId?: string;
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

const signedI32OrUndefined = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isSafeInteger(value) &&
    value >= -0x8000_0000 && value <= 0x7fff_ffff
    ? value
    : undefined;

const itemRarityOrUndefined = (value: unknown): ItemRarity | undefined =>
  value === 'common' || value === 'uncommon' || value === 'rare' || value === 'epic' ||
    value === 'legendary'
    ? value
    : undefined;

const itemTransferPolicyOrUndefined = (value: unknown): ItemTransferPolicy | undefined =>
  value === 'transferable' || value === 'character_bound' || value === 'non_transferable'
    ? value
    : undefined;

const parseItemTransfer = (
  value: Record<string, unknown>,
): Pick<PlayerItemSnapshot, 'transferPolicy' | 'boundToYou'> | undefined => {
  const transferPolicy = itemTransferPolicyOrUndefined(value.transferPolicy);
  if (!transferPolicy || typeof value.boundToYou !== 'boolean' ||
    (transferPolicy !== 'character_bound' && value.boundToYou)) return undefined;
  return { transferPolicy, boundToYou: value.boundToYou };
};

const parseItemAffix = (value: unknown): ItemAffixSnapshot | undefined => {
  if (!isRecord(value) || !hasOnlyKeys(value, [
    'label', 'position', 'attackBonusDelta', 'healAmountDelta',
  ])) return undefined;
  const label = boundedString(value.label, 256);
  const position = value.position;
  const attackBonusDelta = signedI32OrUndefined(value.attackBonusDelta);
  const healAmountDelta = signedI32OrUndefined(value.healAmountDelta);
  if (!label || (position !== 'prefix' && position !== 'suffix') ||
    attackBonusDelta === undefined || healAmountDelta === undefined ||
    (attackBonusDelta === 0 && healAmountDelta === 0)) return undefined;
  return { label, position, attackBonusDelta, healAmountDelta };
};

const parseInventoryCapacity = (value: unknown): InventoryCapacitySnapshot | undefined => {
  if (!isRecord(value)) return undefined;
  const usedSlots = nonNegativeIntegerOrUndefined(value.usedSlots);
  const maxSlots = positiveIntegerOrUndefined(value.maxSlots);
  const usedWeightGrams = nonNegativeIntegerOrUndefined(value.usedWeightGrams);
  const maxWeightGrams = nonNegativeIntegerOrUndefined(value.maxWeightGrams);
  if (usedSlots === undefined || maxSlots === undefined || maxSlots > 1_000 || usedSlots > maxSlots ||
    usedWeightGrams === undefined || maxWeightGrams === undefined ||
    maxWeightGrams > 1_000_000 || usedWeightGrams > maxWeightGrams) {
    return undefined;
  }
  return { usedSlots, maxSlots, usedWeightGrams, maxWeightGrams };
};

interface ItemStackProjection {
  quantity: number;
  stackSlots: number;
  totalWeightGrams: number;
  containerCapacity?: InventoryCapacitySnapshot;
}

interface ItemTradeProjection {
  availableQuantity: number;
  reservedForTrade: number;
  canTrade: boolean;
}

const parseItemTradeProjection = (
  value: Record<string, unknown>,
  quantity: number,
  transferPolicy: ItemTransferPolicy,
): ItemTradeProjection | undefined => {
  const availableQuantity = nonNegativeIntegerOrUndefined(value.availableQuantity);
  const reservedForTrade = nonNegativeIntegerOrUndefined(value.reservedForTrade);
  const canTrade = typeof value.canTrade === 'boolean' ? value.canTrade : undefined;
  if (availableQuantity === undefined || reservedForTrade === undefined || canTrade === undefined ||
    availableQuantity + reservedForTrade !== quantity ||
    (canTrade && (availableQuantity === 0 || transferPolicy !== 'transferable'))) return undefined;
  return { availableQuantity, reservedForTrade, canTrade };
};

const parseItemStackProjection = (value: Record<string, unknown>): ItemStackProjection | undefined => {
  const quantity = positiveIntegerOrUndefined(value.quantity);
  const stackSlots = positiveIntegerOrUndefined(value.stackSlots);
  const totalWeightGrams = nonNegativeIntegerOrUndefined(value.totalWeightGrams);
  const containerCapacity = value.containerCapacity === undefined
    ? undefined
    : parseInventoryCapacity(value.containerCapacity);
  if (quantity === undefined || quantity > 1_000 || stackSlots === undefined ||
    stackSlots > 0xffff_ffff || totalWeightGrams === undefined ||
    totalWeightGrams > 65_536_000_000 ||
    (value.containerCapacity !== undefined && !containerCapacity)) return undefined;
  return {
    quantity,
    stackSlots,
    totalWeightGrams,
    ...(containerCapacity ? { containerCapacity } : {}),
  };
};

const boundedString = (value: unknown, maximum: number): string | undefined => {
  const text = stringOrUndefined(value);
  return text && text.length <= maximum ? text : undefined;
};

const boundedPossiblyEmptyString = (value: unknown, maximum: number): string | undefined =>
  typeof value === 'string' && value.length <= maximum ? value : undefined;

const hasOnlyKeys = (value: Record<string, unknown>, allowed: readonly string[]): boolean => {
  const keys = new Set(allowed);
  return Object.keys(value).every((key) => keys.has(key));
};

const parseSceneLootAccess = (value: unknown): SceneLootAccess | undefined => {
  if (!isRecord(value)) return undefined;
  if (value.access === 'reserved') {
    return hasOnlyKeys(value, ['access']) ? { access: 'reserved' } : undefined;
  }
  if (value.access !== 'yours' || !hasOnlyKeys(value, ['access', 'expiresInTicks'])) {
    return undefined;
  }
  const expiresInTicks = positiveIntegerOrUndefined(value.expiresInTicks);
  return expiresInTicks !== undefined && expiresInTicks <= 20
    ? { access: 'yours', expiresInTicks }
    : undefined;
};

const opaqueId = (value: unknown): string | undefined => {
  const id = stringOrUndefined(value);
  return id && /^o[0-9a-f]{16}$/.test(id) ? id : undefined;
};

const contentId = (value: unknown): string | undefined => {
  const id = stringOrUndefined(value);
  return id && /^[a-z0-9][a-z0-9-]{0,63}$/.test(id) ? id : undefined;
};

const definitionId = (value: unknown): string | undefined => {
  const id = stringOrUndefined(value);
  return id && /^[a-z0-9](?:[a-z0-9_.:-]{0,126}[a-z0-9])?$/.test(id) ? id : undefined;
};

const statId = (value: unknown): string | undefined => {
  const id = stringOrUndefined(value);
  return id && /^[a-z0-9](?:[a-z0-9_.:-]{0,62}[a-z0-9])?$/.test(id) ? id : undefined;
};

const parseActions = (value: unknown): SceneAction[] | undefined => {
  if (!Array.isArray(value) || value.length > 8) return undefined;
  const actions: SceneAction[] = [];
  for (const entry of value) {
    if (!isRecord(entry)) return undefined;
    const label = boundedString(entry.label, 128);
    const command = boundedString(entry.command, 1024);
    if (!label || !command) return undefined;
    actions.push({ label, command });
  }
  return actions;
};

const parseContainerContents = (value: unknown): ContainerContentsData | undefined => {
  if (!isRecord(value) || !hasOnlyKeys(value, [
    'container', 'items', 'page', 'totalPages', 'capacity',
  ]) || !Array.isArray(value.items) || value.items.length > 20 ||
    !isRecord(value.capacity) || !hasOnlyKeys(value.capacity, [
      'usedSlots', 'maxSlots', 'usedWeightGrams', 'maxWeightGrams',
    ])) return undefined;
  const container = opaqueId(value.container);
  const page = positiveIntegerOrUndefined(value.page);
  const totalPages = positiveIntegerOrUndefined(value.totalPages);
  const capacity = parseInventoryCapacity(value.capacity);
  if (!container || page === undefined || totalPages === undefined || page > totalPages ||
    totalPages > 0xffff || !capacity) return undefined;
  const ids = new Set<string>();
  const items: ContainerItemSnapshot[] = [];
  for (const entry of value.items) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, [
      'id', 'label', 'rarity', 'itemLevel', 'weightGrams', 'quantity', 'availableQuantity',
      'reservedForTrade', 'stackSlots', 'totalWeightGrams', 'containerCapacity', 'transferPolicy',
      'boundToYou', 'affix', 'canGive', 'canTrade', 'actions',
    ]) || (entry.containerCapacity !== undefined &&
      (!isRecord(entry.containerCapacity) || !hasOnlyKeys(entry.containerCapacity, [
        'usedSlots', 'maxSlots', 'usedWeightGrams', 'maxWeightGrams',
      ]))) || (Array.isArray(entry.actions) && entry.actions.some((action) =>
      !isRecord(action) || !hasOnlyKeys(action, ['label', 'command'])))) return undefined;
    const id = opaqueId(entry.id);
    const label = boundedString(entry.label, 256);
    const rarity = itemRarityOrUndefined(entry.rarity);
    const itemLevel = positiveIntegerOrUndefined(entry.itemLevel);
    const weightGrams = nonNegativeIntegerOrUndefined(entry.weightGrams);
    const stack = parseItemStackProjection(entry);
    const transfer = parseItemTransfer(entry);
    const affix = entry.affix === undefined ? undefined : parseItemAffix(entry.affix);
    const canGive = typeof entry.canGive === 'boolean' ? entry.canGive : undefined;
    const canTrade = typeof entry.canTrade === 'boolean' ? entry.canTrade : undefined;
    const actions = parseActions(entry.actions);
    if (!id || ids.has(id) || !label || !rarity || itemLevel === undefined || itemLevel > 1_000 ||
      weightGrams === undefined || weightGrams > 1_000_000 || !stack || !transfer ||
      (entry.affix !== undefined && !affix) ||
      canGive !== false ||
      (stack.containerCapacity
        ? stack.quantity !== 1 || stack.totalWeightGrams !==
          weightGrams + stack.containerCapacity.usedWeightGrams
        : stack.totalWeightGrams !== weightGrams * stack.quantity) || !actions) return undefined;
    const trade = parseItemTradeProjection(entry, stack.quantity, transfer.transferPolicy);
    if (!trade) return undefined;
    ids.add(id);
    items.push({
      id, label, rarity, itemLevel, weightGrams, ...stack, ...trade, ...transfer,
      ...(affix ? { affix } : {}), canGive, actions,
    });
  }
  return { container, items, page, totalPages, capacity };
};

const parseTradeContents = (value: unknown): TradeContentsData | undefined => {
  if (!isRecord(value) || !hasOnlyKeys(value, [
    'tradeId', 'container', 'items', 'page', 'totalPages', 'capacity',
  ])) return undefined;
  const tradeId = opaqueId(value.tradeId);
  const contents = parseContainerContents({
    container: value.container,
    items: value.items,
    page: value.page,
    totalPages: value.totalPages,
    capacity: value.capacity,
  });
  return tradeId && contents ? { tradeId, ...contents } : undefined;
};

const parseRewardOdds = (value: unknown): RewardOddsData | undefined => {
  if (!isRecord(value) || !hasOnlyKeys(value, [
    'selector', 'page', 'totalPages', 'guaranteed', 'weighted', 'affixes',
  ]) || !Array.isArray(value.guaranteed) || !Array.isArray(value.weighted) ||
    !Array.isArray(value.affixes) ||
    value.guaranteed.length + value.weighted.length + value.affixes.length > 20) {
    return undefined;
  }
  const selector = opaqueId(value.selector);
  const page = positiveIntegerOrUndefined(value.page);
  const totalPages = positiveIntegerOrUndefined(value.totalPages);
  if (!selector || page === undefined || totalPages === undefined || page > totalPages ||
    totalPages > 0xffff) return undefined;

  const guaranteed: RewardOddsGuaranteedRow[] = [];
  for (const entry of value.guaranteed) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, ['label', 'quantity', 'affixPoolLabel'])) {
      return undefined;
    }
    const label = boundedString(entry.label, 256);
    const quantity = positiveIntegerOrUndefined(entry.quantity);
    const affixPoolLabel = entry.affixPoolLabel === undefined
      ? undefined
      : boundedString(entry.affixPoolLabel, 256);
    if (!label || quantity === undefined || quantity > 1_000 ||
      (entry.affixPoolLabel !== undefined && !affixPoolLabel)) return undefined;
    guaranteed.push({ label, quantity, ...(affixPoolLabel ? { affixPoolLabel } : {}) });
  }

  const weighted: RewardOddsWeightedRow[] = [];
  for (const entry of value.weighted) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, [
      'poolLabel', 'rolls', 'outcomeLabel', 'minQuantity', 'maxQuantity',
      'probabilityNumerator', 'probabilityDenominator', 'affixPoolLabel',
    ])) return undefined;
    const poolLabel = boundedString(entry.poolLabel, 256);
    const rolls = positiveIntegerOrUndefined(entry.rolls);
    const outcomeLabel = boundedString(entry.outcomeLabel, 256);
    const minQuantity = nonNegativeIntegerOrUndefined(entry.minQuantity);
    const maxQuantity = nonNegativeIntegerOrUndefined(entry.maxQuantity);
    const probabilityNumerator = nonNegativeIntegerOrUndefined(entry.probabilityNumerator);
    const probabilityDenominator = positiveIntegerOrUndefined(entry.probabilityDenominator);
    const affixPoolLabel = entry.affixPoolLabel === undefined
      ? undefined
      : boundedString(entry.affixPoolLabel, 256);
    if (!poolLabel || rolls === undefined || rolls > 8 || !outcomeLabel ||
      minQuantity === undefined || maxQuantity === undefined || minQuantity > maxQuantity ||
      maxQuantity > 1_000 || probabilityNumerator === undefined ||
      probabilityDenominator === undefined || probabilityNumerator > probabilityDenominator ||
      (entry.affixPoolLabel !== undefined && !affixPoolLabel)) return undefined;
    weighted.push({
      poolLabel,
      rolls,
      outcomeLabel,
      minQuantity,
      maxQuantity,
      probabilityNumerator,
      probabilityDenominator,
      ...(affixPoolLabel ? { affixPoolLabel } : {}),
    });
  }

  const affixes: RewardOddsAffixRow[] = [];
  for (const entry of value.affixes) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, [
      'parentOutcomeLabel', 'affixLabel', 'position', 'attackBonusDelta', 'healAmountDelta',
      'conditionalProbabilityNumerator', 'conditionalProbabilityDenominator',
    ])) return undefined;
    const parentOutcomeLabel = boundedString(entry.parentOutcomeLabel, 256);
    const affixLabel = boundedString(entry.affixLabel, 256);
    const position = entry.position;
    const attackBonusDelta = signedI32OrUndefined(entry.attackBonusDelta);
    const healAmountDelta = signedI32OrUndefined(entry.healAmountDelta);
    const conditionalProbabilityNumerator = nonNegativeIntegerOrUndefined(
      entry.conditionalProbabilityNumerator,
    );
    const conditionalProbabilityDenominator = positiveIntegerOrUndefined(
      entry.conditionalProbabilityDenominator,
    );
    if (!parentOutcomeLabel || !affixLabel ||
      (position !== undefined && position !== 'prefix' && position !== 'suffix') ||
      attackBonusDelta === undefined || healAmountDelta === undefined ||
      (position === undefined && (attackBonusDelta !== 0 || healAmountDelta !== 0)) ||
      conditionalProbabilityNumerator === undefined ||
      conditionalProbabilityDenominator === undefined ||
      conditionalProbabilityNumerator > conditionalProbabilityDenominator) return undefined;
    affixes.push({
      parentOutcomeLabel,
      affixLabel,
      ...(position ? { position } : {}),
      attackBonusDelta,
      healAmountDelta,
      conditionalProbabilityNumerator,
      conditionalProbabilityDenominator,
    });
  }
  return { selector, page, totalPages, guaranteed, weighted, affixes };
};

const parseTradeOffer = (value: unknown): TradeOfferSnapshot | undefined => {
  if (!isRecord(value) || !hasOnlyKeys(value, ['coins', 'lines']) ||
    !Array.isArray(value.lines) || value.lines.length > 8) return undefined;
  const coins = nonNegativeIntegerOrUndefined(value.coins);
  if (coins === undefined || coins > 1_000_000_000) return undefined;
  const targets = new Set<string>();
  let roots = 0;
  const lines: TradeOfferLineSnapshot[] = [];
  for (const entry of value.lines) {
    if (!isRecord(entry) || !hasOnlyKeys(entry, [
      'target', 'label', 'quantity', 'container',
    ])) return undefined;
    const target = opaqueId(entry.target);
    const label = boundedString(entry.label, 256);
    const quantity = positiveIntegerOrUndefined(entry.quantity);
    if (!target || targets.has(target) || !label || quantity === undefined || quantity > 128 ||
      typeof entry.container !== 'boolean' || (entry.container && quantity !== 1)) {
      return undefined;
    }
    targets.add(target);
    roots += quantity;
    if (roots > 128) return undefined;
    lines.push({ target, label, quantity, container: entry.container });
  }
  return { coins, lines };
};

const parseTrade = (value: unknown): TradeSnapshot | undefined => {
  if (!isRecord(value) || !hasOnlyKeys(value, [
    'id', 'phase', 'counterparty', 'expiresInTicks', 'stateRevision', 'ownConfirmed',
    'counterpartyConfirmed', 'ownOffer', 'counterpartyOffer', 'canAccept', 'canConfirm',
  ])) return undefined;
  const id = opaqueId(value.id);
  const counterparty = opaqueId(value.counterparty);
  const phase = value.phase;
  const expiresInTicks = nonNegativeIntegerOrUndefined(value.expiresInTicks);
  const stateRevision = positiveIntegerOrUndefined(value.stateRevision);
  const ownOffer = parseTradeOffer(value.ownOffer);
  const counterpartyOffer = parseTradeOffer(value.counterpartyOffer);
  if (!id || !counterparty || (phase !== 'requested' && phase !== 'negotiating') ||
    expiresInTicks === undefined || stateRevision === undefined ||
    typeof value.ownConfirmed !== 'boolean' || typeof value.counterpartyConfirmed !== 'boolean' ||
    typeof value.canAccept !== 'boolean' || typeof value.canConfirm !== 'boolean' ||
    !ownOffer || !counterpartyOffer ||
    ownOffer.lines.reduce((sum, line) => sum + line.quantity, 0) +
      counterpartyOffer.lines.reduce((sum, line) => sum + line.quantity, 0) > 128) return undefined;
  if (phase === 'requested' && (ownOffer.lines.length > 0 || counterpartyOffer.lines.length > 0 ||
    ownOffer.coins !== 0 || counterpartyOffer.coins !== 0 || value.ownConfirmed ||
    value.counterpartyConfirmed || value.canConfirm)) return undefined;
  if (phase === 'negotiating' && value.canAccept) return undefined;
  return {
    id,
    phase,
    counterparty,
    expiresInTicks,
    stateRevision,
    ownConfirmed: value.ownConfirmed,
    counterpartyConfirmed: value.counterpartyConfirmed,
    ownOffer,
    counterpartyOffer,
    canAccept: value.canAccept,
    canConfirm: value.canConfirm,
  };
};

type ProgressionOption = PlayerCharacterSnapshot['lineage'];
type RequirementStatus = PlayerCharacterSnapshot['promotions'][number]['requirements'][number];

const parseProgressionOption = (value: unknown): ProgressionOption | undefined => {
  if (!isRecord(value)) return undefined;
  const id = definitionId(value.id);
  const label = boundedString(value.label, 128);
  const description = boundedPossiblyEmptyString(value.description, 2048);
  return id && label && description !== undefined ? { id, label, description } : undefined;
};

const parseRequirementStatuses = (value: unknown): RequirementStatus[] | undefined => {
  if (!Array.isArray(value) || value.length > 128) return undefined;
  const requirements: RequirementStatus[] = [];
  for (const entry of value) {
    if (!isRecord(entry) || typeof entry.met !== 'boolean') return undefined;
    const text = boundedString(entry.text, 512);
    if (!text) return undefined;
    requirements.push({ met: entry.met, text });
  }
  return requirements;
};

const parseCharacterProgression = (
  character: Record<string, unknown>,
): Pick<
  PlayerCharacterSnapshot,
  'lineage' | 'background' | 'baseClass' | 'activeClass' | 'classHistory' |
  'attributes' | 'resistances' | 'statuses' | 'resources' | 'abilities' |
  'progressionPoints' | 'talents' | 'promotions'
> | undefined => {
  const lineage = parseProgressionOption(character.lineage);
  const baseClass = parseProgressionOption(character.baseClass);
  const activeClass = parseProgressionOption(character.activeClass);
  if (!lineage || !baseClass || !activeClass) return undefined;

  let background: PlayerCharacterSnapshot['background'];
  if (character.background !== undefined && character.background !== null) {
    background = parseProgressionOption(character.background);
    if (!background) return undefined;
  }

  if (!Array.isArray(character.classHistory) ||
    character.classHistory.length === 0 || character.classHistory.length > 8) return undefined;
  const classHistory: PlayerCharacterSnapshot['classHistory'] = [];
  const classIds = new Set<string>();
  for (const entry of character.classHistory) {
    const option = parseProgressionOption(entry);
    if (!option || classIds.has(option.id)) return undefined;
    classIds.add(option.id);
    classHistory.push(option);
  }
  if (!classIds.has(baseClass.id) || !classIds.has(activeClass.id)) return undefined;

  const attributes: PlayerCharacterSnapshot['attributes'] = [];
  const attributeIds = new Set<string>();
  if (character.attributes !== undefined) {
    if (!Array.isArray(character.attributes) || character.attributes.length > 64) return undefined;
    for (const entry of character.attributes) {
      if (!isRecord(entry)) return undefined;
      const id = statId(entry.id);
      const label = boundedString(entry.label, 128);
      const value = numberOrUndefined(entry.value);
      if (!id || attributeIds.has(id) || !label || value === undefined || !Number.isSafeInteger(value) ||
        value < -2_147_483_648 || value > 2_147_483_647) return undefined;
      attributeIds.add(id);
      attributes.push({ id, label, value });
    }
  }

  const resistances: PlayerCharacterSnapshot['resistances'] = [];
  const resistanceTypes = new Set<string>();
  if (character.resistances !== undefined) {
    if (!Array.isArray(character.resistances) || character.resistances.length > 16) return undefined;
    for (const entry of character.resistances) {
      if (!isRecord(entry)) return undefined;
      const damageType = definitionId(entry.damageType);
      const percent = numberOrUndefined(entry.percent);
      if (resistanceTypes.has(damageType ?? '') ||
        (damageType !== 'physical' && damageType !== 'arcane' && damageType !== 'fire' &&
        damageType !== 'frost' && damageType !== 'lightning') ||
        percent === undefined || !Number.isSafeInteger(percent) || percent < 0 || percent > 90) return undefined;
      resistanceTypes.add(damageType);
      resistances.push({ damageType, percent });
    }
  }

  const statuses: PlayerCharacterSnapshot['statuses'] = [];
  if (character.statuses !== undefined) {
    if (!Array.isArray(character.statuses) || character.statuses.length > 32) return undefined;
    for (const entry of character.statuses) {
      if (!isRecord(entry)) {
        return undefined;
      }
      const id = definitionId(entry.id);
      const label = boundedString(entry.label, 128);
      const kind = entry.kind;
      const magnitude = numberOrUndefined(entry.magnitude);
      const remainingTicks = positiveIntegerOrUndefined(entry.remainingTicks);
      if (!id || !label ||
        (kind !== 'attribute_modifier' && kind !== 'regeneration' &&
          kind !== 'damage_over_time' && kind !== 'stun') ||
        magnitude === undefined || !Number.isSafeInteger(magnitude) || magnitude < -100_000 || magnitude > 100_000 ||
        remainingTicks === undefined || remainingTicks > 10_000 ||
        (kind === 'attribute_modifier' && magnitude === 0) ||
        ((kind === 'regeneration' || kind === 'damage_over_time') && magnitude <= 0) ||
        (kind === 'stun' && magnitude !== 0)) return undefined;
      statuses.push({ id, label, kind, magnitude, remainingTicks });
    }
  }

  const resources: PlayerCharacterSnapshot['resources'] = [];
  const resourceIds = new Set<string>();
  if (character.resources !== undefined) {
    if (!Array.isArray(character.resources) || character.resources.length > 16) return undefined;
    for (const entry of character.resources) {
      if (!isRecord(entry)) return undefined;
      const id = statId(entry.id);
      const label = boundedString(entry.label, 128);
      const current = nonNegativeIntegerOrUndefined(entry.current);
      const max = positiveIntegerOrUndefined(entry.max);
      if (!id || resourceIds.has(id) || !label || current === undefined || max === undefined ||
        current > max || max > 1_000_000) return undefined;
      resourceIds.add(id);
      resources.push({ id, label, current, max });
    }
  }

  const abilities: PlayerCharacterSnapshot['abilities'] = [];
  const abilityIds = new Set<string>();
  if (character.abilities !== undefined) {
    if (!Array.isArray(character.abilities) || character.abilities.length > 64) return undefined;
    for (const entry of character.abilities) {
      if (!isRecord(entry)) return undefined;
      const id = definitionId(entry.id);
      const name = boundedString(entry.name, 128);
      const description = typeof entry.description === 'string' && entry.description.length <= 2048
        ? entry.description
        : undefined;
      const kind = entry.kind;
      const cooldownRemaining = nonNegativeIntegerOrUndefined(entry.cooldownRemaining);
      const purpose = entry.purpose;
      const targeting = entry.targeting;
      const rawCosts = entry.resourceCosts;
      if (!Array.isArray(rawCosts) || rawCosts.length > 4) return undefined;
      const resourceCosts: PlayerCharacterSnapshot['abilities'][number]['resourceCosts'] = [];
      const costResourceIds = new Set<string>();
      for (const rawCost of rawCosts) {
        if (!isRecord(rawCost)) return undefined;
        const resourceId = statId(rawCost.resourceId);
        const label = boundedString(rawCost.label, 128);
        const amount = positiveIntegerOrUndefined(rawCost.amount);
        if (!resourceId || costResourceIds.has(resourceId) || !resourceIds.has(resourceId) || !label ||
          amount === undefined || amount > 1_000_000) return undefined;
        costResourceIds.add(resourceId);
        resourceCosts.push({ resourceId, label, amount });
      }
      if (
        !id || abilityIds.has(id) || !name || description === undefined ||
        (kind !== 'martial' && kind !== 'spell' && kind !== 'utility') ||
        cooldownRemaining === undefined || cooldownRemaining > 10_000 || typeof entry.usable !== 'boolean' ||
        (purpose !== 'gameplay' && purpose !== 'telepathy') ||
        (targeting !== 'self' && targeting !== 'optionalTarget' && targeting !== 'explicitTarget')
      ) return undefined;
      abilityIds.add(id);
      abilities.push({
        id, name, description, kind, resourceCosts, cooldownRemaining,
        usable: entry.usable, purpose, targeting,
      });
    }
  }

  if (!isRecord(character.progressionPoints) || Object.keys(character.progressionPoints).length > 4) {
    return undefined;
  }
  const progressionPoints: PlayerCharacterSnapshot['progressionPoints'] = {};
  for (const [poolId, rawAmount] of Object.entries(character.progressionPoints)) {
    const amount = nonNegativeIntegerOrUndefined(rawAmount);
    if (!statId(poolId) || amount === undefined || amount > 10_000) return undefined;
    progressionPoints[poolId] = amount;
  }

  if (!Array.isArray(character.talents) || character.talents.length > 16) return undefined;
  const talents: PlayerCharacterSnapshot['talents'] = [];
  const talentGraphIds = new Set<string>();
  for (const rawGraph of character.talents) {
    if (!isRecord(rawGraph) || !Array.isArray(rawGraph.nodes) || rawGraph.nodes.length > 32) return undefined;
    const id = definitionId(rawGraph.id);
    const label = boundedString(rawGraph.label, 128);
    const description = boundedPossiblyEmptyString(rawGraph.description, 2048);
    const pointPoolId = statId(rawGraph.pointPoolId);
    const availablePoints = nonNegativeIntegerOrUndefined(rawGraph.availablePoints);
    if (!id || talentGraphIds.has(id) || !label || description === undefined || !pointPoolId ||
      availablePoints === undefined || availablePoints > 10_000) return undefined;
    talentGraphIds.add(id);

    const nodes: PlayerCharacterSnapshot['talents'][number]['nodes'] = [];
    const nodeIds = new Set<string>();
    for (const rawNode of rawGraph.nodes) {
      if (!isRecord(rawNode)) return undefined;
      const nodeId = definitionId(rawNode.id);
      const nodeLabel = boundedString(rawNode.label, 128);
      const nodeDescription = boundedPossiblyEmptyString(rawNode.description, 2048);
      const rank = nonNegativeIntegerOrUndefined(rawNode.rank);
      const maxRank = positiveIntegerOrUndefined(rawNode.maxRank);
      const nextCost = rawNode.nextCost === null || rawNode.nextCost === undefined
        ? undefined
        : positiveIntegerOrUndefined(rawNode.nextCost);
      const requirements = parseRequirementStatuses(rawNode.requirements);
      const command = boundedString(rawNode.command, 1024);
      if (!nodeId || nodeIds.has(nodeId) || !nodeLabel || nodeDescription === undefined ||
        rank === undefined || maxRank === undefined || maxRank > 5 || rank > maxRank ||
        (rawNode.nextCost !== null && rawNode.nextCost !== undefined &&
          (nextCost === undefined || nextCost > 10_000)) ||
        typeof rawNode.available !== 'boolean' || !requirements || !command ||
        (rank === maxRank && nextCost !== undefined)) return undefined;
      nodeIds.add(nodeId);
      nodes.push({
        id: nodeId,
        label: nodeLabel,
        description: nodeDescription,
        rank,
        maxRank,
        ...(nextCost === undefined ? {} : { nextCost }),
        available: rawNode.available,
        requirements,
        command,
      });
    }
    talents.push({ id, label, description, pointPoolId, availablePoints, nodes });
  }

  if (!Array.isArray(character.promotions) || character.promotions.length > 64) return undefined;
  const promotions: PlayerCharacterSnapshot['promotions'] = [];
  const promotionIds = new Set<string>();
  for (const rawPromotion of character.promotions) {
    if (!isRecord(rawPromotion)) return undefined;
    const promotionClass = parseProgressionOption(rawPromotion.class);
    const requirements = parseRequirementStatuses(rawPromotion.requirements);
    const command = boundedString(rawPromotion.command, 1024);
    if (!promotionClass || promotionIds.has(promotionClass.id) ||
      typeof rawPromotion.available !== 'boolean' || !requirements || !command) return undefined;
    promotionIds.add(promotionClass.id);
    promotions.push({ class: promotionClass, available: rawPromotion.available, requirements, command });
  }

  return {
    lineage,
    ...(background ? { background } : {}),
    baseClass,
    activeClass,
    classHistory,
    attributes,
    resistances,
    statuses,
    resources,
    abilities,
    progressionPoints,
    talents,
    promotions,
  };
};

export function parsePlayerState(value: unknown): PlayerStateSnapshot | undefined {
  if (!isRecord(value)) return undefined;

  const revision = positiveIntegerOrUndefined(value.revision);
  const character = value.character;
  if (
    revision === undefined ||
    !isRecord(character)
  ) return undefined;
  const name = boundedString(character.name, 64);
  const level = positiveIntegerOrUndefined(character.level);
  const experience = nonNegativeIntegerOrUndefined(character.experience);
  const nextLevelExperience = nonNegativeIntegerOrUndefined(character.nextLevelExperience);
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
    if (!isRecord(effect)) return undefined;
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
  const skillIds = new Set<string>();
  if (character.skills !== undefined) {
    if (!Array.isArray(character.skills) || character.skills.length > 64) return undefined;
    for (const skill of character.skills) {
      if (!isRecord(skill)) return undefined;
      const id = statId(skill.id);
      const label = boundedString(skill.label, 128);
      const rank = nonNegativeIntegerOrUndefined(skill.rank);
      if (!id || skillIds.has(id) || !label || rank === undefined || rank > 1_000) return undefined;
      skillIds.add(id);
      skills.push({ id, label, rank });
    }
  }
  const progression = parseCharacterProgression(character);
  if (!progression) return undefined;

  if (!Array.isArray(value.inventory) || value.inventory.length > 256) return undefined;
  const inventory: PlayerItemSnapshot[] = [];
  for (const entry of value.inventory) {
    if (!isRecord(entry)) return undefined;
    const id = opaqueId(entry.id);
    const label = boundedString(entry.label, 256);
    const description = typeof entry.description === 'string' && entry.description.length <= 4096
      ? entry.description
      : undefined;
    const rarity = itemRarityOrUndefined(entry.rarity);
    const itemLevel = positiveIntegerOrUndefined(entry.itemLevel);
    const weightGrams = nonNegativeIntegerOrUndefined(entry.weightGrams);
    const stack = parseItemStackProjection(entry);
    const transfer = parseItemTransfer(entry);
    const affix = entry.affix === undefined ? undefined : parseItemAffix(entry.affix);
    const canGive = typeof entry.canGive === 'boolean' ? entry.canGive : undefined;
    const canTrade = typeof entry.canTrade === 'boolean' ? entry.canTrade : undefined;
    const actions = parseActions(entry.actions);
    if (!id || !label || description === undefined || !rarity || itemLevel === undefined ||
      itemLevel > 1_000 || weightGrams === undefined || weightGrams > 1_000_000 || !stack ||
      (!transfer || (transfer.transferPolicy === 'character_bound' && !transfer.boundToYou)) ||
      (entry.affix !== undefined && !affix) ||
      canGive === undefined || canTrade === undefined ||
      (canGive && transfer.transferPolicy !== 'transferable') ||
      (stack.containerCapacity
        ? stack.quantity !== 1 || stack.totalWeightGrams !==
          weightGrams + stack.containerCapacity.usedWeightGrams
        : stack.totalWeightGrams !== weightGrams * stack.quantity) || !actions) {
      return undefined;
    }
    const trade = parseItemTradeProjection(entry, stack.quantity, transfer.transferPolicy);
    if (!trade || trade.canTrade !== canTrade) return undefined;
    inventory.push({
      id, label, description, rarity, itemLevel, weightGrams, ...stack, ...trade, ...transfer,
      ...(affix ? { affix } : {}), canGive, actions,
    });
  }

  if (!Array.isArray(value.equipment) || value.equipment.length > 3) return undefined;
  const equipment: EquippedItemSnapshot[] = [];
  for (const entry of value.equipment) {
    if (!isRecord(entry)) return undefined;
    const id = opaqueId(entry.id);
    const slot = entry.slot;
    const label = boundedString(entry.label, 256);
    const rarity = itemRarityOrUndefined(entry.rarity);
    const itemLevel = positiveIntegerOrUndefined(entry.itemLevel);
    const weightGrams = nonNegativeIntegerOrUndefined(entry.weightGrams);
    const stack = parseItemStackProjection(entry);
    const transfer = parseItemTransfer(entry);
    const affix = entry.affix === undefined ? undefined : parseItemAffix(entry.affix);
    const actions = parseActions(entry.actions);
    if (!id || (slot !== 'weapon' && slot !== 'body' && slot !== 'trinket') || !label ||
      !rarity || itemLevel === undefined || itemLevel > 1_000 || weightGrams === undefined ||
      weightGrams > 1_000_000 || !stack || !transfer ||
      (transfer.transferPolicy === 'character_bound' && !transfer.boundToYou) || stack.quantity !== 1 ||
      (entry.affix !== undefined && !affix) ||
      (stack.containerCapacity
        ? stack.totalWeightGrams !== weightGrams + stack.containerCapacity.usedWeightGrams
        : stack.totalWeightGrams !== weightGrams) || !actions) {
      return undefined;
    }
    equipment.push({
      id, slot, label, rarity, itemLevel, weightGrams, ...stack, ...transfer,
      ...(affix ? { affix } : {}), actions,
    });
  }

  const inventoryCapacity = parseInventoryCapacity(value.inventoryCapacity);
  if (!inventoryCapacity) return undefined;

  const map = value.map;
  if (
    !isRecord(map) ||
    !Array.isArray(map.rooms) || map.rooms.length > 512 ||
    !Array.isArray(map.paths) || map.paths.length > 3072
  ) return undefined;
  const currentRoomId = opaqueId(map.currentRoomId);
  if (!currentRoomId) return undefined;
  const rooms: PlayerStateSnapshot['map']['rooms'] = [];
  for (const entry of map.rooms) {
    if (!isRecord(entry)) return undefined;
    const id = opaqueId(entry.id);
    const title = boundedString(entry.title, 256);
    if (!id || !title || typeof entry.current !== 'boolean') return undefined;
    rooms.push({ id, title, current: entry.current });
  }
  if (!rooms.some((room) => room.id === currentRoomId && room.current)) return undefined;
  const paths: PlayerStateSnapshot['map']['paths'] = [];
  const roomIds = new Set(rooms.map((room) => room.id));
  for (const entry of map.paths) {
    if (!isRecord(entry)) return undefined;
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
    if (!isRecord(entry)) {
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
    if (!isRecord(value.party) ||
      typeof value.party.leader !== 'boolean' || !Array.isArray(value.party.members) ||
      value.party.members.length === 0 || value.party.members.length > 6) return undefined;
    const members: NonNullable<PlayerStateSnapshot['party']>['members'] = [];
    for (const entry of value.party.members) {
      if (!isRecord(entry)) return undefined;
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
    if (!isRecord(entry)) return undefined;
    const leaderName = boundedString(entry.leaderName, 64);
    const expiresInTicks = nonNegativeIntegerOrUndefined(entry.expiresInTicks);
    const actions = parseActions(entry.actions);
    if (!leaderName || expiresInTicks === undefined || !actions) return undefined;
    partyInvites.push({ leaderName, expiresInTicks, actions });
  }

  const economy = value.economy;
  const coins = isRecord(economy) ? nonNegativeIntegerOrUndefined(economy.coins) : undefined;
  const reservedCoins = isRecord(economy)
    ? nonNegativeIntegerOrUndefined(economy.reservedCoins)
    : undefined;
  const spendableCoins = isRecord(economy)
    ? nonNegativeIntegerOrUndefined(economy.spendableCoins)
    : undefined;
  if (
    !isRecord(economy) ||
    coins === undefined || reservedCoins === undefined || spendableCoins === undefined ||
    reservedCoins > coins || spendableCoins !== coins - reservedCoins ||
    typeof economy.shopAvailable !== 'boolean' ||
    !Array.isArray(economy.commodities) || economy.commodities.length > 32 ||
    !Array.isArray(economy.shop) || economy.shop.length > 32
  ) return undefined;
  const commodities: PlayerStateSnapshot['economy']['commodities'] = [];
  for (const entry of economy.commodities) {
    if (!isRecord(entry)) return undefined;
    const id = contentId(entry.id);
    const label = boundedString(entry.label, 128);
    const quantity = positiveIntegerOrUndefined(entry.quantity);
    if (!id || !label || quantity === undefined || quantity > 99) return undefined;
    commodities.push({ id, label, quantity });
  }
  const shop: PlayerStateSnapshot['economy']['shop'] = [];
  for (const entry of economy.shop) {
    if (!isRecord(entry)) {
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
  if (!isRecord(social) || !Array.isArray(social.online) || social.online.length > 256) {
    return undefined;
  }
  const online: PlayerStateSnapshot['social']['online'] = [];
  for (const entry of social.online) {
    if (!isRecord(entry)) return undefined;
    const id = opaqueId(entry.id);
    const onlineName = boundedString(entry.name, 64);
    if (!id || !onlineName || typeof entry.selfCharacter !== 'boolean' ||
      typeof entry.nearby !== 'boolean') return undefined;
    online.push({
      id, name: onlineName, selfCharacter: entry.selfCharacter, nearby: entry.nearby,
    });
  }

  const trade = value.trade === undefined || value.trade === null
    ? undefined
    : parseTrade(value.trade);
  if (value.trade !== undefined && value.trade !== null && !trade) return undefined;

  return {
    revision,
    character: {
      name, level, experience, nextLevelExperience, health, maxHealth, strength, defence, skills, effects,
      ...(recovery === undefined ? {} : { recoveryInTicks: recovery }),
      ...progression,
    },
    inventory,
    equipment,
    inventoryCapacity,
    map: { currentRoomId, rooms, paths },
    quests,
    ...(party ? { party } : {}),
    partyInvites,
    economy: {
      coins,
      reservedCoins,
      spendableCoins,
      commodities,
      shopAvailable: economy.shopAvailable,
      shop,
    },
    social: { online },
    ...(trade ? { trade } : {}),
  };
}

const PLAYER_VALIDATION_SHELL = {
  revision: 1,
  character: {
    name: 'validation', level: 1, experience: 0, nextLevelExperience: 100,
    health: 1, maxHealth: 1, strength: 1, defence: 0, effects: [], recoveryInTicks: null,
    lineage: { id: 'mortal', label: 'Mortal', description: '' },
    background: null,
    baseClass: { id: 'wanderer', label: 'Wanderer', description: '' },
    activeClass: { id: 'wanderer', label: 'Wanderer', description: '' },
    classHistory: [{ id: 'wanderer', label: 'Wanderer', description: '' }],
    progressionPoints: {}, talents: [], promotions: [],
  },
  inventory: [],
  equipment: [],
  inventoryCapacity: {
    usedSlots: 0, maxSlots: 1, usedWeightGrams: 0, maxWeightGrams: 1,
  },
  map: {
    currentRoomId: 'o0000000000000000',
    rooms: [{ id: 'o0000000000000000', title: 'validation', current: true }],
    paths: [],
  },
  quests: [],
  partyInvites: [],
  economy: {
    coins: 0, reservedCoins: 0, spendableCoins: 0,
    commodities: [], shopAvailable: false, shop: [],
  },
  social: { online: [] },
} as const;

const validatePlayerFragment = (
  revision: number,
  fragment: Partial<Record<'character' | 'inventory' | 'equipment' | 'inventoryCapacity' | 'map', unknown>>,
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

const entityManifestUrlOrUndefined = (value: unknown): string | undefined => {
  const url = stringOrUndefined(value);
  return url && /^\/assets\/[a-f0-9]{64}\/manifest\.json$/.test(url) ? url : undefined;
};

const MAX_SCENE_PRESENTATION_ENTITIES = 12;
const MAX_SCENE_PRESENTATION_POSES = 32;
const MAX_ENTITY_SOURCE_REFERENCES = 2;

const entityRasterUrlOrUndefined = (value: unknown, pose: SceneEntityPose): string | undefined => {
  const url = stringOrUndefined(value);
  return url && new RegExp(
    `^/assets/[a-f0-9]{64}/sprite-${pose}(?:-(?:320|640|1280))?\\.(?:avif|webp|png)$`,
  ).test(url)
    ? url
    : undefined;
};

const safeVisualText = (value: unknown, maximum: number, allowEmpty = false): string | undefined => {
  if (typeof value !== 'string' || value.length > maximum || /[\u0000-\u001f\u007f]/.test(value)) {
    return undefined;
  }
  return allowEmpty || value.trim().length > 0 ? value : undefined;
};

const normalizedUnit = (value: unknown, minimum = 0, maximum = 1): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) && value >= minimum && value <= maximum
    ? value
    : undefined;

const parseEntityVisualSource = (
  value: unknown,
  pose: SceneEntityPose,
): SceneEntityVisualSource | undefined => {
  if (!isRecord(value)) return undefined;
  const source: SceneEntityVisualSource = {
    manifest: entityManifestUrlOrUndefined(value.manifest),
    avif: entityRasterUrlOrUndefined(value.avif, pose),
    webp: entityRasterUrlOrUndefined(value.webp, pose),
    png: entityRasterUrlOrUndefined(value.png, pose),
  };
  const supplied = (['manifest', 'avif', 'webp', 'png'] as const)
    .flatMap((key) => value[key] === undefined ? [] : [[key, value[key]] as const]);
  if (supplied.length === 0 || supplied.length > MAX_ENTITY_SOURCE_REFERENCES ||
    supplied.some(([key, candidate]) => source[key as keyof SceneEntityVisualSource] !== candidate)) {
    return undefined;
  }
  return source.manifest || source.avif || source.webp || source.png ? source : undefined;
};

const parseEntityVisual = (value: unknown): SceneEntityVisual | undefined => {
  if (!isRecord(value)) return undefined;
  if (typeof value.decorative !== 'boolean' || !isRecord(value.footprint) ||
    !isRecord(value.poses)) return undefined;

  const alt = safeVisualText(value.alt, 256, value.decorative);
  const width = normalizedUnit(value.footprint.width, 0.02, 0.6);
  const height = normalizedUnit(value.footprint.height, 0.02, 0.8);
  const priority = numberOrUndefined(value.priority);
  const layer = value.layer;
  if (alt === undefined || (value.decorative && alt !== '') || width === undefined || height === undefined ||
    priority === undefined || !Number.isSafeInteger(priority) || priority < -100 || priority > 100 ||
    (layer !== 'midground' && layer !== 'foreground' && layer !== 'overlay')) return undefined;

  let preferredAnchor: SceneEntityVisual['preferredAnchor'];
  if (value.preferredAnchor !== undefined) {
    if (!isRecord(value.preferredAnchor)) return undefined;
    const x = normalizedUnit(value.preferredAnchor.x);
    const y = normalizedUnit(value.preferredAnchor.y, 0.2, 1);
    if (x === undefined || y === undefined) return undefined;
    preferredAnchor = { x, y };
  }

  const idle = parseEntityVisualSource(value.poses.idle, 'idle');
  if (!idle) return undefined;
  const poses: SceneEntityVisual['poses'] = { idle };
  for (const pose of ['attack', 'hit', 'defeated'] as const) {
    if (value.poses[pose] !== undefined) {
      const source = parseEntityVisualSource(value.poses[pose], pose);
      if (!source) return undefined;
      poses[pose] = source;
    }
  }

  return {
    alt,
    decorative: value.decorative,
    footprint: { width, height },
    ...(preferredAnchor ? { preferredAnchor } : {}),
    priority,
    layer,
    poses,
  };
};

const parseScenePresentation = (value: unknown): ScenePresentation | undefined => {
  if (!isRecord(value) || !Array.isArray(value.entities) ||
    value.entities.length > MAX_SCENE_PRESENTATION_ENTITIES) return undefined;
  const entities: SceneEntityPresentation[] = [];
  const ids = new Set<string>();
  let poseCount = 0;
  for (const entry of value.entities) {
    if (!isRecord(entry)) continue;
    const id = opaqueId(entry.id);
    if (!id || ids.has(id)) continue;
    const annotation = Object.fromEntries(
      Object.entries(entry).filter(([key]) => key !== 'id'),
    );
    const visual = parseEntityVisual(annotation);
    if (!visual) continue;
    poseCount += Object.keys(visual.poses).length;
    if (poseCount > MAX_SCENE_PRESENTATION_POSES) return undefined;
    ids.add(id);
    entities.push({ id, ...visual });
  }
  return entities.length > 0 ? { entities } : undefined;
};

const parseMessagePresentationCue = (
  value: unknown,
  actorId: string,
  targetId: string | undefined,
): MessagePresentationCue | undefined => {
  if (!isRecord(value)) return undefined;
  const kind = value.kind;
  const entityId = value.entityId === undefined ? undefined : opaqueId(value.entityId);
  const pose = value.pose;
  const intensity = value.intensity;
  const durationMs = positiveIntegerOrUndefined(value.durationMs);
  if ((kind !== 'attack' && kind !== 'hit' && kind !== 'heal' && kind !== 'defeat') ||
    (value.entityId !== undefined && (!entityId || (entityId !== actorId && entityId !== targetId))) ||
    (pose !== undefined && pose !== 'idle' && pose !== 'attack' && pose !== 'hit' && pose !== 'defeated') ||
    (intensity !== 1 && intensity !== 2 && intensity !== 3) ||
    durationMs === undefined || durationMs > 800) return undefined;
  return {
    kind,
    ...(entityId ? { entityId } : {}),
    ...(pose ? { pose } : {}),
    intensity,
    durationMs,
  };
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
  return value.slice(0, 256).flatMap((entry, index) => {
    if (!isRecord(entry)) return [];
    const label = stringOrUndefined(entry.label) ?? stringOrUndefined(entry.name);
    if (!label) return [];
    const id = stringOrUndefined(entry.id) ?? `item-${index}`;
    const rarity = itemRarityOrUndefined(entry.rarity);
    const itemLevel = positiveIntegerOrUndefined(entry.itemLevel);
    const weightGrams = nonNegativeIntegerOrUndefined(entry.weightGrams);
    const stack = parseItemStackProjection(entry);
    const transfer = parseItemTransfer(entry);
    const affix = entry.affix === undefined ? undefined : parseItemAffix(entry.affix);
    const loot = entry.loot === undefined ? undefined : parseSceneLootAccess(entry.loot);
    if (!rarity || itemLevel === undefined || itemLevel > 1_000 || weightGrams === undefined ||
      weightGrams > 1_000_000 || !stack || !transfer ||
      (entry.affix !== undefined && !affix) ||
      (entry.loot !== undefined && !loot) || (stack.containerCapacity
        ? stack.quantity !== 1 || stack.totalWeightGrams !==
          weightGrams + stack.containerCapacity.usedWeightGrams
        : stack.totalWeightGrams !== weightGrams * stack.quantity)) return [];
    const suppliedActions = toActions(entry.actions);
    if (loot?.access === 'reserved' && suppliedActions.some((action) => {
      const command = action.command.trim().toLowerCase();
      return command.startsWith('take ') || command.startsWith('loot pass ');
    })) return [];
    return [{
      id,
      label,
      description: stringOrUndefined(entry.description) ?? '',
      rarity,
      itemLevel,
      weightGrams,
      ...stack,
      ...transfer,
      ...(affix ? { affix } : {}),
      ...(loot ? { loot } : {}),
      actions: suppliedActions.length > 0 || loot
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
  return value.slice(0, 256).flatMap((entry, index) => {
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
  presentation: parseScenePresentation(data.presentation),
  exits: toExits(data.exits),
  items: toItems(data.items),
  npcs: toNpcs(data.npcs),
  weather: toWeather(data.weather),
});

const parseScenePatch = (data: Record<string, unknown>): ScenePatchData | undefined => {
  const viewId = opaqueId(data.viewId);
  const revision = positiveIntegerOrUndefined(data.revision);
  const hasKnownUpdate = ['title', 'prose', 'image', 'exits', 'weather']
    .some((key) => data[key] !== undefined);
  if (!viewId || revision === undefined || !hasKnownUpdate) return undefined;
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
  if (!isRecord(data.entity)) {
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
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      if (revision === undefined) return undefined;
      const validated = validatePlayerFragment(revision, { character: candidate.data.character });
      return validated
        ? { ...base, type: candidate.type, data: { revision, character: validated.character } }
        : undefined;
    }
    case 'inventory.patch': {
      const revision = positiveIntegerOrUndefined(candidate.data.revision);
      if (revision === undefined) return undefined;
      const validated = validatePlayerFragment(revision, {
        inventory: candidate.data.inventory,
        equipment: candidate.data.equipment,
        inventoryCapacity: candidate.data.inventoryCapacity,
      });
      return validated
        ? {
            ...base,
            type: candidate.type,
            data: {
              revision,
              inventory: validated.inventory,
              equipment: validated.equipment,
              inventoryCapacity: validated.inventoryCapacity,
            },
          }
        : undefined;
    }
    case 'container.contents': {
      const contents = parseContainerContents(candidate.data);
      return contents ? { ...base, type: candidate.type, data: contents } : undefined;
    }
    case 'trade.contents': {
      const contents = parseTradeContents(candidate.data);
      return contents ? { ...base, type: candidate.type, data: contents } : undefined;
    }
    case 'reward.odds': {
      const odds = parseRewardOdds(candidate.data);
      return odds ? { ...base, type: candidate.type, data: odds } : undefined;
    }
    case 'map.snapshot': {
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
      const actorId = opaqueId(candidate.data.actorId);
      const targetId = candidate.data.targetId;
      const itemId = candidate.data.itemId;
      const normalizedTargetId = targetId === undefined ? undefined : opaqueId(targetId);
      const normalizedItemId = itemId === undefined ? undefined : opaqueId(itemId);
      if (
        revision === undefined ||
        (kind !== 'characterMoved' &&
          kind !== 'itemTaken' &&
          kind !== 'itemDropped' &&
          kind !== 'itemGiven' &&
          kind !== 'tradeRequested' &&
          kind !== 'tradeChanged' &&
          kind !== 'tradeCompleted' &&
          kind !== 'tradeCancelled' &&
          kind !== 'gameplay' &&
          kind !== 'adventure') ||
        !text ||
        !actorId ||
        (targetId !== undefined && !normalizedTargetId) ||
        (itemId !== undefined && !normalizedItemId)
      ) {
        return undefined;
      }
      const cue = parseMessagePresentationCue(candidate.data.cue, actorId, normalizedTargetId);
      return {
        ...base,
        type: candidate.type,
        data: {
          revision,
          kind,
          text,
          actorId,
          targetId: normalizedTargetId,
          itemId: normalizedItemId,
          ...(cue ? { cue } : {}),
        },
      };
    }
    case 'sync.scene': {
      const afterSeq = nonNegativeIntegerOrUndefined(candidate.data.afterSeq);
      const syncId = boundedString(candidate.data.syncId, 25);
      if (
        !syncId || !/^sync-[1-9][0-9]{0,19}$/.test(syncId) ||
        afterSeq === undefined ||
        !isRecord(candidate.data.scene) ||
        typeof candidate.data.scene.title !== 'string'
      ) {
        return undefined;
      }
      return {
        ...base,
        type: candidate.type,
        data: { syncId, afterSeq, scene: normaliseScene(candidate.data.scene) },
      };
    }
    case 'sync.player': {
      const afterSeq = nonNegativeIntegerOrUndefined(candidate.data.afterSeq);
      const syncId = boundedString(candidate.data.syncId, 25);
      const player = parsePlayerState(candidate.data.player);
      if (
        !syncId || !/^sync-[1-9][0-9]{0,19}$/.test(syncId) ||
        afterSeq === undefined || candidate.data.complete !== true || !player
      ) {
        return undefined;
      }
      return {
        ...base,
        type: candidate.type,
        data: { syncId, afterSeq, player, complete: true },
      };
    }
    case 'media.play': {
      const channel = candidate.data.channel;
      const url = stringOrUndefined(candidate.data.url);
      if (
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
        (
          channel !== 'announcement' &&
          channel !== 'say' &&
          channel !== 'tell' &&
          channel !== 'telepathy' &&
          channel !== 'emote' &&
          channel !== 'party' &&
          channel !== 'general' &&
          channel !== 'newbie' &&
          channel !== 'staff'
        ) ||
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
      auth: {
        token: auth.token,
        characterId: auth.characterId,
      },
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
  fields: ActionPerformFields | undefined,
  requestId: string,
): ActionPerform {
  if (action === 'social.telepathy') validateTelepathyActionFields(fields);
  if (action === 'social.emote' || action === 'social.emote.list') {
    validateEmoteActionFields(action, fields);
  }
  if (action.startsWith('trade.')) validateTradeActionFields(action, fields);
  if (action.startsWith('loot.')) validateLootActionFields(action, fields);
  if (action.startsWith('reward.')) validateRewardActionFields(action, fields);
  if (fields?.reportId !== undefined &&
    (!Number.isSafeInteger(fields.reportId) || fields.reportId <= 0)) {
    throw new RangeError('reportId must be a positive safe integer');
  }
  if (fields?.channel !== undefined &&
    !(['general', 'newbie', 'staff'] as const).includes(fields.channel)) {
    throw new RangeError('channel is not sendable');
  }
  if (fields?.quantity !== undefined &&
    (!Number.isSafeInteger(fields.quantity) || fields.quantity < 1 || fields.quantity > 256)) {
    throw new RangeError('quantity must be an integer between 1 and 256');
  }
  if (fields?.page !== undefined &&
    (!Number.isSafeInteger(fields.page) || fields.page < 1 || fields.page > 0xffff)) {
    throw new RangeError('page must be a positive 16-bit integer');
  }
  if (fields?.container !== undefined && fields.container.length === 0) {
    throw new RangeError('container must not be empty');
  }
  if (fields?.recipient !== undefined &&
    (fields.recipient.length === 0 || fields.recipient.length > 64 ||
      /[\u0000-\u001f\u007f]/.test(fields.recipient))) {
    throw new RangeError('recipient must be a bounded visible selector');
  }
  if (fields?.tradeId !== undefined && !/^o[0-9a-f]{16}$/.test(fields.tradeId)) {
    throw new RangeError('tradeId must be an opaque selector');
  }
  if (fields?.coins !== undefined &&
    (!Number.isSafeInteger(fields.coins) || fields.coins < 0 || fields.coins > 1_000_000_000)) {
    throw new RangeError('coins must be an integer between 0 and 1000000000');
  }
  if (fields?.stateRevision !== undefined &&
    (!Number.isSafeInteger(fields.stateRevision) || fields.stateRevision < 1)) {
    throw new RangeError('stateRevision must be a positive safe integer');
  }
  return {
    v: PROTOCOL_VERSION,
    type: 'action.perform',
    data: { requestId, action, ...fields },
  };
}

function validateTelepathyActionFields(fields: ActionPerformFields | undefined): void {
  const expected = ['abilityId', 'message', 'target'];
  const actual = Object.keys(fields ?? {}).toSorted();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new RangeError('social.telepathy requires its exact closed field set');
  }
  if (typeof fields?.abilityId !== 'string' ||
    !/^[a-z0-9](?:[a-z0-9_.:-]{0,62}[a-z0-9])?$/.test(fields.abilityId)) {
    throw new RangeError('telepathy abilityId must be a bounded definition id');
  }
  for (const [label, value, maximum] of [
    ['target', fields?.target, 64],
    ['message', fields?.message, 512],
  ] as const) {
    if (typeof value !== 'string' || value.trim() !== value || value.length === 0 ||
      new TextEncoder().encode(value).length > maximum || /[\u0000-\u001f\u007f]/.test(value)) {
      throw new RangeError(`telepathy ${label} must be bounded visible UTF-8 text`);
    }
  }
}

function validateEmoteActionFields(
  action: 'social.emote' | 'social.emote.list',
  fields: ActionPerformFields | undefined,
): void {
  const actual = Object.keys(fields ?? {}).toSorted();
  if (action === 'social.emote.list') {
    if (actual.length !== 0) {
      throw new RangeError('social.emote.list accepts no fields');
    }
    return;
  }
  const exact = actual.length === 1 && actual[0] === 'emote';
  const targeted = actual.length === 2 && actual[0] === 'emote' && actual[1] === 'target';
  if (!exact && !targeted) {
    throw new RangeError('social.emote requires emote and optional target only');
  }
  if (typeof fields?.emote !== 'string' || !/^[a-z][a-z0-9-]{0,31}$/.test(fields.emote)) {
    throw new RangeError('emote command must be bounded lowercase ASCII');
  }
  if (fields.target !== undefined && (
    fields.target.trim() !== fields.target || fields.target.length === 0 ||
    new TextEncoder().encode(fields.target).length > 64 ||
    /[\u0000-\u001f\u007f]/.test(fields.target)
  )) {
    throw new RangeError('emote target must be bounded visible UTF-8 text');
  }
}

function validateLootActionFields(
  action: string,
  fields: ActionPerformFields | undefined,
): void {
  const expectedByAction: Record<string, readonly (keyof ActionPerformFields)[]> = {
    'loot.status': [],
    'loot.pass': ['target'],
  };
  const expected = expectedByAction[action];
  if (!expected) throw new RangeError('unknown loot action');
  const actual = Object.keys(fields ?? {}).toSorted();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new RangeError(`${action} requires its exact closed field set`);
  }
  if (action === 'loot.pass' && (
    typeof fields?.target !== 'string' || fields.target.length === 0 || fields.target.length > 256 ||
    /[\u0000-\u001f\u007f]/.test(fields.target)
  )) {
    throw new RangeError('loot target must be a bounded visible selector');
  }
}

function validateRewardActionFields(
  action: string,
  fields: ActionPerformFields | undefined,
): void {
  const expectedByAction: Record<string, readonly (keyof ActionPerformFields)[]> = {
    'reward.open': ['target'],
    'reward.odds': ['page', 'target'],
  };
  const expected = expectedByAction[action];
  if (!expected) throw new RangeError('unknown reward action');
  const actual = Object.keys(fields ?? {}).toSorted();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new RangeError(`${action} requires its exact closed field set`);
  }
  if (typeof fields?.target !== 'string' || !/^o[0-9a-f]{16}$/.test(fields.target)) {
    throw new RangeError('reward target must be an opaque selector');
  }
  if (action === 'reward.odds') {
    const page = fields?.page;
    if (typeof page !== 'number' || !Number.isSafeInteger(page) || page < 1 || page > 0xffff) {
      throw new RangeError('reward odds page must be a positive 16-bit integer');
    }
  }
}

function validateTradeActionFields(
  action: string,
  fields: ActionPerformFields | undefined,
): void {
  const expectedByAction: Record<string, readonly (keyof ActionPerformFields)[]> = {
    'trade.request': ['recipient'],
    'trade.accept': ['tradeId'],
    'trade.decline': ['tradeId'],
    'trade.item.set': ['tradeId', 'target', 'quantity'],
    'trade.item.remove': ['tradeId', 'target'],
    'trade.coins.set': ['tradeId', 'coins'],
    'trade.contents': ['tradeId', 'target', 'page'],
    'trade.confirm': ['tradeId', 'stateRevision'],
    'trade.status': ['tradeId'],
    'trade.cancel': ['tradeId'],
  };
  const expected = expectedByAction[action];
  if (!expected) throw new RangeError('unknown trade action');
  const actual = Object.keys(fields ?? {}).toSorted();
  const keys = [...expected].toSorted();
  if (actual.length !== keys.length || actual.some((key, index) => key !== keys[index])) {
    throw new RangeError(`${action} requires its exact closed field set`);
  }
  if (action === 'trade.request' && !opaqueId(fields?.recipient)) {
    throw new RangeError('trade recipient must be an opaque selector');
  }
  if (action !== 'trade.request' && !opaqueId(fields?.tradeId)) {
    throw new RangeError('tradeId must be an opaque selector');
  }
  if ((action === 'trade.item.set' || action === 'trade.item.remove' ||
      action === 'trade.contents') && !opaqueId(fields?.target)) {
    throw new RangeError('trade target must be an opaque selector');
  }
  if (action === 'trade.item.set' &&
    (!Number.isSafeInteger(fields?.quantity) || (fields?.quantity ?? 0) < 1 ||
      (fields?.quantity ?? 0) > 128)) {
    throw new RangeError('trade item quantity must be an integer between 1 and 128');
  }
  if (action === 'trade.coins.set' &&
    (!Number.isSafeInteger(fields?.coins) || (fields?.coins ?? -1) < 0 ||
      (fields?.coins ?? 1_000_000_001) > 1_000_000_000)) {
    throw new RangeError('trade coins must be an integer between 0 and 1000000000');
  }
  if (action === 'trade.contents' &&
    (!Number.isSafeInteger(fields?.page) || (fields?.page ?? 0) < 1 ||
      (fields?.page ?? 0x1_0000) > 0xffff)) {
    throw new RangeError('trade contents page must be a positive 16-bit integer');
  }
  if (action === 'trade.confirm' &&
    (!Number.isSafeInteger(fields?.stateRevision) || (fields?.stateRevision ?? 0) < 1)) {
    throw new RangeError('trade stateRevision must be a positive safe integer');
  }
}

export function createAbilityPerform(
  abilityId: string,
  target: string | undefined,
  requestId: string,
): ActionPerform {
  return {
    v: PROTOCOL_VERSION,
    type: 'action.perform',
    data: {
      requestId,
      action: 'combat.ability',
      abilityId,
      ...(target === undefined ? {} : { target }),
    },
  };
}

export function createPromotionPerform(classId: string, requestId: string): ActionPerform {
  return {
    v: PROTOCOL_VERSION,
    type: 'action.perform',
    data: { requestId, action: 'progression.promote', classId },
  };
}

export function createTalentPerform(
  graphId: string,
  nodeId: string,
  requestId: string,
): ActionPerform {
  return {
    v: PROTOCOL_VERSION,
    type: 'action.perform',
    data: { requestId, action: 'progression.talent', graphId, nodeId },
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
