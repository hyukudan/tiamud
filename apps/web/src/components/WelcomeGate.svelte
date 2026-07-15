<script lang="ts">
  import type { Snippet } from 'svelte';
  import { PRODUCT_NAME } from '../lib/brand';
  import { ENTRY_IMAGE } from '../lib/entry';

  type EnterWorld = () => void;

  let {
    onenter,
    access,
    showArtwork = true,
  }: {
    onenter: EnterWorld;
    /**
     * Integration boundary for account access and character selection. The
     * injected UI decides when it is ready and then calls `enterWorld`.
     */
    access?: Snippet<[enterWorld: EnterWorld]>;
    showArtwork?: boolean;
  } = $props();
</script>

<main class="welcome" aria-labelledby="welcome-title">
  {#if showArtwork}
    <picture class="welcome-art">
      <source
        srcset={ENTRY_IMAGE.avifSrcset}
        sizes="100vw"
        type="image/avif"
      />
      <source
        srcset={ENTRY_IMAGE.webpSrcset}
        sizes="100vw"
        type="image/webp"
      />
      <img
        src={ENTRY_IMAGE.jpeg}
        srcset={ENTRY_IMAGE.jpegSrcset}
        sizes="100vw"
        alt={ENTRY_IMAGE.alt}
        width={ENTRY_IMAGE.width}
        height={ENTRY_IMAGE.height}
        fetchpriority="high"
      />
    </picture>
  {/if}

  <div class="welcome-shade" aria-hidden="true"></div>

  <section class="welcome-panel">
    <p class="welcome-kicker">A living world of words and consequence</p>
    <p class="welcome-mark" aria-hidden="true"><span>T</span></p>
    <h1 id="welcome-title">{PRODUCT_NAME}</h1>
    <p id="welcome-copy" class="welcome-lead">
      Beyond Lantern Square, old roads wake beneath the rain. Cross the threshold, choose
      your path, and leave a story in a world shared with other adventurers.
    </p>

    <div class="welcome-access" aria-describedby="welcome-copy" data-auth-boundary>
      {#if access}
        {@render access(onenter)}
      {:else}
        <button class="enter-button" type="button" onclick={onenter}>
          <span>Enter {PRODUCT_NAME}</span>
          <span aria-hidden="true">→</span>
        </button>
      {/if}
    </div>

    <p class="welcome-promise">
      The illustration sets the mood; every place, action, and route is also fully described
      in text.
    </p>
  </section>

  <p class="welcome-place"><span>Lantern Square</span> · The western marches</p>
</main>
