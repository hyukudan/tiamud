import type {
  ActionPerformFields,
  ActionResultData,
  BoardCatalogData,
  BoardEntryData,
  BoardListData,
  ConnectionState,
  ContainerContentsData,
  LiveServerMessage,
  MailListData,
  MailMessageData,
  MessageAppendData,
  MessagePresentationCue,
  PlayerStateSnapshot,
  RewardOddsData,
  SceneSnapshot,
  ServerNoticeData,
  StoryError,
  StoryView,
  SyncRecoveryData,
  TradeContentsData,
} from '../protocol';
import { StoryViewAssembler, type AssembledStoryView } from '../story-protocol';
import { isLootRollProjection } from '../dashboard/loot';
import {
  clearStoredAccess,
  logout as revokeAccess,
  sessionHasExpired,
  type CharacterSelection,
} from '../auth';
import {
  TiamudConnection,
  websocketUrl,
  type BrowserCapabilities,
  type ConnectionAuth,
  type ConnectionCallbacks,
} from '../socket';
import { loadCommandHistory, rememberCommand } from '../history';
import { createRequestId } from '../request-id';
import { MAX_SCROLLBACK_ENTRIES, boundedScrollback } from '../scrollback';
import {
  mapVisibleScenePresentation,
  presentationCueEntityId,
  presentationCuePose,
  reconcileScenePresentationCues,
  type ActiveScenePresentationCue,
} from '../scene-presentation';
import { directVisualSource } from './visual-source';
import { socialHistoryLabel } from './history-format';
import {
  CONFIRMED_STRUCTURED_ACTIONS,
  clearedPanels,
  confirmedActionPatch,
  reduceWorldMessage,
  submittedActionPatch,
  type PendingStructuredAction,
  type WorldPatch,
  type WorldViews,
} from './world-views';

export interface HistoryEntry {
  id: string;
  command: string;
  state: 'pending' | 'complete' | 'failed';
  response?: string;
}

export type VisualSceneEntity = ReturnType<typeof mapVisibleScenePresentation>[number];

/**
 * Scroll behaviour is owned by the view; the session only reports when the
 * history changed and how many entries of new activity arrived.
 */
export interface HistoryView {
  /** Called before history changes; returns whether the reader is anchored. */
  beforeReplace(): boolean;
  afterReplace(wasAnchored: boolean, activity: number): void;
  reset(): void;
}

/** The subset of `TiamudConnection` the session drives. */
export type SessionConnection = Pick<
  TiamudConnection,
  'connect' | 'close' | 'submit' | 'perform' | 'performAbility' | 'promote' |
  'acquireTalent' | 'updateCapabilities'
> & Partial<Pick<
  TiamudConnection,
  'storyChoose' | 'storyDialogue' | 'storyMenu' | 'storyUndo' | 'storyLeave'
>>;

export interface AudioSink {
  play(channel: 'music' | 'ambience' | 'effects', url: string, loop?: boolean): Promise<boolean>;
  stop(channel?: 'music' | 'ambience' | 'effects'): void;
}

export interface GameSessionDeps {
  storage: () => Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | undefined;
  capabilities: () => BrowserCapabilities;
  audio?: AudioSink;
  historyView?: HistoryView;
  createConnection?: (
    url: string,
    auth: ConnectionAuth,
    callbacks: ConnectionCallbacks,
    capabilities: BrowserCapabilities,
  ) => SessionConnection;
  socketUrl?: () => string;
  revoke?: (token: string) => Promise<void>;
}

const MAX_SEEN_CUE_KEYS = 128;

/**
 * Browser game session: owns the live connection, applies every incoming
 * server message to the authoritative world views, and records command
 * history. `App.svelte` renders it; it has no layout knowledge.
 */
export class GameSession {
  connectionState = $state<ConnectionState>('connecting');
  hasEntered = $state(false);
  scene = $state<SceneSnapshot>();
  player = $state<PlayerStateSnapshot>();
  containerContents = $state<ContainerContentsData>();
  tradeContents = $state<TradeContentsData>();
  rewardOdds = $state<RewardOddsData>();
  mailList = $state<MailListData>();
  mailMessage = $state<MailMessageData>();
  boardCatalog = $state<BoardCatalogData>();
  boardList = $state<BoardListData>();
  boardEntry = $state<BoardEntryData>();
  lastActionResult = $state<ActionResultData>();
  lootRoll = $state<string>();
  notice = $state<ServerNoticeData>();
  history = $state<HistoryEntry[]>([]);
  activeVisualCues = $state<Record<string, ActiveScenePresentationCue>>({});
  selectedAccess = $state<CharacterSelection>();
  accessError = $state<string>();
  command = $state('');
  rememberedCommands = $state<string[]>([]);
  historyCursor = $state(0);
  /** Complete current story view while the session is inside a story. */
  story = $state<AssembledStoryView>();
  storyError = $state<StoryError>();
  /** True between sending a story input and its answer. */
  storyPending = $state(false);

  /** Scene entities that can carry an optional sprite. */
  readonly visualEntities = $derived.by((): VisualSceneEntity[] =>
    mapVisibleScenePresentation(this.scene).filter((entity) =>
      Boolean(directVisualSource(entity.visual.poses.idle)),
    ),
  );

  #connection: SessionConnection | undefined;
  #eventCounter = 0;
  readonly #visualCueTimers = new Map<string, ReturnType<typeof setTimeout>>();
  readonly #pendingStructuredActions = new Map<string, PendingStructuredAction>();
  readonly #seenMessageCueKeys: string[] = [];
  readonly #storyParts = new StoryViewAssembler();
  readonly #deps: GameSessionDeps;

  constructor(deps: GameSessionDeps) {
    this.#deps = deps;
    this.rememberedCommands = loadCommandHistory(deps.storage());
    this.historyCursor = this.rememberedCommands.length;
  }

  get pendingStructuredActionCount(): number {
    return this.#pendingStructuredActions.size;
  }

  updateCapabilities(capabilities: BrowserCapabilities): void {
    this.#connection?.updateCapabilities(capabilities);
  }

  dismissNotice = (): void => {
    this.notice = undefined;
  };

  // --- incoming messages -------------------------------------------------

  handleMessage = (message: LiveServerMessage): void => {
    switch (message.type) {
      case 'server.welcome':
        // A new connection starts outside any story; the server re-sends
        // `story.entered` and the current view if the instance still lives.
        this.#clearStory();
        if (message.data.motd) {
          this.notice = { level: 'info', message: message.data.motd };
        }
        return;
      case 'story.entered':
        this.#storyParts.reset();
        this.storyError = undefined;
        return;
      case 'story.view':
        this.#applyStoryPart(message.data);
        return;
      case 'story.error':
        this.storyPending = false;
        this.storyError = message.data;
        return;
      case 'story.left':
        this.#clearStory();
        this.#replaceHistory([
          ...this.history,
          {
            id: `story-left-${message.seq ?? createRequestId()}`,
            command: 'Story',
            state: 'complete',
            response: message.data.reason === 'left'
              ? 'You leave the story and return to the world.'
              : 'The story closed after inactivity; its progress was not saved.',
          },
        ]);
        return;
      case 'scene.snapshot': {
        const retained = reconcileScenePresentationCues(
          this.scene,
          message.data,
          this.activeVisualCues,
        );
        for (const entityId of Object.keys(this.activeVisualCues)) {
          if (!retained[entityId]) this.#clearEntityVisualCue(entityId);
        }
        this.activeVisualCues = retained;
        break;
      }
      case 'entity.remove':
        if ('scene' in reduceWorldMessage(this.#views(), message)) {
          this.#clearEntityVisualCue(message.data.id);
        }
        break;
      case 'action.result':
        this.#handleActionResult(message.data);
        return;
      case 'server.notice':
        this.notice = message.data;
        return;
      case 'media.play':
        void this.#deps.audio?.play(message.data.channel, message.data.url, message.data.loop);
        return;
      case 'media.stop':
        this.#deps.audio?.stop(message.data.channel);
        return;
      case 'chat.message':
        this.#replaceHistory([
          ...this.history,
          {
            id: `chat-${message.seq ?? createRequestId()}`,
            command: socialHistoryLabel(message.data),
            state: 'complete',
            response: message.data.text,
          },
        ]);
        return;
    }

    this.#apply(reduceWorldMessage(this.#views(), message));

    if (message.type === 'message.append') {
      this.#eventCounter += 1;
      this.#replaceHistory([
        ...this.history,
        {
          id: `event-${message.seq ?? this.#eventCounter}-${this.#eventCounter}`,
          command: 'World event',
          state: 'complete' as const,
          response: message.data.text,
        },
      ]);
      if (message.data.cue) {
        this.#applyVisualCue(
          message.data.cue,
          message.data,
          `${message.seq ?? 0}:${message.id ?? ''}:${message.data.revision}`,
        );
      }
    }
  };

  handleSyncRecovery = (recovery: SyncRecoveryData): void => {
    this.#cancelVisualCues();
    this.scene = recovery.scene;
    this.player = recovery.player;
    this.#apply(clearedPanels());
  };

  handleStateChange = (state: ConnectionState): void => {
    this.connectionState = state;
    if (state === 'disconnected') {
      this.storyPending = false;
      this.#storyParts.reset();
      this.#markPendingInterrupted();
      this.#cancelVisualCues();
      this.#apply(clearedPanels());
    }
  };

  #handleActionResult(result: ActionResultData): void {
    if (!result.requestId) {
      if (result.message) {
        this.notice = { level: result.ok ? 'info' : 'error', message: result.message };
        if (result.ok && isLootRollProjection(result.message)) {
          this.lootRoll = result.message;
        }
      }
      return;
    }

    const pending = this.#pendingStructuredActions.get(result.requestId);
    this.#pendingStructuredActions.delete(result.requestId);
    this.lastActionResult = result;
    if (result.ok && result.message && isLootRollProjection(result.message)) {
      this.lootRoll = result.message;
    }
    if (result.ok && pending) {
      this.#apply(confirmedActionPatch(this.#views(), pending));
    }

    this.#replaceHistory(this.history.map((entry) =>
      entry.id === result.requestId
        ? {
            ...entry,
            state: result.ok ? 'complete' : 'failed',
            response: result.message ?? (result.ok ? 'Done.' : 'The action failed.'),
          }
        : entry,
    ));
  }

  // --- outgoing actions ---------------------------------------------------

  submitCommand = (value: string = this.command): void => {
    const submitted = value.trim();
    if (!submitted) return;
    const id = createRequestId();
    const sent = this.#connection?.submit(submitted, id) ?? false;
    this.#recordSubmission(submitted, id, sent);
  };

  submitAction = (
    action: string,
    fields: ActionPerformFields | undefined,
    label: string,
  ): string | undefined => {
    const id = createRequestId();
    const sent = this.#connection?.perform(action, fields, id) ?? false;
    if (sent) this.#apply(submittedActionPatch(action));
    if (sent && CONFIRMED_STRUCTURED_ACTIONS.includes(action)) {
      if (this.#pendingStructuredActions.size >= MAX_SCROLLBACK_ENTRIES) {
        const oldest = this.#pendingStructuredActions.keys().next().value;
        if (oldest) this.#pendingStructuredActions.delete(oldest);
      }
      this.#pendingStructuredActions.set(id, { action, fields });
    }
    this.#recordSubmission(label, id, sent);
    return sent ? id : undefined;
  };

  submitAbility = (abilityId: string, target: string | undefined, label: string): void => {
    const id = createRequestId();
    const sent = this.#connection?.performAbility(abilityId, target, id) ?? false;
    this.#recordSubmission(label, id, sent);
  };

  submitPromotion = (classId: string, label: string): void => {
    const id = createRequestId();
    const sent = this.#connection?.promote(classId, id) ?? false;
    this.#recordSubmission(label, id, sent);
  };

  submitTalent = (graphId: string, nodeId: string, label: string): void => {
    const id = createRequestId();
    const sent = this.#connection?.acquireTalent(graphId, nodeId, id) ?? false;
    this.#recordSubmission(label, id, sent);
  };

  /** Takes a gamebook choice or answers the dialogue with an opaque id. */
  chooseStory = (id: string): void => {
    const view = this.story;
    const item = view?.items.find((candidate) => 'id' in candidate && candidate.id === id);
    if (!view || this.storyPending || !item) return;
    const sent = item.type === 'choice'
      ? this.#connection?.storyChoose?.(id, view.viewRev)
      : item.type === 'dialogueOption'
        ? this.#connection?.storyDialogue?.(id, view.viewRev)
        : false;
    this.#markStorySent(sent);
  };

  /** Applies a menu verb, with one of its targets when the verb needs one. */
  storyMenu = (verb: string, target?: string): void => {
    const view = this.story;
    if (!view || this.storyPending) return;
    const verbItem = view.items.find((item) => item.type === 'verb' && item.id === verb);
    if (verbItem?.type !== 'verb' || verbItem.needsTarget !== (target !== undefined)) return;
    if (target !== undefined &&
      !view.items.some((item) => item.type === 'target' && item.id === target && item.verb === verb)) {
      return;
    }
    this.#markStorySent(this.#connection?.storyMenu?.(verb, target, view.viewRev));
  };

  #markStorySent(sent: boolean | undefined): void {
    if (sent) {
      this.storyPending = true;
      this.storyError = undefined;
    }
  }

  undoStory = (): void => {
    if (!this.story || this.storyPending) return;
    if (this.#connection?.storyUndo?.()) {
      this.storyPending = true;
      this.storyError = undefined;
    }
  };

  leaveStory = (): void => {
    if (!this.story) return;
    if (this.#connection?.storyLeave?.()) this.storyPending = true;
  };

  navigateCommandHistory = (event: KeyboardEvent): void => {
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
    if (this.rememberedCommands.length === 0) return;
    event.preventDefault();
    if (event.key === 'ArrowUp') {
      this.historyCursor = Math.max(0, this.historyCursor - 1);
      this.command = this.rememberedCommands[this.historyCursor] ?? '';
    } else {
      this.historyCursor = Math.min(this.rememberedCommands.length, this.historyCursor + 1);
      this.command = this.historyCursor === this.rememberedCommands.length
        ? ''
        : (this.rememberedCommands[this.historyCursor] ?? '');
    }
  };

  resetHistoryCursor = (): void => {
    this.historyCursor = this.rememberedCommands.length;
  };

  #recordSubmission(submitted: string, id: string, sent: boolean): void {
    this.#replaceHistory([
      ...this.history,
      {
        id,
        command: submitted,
        state: sent ? 'pending' : 'failed',
        response: sent ? undefined : 'The command was not sent because the server is unavailable.',
      },
    ]);
    this.rememberedCommands = rememberCommand(
      this.rememberedCommands,
      submitted,
      this.#deps.storage(),
    );
    this.historyCursor = this.rememberedCommands.length;
    this.command = '';
  }

  // --- session lifecycle --------------------------------------------------

  /** Returns true when the world was entered (false for an expired session). */
  enterWorld = (selection: CharacterSelection): boolean => {
    if (sessionHasExpired(selection)) {
      this.returnToLogin('Your session has expired. Sign in again.');
      return false;
    }
    this.#apply(clearedPanels());
    this.accessError = undefined;
    this.selectedAccess = selection;
    if (!this.#connection) {
      const auth: ConnectionAuth = {
        token: selection.token,
        characterId: selection.characterId,
        expiresAt: selection.expiresAt,
      };
      const callbacks: ConnectionCallbacks = {
        onMessage: this.handleMessage,
        onSync: this.handleSyncRecovery,
        onStateChange: this.handleStateChange,
        onProtocolError: () => {
          this.notice = {
            level: 'warning',
            message: 'The server sent a message this client could not read.',
          };
        },
        onAccessRejected: (message, clearCredentials) => this.returnToLogin(
          message || 'Your session was rejected. Sign in again.',
          clearCredentials,
        ),
      };
      const url = this.#deps.socketUrl?.() ?? websocketUrl(window.location);
      const capabilities = this.#deps.capabilities();
      this.#connection = this.#deps.createConnection?.(url, auth, callbacks, capabilities) ??
        new TiamudConnection(url, auth, callbacks, capabilities);
      this.#connection.connect();
    }
    this.hasEntered = true;
    return true;
  };

  returnToLogin = (message: string, clearCredentials = true): void => {
    this.#deps.audio?.stop();
    this.#connection = undefined;
    this.#resetSession('disconnected');
    this.accessError = message;
    if (clearCredentials) clearStoredAccess(this.#deps.storage());
  };

  signOut = async (): Promise<void> => {
    const token = this.selectedAccess?.token;
    this.#connection?.close();
    this.#deps.audio?.stop();
    this.#connection = undefined;
    this.#resetSession('connecting');
    this.accessError = undefined;
    clearStoredAccess(this.#deps.storage());
    if (token) {
      try {
        await (this.#deps.revoke ?? revokeAccess)(token);
      } catch {
        // Local sign-out remains effective when an expired token cannot be revoked.
      }
    }
  };

  dispose(): void {
    this.#connection?.close();
    this.#deps.audio?.stop();
    this.#cancelVisualCues();
  }

  #resetSession(state: ConnectionState): void {
    this.selectedAccess = undefined;
    this.hasEntered = false;
    this.connectionState = state;
    this.scene = undefined;
    this.#cancelVisualCues();
    this.player = undefined;
    this.#clearStory();
    this.#apply(clearedPanels());
    this.notice = undefined;
    this.history = [];
    this.#deps.historyView?.reset();
  }

  // --- helpers ------------------------------------------------------------

  #applyStoryPart(part: StoryView): void {
    const view = this.#storyParts.push(part);
    if (!view) return;
    this.story = view;
    this.storyPending = false;
    // Cues are presentation only and pass the same audio allowlist as scenes.
    for (const cue of view.items) {
      if (cue.type !== 'cue') continue;
      if (cue.action === 'play' && cue.url) {
        void this.#deps.audio?.play(cue.channel, cue.url, cue.loop);
      } else if (cue.action === 'stop') {
        this.#deps.audio?.stop(cue.channel);
      }
    }
  }

  #clearStory(): void {
    this.#storyParts.reset();
    this.story = undefined;
    this.storyError = undefined;
    this.storyPending = false;
  }

  #views(): WorldViews {
    return {
      scene: this.scene,
      player: this.player,
      containerContents: this.containerContents,
      tradeContents: this.tradeContents,
      rewardOdds: this.rewardOdds,
      mailList: this.mailList,
      mailMessage: this.mailMessage,
      boardCatalog: this.boardCatalog,
      boardList: this.boardList,
      boardEntry: this.boardEntry,
    };
  }

  #apply(patch: WorldPatch): void {
    if ('scene' in patch) this.scene = patch.scene;
    if ('player' in patch) this.player = patch.player;
    if ('containerContents' in patch) this.containerContents = patch.containerContents;
    if ('tradeContents' in patch) this.tradeContents = patch.tradeContents;
    if ('rewardOdds' in patch) this.rewardOdds = patch.rewardOdds;
    if ('mailList' in patch) this.mailList = patch.mailList;
    if ('mailMessage' in patch) this.mailMessage = patch.mailMessage;
    if ('boardCatalog' in patch) this.boardCatalog = patch.boardCatalog;
    if ('boardList' in patch) this.boardList = patch.boardList;
    if ('boardEntry' in patch) this.boardEntry = patch.boardEntry;
  }

  #replaceHistory(next: HistoryEntry[], activity = 1): void {
    const wasAnchored = this.#deps.historyView?.beforeReplace() ?? true;
    this.history = boundedScrollback(next);
    this.#deps.historyView?.afterReplace(wasAnchored, activity);
  }

  #markPendingInterrupted(): void {
    this.#pendingStructuredActions.clear();
    if (!this.history.some((entry) => entry.state === 'pending')) return;
    this.#replaceHistory(this.history.map((entry) => entry.state === 'pending'
      ? {
          ...entry,
          state: 'failed' as const,
          response: 'The connection closed before the server confirmed this action.',
        }
      : entry));
  }

  #clearEntityVisualCue(entityId: string): void {
    const timer = this.#visualCueTimers.get(entityId);
    if (timer !== undefined) clearTimeout(timer);
    this.#visualCueTimers.delete(entityId);
    if (this.activeVisualCues[entityId]) {
      const { [entityId]: _removed, ...remaining } = this.activeVisualCues;
      this.activeVisualCues = remaining;
    }
  }

  #cancelVisualCues(): void {
    for (const timer of this.#visualCueTimers.values()) clearTimeout(timer);
    this.#visualCueTimers.clear();
    this.#seenMessageCueKeys.length = 0;
    this.activeVisualCues = {};
  }

  #applyVisualCue(cue: MessagePresentationCue, message: MessageAppendData, eventKey: string): void {
    if (this.#seenMessageCueKeys.includes(eventKey)) return;
    this.#seenMessageCueKeys.push(eventKey);
    if (this.#seenMessageCueKeys.length > MAX_SEEN_CUE_KEYS) {
      this.#seenMessageCueKeys.splice(0, this.#seenMessageCueKeys.length - MAX_SEEN_CUE_KEYS);
    }
    const entityId = presentationCueEntityId(cue, message);
    if (!this.visualEntities.some((entity) => entity.id === entityId)) return;

    this.#clearEntityVisualCue(entityId);
    this.activeVisualCues = {
      ...this.activeVisualCues,
      [entityId]: { ...cue, pose: presentationCuePose(cue), eventKey },
    };
    this.#visualCueTimers.set(entityId, setTimeout(() => {
      this.#visualCueTimers.delete(entityId);
      const { [entityId]: _expired, ...remaining } = this.activeVisualCues;
      this.activeVisualCues = remaining;
    }, cue.durationMs));
  }
}
