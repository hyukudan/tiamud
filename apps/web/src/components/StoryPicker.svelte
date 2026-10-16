<script lang="ts">
  import { tick } from 'svelte';
  import type { StoryCatalogEntry } from '../lib/generated/protocol';
  import { preferredStoryLocale } from '../lib/story-protocol';

  interface Props {
    /** Stories of `story.catalog`, in server order. */
    stories: StoryCatalogEntry[];
    connected: boolean;
    /** The reader's ordered language preferences (`navigator.languages`). */
    preferred?: readonly string[];
    /** Enters `storyId` in `locale` (one of the story's locales). */
    onenter: (storyId: string, locale: string) => void;
    onclose: () => void;
  }

  let { stories, connected, preferred = [], onenter, onclose }: Props = $props();

  /** Languages the reader picked, by story id; others use the preference. */
  let chosen = $state<Record<string, string>>({});
  let heading = $state<HTMLHeadingElement>();

  const localeOf = (story: StoryCatalogEntry): string => {
    const picked = chosen[story.storyId];
    return picked && story.locales.includes(picked)
      ? picked
      : preferredStoryLocale(story.locales, preferred);
  };

  /** A language named in itself (`español`, `English`), or its tag. */
  function languageName(tag: string): string {
    try {
      const name = new Intl.DisplayNames([tag], { type: 'language' }).of(tag);
      return name && name !== tag ? `${name.charAt(0).toLocaleUpperCase(tag)}${name.slice(1)}` : tag;
    } catch {
      return tag;
    }
  }

  function choose(story: StoryCatalogEntry, event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (story.locales.includes(value)) chosen = { ...chosen, [story.storyId]: value };
  }

  function handleKey(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    onclose();
  }

  // Move keyboard and screen reader users to the picker when it opens.
  $effect(() => {
    void tick().then(() => heading?.focus());
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="story-picker"
  aria-labelledby="story-picker-title"
  onkeydown={handleKey}
>
  <h2 id="story-picker-title" tabindex="-1" bind:this={heading}>Stories</h2>
  <p class="story-picker-help">
    Choose a story and the language to play it in. Your character waits in this room while you
    play. Press Escape to close.
  </p>
  {#if stories.length === 0}
    <p>This server offers no stories right now.</p>
  {:else}
    <ul class="story-picker-list" aria-label="Available stories">
      {#each stories as story (story.storyId)}
        {@const locale = localeOf(story)}
        {@const titleId = `story-picker-${story.storyId}`}
        <li>
          <h3 id={titleId} lang={story.locales[0]}>{story.title}</h3>
          {#if story.locales.length > 1}
            <label for={`${titleId}-locale`}>
              Language<span class="sr-only"> for <span lang={story.locales[0]}>{story.title}</span></span>
            </label>
            <select
              id={`${titleId}-locale`}
              value={locale}
              onchange={(event) => choose(story, event)}
            >
              {#each story.locales as option (option)}
                <option value={option} lang={option}>{languageName(option)}</option>
              {/each}
            </select>
          {:else}
            <p class="story-picker-language">
              Language: <span lang={locale}>{languageName(locale)}</span>
            </p>
          {/if}
          <button
            type="button"
            disabled={!connected}
            aria-describedby={titleId}
            onclick={() => onenter(story.storyId, locale)}
          >Play</button>
        </li>
      {/each}
    </ul>
  {/if}
  <button type="button" class="story-picker-close" onclick={onclose}>Close stories</button>
</section>

<style>
  .story-picker {
    display: grid;
    gap: 0.75rem;
    margin: 0 0 1rem;
    padding: 1rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
  }

  .story-picker h2,
  .story-picker h3,
  .story-picker p {
    margin: 0;
  }

  .story-picker h3 {
    flex: 1 1 100%;
    font-size: 1.05rem;
  }

  .story-picker-list {
    display: grid;
    gap: 0.75rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .story-picker-list li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }

  .story-picker select,
  .story-picker button {
    min-height: 2.75rem;
    padding: 0.4rem 0.7rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
    background: #2f2b1f;
    color: #f3ecd9;
    font: inherit;
  }

  .story-picker button {
    cursor: pointer;
  }

  .story-picker-close {
    justify-self: start;
  }

  h2:focus {
    outline-offset: 0.25rem;
  }
</style>
