<script lang="ts">
  import type { Snippet } from 'svelte';
  import { imageAfterFailures, imageUrl } from '../lib/assets';
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

  let failedArtworkUrls = $state<string[]>([]);
  const artwork = $derived(
    showArtwork ? imageAfterFailures(ENTRY_IMAGE, failedArtworkUrls) : undefined,
  );
  const artworkRenderKey = $derived(failedArtworkUrls.join('|'));
</script>

<main class="welcome" aria-labelledby="welcome-title">
  {#if artwork}
    {#key artworkRenderKey}
      <picture class="welcome-art">
        {#if artwork.avif}
          <source
            srcset={artwork.avifSrcset ?? artwork.avif}
            sizes="100vw"
            type="image/avif"
          />
        {/if}
        {#if artwork.webp}
          <source
            srcset={artwork.webpSrcset ?? artwork.webp}
            sizes="100vw"
            type="image/webp"
          />
        {/if}
        <img
          src={imageUrl(artwork)}
          srcset={artwork.jpegSrcset ?? artwork.pngSrcset}
          sizes="100vw"
          alt={artwork.alt}
          width={artwork.width}
          height={artwork.height}
          fetchpriority="high"
          onerror={(event) => {
            const target = event.currentTarget as HTMLImageElement;
            const failed = target.currentSrc || target.src;
            if (failed && !failedArtworkUrls.includes(failed)) {
              failedArtworkUrls = [...failedArtworkUrls, failed];
            }
          }}
        />
      </picture>
    {/key}
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
