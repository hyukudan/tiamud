<script lang="ts">
  import { onMount } from 'svelte';
  import type {
    ActionResultData,
    ConnectionState,
    PlayerStateSnapshot,
    SceneSnapshot,
    ServerMessage,
    ServerNoticeData,
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
    type CharacterSelection,
  } from './lib/auth';
  import { TiamudConnection, websocketUrl } from './lib/socket';
  import {
    loadPreferences,
    motionReduced,
    multimediaAllowed,
    savePreferences,
    type ClientPreferences,
  } from './lib/preferences';
  import { loadCommandHistory, rememberCommand } from './lib/history';
  import { AudioController } from './lib/audio';

  interface HistoryEntry {
    id: string;
    command: string;
    state: 'pending' | 'complete' | 'failed';
    response?: string;
  }

  let connectionState = $state<ConnectionState>('connecting');
  let hasEntered = $state(false);
  let scene = $state<SceneSnapshot>();
  let player = $state<PlayerStateSnapshot>();
  let manifestImage = $state<SceneSnapshot['image']>();
  let activeAssetKey = $state<string>();
  let failedImageUrls = $state<string[]>([]);
  let notice = $state<ServerNoticeData>();
  let command = $state('');
  let history = $state<HistoryEntry[]>([]);
  let eventCounter = 0;
  let connection: TiamudConnection | undefined;
  let selectedAccess = $state<CharacterSelection>();
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
  const commandSuggestions = $derived.by(() => {
    const commands = [
      'look',
      'inventory',
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
  }

  function handleActionResult(result: ActionResultData): void {
    if (!result.requestId) {
      if (result.message) {
        notice = { level: result.ok ? 'info' : 'error', message: result.message };
      }
      return;
    }

    history = history.map((entry) =>
      entry.id === result.requestId
        ? {
            ...entry,
            state: result.ok ? 'complete' : 'failed',
            response: result.message ?? (result.ok ? 'Done.' : 'The action failed.'),
          }
        : entry,
    );
  }

  function handleMessage(message: ServerMessage): void {
    switch (message.type) {
      case 'server.welcome':
        if (message.data.motd) {
          notice = { level: 'info', message: message.data.motd };
        }
        break;
      case 'scene.snapshot':
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
          };
        }
        break;
      case 'map.snapshot':
        if (player && player.revision <= message.data.revision) {
          player = { ...player, revision: message.data.revision, map: message.data.map };
        }
        break;
      case 'sync.snapshot':
        scene = message.data.scene;
        player = message.data.player;
        break;
      case 'message.append':
        eventCounter += 1;
        history = [
          ...history,
          {
            id: `event-${message.seq ?? eventCounter}-${eventCounter}`,
            command: 'World event',
            state: 'complete' as const,
            response: message.data.text,
          },
        ].slice(-50);
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
        history = [
          ...history.slice(-49),
          {
            id: `chat-${message.seq ?? crypto.randomUUID()}`,
            command: message.data.channel === 'tell'
              ? `${message.data.sender} tells you`
              : `${message.data.sender} says`,
            state: 'complete',
            response: message.data.text,
          },
        ];
        break;
    }
  }

  function submitCommand(value = command): void {
    const submitted = value.trim();
    if (!submitted) {
      return;
    }

    const id = crypto.randomUUID();
    const sent = connection?.submit(submitted, id) ?? false;
    recordSubmission(submitted, id, sent);
  }

  function submitAction(action: string, target: string | undefined, label: string): void {
    const id = crypto.randomUUID();
    const sent = connection?.perform(action, target, id) ?? false;
    recordSubmission(label, id, sent);
  }

  function recordSubmission(submitted: string, id: string, sent: boolean): void {

    history = [
      ...history.slice(-49),
      {
        id,
        command: submitted,
        state: sent ? 'pending' : 'failed',
        response: sent ? undefined : 'The command was not sent because the server is unavailable.',
      },
    ];
    rememberedCommands = rememberCommand(
      rememberedCommands,
      submitted,
      typeof window === 'undefined' ? undefined : window.localStorage,
    );
    historyCursor = rememberedCommands.length;
    command = '';
  }

  function itemActionName(commandValue: string): string | undefined {
    const verb = commandValue.trim().split(/\s+/, 1)[0]?.toLowerCase();
    return verb && ['take', 'drop', 'examine', 'use', 'equip', 'unequip'].includes(verb)
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
    selectedAccess = selection;
    if (!connection) {
      connection = new TiamudConnection(
        websocketUrl(window.location),
        { token: selection.token, characterId: selection.characterId },
        {
          onMessage: handleMessage,
          onStateChange: (state) => {
            connectionState = state;
          },
          onProtocolError: () => {
            notice = { level: 'warning', message: 'The server sent a message this client could not read.' };
          },
        },
        {
          images: showMultimedia,
          reducedMotion: motionReduced(
            preferences,
            window.matchMedia('(prefers-reduced-motion: reduce)').matches,
          ),
          audio: preferences.audio,
        },
      );
      connection.connect();
    }

    hasEntered = true;
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
    player = undefined;
    notice = undefined;
    history = [];
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
    };
  });
</script>

<svelte:head>
  <title>{PRODUCT_NAME}</title>
</svelte:head>

{#if !hasEntered}
  <WelcomeGate onenter={() => undefined} showArtwork={showMultimedia}>
    {#snippet access(_enterWorld)}
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
                  <li><button onclick={() => submitAction('move', exit.command, exit.command)}>{exit.label}</button></li>
                {/each}
              </ul>
            </nav>
          {/if}

          {#if scene.items.length > 0}
            <section class="scene-items" aria-labelledby="items-title">
              <h2 id="items-title">Within reach</h2>
              <ul>
                {#each scene.items as item (item.id)}
                  <li>
                    <div>
                      <strong>{item.label}</strong>
                      {#if item.description}<p>{item.description}</p>{/if}
                    </div>
                    <div class="item-actions">
                      {#each item.actions as action (action.command)}
                        <button
                          type="button"
                          onclick={() => {
                            const actionName = itemActionName(action.command);
                            if (actionName) submitAction(actionName, item.id, action.command);
                            else submitCommand(action.command);
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
                            ? submitAction('combat.attack', npc.id, action.command)
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
      <PlayerDashboard value={player} oncommand={submitCommand} onaction={submitAction} />

      <section class="console" aria-labelledby="console-title">
      <div class="console-heading">
        <h2 id="console-title">Command history</h2>
        <div><button type="button" onclick={() => submitCommand('inventory')}>Inventory</button><span>{history.length} / 50</span></div>
      </div>

      <div role="log" aria-label="Command and world event history" aria-live="polite" aria-relevant="additions text">
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
