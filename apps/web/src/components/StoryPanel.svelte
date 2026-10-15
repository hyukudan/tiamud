<script lang="ts">
  import { tick } from 'svelte';
  import type { StoryError } from '../lib/protocol';
  import type {
    StoryDecisionMode,
    StoryLobby as StoryLobbyState,
    StorySlotSummary,
    StoryTally,
  } from '../lib/generated/protocol';
  import { presentStory, type AssembledStoryView } from '../lib/story-protocol';
  import StoryImage from './StoryImage.svelte';
  import StoryLobby from './StoryLobby.svelte';

  interface Props {
    view: AssembledStoryView;
    connected: boolean;
    pending: boolean;
    error?: StoryError;
    headingId?: string;
    /** Story text language: the instance locale from `story.entered`. */
    lang?: string;
    /**
     * Download reviewed backgrounds and portraits. Off in reader mode, with
     * Save-Data or multimedia off: the background's alt text is shown instead.
     */
    images?: boolean;
    onchoose: (id: string) => void;
    onmenu: (verb: string, target?: string) => void;
    onundo: () => void;
    onleave: () => void;
    /** Save slots of this story (`undefined` until listed). */
    slots?: StorySlotSummary[];
    /** Offer to continue the autosave right after entering. */
    offerContinue?: boolean;
    /** Confirmation of the last slot operation. */
    notice?: string;
    onsave?: (slot: number) => void;
    onrestore?: (slot: number) => void;
    ondeleteslot?: (slot: number) => void;
    onslots?: () => void;
    oncontinue?: () => void;
    ondismisscontinue?: () => void;
    /** Seats and rules while the story is shared (ADR 0033). */
    lobby?: StoryLobbyState;
    tally?: StoryTally;
    deadline?: number;
    /** True when choosing casts a vote instead of acting. */
    votes?: boolean;
    onshare?: () => void;
    oninvite?: (name: string) => boolean;
    onkick?: (name: string) => void;
    onmode?: (mode: StoryDecisionMode, voteSeconds?: number) => void;
    onwithdraw?: () => void;
    /** Sends one parser command line; returns whether it was sent. */
    oncommand?: (text: string) => boolean;
  }

  let {
    view,
    connected,
    pending,
    error,
    headingId = 'story-title',
    lang = 'es',
    images = false,
    onchoose,
    onmenu,
    onundo,
    onleave,
    slots,
    offerContinue = false,
    notice,
    onsave,
    onrestore,
    ondeleteslot,
    onslots,
    oncontinue,
    ondismisscontinue,
    lobby,
    tally,
    deadline,
    votes = false,
    onshare,
    oninvite,
    onkick,
    onmode,
    onwithdraw,
    oncommand,
  }: Props = $props();

  /** Commands typed in this panel, oldest first (bounded). */
  const MAX_COMMAND_HISTORY = 50;
  let commandText = $state('');
  let commandInput = $state<HTMLInputElement>();
  let commandHistory = $state<string[]>([]);
  let historyIndex = $state<number>();

  function submitCommand(event: SubmitEvent): void {
    event.preventDefault();
    const line = commandText.trim();
    if (!line || !oncommand?.(line)) return;
    if (commandHistory[commandHistory.length - 1] !== line) {
      commandHistory = [...commandHistory, line].slice(-MAX_COMMAND_HISTORY);
    }
    historyIndex = undefined;
    commandText = '';
  }

  function handleCommandKey(event: KeyboardEvent): void {
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
    if (commandHistory.length === 0) return;
    event.preventDefault();
    if (event.key === 'ArrowUp') {
      historyIndex = historyIndex === undefined
        ? commandHistory.length - 1
        : Math.max(0, historyIndex - 1);
      commandText = commandHistory[historyIndex];
    } else if (historyIndex !== undefined) {
      historyIndex += 1;
      if (historyIndex >= commandHistory.length) {
        historyIndex = undefined;
        commandText = '';
      } else {
        commandText = commandHistory[historyIndex];
      }
    }
  }

  const spectating = $derived(lobby?.you === 'spectator');
  /** Votes of the current view by opaque leaf id. */
  const voteCounts = $derived(new Map(
    tally && tally.viewRev === view.viewRev
      ? tally.choices.map((entry) => [entry.choiceId, entry.votes] as const)
      : [],
  ));
  const yourVote = $derived(tally?.viewRev === view.viewRev ? tally.yourVote : undefined);
  const voteLabel = (id: string): string => {
    const count = voteCounts.get(id) ?? 0;
    return count === 0 ? '' : count === 1 ? '1 vote' : `${count} votes`;
  };

  /** Slot rows: the autosave (0) then the nine manual slots. */
  const SLOT_NUMBERS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
  let savesOpen = $state(false);
  let savesToggle = $state<HTMLButtonElement>();
  let savesHeading = $state<HTMLHeadingElement>();
  // Save slots of a shared story belong to its host.
  const slotsAvailable = $derived(Boolean(onsave && onrestore) && (!lobby || lobby.you === 'host'));
  const autosave = $derived(slots?.find((slot) => slot.slot === 0));

  /** Deterministic, locale-independent timestamp. */
  function formatSaved(seconds: number): string {
    return `${new Date(seconds * 1000).toISOString().slice(0, 16).replace('T', ' ')} UTC`;
  }

  function slotFor(slot: number): StorySlotSummary | undefined {
    return slots?.find((entry) => entry.slot === slot);
  }

  function openSaves(): void {
    savesOpen = true;
    onslots?.();
    void tick().then(() => savesHeading?.focus());
  }

  function closeSaves(): void {
    savesOpen = false;
    void tick().then(() => savesToggle?.focus());
  }

  function handleSavesKey(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    closeSaves();
  }

  const story = $derived(presentStory(view));
  const disabled = $derived(!connected || pending || spectating);
  /** Numbered actions in the same order Telnet numbers them. */
  const actions = $derived([
    ...story.choices.map((choice) => ({ label: choice.label, run: () => onchoose(choice.id) })),
    ...story.dialogue.map((option) => ({ label: option.label, run: () => onchoose(option.id) })),
    ...story.verbs.flatMap((verb) => verb.needsTarget
      ? verb.targets.map((target) => ({
          label: `${verb.label} → ${target.label}`,
          run: () => onmenu(verb.id, target.id),
        }))
      : [{ label: verb.label, run: () => onmenu(verb.id) }]),
  ]);
  const endingLabel = $derived(
    story.ending === 'victory' ? 'Victory' : story.ending === 'defeat' ? 'Defeat' : 'The end',
  );

  let heading = $state<HTMLHeadingElement>();
  let panel = $state<HTMLElement>();
  let shownRevision = 0;

  // Keep keyboard users oriented: when a new view replaces the button they
  // pressed, move focus to the new heading instead of losing it to <body>.
  $effect(() => {
    const revision = view.viewRev;
    if (revision === shownRevision) return;
    shownRevision = revision;
    const active = typeof document === 'undefined' ? null : document.activeElement;
    // Typing parser commands keeps the focus in the command field.
    if (active && active === commandInput) return;
    const focusWasInside = !active || active === document.body ||
      (panel?.contains(active) ?? false);
    if (focusWasInside) void tick().then(() => heading?.focus());
  });

  function handleKey(event: KeyboardEvent): void {
    if (event.altKey || event.ctrlKey || event.metaKey || disabled || savesOpen) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (!/^[1-9]$/.test(event.key)) return;
    const action = actions[Number(event.key) - 1];
    if (!action) return;
    event.preventDefault();
    action.run();
  }
</script>

<svelte:window onkeydown={handleKey} />

<section class="story-panel" aria-labelledby={headingId} bind:this={panel}>
  <p class="eyebrow">Story · turn {view.turn} · score {story.score}</p>
  <h1 id={headingId} tabindex="-1" bind:this={heading} {lang}>{story.title}</h1>

  {#if story.background}
    {#if images}
      {#key story.background.manifest}
        <StoryImage image={story.background} kind="background" fallbackText {lang} />
      {/key}
    {:else}
      <p class="story-image-text" {lang}>{story.background.alt}</p>
    {/if}
  {/if}

  {#if offerContinue && autosave && oncontinue}
    <div class="story-continue" role="group" aria-labelledby="story-continue-text">
      <p id="story-continue-text">
        You have an autosaved game from {formatSaved(autosave.updatedAt)}.
      </p>
      <button type="button" {disabled} onclick={oncontinue}>Continue saved game</button>
      <button type="button" disabled={!connected} onclick={() => ondismisscontinue?.()}>Start over</button>
    </div>
  {/if}

  <div class="story-text" aria-live="polite" {lang}>
    {#if story.events.length > 0}
      <ul class="story-events" aria-label="What just happened">
        {#each story.events as event, index (index)}<li>{event}</li>{/each}
      </ul>
    {/if}
    {#each story.paragraphs as paragraph, index (index)}<p>{paragraph}</p>{/each}
    {#each story.lines as line, index (index)}
      <p class="story-line">
        {#if line.speaker}<strong>{line.speaker}:</strong> {/if}{line.text}
      </p>
    {/each}
    {#if story.ending}
      <p class="story-ending" role="status"><strong>{endingLabel}.</strong> Final score: {story.score}.</p>
    {/if}
  </div>

  {#if story.command || story.replies.length > 0}
    <div class="story-replies" role="log" aria-live="polite" aria-label="Story replies" {lang}>
      {#if story.command}
        <p class="story-command"><span aria-hidden="true">&gt;</span> {story.command}</p>
      {/if}
      {#each story.replies as reply, index (index)}
        {#each reply.split('\n').filter((line) => line.trim().length > 0) as line, part (part)}
          <p class="story-reply">{line}</p>
        {/each}
      {/each}
    </div>
  {/if}

  {#if error}<p class="story-error" role="alert">{error.message}</p>{/if}

  {#if story.prompt && oncommand && !spectating}
    <form class="story-prompt" onsubmit={submitCommand}>
      <label for="story-command">
        {story.prompt.question ? 'Answer the question' : 'Your command'}
      </label>
      <div class="story-prompt-row">
        <span class="story-prompt-symbol" aria-hidden="true">{story.prompt.text}</span>
        <input
          id="story-command"
          type="text"
          bind:this={commandInput}
          bind:value={commandText}
          onkeydown={handleCommandKey}
          maxlength="256"
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          readonly={!connected}
          aria-describedby="story-command-help"
          {lang}
        />
        <button type="submit" disabled={!connected || pending}>Send</button>
      </div>
      <p id="story-command-help" class="story-hint">
        Type what you do, for example «mira», «coge la llave» or «n». Up and Down recall earlier
        commands.
      </p>
    </form>
  {/if}

  {#if lobby}
    <StoryLobby
      {lobby}
      {tally}
      {deadline}
      {connected}
      {oninvite}
      {onkick}
      {onmode}
      {onwithdraw}
    />
    {#if spectating}<p class="story-hint">You are watching as a spectator.</p>{/if}
  {/if}

  {#if story.present.length > 0}
    {#if images && story.portraits.some((portrait) => portrait.image)}
      <ul class="story-portraits" aria-label="Portraits">
        {#each story.portraits as portrait, index (index)}
          {#if portrait.image}
            <li>
              {#key portrait.image.manifest}
                <StoryImage image={portrait.image} kind="portrait" {lang} />
              {/key}
            </li>
          {/if}
        {/each}
      </ul>
    {/if}
    <p class="story-present" {lang}>Present: {story.present.join(', ')}</p>
  {/if}

  {#if story.choices.length > 0 || story.dialogue.length > 0}
    <ol class="story-choices" aria-label={story.dialogue.length > 0 ? 'Dialogue options' : 'Choices'} {lang}>
      {#each [...story.choices, ...story.dialogue] as option, index (option.id)}
        <li>
          <button
            type="button"
            {disabled}
            aria-pressed={votes ? yourVote === option.id : undefined}
            onclick={() => onchoose(option.id)}
          >
            <span aria-hidden="true">{index + 1}.</span> {option.label}
            {#if voteLabel(option.id)}<span class="story-votes" lang="en">· {voteLabel(option.id)}</span>{/if}
          </button>
        </li>
      {/each}
    </ol>
  {/if}

  {#if story.verbs.length > 0}
    <div class="story-menu" role="group" aria-label="Actions" {lang}>
      {#each story.verbs as verb (verb.id)}
        {#if verb.needsTarget}
          {#if verb.targets.length > 0}
            <fieldset>
              <legend>{verb.label}</legend>
              {#each verb.targets as target (target.id)}
                <button type="button" {disabled} onclick={() => onmenu(verb.id, target.id)}>
                  {target.label}
                  {#if voteLabel(target.id)}<span class="story-votes" lang="en">· {voteLabel(target.id)}</span>{/if}
                </button>
              {/each}
            </fieldset>
          {/if}
        {:else}
          <button type="button" {disabled} onclick={() => onmenu(verb.id)}>
            {verb.label}
            {#if voteLabel(verb.id)}<span class="story-votes" lang="en">· {voteLabel(verb.id)}</span>{/if}
          </button>
        {/if}
      {/each}
    </div>
  {/if}

  {#if story.stats.length > 0 || story.inventory.length > 0}
    <dl class="story-sheet" {lang}>
      {#each story.stats as stat (stat.label)}
        <div><dt>{stat.label}</dt><dd>{stat.value}</dd></div>
      {/each}
      <div><dt>Inventory</dt><dd>{story.inventory.length > 0 ? story.inventory.join(', ') : 'Empty'}</dd></div>
    </dl>
  {/if}

  {#if notice}<p class="story-notice" role="status">{notice}</p>{/if}

  <div class="story-controls">
    <button type="button" disabled={disabled || !view.canUndo} onclick={onundo}>Undo</button>
    {#if onshare && !lobby}
      <button type="button" disabled={!connected || pending} onclick={onshare}>Share story</button>
    {/if}
    {#if slotsAvailable}
      <button
        type="button"
        bind:this={savesToggle}
        aria-expanded={savesOpen}
        aria-controls="story-saves"
        disabled={!connected}
        onclick={() => (savesOpen ? closeSaves() : openSaves())}
      >Saved games</button>
    {/if}
    <button type="button" disabled={!connected} onclick={onleave}>Leave story</button>
  </div>

  {#if slotsAvailable && savesOpen}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <section
      id="story-saves"
      class="story-saves"
      aria-labelledby="story-saves-title"
      onkeydown={handleSavesKey}
    >
      <h2 id="story-saves-title" tabindex="-1" bind:this={savesHeading}>Saved games</h2>
      <p class="story-saves-help">
        Slot 0 is the autosave written when you leave. Loading replaces the current game; undo
        history is not saved. Press Escape to close.
      </p>
      {#if slots === undefined}
        <p>Loading saved games…</p>
      {:else}
        <ul class="story-slot-list" aria-label="Save slots">
          {#each SLOT_NUMBERS as number (number)}
            {@const entry = slotFor(number)}
            {@const name = number === 0 ? 'Autosave' : `Slot ${number}`}
            <li>
              <span class="story-slot-name">{name}</span>
              <span class="story-slot-state">
                {#if entry}
                  {formatSaved(entry.updatedAt)}{#if !entry.compatible} · older story version, cannot be loaded{/if}
                {:else}
                  Empty
                {/if}
              </span>
              {#if number > 0}
                <button type="button" {disabled} aria-label={`Save to ${name}`} onclick={() => onsave?.(number)}>Save</button>
              {/if}
              <button
                type="button"
                disabled={disabled || !entry?.compatible}
                aria-label={`Load ${name}`}
                onclick={() => onrestore?.(number)}
              >Load</button>
              <button
                type="button"
                disabled={disabled || !entry}
                aria-label={`Delete ${name}`}
                onclick={() => ondeleteslot?.(number)}
              >Delete</button>
            </li>
          {/each}
        </ul>
      {/if}
      <button type="button" onclick={closeSaves}>Close saved games</button>
    </section>
  {/if}
  {#if actions.length > 0 && !spectating}
    <p class="story-hint">
      Press 1–{Math.min(9, actions.length)} to {votes ? 'vote' : 'choose'}.
    </p>
  {/if}
</section>

<style>
  .story-panel {
    display: grid;
    gap: 1rem;
    padding: clamp(1rem, 3vw, 2rem);
  }

  .story-text p {
    margin: 0 0 0.75rem;
    line-height: 1.6;
  }

  .story-image-text {
    margin: 0;
    font-size: 0.875rem;
    font-style: italic;
  }

  .story-portraits {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .story-events {
    margin: 0 0 0.75rem;
    padding-left: 1.25rem;
    font-style: italic;
  }

  .story-choices {
    display: grid;
    gap: 0.5rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .story-panel button {
    padding: 0.5rem 0.8rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
    background: #2f2b1f;
    color: #f3ecd9;
    cursor: pointer;
  }

  .story-choices button,
  .story-menu button {
    min-height: 2.75rem;
    text-align: start;
  }

  .story-choices button {
    width: 100%;
  }

  .story-menu {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .story-menu fieldset {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0;
  }

  .story-sheet {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1.5rem;
    margin: 0;
  }

  .story-sheet dd {
    margin: 0;
  }

  .story-controls {
    display: flex;
    gap: 0.5rem;
  }

  .story-continue,
  .story-saves {
    display: grid;
    gap: 0.5rem;
    padding: 0.75rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
  }

  .story-continue p,
  .story-saves p,
  .story-saves h2 {
    margin: 0;
  }

  .story-slot-list {
    display: grid;
    gap: 0.4rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .story-slot-list li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }

  .story-slot-name {
    min-width: 5.5rem;
    font-weight: 600;
  }

  .story-slot-state {
    flex: 1 1 12rem;
  }

  .story-votes {
    margin-inline-start: 0.4rem;
    font-size: 0.875rem;
    font-style: italic;
  }

  .story-choices button[aria-pressed='true'] {
    outline: 2px solid #d9c27a;
  }

  .story-replies p {
    margin: 0 0 0.5rem;
    line-height: 1.6;
  }

  .story-command {
    font-family: ui-monospace, monospace;
    opacity: 0.85;
  }

  .story-prompt {
    display: grid;
    gap: 0.35rem;
  }

  .story-prompt-row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }

  .story-prompt-symbol {
    font-family: ui-monospace, monospace;
  }

  .story-prompt input {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 2.75rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
    background: #1d1a12;
    color: #f3ecd9;
    font: inherit;
  }

  .story-notice,
  .story-hint {
    margin: 0;
    font-size: 0.875rem;
  }

  h1:focus {
    outline-offset: 0.25rem;
  }
</style>
