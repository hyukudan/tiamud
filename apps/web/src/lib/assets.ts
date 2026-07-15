import { localizedImageAlt, type SceneImage } from './protocol';

const SHA256 = /^[a-f0-9]{64}$/;
const HEX_COLOR = /^#[a-f0-9]{6}$/;
const MAX_MANIFEST_BYTES = 64 * 1024;
const MEDIA_TYPES = {
  avif: 'image/avif',
  webp: 'image/webp',
  jpeg: 'image/jpeg',
  png: 'image/png',
} as const;

type AssetFormat = keyof typeof MEDIA_TYPES;

interface ManifestVariant {
  format: AssetFormat;
  mediaType: string;
  width: number;
  height: number;
  bytes: number;
  sha256: string;
  path: string;
}

export interface SceneAssetReference {
  manifestUrl: string;
  assetRoot: string;
  assetHash: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const isPositiveInteger = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value > 0;

const isSha256 = (value: unknown): value is string =>
  typeof value === 'string' && SHA256.test(value);

const isMetadataText = (value: unknown): value is string =>
  typeof value === 'string' && value.trim().length > 0 && value.length <= 1024 && !/[\u0000-\u001f\u007f]/.test(value);

function hasReviewedMetadata(value: Record<string, unknown>): boolean {
  const focalPoint = value.focalPoint;
  return (
    isPositiveInteger(value.revision) &&
    isRecord(focalPoint) &&
    typeof focalPoint.xPerMille === 'number' &&
    Number.isSafeInteger(focalPoint.xPerMille) &&
    focalPoint.xPerMille >= 0 &&
    focalPoint.xPerMille <= 1000 &&
    typeof focalPoint.yPerMille === 'number' &&
    Number.isSafeInteger(focalPoint.yPerMille) &&
    focalPoint.yPerMille >= 0 &&
    focalPoint.yPerMille <= 1000 &&
    typeof value.dominantColor === 'string' &&
    HEX_COLOR.test(value.dominantColor) &&
    isMetadataText(value.author) &&
    isMetadataText(value.license) &&
    isMetadataText(value.provenance) &&
    (value.generator === undefined || isMetadataText(value.generator)) &&
    isMetadataText(value.contentClassification) &&
    isMetadataText(value.humanReviewedAt)
  );
}

function parseVariant(value: unknown, assetHash: string): ManifestVariant | undefined {
  if (!isRecord(value)) {
    return undefined;
  }

  const format = value.format;
  if (format !== 'avif' && format !== 'webp' && format !== 'jpeg' && format !== 'png') {
    return undefined;
  }

  const path = value.path;
  const mediaType = value.mediaType;
  const sha256 = value.sha256;
  if (typeof path !== 'string' || typeof mediaType !== 'string' || typeof sha256 !== 'string') {
    return undefined;
  }
  if (
    !isPositiveInteger(value.width) ||
    !isPositiveInteger(value.height) ||
    !isPositiveInteger(value.bytes) ||
    !isSha256(sha256)
  ) {
    return undefined;
  }

  const extension = format === 'jpeg' ? 'jpg' : format;
  const expectedPath = `${assetHash}/scene-${value.width}.${extension}`;
  if (mediaType !== MEDIA_TYPES[format] || path !== expectedPath) {
    return undefined;
  }

  return {
    format,
    mediaType,
    width: value.width,
    height: value.height,
    bytes: value.bytes,
    sha256,
    path,
  };
}

function assetUrl(assetRoot: string, path: string): string | undefined {
  if (!assetRoot.startsWith('/') || !assetRoot.endsWith('/')) {
    return undefined;
  }

  return `${assetRoot}${path}`;
}

function srcset(variants: ManifestVariant[], assetRoot: string): string | undefined {
  const entries = variants
    .toSorted((left, right) => left.width - right.width)
    .flatMap((variant) => {
      const url = assetUrl(assetRoot, variant.path);
      return url ? [`${url} ${variant.width}w`] : [];
    });
  return entries.length > 0 ? entries.join(', ') : undefined;
}

/**
 * Validate schema v2 from the offline asset pipeline and adapt it to the scene
 * renderer. Any structural or integrity-metadata error rejects the whole
 * optional illustration; playable text never depends on this result.
 */
export function sceneImageFromManifest(
  value: unknown,
  assetRoot: string,
  expectedAssetHash?: string,
): SceneImage | undefined {
  if (!isRecord(value)) {
    return undefined;
  }

  const assetId = value.assetId;
  const assetHash = typeof assetId === 'string' && assetId.startsWith('sha256:')
    ? assetId.slice('sha256:'.length)
    : undefined;
  const source = value.source;
  const variants = value.variants;

  if (
    value.schema !== 2 ||
    !hasReviewedMetadata(value) ||
    value.kind !== 'atmosphere' ||
    typeof value.alt !== 'string' ||
    !value.alt.trim() ||
    !assetHash ||
    !SHA256.test(assetHash) ||
    (expectedAssetHash !== undefined && assetHash !== expectedAssetHash) ||
    !isRecord(source) ||
    !isSha256(source.sha256) ||
    !isPositiveInteger(source.width) ||
    !isPositiveInteger(source.height) ||
    !Array.isArray(variants) ||
    variants.length === 0
  ) {
    return undefined;
  }

  const parsed = variants.map((variant) => parseVariant(variant, assetHash));
  if (parsed.some((variant) => !variant)) {
    return undefined;
  }

  const validVariants = parsed as ManifestVariant[];
  const byFormat = (format: AssetFormat): ManifestVariant[] =>
    validVariants.filter((variant) => variant.format === format);
  const largest = validVariants.toSorted((left, right) => right.width - left.width)[0];
  const image: SceneImage = {
    alt: value.alt.trim(),
    width: largest.width,
    height: largest.height,
  };

  for (const format of ['avif', 'webp', 'jpeg', 'png'] as const) {
    const formatVariants = byFormat(format);
    const preferred = formatVariants.toSorted((left, right) => right.width - left.width)[0];
    if (preferred) {
      image[format] = assetUrl(assetRoot, preferred.path);
      image[`${format}Srcset`] = srcset(formatVariants, assetRoot);
    }
  }

  return image.png || (image.jpeg && (image.avif || image.webp)) ? image : undefined;
}

export function sceneAssetReference(manifestUrl: string | undefined): SceneAssetReference | undefined {
  const match = manifestUrl?.match(/^\/assets\/([a-f0-9]{64})\/manifest\.json$/);
  if (!manifestUrl || !match) {
    return undefined;
  }

  return { manifestUrl, assetRoot: '/assets/', assetHash: match[1] };
}

export async function loadSceneImageFromManifest(
  reference: SceneAssetReference,
  signal?: AbortSignal,
  fetcher: typeof fetch = fetch,
): Promise<SceneImage | undefined> {
  try {
    const response = await fetcher(reference.manifestUrl, {
      headers: { accept: 'application/json' },
      signal,
    });
    if (!response.ok) {
      return undefined;
    }

    const declaredLength = Number(response.headers.get('content-length'));
    if (Number.isFinite(declaredLength) && declaredLength > MAX_MANIFEST_BYTES) {
      return undefined;
    }

    const bytes = await response.arrayBuffer();
    if (bytes.byteLength > MAX_MANIFEST_BYTES) {
      return undefined;
    }

    const manifest = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    return sceneImageFromManifest(manifest, reference.assetRoot, reference.assetHash);
  } catch {
    return undefined;
  }
}

export function preserveContextualImageAlt(
  manifestImage: SceneImage | undefined,
  contextualImage: SceneImage | undefined,
  locale: string,
): SceneImage | undefined {
  if (!manifestImage || !contextualImage) return manifestImage;
  return {
    ...manifestImage,
    alt: localizedImageAlt(contextualImage, locale) ?? contextualImage.alt,
    localizedAlt: contextualImage.localizedAlt,
  };
}

export function imageUrl(image: SceneImage | undefined): string | undefined {
  return image?.jpeg ?? image?.png ?? image?.webp ?? image?.avif;
}

function srcsetUrls(value: string | undefined): string[] {
  return value
    ? value.split(',').flatMap((candidate) => {
        const url = candidate.trim().split(/\s+/, 1)[0];
        return url ? [url] : [];
      })
    : [];
}

function comparablePath(value: string): string | undefined {
  try {
    return new URL(value, 'http://mud.invalid').pathname;
  } catch {
    return undefined;
  }
}

export function imageAfterFailures(
  image: SceneImage | undefined,
  failedUrls: readonly string[],
): SceneImage | undefined {
  if (!image) {
    return undefined;
  }

  const failedPaths = new Set(failedUrls.flatMap((url) => comparablePath(url) ?? []));
  const available = { ...image };
  for (const format of ['avif', 'webp', 'jpeg', 'png'] as const) {
    const candidates = [image[format], ...srcsetUrls(image[`${format}Srcset`])]
      .flatMap((url) => (url ? comparablePath(url) ?? [] : []));
    if (candidates.some((url) => failedPaths.has(url))) {
      available[format] = undefined;
      available[`${format}Srcset`] = undefined;
    }
  }

  return imageUrl(available) ? available : undefined;
}
