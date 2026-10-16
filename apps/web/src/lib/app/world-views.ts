import type {
  ActionPerformFields,
  BoardCatalogData,
  BoardEntryData,
  BoardListData,
  ContainerContentsData,
  LiveServerMessage,
  MailListData,
  MailMessageData,
  PlayerStateSnapshot,
  RewardOddsData,
  SceneSnapshot,
  TradeContentsData,
} from '../protocol';

/**
 * Authoritative world projections held by the browser session. Every field is
 * replaced, never mutated in place, so the reducer below stays a pure function
 * of the previous views and one validated server message.
 */
export interface WorldViews {
  scene?: SceneSnapshot;
  player?: PlayerStateSnapshot;
  containerContents?: ContainerContentsData;
  tradeContents?: TradeContentsData;
  rewardOdds?: RewardOddsData;
  mailList?: MailListData;
  mailMessage?: MailMessageData;
  boardCatalog?: BoardCatalogData;
  boardList?: BoardListData;
  boardEntry?: BoardEntryData;
}

/** Keys present in a patch replace the view; `undefined` clears it. */
export type WorldPatch = Partial<WorldViews>;

export interface PendingStructuredAction {
  action: string;
  fields?: ActionPerformFields;
}

/** Board mutations whose local effect waits for a correlated success. */
export const CONFIRMED_STRUCTURED_ACTIONS: readonly string[] = [
  'board.post',
  'board.delete',
  'board.report',
];

const CONTAINER_INVALIDATING_ACTIONS: readonly string[] = [
  'item.take', 'item.drop', 'item.put', 'item.get', 'item.give', 'item.contents',
  'item.use', 'item.equip', 'item.unequip', 'loot.pass',
];

/**
 * Paginated and pull-only panels. They never survive a recovery, a
 * disconnect or a session boundary because no server state backs them.
 */
export function clearedPanels(): WorldPatch {
  return {
    containerContents: undefined,
    tradeContents: undefined,
    rewardOdds: undefined,
    mailList: undefined,
    mailMessage: undefined,
    boardCatalog: undefined,
    boardList: undefined,
    boardEntry: undefined,
  };
}

/** Applies one live message to the world views and returns only the changes. */
export function reduceWorldMessage(views: Readonly<WorldViews>, message: LiveServerMessage): WorldPatch {
  const { scene, player } = views;
  switch (message.type) {
    case 'scene.snapshot':
      return { scene: message.data };
    case 'scene.patch':
      if (scene?.viewId === message.data.viewId && (scene.revision ?? 0) <= message.data.revision) {
        const { image, ...patch } = message.data;
        return {
          scene: {
            ...scene,
            ...patch,
            ...(image === null ? { image: undefined } : image ? { image } : {}),
          },
        };
      }
      return {};
    case 'entity.upsert':
      if (scene?.viewId === message.data.viewId && (scene.revision ?? 0) <= message.data.revision) {
        if (message.data.kind === 'item') {
          return {
            scene: {
              ...scene,
              revision: message.data.revision,
              items: [
                ...scene.items.filter((entity) => entity.id !== message.data.id),
                message.data.entity,
              ],
            },
          };
        }
        return {
          scene: {
            ...scene,
            revision: message.data.revision,
            npcs: [
              ...scene.npcs.filter((entity) => entity.id !== message.data.id),
              message.data.entity,
            ],
          },
        };
      }
      return {};
    case 'entity.remove':
      if (scene?.viewId === message.data.viewId && (scene.revision ?? 0) <= message.data.revision) {
        return {
          scene: message.data.kind === 'item'
            ? {
                ...scene,
                revision: message.data.revision,
                items: scene.items.filter((entity) => entity.id !== message.data.id),
              }
            : {
                ...scene,
                revision: message.data.revision,
                npcs: scene.npcs.filter((entity) => entity.id !== message.data.id),
              },
        };
      }
      return {};
    case 'player.snapshot': {
      const patch: WorldPatch = { player: message.data };
      if (
        !message.data.trade
        || views.tradeContents?.tradeId !== message.data.trade.id
        || (
          player?.trade?.stateRevision !== undefined
          && player.trade.stateRevision !== message.data.trade.stateRevision
        )
      ) {
        patch.tradeContents = undefined;
      }
      const odds = views.rewardOdds;
      if (odds && !message.data.inventory.some((item) => item.id === odds.selector)) {
        patch.rewardOdds = undefined;
      }
      return patch;
    }
    case 'vitals.update':
      if (player && player.revision <= message.data.revision) {
        return {
          player: { ...player, revision: message.data.revision, character: message.data.character },
        };
      }
      return {};
    case 'inventory.patch':
      if (player && player.revision <= message.data.revision) {
        const patch: WorldPatch = {
          player: {
            ...player,
            revision: message.data.revision,
            inventory: message.data.inventory,
            equipment: message.data.equipment,
            inventoryCapacity: message.data.inventoryCapacity,
          },
          containerContents: undefined,
          tradeContents: undefined,
        };
        const odds = views.rewardOdds;
        if (odds && !message.data.inventory.some((item) => item.id === odds.selector)) {
          patch.rewardOdds = undefined;
        }
        return patch;
      }
      return {};
    case 'container.contents':
      return { containerContents: message.data };
    case 'trade.contents':
      return { tradeContents: message.data };
    case 'reward.odds':
      return { rewardOdds: message.data };
    case 'mail.list': {
      const patch: WorldPatch = { mailList: message.data };
      const open = views.mailMessage;
      if (open && !message.data.messages.some((entry) => entry.id === open.id)) {
        patch.mailMessage = undefined;
      }
      return patch;
    }
    case 'mail.message': {
      const patch: WorldPatch = { mailMessage: message.data };
      const list = views.mailList;
      if (list) {
        patch.mailList = {
          ...list,
          unreadCount: Math.max(
            0,
            list.unreadCount - (list.messages.some(
              (entry) => entry.id === message.data.id && !entry.read,
            ) ? 1 : 0),
          ),
          messages: list.messages.map((entry) => entry.id === message.data.id
            ? { ...entry, read: true }
            : entry),
        };
      }
      return patch;
    }
    case 'board.catalog':
      return { boardCatalog: message.data };
    case 'board.list': {
      const patch: WorldPatch = { boardList: message.data };
      const entry = views.boardEntry;
      if (entry && !message.data.posts.some((post) => post.postId === entry.postId)) {
        patch.boardEntry = undefined;
      }
      return patch;
    }
    case 'board.entry': {
      const patch: WorldPatch = { boardEntry: message.data };
      const list = views.boardList;
      if (list?.board === message.data.board) {
        patch.boardList = {
          ...list,
          posts: list.posts.map((entry) => entry.postId === message.data.postId
            ? {
                postId: message.data.postId,
                board: message.data.board,
                author: message.data.author,
                subject: message.data.subject,
                createdAt: message.data.createdAt,
                expiresAt: message.data.expiresAt,
                canDelete: message.data.canDelete,
                canReport: message.data.canReport,
              }
            : entry),
        };
      }
      return patch;
    }
    case 'map.snapshot':
      if (player && player.revision <= message.data.revision) {
        return { player: { ...player, revision: message.data.revision, map: message.data.map } };
      }
      return {};
    case 'message.append':
      return message.data.kind === 'characterMoved' ? { containerContents: undefined } : {};
    default:
      return {};
  }
}

/** Views invalidated as soon as a structured action is sent. */
export function submittedActionPatch(action: string): WorldPatch {
  const patch: WorldPatch = {};
  if (CONTAINER_INVALIDATING_ACTIONS.includes(action)) patch.containerContents = undefined;
  if (action.startsWith('trade.') && action !== 'trade.contents') patch.tradeContents = undefined;
  if (action === 'reward.open') patch.rewardOdds = undefined;
  if (action === 'mail.delete') patch.mailMessage = undefined;
  return patch;
}

/** Local effect of a board/mail mutation after the server confirmed it. */
export function confirmedActionPatch(
  views: Readonly<WorldViews>,
  pending: PendingStructuredAction,
): WorldPatch {
  if (pending.action === 'mail.delete' && pending.fields?.mailId) {
    const patch: WorldPatch = {};
    if (views.mailMessage?.id === pending.fields.mailId) patch.mailMessage = undefined;
    // Reload authoritative totals/pagination rather than retaining a deleted
    // entry or guessing whether the last page still exists.
    patch.mailList = undefined;
    return patch;
  }
  if (pending.action === 'board.post') {
    return { boardList: undefined };
  }
  const postId = pending.fields?.postId;
  if (pending.action === 'board.delete' && postId) {
    const patch: WorldPatch = {};
    if (views.boardEntry?.postId === postId) patch.boardEntry = undefined;
    if (views.boardList?.posts.some((entry) => entry.postId === postId)) patch.boardList = undefined;
    return patch;
  }
  if (pending.action === 'board.report' && postId) {
    const patch: WorldPatch = {};
    if (views.boardEntry?.postId === postId) {
      patch.boardEntry = { ...views.boardEntry, canReport: false };
    }
    if (views.boardList) {
      patch.boardList = {
        ...views.boardList,
        posts: views.boardList.posts.map((entry) => entry.postId === postId
          ? { ...entry, canReport: false }
          : entry),
      };
    }
    return patch;
  }
  return {};
}
