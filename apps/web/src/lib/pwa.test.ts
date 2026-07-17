import { describe, expect, it } from 'vitest';
import worker from '../../public/sw.js?raw';
import manifestSource from '../../public/manifest.webmanifest?raw';
import iconSource from '../../public/icons/tiamud-icon.svg?raw';
import icon192Url from '../../public/icons/tiamud-icon-192.png?url';
import icon512Url from '../../public/icons/tiamud-icon-512.png?url';
import maskableIconUrl from '../../public/icons/tiamud-icon-maskable-512.png?url';
import appleTouchIconUrl from '../../public/icons/apple-touch-icon.png?url';
import indexSource from '../../index.html?raw';

describe('offline shell', () => {
  it('uses a versioned bounded cache and never caches optional multimedia or APIs', () => {
    expect(worker).toContain("const CACHE_VERSION = 'tiamud-shell-v1'");
    expect(worker).toContain('const MAX_ENTRIES = 32');
    expect(worker).toContain("url.pathname.startsWith('/api/')");
    expect(worker).toContain("url.pathname === '/ws'");
    expect(worker).toContain("request.destination === 'image'");
    expect(worker).toContain("request.destination === 'audio'");
  });

  it('publishes a standalone same-origin web app manifest', () => {
    const manifest = JSON.parse(manifestSource);
    expect(manifest).toMatchObject({
      id: '/',
      name: 'Tiamud',
      short_name: 'Tiamud',
      description: 'A browser-first, text-complete multiplayer narrative world.',
      start_url: '/',
      scope: '/',
      display: 'standalone',
      background_color: '#11130f',
      theme_color: '#11130f',
      lang: 'en',
      dir: 'ltr',
    });
    expect(manifest.icons).toEqual([
      { src: '/icons/tiamud-icon.svg', sizes: 'any', type: 'image/svg+xml', purpose: 'any' },
      { src: '/icons/tiamud-icon-192.png', sizes: '192x192', type: 'image/png', purpose: 'any' },
      { src: '/icons/tiamud-icon-512.png', sizes: '512x512', type: 'image/png', purpose: 'any' },
      {
        src: '/icons/tiamud-icon-maskable-512.png', sizes: '512x512',
        type: 'image/png', purpose: 'maskable',
      },
    ]);
  });

  it('ships regular, maskable and Apple icons derived from the reviewed identity', () => {
    expect(icon192Url).toContain('tiamud-icon-192.png');
    expect(icon512Url).toContain('tiamud-icon-512.png');
    expect(maskableIconUrl).toContain('tiamud-icon-maskable-512.png');
    expect(appleTouchIconUrl).toContain('apple-touch-icon.png');
    expect(iconSource).toContain('<title id="tiamud-icon-title">Tiamud</title>');
    expect(iconSource).toContain('fill="#11130f"');
    expect(iconSource).toContain('fill="#72d0ee"');
  });

  it('connects the document metadata to the reviewed local identity assets', () => {
    expect(indexSource).toContain('<html lang="en">');
    expect(indexSource).toContain('<meta name="application-name" content="Tiamud" />');
    expect(indexSource).toContain('<meta name="apple-mobile-web-app-capable" content="yes" />');
    expect(indexSource).toContain(
      '<link rel="icon" href="/icons/tiamud-icon.svg" type="image/svg+xml" />',
    );
    expect(indexSource).toContain(
      '<link rel="apple-touch-icon" href="/icons/apple-touch-icon.png" sizes="180x180" />',
    );
    expect(indexSource).not.toContain('canonical');
    expect(indexSource).not.toContain('og:url');
  });
});
