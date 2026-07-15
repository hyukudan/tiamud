import { describe, expect, it, vi } from 'vitest';
import {
  imageAfterFailures,
  imageUrl,
  loadSceneImageFromManifest,
  preserveContextualImageAlt,
  sceneAssetReference,
  sceneImageFromManifest,
} from './assets';

const HASH = 'a'.repeat(64);
const DIGEST = 'b'.repeat(64);

const manifest = {
  schema: 2,
  assetId: `sha256:${HASH}`,
  revision: 1,
  kind: 'atmosphere',
  alt: 'A rain-dark square lit by a blue lantern.',
  focalPoint: { xPerMille: 500, yPerMille: 460 },
  dominantColor: '#14191f',
  author: 'Tiamud test suite',
  license: 'LicenseRef-Test',
  provenance: 'Generated fixture',
  contentClassification: 'general',
  humanReviewedAt: '2026-07-15T00:00:00Z',
  source: { sha256: DIGEST, width: 1600, height: 700 },
  variants: [
    {
      format: 'avif',
      mediaType: 'image/avif',
      width: 640,
      height: 280,
      bytes: 100,
      sha256: DIGEST,
      path: `${HASH}/scene-640.avif`,
    },
    {
      format: 'avif',
      mediaType: 'image/avif',
      width: 1280,
      height: 560,
      bytes: 200,
      sha256: DIGEST,
      path: `${HASH}/scene-1280.avif`,
    },
    {
      format: 'webp',
      mediaType: 'image/webp',
      width: 1280,
      height: 560,
      bytes: 250,
      sha256: DIGEST,
      path: `${HASH}/scene-1280.webp`,
    },
    {
      format: 'jpeg',
      mediaType: 'image/jpeg',
      width: 1280,
      height: 560,
      bytes: 300,
      sha256: DIGEST,
      path: `${HASH}/scene-1280.jpg`,
    },
  ],
};

describe('asset manifests', () => {
  it('adapts valid pipeline metadata to responsive scene sources', () => {
    expect(sceneImageFromManifest(manifest, '/assets/')).toEqual({
      alt: manifest.alt,
      avif: `/assets/${HASH}/scene-1280.avif`,
      avifSrcset:
        `/assets/${HASH}/scene-640.avif 640w, /assets/${HASH}/scene-1280.avif 1280w`,
      jpeg: `/assets/${HASH}/scene-1280.jpg`,
      jpegSrcset: `/assets/${HASH}/scene-1280.jpg 1280w`,
      webp: `/assets/${HASH}/scene-1280.webp`,
      webpSrcset: `/assets/${HASH}/scene-1280.webp 1280w`,
      width: 1280,
      height: 560,
    });
  });

  it('keeps contextual localized alt when a manifest supplies image sources', () => {
    const responsive = sceneImageFromManifest(manifest, '/assets/');
    const contextual = {
      alt: 'Lantern Square after rain.',
      localizedAlt: {
        en: 'Lantern Square after rain.',
        es: 'La Plaza del Farol tras la lluvia.',
      },
      manifest: `/assets/${HASH}/manifest.json`,
    };
    expect(preserveContextualImageAlt(responsive, contextual, 'es-MX')).toMatchObject({
      alt: 'La Plaza del Farol tras la lluvia.',
      localizedAlt: contextual.localizedAlt,
      jpeg: `/assets/${HASH}/scene-1280.jpg`,
    });
  });

  it('accepts a lossless or transparent PNG-only publication', () => {
    const pngManifest = {
      ...manifest,
      variants: [640, 1280].map((width) => ({
        format: 'png',
        mediaType: 'image/png',
        width,
        height: width / 2,
        bytes: 400,
        sha256: DIGEST,
        path: `${HASH}/scene-${width}.png`,
      })),
    };

    expect(sceneImageFromManifest(pngManifest, '/assets/')).toMatchObject({
      png: `/assets/${HASH}/scene-1280.png`,
      pngSrcset: `/assets/${HASH}/scene-640.png 640w, /assets/${HASH}/scene-1280.png 1280w`,
    });
    expect(imageUrl(sceneImageFromManifest(pngManifest, '/assets/')))
      .toBe(`/assets/${HASH}/scene-1280.png`);
  });

  it('rejects corrupt metadata and unsafe asset paths', () => {
    expect(sceneImageFromManifest({ ...manifest, schema: 1 }, '/assets/')).toBeUndefined();
    expect(
      sceneImageFromManifest({ ...manifest, focalPoint: { xPerMille: 1001, yPerMille: 0 } }, '/assets/'),
    ).toBeUndefined();
    expect(
      sceneImageFromManifest(
        {
          ...manifest,
          variants: [{ ...manifest.variants[0], path: `${HASH}/../secret.avif` }],
        },
        '/assets/',
      ),
    ).toBeUndefined();
  });

  it('keeps a missing or unreadable manifest optional', async () => {
    const reference = {
      manifestUrl: `/assets/${HASH}/manifest.json`,
      assetRoot: '/assets/',
      assetHash: HASH,
    };
    const missing = vi.fn<typeof fetch>().mockResolvedValue(
      new Response('not found', { status: 404 }),
    );
    const corrupt = vi.fn<typeof fetch>().mockResolvedValue(
      new Response('{', { status: 200, headers: { 'content-type': 'application/json' } }),
    );

    await expect(loadSceneImageFromManifest(reference, undefined, missing)).resolves.toBeUndefined();
    await expect(loadSceneImageFromManifest(reference, undefined, corrupt)).resolves.toBeUndefined();
  });

  it('falls back from AVIF to WebP to JPEG before hiding an illustration', () => {
    const image = sceneImageFromManifest(manifest, '/assets/');
    const withoutAvif = imageAfterFailures(image, [
      `https://mud.test/assets/${HASH}/scene-640.avif`,
    ]);
    expect(withoutAvif?.avif).toBeUndefined();
    expect(withoutAvif?.webp).toBe(`/assets/${HASH}/scene-1280.webp`);

    const withoutModernFormats = imageAfterFailures(withoutAvif, [
      `/assets/${HASH}/scene-1280.webp`,
    ]);
    expect(withoutModernFormats?.webp).toBeUndefined();
    expect(imageUrl(withoutModernFormats)).toBe(`/assets/${HASH}/scene-1280.jpg`);
    expect(
      imageAfterFailures(withoutModernFormats, [`/assets/${HASH}/scene-1280.jpg`]),
    ).toBeUndefined();
  });

  it('accepts only a same-origin content-addressed asset manifest path', () => {
    const path = `/assets/${HASH}/manifest.json`;
    expect(sceneAssetReference(path)).toEqual({
      manifestUrl: path,
      assetRoot: '/assets/',
      assetHash: HASH,
    });
    expect(sceneAssetReference(`https://example.test${path}`)).toBeUndefined();
    expect(sceneAssetReference(`/assets/${HASH}/../manifest.json`)).toBeUndefined();
    expect(sceneAssetReference(`${path}?newer=1`)).toBeUndefined();
  });

  it('cancels an in-flight optional manifest without surfacing an error', async () => {
    const controller = new AbortController();
    const reference = sceneAssetReference(`/assets/${HASH}/manifest.json`)!;
    const fetcher = vi.fn<typeof fetch>((_input, init) =>
      new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')));
      }),
    );

    const loading = loadSceneImageFromManifest(reference, controller.signal, fetcher);
    controller.abort();

    await expect(loading).resolves.toBeUndefined();
  });
});
