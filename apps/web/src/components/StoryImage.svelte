<script lang="ts">
  import type { StoryImage } from '../lib/generated/protocol';
  import type { SceneImage } from '../lib/protocol';
  import { imageUrl, loadSceneImageFromManifest, sceneAssetReference } from '../lib/assets';

  interface Props {
    /** Reviewed story image sent by the server (manifest + alt text). */
    image: StoryImage;
    /**
     * `background` spans the panel; `portrait` is a small framed picture;
     * `line` is the smaller portrait beside a dialogue line.
     */
    kind: 'background' | 'portrait' | 'line';
    /** Show the alt text as plain text when the picture is not available. */
    fallbackText?: boolean;
    lang?: string;
  }

  let { image, kind, fallbackText = false, lang }: Props = $props();

  let loaded = $state<SceneImage>();
  let failed = $state(false);

  // Fetch the integrity-checked pipeline manifest of this image only; a
  // missing or malformed manifest simply leaves the text in charge.
  $effect(() => {
    const manifest = image.manifest;
    loaded = undefined;
    failed = false;
    const reference = sceneAssetReference(manifest);
    if (!reference) {
      failed = true;
      return;
    }
    const controller = new AbortController();
    void loadSceneImageFromManifest(reference, controller.signal).then((result) => {
      if (controller.signal.aborted) return;
      if (result) loaded = result;
      else failed = true;
    });
    return () => controller.abort();
  });

  const sizes = $derived(
    kind === 'background' ? '(min-width: 58rem) 65vw, 100vw' : kind === 'line' ? '4rem' : '8rem',
  );
</script>

{#if loaded && !failed}
  <picture class="story-image story-image-{kind}">
    {#if loaded.avif}
      <source srcset={loaded.avifSrcset ?? loaded.avif} {sizes} type="image/avif" />
    {/if}
    {#if loaded.webp}
      <source srcset={loaded.webpSrcset ?? loaded.webp} {sizes} type="image/webp" />
    {/if}
    <img
      src={imageUrl(loaded)}
      srcset={loaded.jpegSrcset ?? loaded.pngSrcset}
      {sizes}
      alt={image.alt}
      width={loaded.width}
      height={loaded.height}
      decoding="async"
      loading="lazy"
      {lang}
      onerror={() => (failed = true)}
    />
  </picture>
{:else if failed && fallbackText}
  <p class="story-image-text" {lang}>{image.alt}</p>
{/if}

<style>
  .story-image {
    display: block;
  }

  .story-image img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: 0.2rem;
  }

  .story-image-background img {
    max-height: 22rem;
    object-fit: cover;
  }

  .story-image-portrait img {
    width: 6rem;
    aspect-ratio: 1;
    object-fit: cover;
  }

  .story-image-line {
    flex: 0 0 auto;
  }

  .story-image-line img {
    width: 3.5rem;
    aspect-ratio: 1;
    object-fit: cover;
  }

  .story-image-text {
    margin: 0;
    font-size: 0.875rem;
    font-style: italic;
  }
</style>
