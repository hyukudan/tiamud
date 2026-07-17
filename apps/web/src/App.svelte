<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type {
    ActionResultData,
    ActionPerformFields,
    ConnectionState,
    ContainerContentsData,
    LiveServerMessage,
    MailListData,
    MailMessageData,
    MessageAppendData,
    MessagePresentationCue,
    PlayerStateSnapshot,
    RewardOddsData,
    SceneEntityVisualSource,
    SceneSnapshot,
    ServerNoticeData,
    SocialMessageData,
    SyncRecoveryData,
    TradeContentsData,
  } from './lib/protocol';
  import { localizedImageAlt } from './lib/protocol';
  import {
    imageAfterFailures,
    imageUrl,
    loadSceneImageFromManifest,
    preserveContextualImageAlt,
    sceneAssetReference,
  } from './lib/assets';
  import WelcomeGate from './components/WelcomeGate.svelte';
  import AccountAccess from './components/AccountAccess.svelte';
  import PreferencesPanel from './components/PreferencesPanel.svelte';
  import PlayerDashboard from './components/PlayerDashboard.svelte';
  import { PRODUCT_NAME } from './lib/brand';
  import {
    clearStoredAccess,
    logout as revokeAccess,
    sessionHasExpired,
    type CharacterSelection,
  } from './lib/auth';
  import {
    TiamudConnection,
    websocketUrl,
    type BrowserCapabilities,
  } from './lib/socket';
  import {
    loadPreferences,
    motionReduced,
    multimediaAllowed,
    savePreferences,
    type ClientPreferences,
  } from './lib/preferences';
  import { loadCommandHistory, rememberCommand } from './lib/history';
  import { AudioController } from './lib/audio';
  import { createRequestId } from './lib/request-id';
  import {
    MAX_SCROLLBACK_ENTRIES,
    boundedScrollback,
    isScrollAnchored,
  } from './lib/scrollback';
  import {
    layoutSceneEntities,
    type VisualPlacement,
  } from './lib/scene-layout';
  import {
    mapVisibleScenePresentation,
    presentationCueEntityId,
    presentationCuePose,
    reconcileScenePresentationCues,
    visualSourceForCue,
    type ActiveScenePresentationCue,
  } from './lib/scene-presentation';

  interface HistoryEntry {
    id: string;
    command: string;
    state: 'pending' | 'complete' | 'failed';
    response?: string;
  }

  type VisualSceneEntity = ReturnType<typeof mapVisibleScenePresentation>[number];

  interface RenderedVisualEntity {
    entity: VisualSceneEntity;
    placement: VisualPlacement;
    source: SceneEntityVisualSource;
    cue?: ActiveScenePresentationCue;
  }

  let connectionState = $state<ConnectionState>('connecting');
  let hasEntered = $state(false);
  let scene = $state<SceneSnapshot>();
  let player = $state<PlayerStateSnapshot>();
  let containerContents = $state<ContainerContentsData>();
  let tradeContents = $state<TradeContentsData>();
  let rewardOdds = $state<RewardOddsData>();
  let mailList = $state<MailListData>();
  let mailMessage = $state<MailMessageData>();
  let manifestImage = $state<SceneSnapshot['image']>();
  let activeAssetKey: string | undefined;
  let failedImageUrls = $state<string[]>([]);
  let notice = $state<ServerNoticeData>();
  let command = $state('');
  let commandInput = $state<HTMLInputElement>();
  let history = $state<HistoryEntry[]>([]);
  let historyScroller = $state<HTMLDivElement>();
  let historyAnchored = $state(true);
  let unseenHistory = $state(0);
  let eventCounter = 0;
  let activeVisualCues = $state<Record<string, ActiveScenePresentationCue>>({});
  const visualCueTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const seenMessageCueKeys: string[] = [];
  let connection: TiamudConnection | undefined;
  let selectedAccess = $state<CharacterSelection>();
  let accessError = $state<string>();
  const browserSaveData = typeof navigator !== 'undefined' &&
    (navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData === true;
  const initialPreferences = loadPreferences(
    typeof window === 'undefined' ? undefined : window.localStorage,
    browserSaveData,
  );
  let preferences = $state(initialPreferences);
  let rememberedCommands = $state(
    loadCommandHistory(typeof window === 'undefined' ? undefined : window.localStorage),
  );
  let historyCursor = $state(0);
  const audioController = typeof Audio === 'undefined'
    ? undefined
    : new AudioController((url) => new Audio(url), initialPreferences);
  const showMultimedia = $derived(multimediaAllowed(preferences));
  const systemReducedMotion = typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const reduceVisualMotion = $derived(motionReduced(preferences, systemReducedMotion));

  const connectionLabel = $derived(
    connectionState === 'connected'
      ? 'Connected'
      : connectionState === 'connecting'
        ? 'Connecting…'
        : 'Disconnected — reconnecting…',
  );
  const browserLocale = typeof navigator === 'undefined' ? 'en' : navigator.language;
  const localizedSceneImage = $derived(
    scene?.image
      ? { ...scene.image, alt: localizedImageAlt(scene.image, browserLocale) ?? scene.image.alt }
      : undefined,
  );
  const directImageUrl = $derived(imageUrl(scene?.image));
  const manifestUrl = $derived(directImageUrl ? undefined : scene?.image?.manifest);
  const sceneAssetKey = $derived(showMultimedia ? (directImageUrl ?? manifestUrl) : undefined);
  const atmosphereImage = $derived(
    showMultimedia
      ? imageAfterFailures((directImageUrl ? localizedSceneImage : undefined) ?? manifestImage, failedImageUrls)
      : undefined,
  );
  const imageRenderKey = $derived(`${sceneAssetKey ?? 'none'}:${failedImageUrls.join('|')}`);
  const visualEntities = $derived.by((): VisualSceneEntity[] => {
    return mapVisibleScenePresentation(scene).filter((entity) =>
      Boolean(directVisualSource(entity.visual.poses.idle)),
    );
  });
  const visualLayout = $derived.by(() =>
    showMultimedia && Boolean(atmosphereImage)
      ? layoutSceneEntities(visualEntities.map((entity) => ({
          id: entity.id,
          priority: entity.visual.priority,
          footprint: entity.visual.footprint,
          preferredAnchor: entity.visual.preferredAnchor,
        })), { maxVisible: 10 })
      : { placements: [] as VisualPlacement[], overflow: [] as string[] },
  );
  const renderedVisualEntities = $derived.by((): RenderedVisualEntity[] =>
    visualLayout.placements.flatMap((placement) => {
      const entity = visualEntities.find((candidate) => candidate.id === placement.id);
      if (!entity) return [];
      const cue = activeVisualCues[entity.id];
      const requested = visualSourceForCue(entity.visual, cue);
      const source = directVisualSource(requested) ?? directVisualSource(entity.visual.poses.idle);
      return source ? [{ entity, placement, source, cue }] : [];
    }),
  );
  const overflowVisualEntities = $derived(visualLayout.overflow.slice(0, 5).flatMap((id) =>
    visualEntities.find((entity) => entity.id === id) ?? [],
  ));
  const commandSuggestions = $derived.by(() => {
    const commands = [
      'look',
      'inventory',
      'loot',
      'north',
      'east',
      'south',
      'west',
      'up',
      'down',
      'say ',
      'tell ',
      'who',
      'help',
      'ignore ',
      'unignore ',
      'block ',
      'unblock ',
      'report ',
      ...(scene?.exits.map((exit) => exit.command) ?? []),
      ...rememberedCommands.toReversed(),
    ];
    return [...new Set(commands)].slice(0, 24);
  });

  $effect(() => {
    const assetKey = sceneAssetKey;
    if (assetKey === activeAssetKey) {
      return;
    }

    activeAssetKey = assetKey;
    manifestImage = undefined;
    failedImageUrls = [];

    const reference = sceneAssetReference(manifestUrl);
    if (directImageUrl || !reference) {
      return;
    }

    const controller = new AbortController();
    const contextualImage = localizedSceneImage;
    void loadSceneImageFromManifest(reference, controller.signal).then((image) => {
      if (!controller.signal.aborted) {
        manifestImage = preserveContextualImageAlt(image, contextualImage, browserLocale);
      }
    });

    return () => controller.abort();
  });

  $effect(() => {
    const current = preferences;
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.contrast = current.highContrast ? 'high' : 'normal';
    document.documentElement.dataset.motion = current.reduceMotion || current.screenReader
      ? 'reduce'
      : 'normal';
    document.documentElement.dataset.reader = current.screenReader ? 'true' : 'false';
    document.documentElement.style.fontSize = `${current.fontScale}%`;
    savePreferences(window.localStorage, current);
  });

  function updatePreferences(next: ClientPreferences): void {
    if (next.audio && !preferences.audio) audioController?.unlockFromGesture();
    audioController?.updatePreferences(next);
    preferences = next;
    connection?.updateCapabilities(capabilitiesFor(next));
  }

  function capabilitiesFor(value: ClientPreferences): BrowserCapabilities {
    return {
      images: multimediaAllowed(value),
      reducedMotion: motionReduced(
        value,
        window.matchMedia('(prefers-reduced-motion: reduce)').matches,
      ),
      audio: value.audio,
    };
  }

  function replaceHistory(next: HistoryEntry[], activity = 1): void {
    const scroller = historyScroller;
    const wasAnchored = scroller ? isScrollAnchored(scroller) : true;
    historyAnchored = wasAnchored;
    history = boundedScrollback(next);
    void tick().then(() => {
      if (!historyScroller) return;
      if (wasAnchored) {
        historyScroller.scrollTop = historyScroller.scrollHeight;
        historyAnchored = true;
        unseenHistory = 0;
      } else if (activity > 0) {
        historyAnchored = false;
        unseenHistory = Math.min(MAX_SCROLLBACK_ENTRIES, unseenHistory + activity);
      }
    });
  }

  function handleHistoryScroll(): void {
    if (!historyScroller) return;
    historyAnchored = isScrollAnchored(historyScroller);
    if (historyAnchored) unseenHistory = 0;
  }

  function jumpToLatest(): void {
    if (!historyScroller) return;
    const top = historyScroller.scrollHeight;
    if (typeof historyScroller.scrollTo === 'function') {
      historyScroller.scrollTo({
        top,
        behavior: motionReduced(
          preferences,
          window.matchMedia('(prefers-reduced-motion: reduce)').matches,
        ) ? 'auto' : 'smooth',
      });
    } else {
      historyScroller.scrollTop = top;
    }
    historyAnchored = true;
    unseenHistory = 0;
  }

  function markPendingInterrupted(): void {
    if (!history.some((entry) => entry.state === 'pending')) return;
    replaceHistory(history.map((entry) => entry.state === 'pending'
      ? {
          ...entry,
          state: 'failed' as const,
          response: 'The connection closed before the server confirmed this action.',
        }
      : entry));
  }

  function handleActionResult(result: ActionResultData): void {
    if (!result.requestId) {
      if (result.message) {
        notice = { level: result.ok ? 'info' : 'error', message: result.message };
      }
      return;
    }

    replaceHistory(history.map((entry) =>
      entry.id === result.requestId
        ? {
            ...entry,
            state: result.ok ? 'complete' : 'failed',
            response: result.message ?? (result.ok ? 'Done.' : 'The action failed.'),
          }
        : entry,
    ));
  }

  function socialHistoryLabel(message: SocialMessageData): string {
    switch (message.channel) {
      case 'announcement':
        return `${message.sender} announces`;
      case 'say':
        return `${message.sender} says`;
      case 'tell':
        return `${message.sender} tells you`;
      case 'telepathy':
        return `${message.sender} reaches your mind`;
      case 'emote':
        return 'Emote';
      case 'party':
        return `${message.sender} to the party`;
      default:
        return `${message.sender} in #${message.channel}`;
    }
  }

  function directVisualSource(
    source: SceneEntityVisualSource | undefined,
  ): SceneEntityVisualSource | undefined {
    return source && (source.avif || source.webp || source.png) ? source : undefined;
  }

  function visualSourceUrl(source: SceneEntityVisualSource): string {
    return source.png ?? source.webp ?? source.avif ?? '';
  }

  function clearEntityVisualCue(entityId: string): void {
    const timer = visualCueTimers.get(entityId);
    if (timer !== undefined) clearTimeout(timer);
    visualCueTimers.delete(entityId);
    if (activeVisualCues[entityId]) {
      const { [entityId]: _removed, ...remaining } = activeVisualCues;
      activeVisualCues = remaining;
    }
  }

  function cancelVisualCues(): void {
    for (const timer of visualCueTimers.values()) clearTimeout(timer);
    visualCueTimers.clear();
    seenMessageCueKeys.length = 0;
    activeVisualCues = {};
  }

  function resetVisualPresentation(): void {
    cancelVisualCues();
  }

  function applyVisualCue(
    cue: MessagePresentationCue,
    message: MessageAppendData,
    eventKey: string,
  ): void {
    if (seenMessageCueKeys.includes(eventKey)) return;
    seenMessageCueKeys.push(eventKey);
    if (seenMessageCueKeys.length > 128) {
      seenMessageCueKeys.splice(0, seenMessageCueKeys.length - 128);
    }
    const entityId = presentationCueEntityId(cue, message);
    if (!visualEntities.some((entity) => entity.id === entityId)) return;

    clearEntityVisualCue(entityId);
    activeVisualCues = {
      ...activeVisualCues,
      [entityId]: { ...cue, pose: presentationCuePose(cue), eventKey },
    };
    visualCueTimers.set(entityId, setTimeout(() => {
      visualCueTimers.delete(entityId);
      const { [entityId]: _expired, ...remaining } = activeVisualCues;
      activeVisualCues = remaining;
    }, cue.durationMs));
  }

  function handleMessage(message: LiveServerMessage): void {
    switch (message.type) {
      case 'server.welcome':
        if (message.data.motd) {
          notice = { level: 'info', message: message.data.motd };
        }
        break;
      case 'scene.snapshot':
        {
          const retained = reconcileScenePresentationCues(scene, message.data, activeVisualCues);
          for (const entityId of Object.keys(activeVisualCues)) {
            if (!retained[entityId]) clearEntityVisualCue(entityId);
          }
          activeVisualCues = retained;
        }
        scene = message.data;
        break;
      case 'scene.patch':
        if (scene?.viewId === message.data.viewId && (scene.revision ?? 0) <= message.data.revision) {
          const { image, ...patch } = message.data;
          scene = {
            ...scene,
            ...patch,
            ...(image === null ? { image: undefined } : image ? { image } : {}),
          };
        }
        break;
      case 'entity.upsert':
        if (scene?.viewId === message.data.viewId && (scene.revision ?? 0) <= message.data.revision) {
          if (message.data.kind === 'item') {
            scene = {
              ...scene,
              revision: message.data.revision,
              items: [
                ...scene.items.filter((entity) => entity.id !== message.data.id),
                message.data.entity,
              ],
            };
          } else {
            scene = {
              ...scene,
              revision: message.data.revision,
              npcs: [
                ...scene.npcs.filter((entity) => entity.id !== message.data.id),
                message.data.entity,
              ],
            };
          }
        }
        break;
      case 'entity.remove':
        if (scene?.viewId === message.data.viewId && (scene.revision ?? 0) <= message.data.revision) {
          clearEntityVisualCue(message.data.id);
          scene = message.data.kind === 'item'
            ? {
                ...scene,
                revision: message.data.revision,
                items: scene.items.filter((entity) => entity.id !== message.data.id),
              }
            : {
                ...scene,
                revision: message.data.revision,
                npcs: scene.npcs.filter((entity) => entity.id !== message.data.id),
              };
        }
        break;
      case 'player.snapshot':
        if (
          !message.data.trade
          || tradeContents?.tradeId !== message.data.trade.id
          || (
            player?.trade?.stateRevision !== undefined
            && player.trade.stateRevision !== message.data.trade.stateRevision
          )
        ) {
          tradeContents = undefined;
        }
        if (rewardOdds && !message.data.inventory.some((item) => item.id === rewardOdds?.selector)) {
          rewardOdds = undefined;
        }
        player = message.data;
        break;
      case 'vitals.update':
        if (player && player.revision <= message.data.revision) {
          player = { ...player, revision: message.data.revision, character: message.data.character };
        }
        break;
      case 'inventory.patch':
        if (player && player.revision <= message.data.revision) {
          player = {
            ...player,
            revision: message.data.revision,
            inventory: message.data.inventory,
            equipment: message.data.equipment,
            inventoryCapacity: message.data.inventoryCapacity,
          };
          containerContents = undefined;
          tradeContents = undefined;
          if (rewardOdds && !message.data.inventory.some((item) => item.id === rewardOdds?.selector)) {
            rewardOdds = undefined;
          }
        }
        break;
      case 'container.contents':
        containerContents = message.data;
        break;
      case 'trade.contents':
        tradeContents = message.data;
        break;
      case 'reward.odds':
        rewardOdds = message.data;
        break;
      case 'mail.list':
        mailList = message.data;
        if (mailMessage && !message.data.messages.some((entry) => entry.id === mailMessage?.id)) {
          mailMessage = undefined;
        }
        break;
      case 'mail.message':
        mailMessage = message.data;
        if (mailList) {
          mailList = {
            ...mailList,
            unreadCount: Math.max(
              0,
              mailList.unreadCount - (mailList.messages.some(
                (entry) => entry.id === message.data.id && !entry.read,
              ) ? 1 : 0),
            ),
            messages: mailList.messages.map((entry) => entry.id === message.data.id
              ? { ...entry, read: true }
              : entry),
          };
        }
        break;
      case 'map.snapshot':
        if (player && player.revision <= message.data.revision) {
          player = { ...player, revision: message.data.revision, map: message.data.map };
        }
        break;
      case 'message.append':
        if (message.data.kind === 'characterMoved') containerContents = undefined;
        eventCounter += 1;
        replaceHistory([
          ...history,
          {
            id: `event-${message.seq ?? eventCounter}-${eventCounter}`,
            command: 'World event',
            state: 'complete' as const,
            response: message.data.text,
          },
        ]);
        if (message.data.cue) {
          applyVisualCue(
            message.data.cue,
            message.data,
            `${message.seq ?? 0}:${message.id ?? ''}:${message.data.revision}`,
          );
        }
        break;
      case 'action.result':
        handleActionResult(message.data);
        break;
      case 'server.notice':
        notice = message.data;
        break;
      case 'media.play':
        void audioController?.play(message.data.channel, message.data.url, message.data.loop);
        break;
      case 'media.stop':
        audioController?.stop(message.data.channel);
        break;
      case 'chat.message':
        replaceHistory([
          ...history,
          {
            id: `chat-${message.seq ?? createRequestId()}`,
            command: socialHistoryLabel(message.data),
            state: 'complete',
            response: message.data.text,
          },
        ]);
        break;
    }
  }

  function handleSyncRecovery(recovery: SyncRecoveryData): void {
    resetVisualPresentation();
    scene = recovery.scene;
    player = recovery.player;
    containerContents = undefined;
    tradeContents = undefined;
    rewardOdds = undefined;
    mailList = undefined;
    mailMessage = undefined;
  }

  function submitCommand(value = command): void {
    const submitted = value.trim();
    if (!submitted) {
      return;
    }

    const id = createRequestId();
    const sent = connection?.submit(submitted, id) ?? false;
    recordSubmission(submitted, id, sent);
  }

  function submitAction(action: string, fields: ActionPerformFields | undefined, label: string): void {
    const id = createRequestId();
    const sent = connection?.perform(action, fields, id) ?? false;
    if (sent && [
      'item.take', 'item.drop', 'item.put', 'item.get', 'item.give', 'item.contents',
      'item.use', 'item.equip', 'item.unequip', 'loot.pass',
    ].includes(action)) {
      containerContents = undefined;
    }
    if (sent && action.startsWith('trade.') && action !== 'trade.contents') {
      tradeContents = undefined;
    }
    if (sent && action === 'reward.open') rewardOdds = undefined;
    if (sent && action === 'mail.delete') mailMessage = undefined;
    recordSubmission(label, id, sent);
  }

  function submitAbility(abilityId: string, target: string | undefined, label: string): void {
    const id = createRequestId();
    const sent = connection?.performAbility(abilityId, target, id) ?? false;
    recordSubmission(label, id, sent);
  }

  function submitPromotion(classId: string, label: string): void {
    const id = createRequestId();
    const sent = connection?.promote(classId, id) ?? false;
    recordSubmission(label, id, sent);
  }

  function submitTalent(graphId: string, nodeId: string, label: string): void {
    const id = createRequestId();
    const sent = connection?.acquireTalent(graphId, nodeId, id) ?? false;
    recordSubmission(label, id, sent);
  }

  function recordSubmission(submitted: string, id: string, sent: boolean): void {

    replaceHistory([
      ...history,
      {
        id,
        command: submitted,
        state: sent ? 'pending' : 'failed',
        response: sent ? undefined : 'The command was not sent because the server is unavailable.',
      },
    ]);
    rememberedCommands = rememberCommand(
      rememberedCommands,
      submitted,
      typeof window === 'undefined' ? undefined : window.localStorage,
    );
    historyCursor = rememberedCommands.length;
    command = '';
  }

  function itemActionName(commandValue: string): string | undefined {
    const normalized = commandValue.trim().toLowerCase();
    if (normalized.startsWith('reward odds ')) return 'reward.odds';
    const verb = normalized.split(/\s+/, 1)[0];
    if (verb === 'open') return 'reward.open';
    return verb && ['take', 'drop', 'examine', 'use', 'equip', 'unequip', 'contents'].includes(verb)
      ? `item.${verb}`
      : undefined;
  }

  function navigateCommandHistory(event: KeyboardEvent): void {
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
    if (rememberedCommands.length === 0) return;
    event.preventDefault();
    if (event.key === 'ArrowUp') {
      historyCursor = Math.max(0, historyCursor - 1);
      command = rememberedCommands[historyCursor] ?? '';
    } else {
      historyCursor = Math.min(rememberedCommands.length, historyCursor + 1);
      command = historyCursor === rememberedCommands.length
        ? ''
        : (rememberedCommands[historyCursor] ?? '');
    }
  }

  function enterWorld(selection: CharacterSelection): void {
    if (sessionHasExpired(selection)) {
      returnToLogin('Your session has expired. Sign in again.');
      return;
    }
    containerContents = undefined;
    tradeContents = undefined;
    rewardOdds = undefined;
    mailList = undefined;
    mailMessage = undefined;
    accessError = undefined;
    selectedAccess = selection;
    if (!connection) {
      connection = new TiamudConnection(
        websocketUrl(window.location),
        {
          token: selection.token,
          characterId: selection.characterId,
          expiresAt: selection.expiresAt,
        },
        {
          onMessage: handleMessage,
          onSync: handleSyncRecovery,
          onStateChange: (state) => {
            connectionState = state;
            if (state === 'disconnected') {
              markPendingInterrupted();
              cancelVisualCues();
              containerContents = undefined;
              tradeContents = undefined;
              rewardOdds = undefined;
              mailList = undefined;
              mailMessage = undefined;
            }
          },
          onProtocolError: () => {
            notice = { level: 'warning', message: 'The server sent a message this client could not read.' };
          },
          onAccessRejected: (message, clearCredentials) => returnToLogin(
            message || 'Your session was rejected. Sign in again.',
            clearCredentials,
          ),
        },
        capabilitiesFor(preferences),
      );
      connection.connect();
    }

    hasEntered = true;
    void tick().then(() => commandInput?.focus());
  }

  function returnToLogin(message: string, clearCredentials = true): void {
    audioController?.stop();
    connection = undefined;
    selectedAccess = undefined;
    hasEntered = false;
    connectionState = 'disconnected';
    scene = undefined;
    resetVisualPresentation();
    player = undefined;
    containerContents = undefined;
    tradeContents = undefined;
    rewardOdds = undefined;
    mailList = undefined;
    mailMessage = undefined;
    notice = undefined;
    history = [];
    historyAnchored = true;
    unseenHistory = 0;
    accessError = message;
    if (clearCredentials) clearStoredAccess(window.localStorage);
  }

  async function signOut(): Promise<void> {
    const token = selectedAccess?.token;
    connection?.close();
    audioController?.stop();
    connection = undefined;
    selectedAccess = undefined;
    hasEntered = false;
    connectionState = 'connecting';
    scene = undefined;
    resetVisualPresentation();
    player = undefined;
    containerContents = undefined;
    tradeContents = undefined;
    rewardOdds = undefined;
    mailList = undefined;
    mailMessage = undefined;
    notice = undefined;
    history = [];
    historyAnchored = true;
    unseenHistory = 0;
    accessError = undefined;
    clearStoredAccess(window.localStorage);
    if (token) {
      try {
        await revokeAccess(token);
      } catch {
        // Local sign-out remains effective when an expired token cannot be revoked.
      }
    }
  }

  onMount(() => {
    historyCursor = rememberedCommands.length;
    return () => {
      connection?.close();
      audioController?.stop();
      resetVisualPresentation();
    };
  });
</script>

<svelte:head>
  <title>{PRODUCT_NAME}</title>
</svelte:head>

{#if !hasEntered}
  <WelcomeGate onenter={() => undefined} showArtwork={showMultimedia}>
    {#snippet access(_enterWorld)}
      {#if accessError}<p class="access-error" role="alert">{accessError}</p>{/if}
      <AccountAccess onselect={enterWorld} />
      <PreferencesPanel value={preferences} onchange={updatePreferences} />
    {/snippet}
  </WelcomeGate>
{:else}
<div class="shell">
  <header class="topbar">
    <a class="brand" href="/" aria-label={`${PRODUCT_NAME} home`}>{PRODUCT_NAME}</a>
    <div class="session-controls">
      {#if selectedAccess}<span class="character-name">{selectedAccess.characterName}</span>{/if}
      <p class:connected={connectionState === 'connected'} class="connection" role="status">
        <span aria-hidden="true"></span>{connectionLabel}
      </p>
      <button type="button" onclick={() => void signOut()}>Sign out</button>
      <PreferencesPanel value={preferences} onchange={updatePreferences} />
    </div>
  </header>

  {#if notice}
    <aside class:warning={notice.level === 'warning'} class:error={notice.level === 'error'} class="notice">
      <p role={notice.level === 'error' ? 'alert' : 'status'}>{notice.message}</p>
      <button aria-label="Dismiss notice" onclick={() => (notice = undefined)}>×</button>
    </aside>
  {/if}

  <main>
    <section class="scene" aria-labelledby="scene-title">
      {#if scene}
        {#if atmosphereImage}
          <div class="visual-stage">
            {#key imageRenderKey}
              <picture class="atmosphere">
                {#if atmosphereImage.avif}
                  <source
                    srcset={atmosphereImage.avifSrcset ?? atmosphereImage.avif}
                    sizes="(min-width: 58rem) 65vw, 100vw"
                    type="image/avif"
                  />
                {/if}
                {#if atmosphereImage.webp}
                  <source
                    srcset={atmosphereImage.webpSrcset ?? atmosphereImage.webp}
                    sizes="(min-width: 58rem) 65vw, 100vw"
                    type="image/webp"
                  />
                {/if}
                <img
                  src={imageUrl(atmosphereImage)}
                  srcset={atmosphereImage.jpegSrcset ?? atmosphereImage.pngSrcset}
                  sizes="(min-width: 58rem) 65vw, 100vw"
                  alt={atmosphereImage.alt}
                  width={atmosphereImage.width}
                  height={atmosphereImage.height}
                  fetchpriority="high"
                  onerror={(event) => {
                    const target = event.currentTarget as HTMLImageElement;
                    const failed = target.currentSrc || target.src;
                    if (failed && !failedImageUrls.includes(failed)) {
                      failedImageUrls = [...failedImageUrls, failed];
                    }
                  }}
                />
              </picture>
            {/key}

            {#if renderedVisualEntities.length > 0}
              <div class="entity-visual-layer" aria-hidden="true">
                {#each renderedVisualEntities as visual (visual.entity.id)}
                  <div
                    class="entity-visual entity-layer-{visual.entity.visual.layer}"
                    class:cue-attack={visual.cue?.kind === 'attack'}
                    class:cue-hit={visual.cue?.kind === 'hit'}
                    class:cue-heal={visual.cue?.kind === 'heal'}
                    class:cue-defeat={visual.cue?.kind === 'defeat'}
                    class:intensity-1={visual.cue?.intensity === 1}
                    class:intensity-2={visual.cue?.intensity === 2}
                    class:intensity-3={visual.cue?.intensity === 3}
                    class:motion-reduced={reduceVisualMotion}
                    style={`left:${visual.placement.rect.x * 100}%;top:${visual.placement.rect.y * 100}%;width:${visual.placement.rect.width * 100}%;height:${visual.placement.rect.height * 100}%;--cue-duration:${visual.cue?.durationMs ?? 1}ms`}
                  >
                    {#key visual.cue?.eventKey ?? 'idle'}
                      <picture class="entity-sprite">
                        {#if visual.source.avif}<source srcset={visual.source.avif} type="image/avif" />{/if}
                        {#if visual.source.webp}<source srcset={visual.source.webp} type="image/webp" />{/if}
                        <img
                          src={visualSourceUrl(visual.source)}
                          alt=""
                          aria-hidden="true"
                          decoding="async"
                        />
                      </picture>
                    {/key}
                  </div>
                {/each}

                {#if visualLayout.overflow.length > 0}
                  <div
                    class="entity-visual-overflow"
                    title={overflowVisualEntities.map((entity) => entity.label).join(', ')}
                  >
                    <span>+{visualLayout.overflow.length}</span>
                    <ul>
                      {#each overflowVisualEntities as entity (entity.id)}<li>{entity.label}</li>{/each}
                    </ul>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        {/if}

        <div class="narrative">
          <p class="eyebrow">Current location</p>
          <h1 id="scene-title">{scene.title}</h1>
          <p class="scene-weather">
            <span>{scene.weather.condition}</span>
            <span aria-hidden="true">·</span>
            <span>Day {scene.weather.day}, hour {scene.weather.hour}</span>
            <span class="sr-only">Weather may change in {scene.weather.nextChangeInTicks} world ticks.</span>
          </p>
          {#each scene.prose as paragraph}
            <p>{paragraph}</p>
          {/each}

          {#if scene.exits.length > 0}
            <nav class="exits" aria-label="Exits">
              <h2>Ways onward</h2>
              <ul>
                {#each scene.exits as exit (exit.id)}
                  <li><button onclick={() => submitAction('move', { target: exit.command }, exit.command)}>{exit.label}</button></li>
                {/each}
              </ul>
            </nav>
          {/if}

          {#if scene.items.length > 0}
            <section class="scene-items" aria-labelledby="items-title">
              <h2 id="items-title">Within reach</h2>
              <button
                class="loot-status-button"
                type="button"
                onclick={() => submitAction('loot.status', undefined, 'loot status')}
              >Review loot rights</button>
              <ul>
                {#each scene.items as item (item.id)}
                  <li>
                    <div>
                      <strong>{item.label} ×{item.quantity}</strong>
                      <p>
                        {item.rarity} · level {item.itemLevel} ·
                        {(item.weightGrams / 1000).toFixed(1)} kg each ·
                        {(item.totalWeightGrams / 1000).toFixed(1)} kg total ·
                        {item.stackSlots} {item.stackSlots === 1 ? 'stack slot' : 'stack slots'}
                      </p>
                      {#if item.containerCapacity}
                        <p>
                          Container: {item.containerCapacity.usedSlots}/{item.containerCapacity.maxSlots} slots ·
                          {(item.containerCapacity.usedWeightGrams / 1000).toFixed(1)}/
                          {(item.containerCapacity.maxWeightGrams / 1000).toFixed(1)} kg
                        </p>
                      {/if}
                      {#if item.description}<p>{item.description}</p>{/if}
                      {#if item.affix}
                        <p>
                          Modifier: {item.affix.label} ({item.affix.position}) ·
                          attack {item.affix.attackBonusDelta >= 0 ? '+' : ''}{item.affix.attackBonusDelta} ·
                          healing {item.affix.healAmountDelta >= 0 ? '+' : ''}{item.affix.healAmountDelta}
                        </p>
                      {/if}
                      {#if item.loot?.access === 'yours'}
                        <p class="loot-access">
                          Reserved for you · {item.loot.expiresInTicks}
                          {item.loot.expiresInTicks === 1 ? 'tick' : 'ticks'} remaining
                        </p>
                      {:else if item.loot?.access === 'reserved'}
                        <p class="loot-access">Reserved for another adventurer</p>
                      {/if}
                    </div>
                    <div class="item-actions">
                      {#each item.actions as action (action.command)}
                        <button
                          type="button"
                          onclick={() => {
                            if (action.command.trim().toLowerCase().startsWith('loot pass ')) {
                              submitAction('loot.pass', { target: item.id }, action.command);
                            } else {
                              const actionName = itemActionName(action.command);
                              if (actionName === 'item.contents') {
                                submitAction(actionName, { container: item.id, page: 1 }, action.command);
                              } else if (actionName === 'reward.odds') {
                                submitAction(actionName, { target: item.id, page: 1 }, action.command);
                              } else if (actionName) {
                                submitAction(actionName, { target: item.id }, action.command);
                              }
                              else submitCommand(action.command);
                            }
                          }}
                        >{action.label}</button>
                      {/each}
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if scene.npcs.length > 0}
            <section class="scene-items scene-npcs" aria-labelledby="npcs-title">
              <h2 id="npcs-title">Nearby creatures</h2>
              <ul>
                {#each scene.npcs as npc (npc.id)}
                  <li>
                    <div>
                      <strong>{npc.label}</strong>
                      {#if npc.description}<p>{npc.description}</p>{/if}
                      <p class="vital">
                        <span>Health</span>
                        <meter min="0" max={npc.maxHealth} value={npc.health}>{npc.health} / {npc.maxHealth}</meter>
                        <span>{npc.health} / {npc.maxHealth}</span>
                      </p>
                    </div>
                    <div class="item-actions">
                      {#each npc.actions as action (action.command)}
                        <button
                          type="button"
                          onclick={() => action.command.trim().toLowerCase().startsWith('attack ')
                            ? submitAction('combat.attack', { target: npc.id }, action.command)
                            : submitCommand(action.command)}
                        >{action.label}</button>
                      {/each}
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}
        </div>
        <p class="sr-only" aria-live="polite">Current location: {scene.title}</p>
      {:else}
        <div class="empty-scene">
          <p class="eyebrow">Entering the world</p>
          <h1 id="scene-title">Awaiting the first scene…</h1>
          <p>The world will appear here when the server is ready.</p>
        </div>
      {/if}
    </section>

    <div class="side-stack">
      <PlayerDashboard
        value={player}
        {containerContents}
        {tradeContents}
        {rewardOdds}
        {mailList}
        {mailMessage}
        abilityTargets={scene?.npcs ?? []}
        oncommand={submitCommand}
        onaction={submitAction}
        onability={submitAbility}
        onpromote={submitPromotion}
        ontalent={submitTalent}
      />

      <section class="console" aria-labelledby="console-title">
      <div class="console-heading">
        <h2 id="console-title">Command history</h2>
        <div><button type="button" onclick={() => submitCommand('inventory')}>Inventory</button><span>{history.length} / {MAX_SCROLLBACK_ENTRIES}</span></div>
      </div>

      <div
        class="history-scroll"
        role="log"
        aria-label="Command and world event history"
        aria-live="polite"
        aria-relevant="additions text"
        bind:this={historyScroller}
        onscroll={handleHistoryScroll}
      >
        <ol class="history">
          {#if history.length === 0}
            <li class="history-empty">Try <kbd>look</kbd> to examine your surroundings.</li>
          {:else}
            {#each history as entry (entry.id)}
              <li class:failed={entry.state === 'failed'}>
                <div><span aria-hidden="true">›</span> <code>{entry.command}</code></div>
                {#if entry.response}<p>{entry.response}</p>{/if}
                {#if entry.state === 'pending'}<span class="sr-only">Waiting for the server.</span>{/if}
              </li>
            {/each}
          {/if}
        </ol>
      </div>

      {#if !historyAnchored && unseenHistory > 0}
        <div class="jump-latest">
          <button type="button" onclick={jumpToLatest}>
            Jump to latest <span>({unseenHistory} new)</span>
          </button>
        </div>
      {/if}

      <form
        class="command-form"
        onsubmit={(event) => {
          event.preventDefault();
          submitCommand();
        }}
      >
        <label for="command">What do you do?</label>
        <div>
          <span aria-hidden="true">›</span>
          <input
            id="command"
            bind:this={commandInput}
            bind:value={command}
            autocomplete="off"
            autocapitalize="none"
            enterkeyhint="send"
            list="command-suggestions"
            placeholder="look, north, take lantern…"
            onkeydown={navigateCommandHistory}
            oninput={() => (historyCursor = rememberedCommands.length)}
          />
          <datalist id="command-suggestions">
            {#each commandSuggestions as suggestion}
              <option value={suggestion}></option>
            {/each}
          </datalist>
          <button type="submit" disabled={!command.trim()}>Send</button>
        </div>
      </form>
      </section>
    </div>
  </main>
</div>
{/if}
