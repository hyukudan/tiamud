import { describe, expect, it } from 'vitest';
import { directVisualSource, visualSourceUrl } from './visual-source';

describe('directVisualSource', () => {
  it('returns the source when any raster format is present', () => {
    const png = { png: 'a.png' };
    const webp = { webp: 'a.webp' };
    const avif = { avif: 'a.avif' };
    expect(directVisualSource(png)).toBe(png);
    expect(directVisualSource(webp)).toBe(webp);
    expect(directVisualSource(avif)).toBe(avif);
  });

  it('returns undefined when no raster format is present', () => {
    expect(directVisualSource(undefined)).toBeUndefined();
    expect(directVisualSource({})).toBeUndefined();
    expect(directVisualSource({ manifest: 'only-manifest' })).toBeUndefined();
  });
});

describe('visualSourceUrl', () => {
  it('prefers png, then webp, then avif', () => {
    expect(visualSourceUrl({ png: 'a.png', webp: 'a.webp', avif: 'a.avif' })).toBe('a.png');
    expect(visualSourceUrl({ webp: 'a.webp', avif: 'a.avif' })).toBe('a.webp');
    expect(visualSourceUrl({ avif: 'a.avif' })).toBe('a.avif');
  });

  it('returns an empty string when no raster format is present', () => {
    expect(visualSourceUrl({})).toBe('');
    expect(visualSourceUrl({ manifest: 'only-manifest' })).toBe('');
  });
});
