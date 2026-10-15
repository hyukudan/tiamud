<script lang="ts">
  import { browserStorage } from './lib/storage';
  import { onMount, tick } from 'svelte';
  import type { SceneEntityVisualSource, SceneSnapshot } from './lib/protocol';
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
  import StoryPanel from './components/StoryPanel.svelte';
  import { PRODUCT_NAME } from './lib/brand';
  import type { CharacterSelection } from './lib/auth';
  import {
    loadPreferences,
    motionReduced,
    multimediaAllowed,
    savePreferences,
    type ClientPreferences,
  } from './lib/preferences';
  import { AudioController } from './lib/audio';
  import {
    MAX_SCROLLBACK_ENTRIES,
    isScrollAnchored,
  } from './lib/scrollback';
  import {
    layoutSceneEntities,
    type VisualPlacement,
  } from './lib/scene-layout';
  import {
    visualSourceForCue,
    type ActiveScenePresentationCue,
  } from './lib/scene-presentation';
  import { capabilitiesFor } from './lib/app/capabilities';
  import { connectionLabelFor, itemActionName } from './lib/app/history-format';
  import { directVisualSource, visualSourceUrl } from './lib/app/visual-source';
  import { computeCommandSuggestions } from './lib/app/command-suggestions';
  import {
    GameSession,
    type HistoryView,
    type VisualSceneEntity,
  } from './lib/app/game-session.svelte';
  import { loadPlayerDashboard, type PlayerDashboardComponent } from './lib/app/lazy-dashboard';
  import { keepCommandVisible } from './lib/app/visual-viewport';

  interface RenderedVisualEntity {
    entity: VisualSceneEntity;
    placement: VisualPlacement;
    source: SceneEntityVisualSource;
    cue?: ActiveScenePresentationCue;
  }

  let manifestImage = $state<SceneSnapshot['image']>();
  let activeAssetKey: string | undefined;
  let failedImageUrls = $state<string[]>([]);
  let commandInput = $state<HTMLInputElement>();
  let commandForm = $state<HTMLFormElement>();
  let historyScroller = $state<HTMLDivElement>();
  let historyAnchored = $state(true);
  let unseenHistory = $state(0);
  let PlayerDashboard = $state<PlayerDashboardComponent>();
  let dashboardLoadFailed = $state(false);
  const browserSaveData = typeof navigator !== 'undefined' &&
    (navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData === true;
  const initialPreferences = loadPreferences(
    browserStorage(),
    browserSaveData,
  );
  let preferences = $state(initialPreferences);
  const systemReducedMotionNow = (): boolean => typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const audioController = typeof Audio === 'undefined'
    ? undefined
    : new AudioController((url) => new Audio(url), initialPreferences, {
        systemReducedMotion: systemReducedMotionNow,
      });

  const historyView: HistoryView = {
    beforeReplace() {
      const scroller = historyScroller;
      const wasAnchored = scroller ? isScrollAnchored(scroller) : true;
      historyAnchored = wasAnchored;
      return wasAnchored;
    },
    afterReplace(wasAnchored, activity) {
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
    },
    reset() {
      historyAnchored = true;
      unseenHistory = 0;
    },
  };

  const session = new GameSession({
    storage: browserStorage,
    capabilities: () => capabilitiesFor(preferences, systemReducedMotionNow()),
    audio: audioController,
    historyView,
  });

  const showMultimedia = $derived(multimediaAllowed(preferences));
  const systemReducedMotion = systemReducedMotionNow();
  const reduceVisualMotion = $derived(motionReduced(preferences, systemReducedMotion));

  const connectionLabel = $derived(connectionLabelFor(session.connectionState));
  const browserLocale = typeof navigator === 'undefined' ? 'en' : navigator.language;
  const localizedSceneImage = $derived(
    session.scene?.image
      ? {
          ...session.scene.image,
          alt: localizedImageAlt(session.scene.image, browserLocale) ?? session.scene.image.alt,
        }
      : undefined,
  );
  const directImageUrl = $derived(imageUrl(session.scene?.image));
  const manifestUrl = $derived(directImageUrl ? undefined : session.scene?.image?.manifest);
  const sceneAssetKey = $derived(showMultimedia ? (directImageUrl ?? manifestUrl) : undefined);
  const atmosphereImage = $derived(
    showMultimedia
      ? imageAfterFailures((directImageUrl ? localizedSceneImage : undefined) ?? manifestImage, failedImageUrls)
      : undefined,
  );
  const imageRenderKey = $derived(`${sceneAssetKey ?? 'none'}:${failedImageUrls.join('|')}`);
  const visualLayout = $derived.by(() =>
    showMultimedia && Boolean(atmosphereImage)
      ? layoutSceneEntities(session.visualEntities.map((entity) => ({
          id: entity.id,
          priority: entity.visual.priority,
          footprint: entity.visual.footprint,
          preferredAnchor: entity.visual.preferredAnchor,
        })), { maxVisible: 10 })
      : { placements: [] as VisualPlacement[], overflow: [] as string[] },
  );
  const renderedVisualEntities = $derived.by((): RenderedVisualEntity[] =>
    visualLayout.placements.flatMap((placement) => {
      const entity = session.visualEntities.find((candidate) => candidate.id === placement.id);
      if (!entity) return [];
      const cue = session.activeVisualCues[entity.id];
      const requested = visualSourceForCue(entity.visual, cue);
      const source = directVisualSource(requested) ?? directVisualSource(entity.visual.poses.idle);
      return source ? [{ entity, placement, source, cue }] : [];
    }),
  );
  const overflowVisualEntities = $derived(visualLayout.overflow.slice(0, 5).flatMap((id) =>
    session.visualEntities.find((entity) => entity.id === id) ?? [],
  ));
  const commandSuggestions = $derived.by(() =>
    computeCommandSuggestions(
      session.scene?.exits.map((exit) => exit.command) ?? [],
      session.rememberedCommands,
    ),
  );

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
    savePreferences(browserStorage(), current);
  });

  // Keep the fixed mobile command bar above the on-screen keyboard.
  $effect(() => {
    if (!session.hasEntered || !commandForm || typeof window === 'undefined') return;
    return keepCommandVisible(window, commandForm);
  });

  function updatePreferences(next: ClientPreferences): void {
    if (next.audio && !preferences.audio) audioController?.unlockFromGesture();
    audioController?.updatePreferences(next);
    preferences = next;
    session.updateCapabilities(capabilitiesFor(next, systemReducedMotionNow()));
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
        behavior: motionReduced(preferences, systemReducedMotionNow()) ? 'auto' : 'smooth',
      });
    } else {
      historyScroller.scrollTop = top;
    }
    historyAnchored = true;
    unseenHistory = 0;
  }

  function requestDashboard(): void {
    if (PlayerDashboard) return;
    dashboardLoadFailed = false;
    void loadPlayerDashboard().then(
      (component) => {
        PlayerDashboard = component;
      },
      () => {
        dashboardLoadFailed = true;
      },
    );
  }

  function enterWorld(selection: CharacterSelection): void {
    requestDashboard();
    if (session.enterWorld(selection)) {
      void tick().then(() => commandInput?.focus());
    }
  }

  onMount(() => {
    // Warm the dashboard chunk while the reader is still on the welcome gate,
    // without competing with first paint.
    // Safari has no requestIdleCallback; fall back to a short timer.
    const idle = typeof window.requestIdleCallback === 'function';
    const handle = idle
      ? window.requestIdleCallback(requestDashboard, { timeout: 3_000 })
      : window.setTimeout(requestDashboard, 1_500);
    return () => {
      if (idle) window.cancelIdleCallback(handle);
      else window.clearTimeout(handle);
      session.dispose();
    };
  });
</script>

<svelte:head>
  <title>{PRODUCT_NAME}</title>
</svelte:head>

{#if !session.hasEntered}
  <WelcomeGate onenter={() => undefined} showArtwork={showMultimedia}>
    {#snippet access(_enterWorld)}
      {#if session.accessError}<p class="access-error" role="alert">{session.accessError}</p>{/if}
      <AccountAccess onselect={enterWorld} />
      <PreferencesPanel value={preferences} onchange={updatePreferences} />
    {/snippet}
  </WelcomeGate>
{:else}
<div class="shell">
  <header class="topbar">
    <a class="brand" href="/" aria-label={`${PRODUCT_NAME} home`}>{PRODUCT_NAME}</a>
    <div class="session-controls">
      {#if session.selectedAccess}<span class="character-name">{session.selectedAccess.characterName}</span>{/if}
      <p class:connected={session.connectionState === 'connected'} class="connection" role="status">
        <span aria-hidden="true"></span>{connectionLabel}
      </p>
      <button type="button" onclick={() => void session.signOut()}>Sign out</button>
      <PreferencesPanel value={preferences} onchange={updatePreferences} />
    </div>
  </header>

  {#if session.notice}
    <aside class:warning={session.notice.level === 'warning'} class:error={session.notice.level === 'error'} class="notice">
      <p role={session.notice.level === 'error' ? 'alert' : 'status'}>{session.notice.message}</p>
      <button aria-label="Dismiss notice" onclick={session.dismissNotice}>×</button>
    </aside>
  {/if}

  <main>
    <section class="scene" aria-labelledby="scene-title">
      {#if session.story}
        <!-- While absorbed in a story the character stays parked in its room. -->
        <StoryPanel
          view={session.story}
          connected={session.connectionState === 'connected'}
          pending={session.storyPending}
          error={session.storyError}
          headingId="scene-title"
          onchoose={session.chooseStory}
          onmenu={session.storyMenu}
          onundo={session.undoStory}
          onleave={session.leaveStory}
        />
      {:else if session.scene}
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
          <h1 id="scene-title">{session.scene.title}</h1>
          <p class="scene-weather">
            <span>{session.scene.weather.condition}</span>
            <span aria-hidden="true">·</span>
            <span>Day {session.scene.weather.day}, hour {session.scene.weather.hour}</span>
            <span class="sr-only">Weather may change in {session.scene.weather.nextChangeInTicks} world ticks.</span>
          </p>
          {#each session.scene.prose as paragraph}
            <p>{paragraph}</p>
          {/each}

          {#if session.scene.exits.length > 0}
            <nav class="exits" aria-label="Exits">
              <h2>Ways onward</h2>
              <ul>
                {#each session.scene.exits as exit (exit.id)}
                  <li><button onclick={() => session.submitAction('move', { target: exit.command }, exit.command)}>{exit.label}</button></li>
                {/each}
              </ul>
            </nav>
          {/if}

          {#if session.scene.items.length > 0}
            <section class="scene-items" aria-labelledby="items-title">
              <h2 id="items-title">Within reach</h2>
              <button
                class="loot-status-button"
                type="button"
                onclick={() => session.submitAction('loot.status', undefined, 'loot status')}
              >Review loot rights</button>
              <ul>
                {#each session.scene.items as item (item.id)}
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
                              session.submitAction('loot.pass', { target: item.id }, action.command);
                            } else {
                              const actionName = itemActionName(action.command);
                              if (actionName === 'item.contents') {
                                session.submitAction(actionName, { container: item.id, page: 1 }, action.command);
                              } else if (actionName === 'reward.odds') {
                                session.submitAction(actionName, { target: item.id, page: 1 }, action.command);
                              } else if (actionName) {
                                session.submitAction(actionName, { target: item.id }, action.command);
                              }
                              else session.submitCommand(action.command);
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

          {#if session.scene.npcs.length > 0}
            <section class="scene-items scene-npcs" aria-labelledby="npcs-title">
              <h2 id="npcs-title">Nearby creatures</h2>
              <ul>
                {#each session.scene.npcs as npc (npc.id)}
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
                            ? session.submitAction('combat.attack', { target: npc.id }, action.command)
                            : session.submitCommand(action.command)}
                        >{action.label}</button>
                      {/each}
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}
        </div>
        <p class="sr-only" aria-live="polite">Current location: {session.scene.title}</p>
      {:else}
        <div class="empty-scene">
          <p class="eyebrow">Entering the world</p>
          <h1 id="scene-title">Awaiting the first scene…</h1>
          <p>The world will appear here when the server is ready.</p>
        </div>
      {/if}
    </section>

    <div class="side-stack">
      {#if PlayerDashboard}
      <PlayerDashboard
        value={session.player}
        containerContents={session.containerContents}
        tradeContents={session.tradeContents}
        rewardOdds={session.rewardOdds}
        mailList={session.mailList}
        mailMessage={session.mailMessage}
        boardCatalog={session.boardCatalog}
        boardList={session.boardList}
        boardEntry={session.boardEntry}
        actionResult={session.lastActionResult}
        lootRoll={session.lootRoll}
        connectionState={session.connectionState}
        abilityTargets={session.scene?.npcs ?? []}
        oncommand={session.submitCommand}
        onaction={session.submitAction}
        onability={session.submitAbility}
        onpromote={session.submitPromotion}
        ontalent={session.submitTalent}
      />
      {:else}
        <section class="dashboard-loading" aria-label="Character panels" aria-busy={!dashboardLoadFailed}>
          {#if dashboardLoadFailed}
            <p role="alert">The character panels could not be loaded.</p>
            <button type="button" onclick={requestDashboard}>Retry loading panels</button>
          {:else}
            <p role="status">Loading character panels…</p>
          {/if}
        </section>
      {/if}

      <section class="console" aria-labelledby="console-title">
      <div class="console-heading">
        <h2 id="console-title">Command history</h2>
        <div><button type="button" onclick={() => session.submitCommand('inventory')}>Inventory</button><span>{session.history.length} / {MAX_SCROLLBACK_ENTRIES}</span></div>
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
          {#if session.history.length === 0}
            <li class="history-empty">Try <kbd>look</kbd> to examine your surroundings.</li>
          {:else}
            {#each session.history as entry (entry.id)}
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
        bind:this={commandForm}
        class="command-form"
        onsubmit={(event) => {
          event.preventDefault();
          session.submitCommand();
        }}
      >
        <label for="command">What do you do?</label>
        <div>
          <span aria-hidden="true">›</span>
          <input
            id="command"
            bind:this={commandInput}
            bind:value={session.command}
            autocomplete="off"
            autocapitalize="none"
            enterkeyhint="send"
            list="command-suggestions"
            placeholder="look, north, take lantern…"
            onkeydown={session.navigateCommandHistory}
            oninput={session.resetHistoryCursor}
          />
          <datalist id="command-suggestions">
            {#each commandSuggestions as suggestion}
              <option value={suggestion}></option>
            {/each}
          </datalist>
          <button type="submit" disabled={!session.command.trim()}>Send</button>
        </div>
      </form>
      </section>
    </div>
  </main>
</div>
{/if}
