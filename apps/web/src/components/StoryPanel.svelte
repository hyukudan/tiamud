<script lang="ts">
  import { tick } from 'svelte';
  import type { StoryError } from '../lib/protocol';
  import { presentStory, type AssembledStoryView } from '../lib/story-protocol';

  interface Props {
    view: AssembledStoryView;
    connected: boolean;
    pending: boolean;
    error?: StoryError;
    headingId?: string;
    /** Story text language (S2 resolves Spanish first). */
    lang?: string;
    onchoose: (id: string) => void;
    onmenu: (verb: string, target?: string) => void;
    onundo: () => void;
    onleave: () => void;
  }

  let {
    view,
    connected,
    pending,
    error,
    headingId = 'story-title',
    lang = 'es',
    onchoose,
    onmenu,
    onundo,
    onleave,
  }: Props = $props();

  const story = $derived(presentStory(view));
  const disabled = $derived(!connected || pending);
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
    const focusWasInside = !active || active === document.body ||
      (panel?.contains(active) ?? false);
    if (focusWasInside) void tick().then(() => heading?.focus());
  });

  function handleKey(event: KeyboardEvent): void {
    if (event.altKey || event.ctrlKey || event.metaKey || disabled) return;
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

  {#if error}<p class="story-error" role="alert">{error.message}</p>{/if}

  {#if story.present.length > 0}
    <p class="story-present" {lang}>Present: {story.present.join(', ')}</p>
  {/if}

  {#if story.choices.length > 0 || story.dialogue.length > 0}
    <ol class="story-choices" aria-label={story.dialogue.length > 0 ? 'Dialogue options' : 'Choices'} {lang}>
      {#each [...story.choices, ...story.dialogue] as option, index (option.id)}
        <li>
          <button type="button" {disabled} onclick={() => onchoose(option.id)}>
            <span aria-hidden="true">{index + 1}.</span> {option.label}
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
                </button>
              {/each}
            </fieldset>
          {/if}
        {:else}
          <button type="button" {disabled} onclick={() => onmenu(verb.id)}>{verb.label}</button>
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

  <div class="story-controls">
    <button type="button" disabled={disabled || !view.canUndo} onclick={onundo}>Undo</button>
    <button type="button" disabled={!connected} onclick={onleave}>Leave story</button>
  </div>
  {#if actions.length > 0}
    <p class="story-hint">Press 1–{Math.min(9, actions.length)} to choose.</p>
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

  .story-hint {
    margin: 0;
    font-size: 0.875rem;
  }

  h1:focus {
    outline-offset: 0.25rem;
  }
</style>
