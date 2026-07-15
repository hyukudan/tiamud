import { describe, expect, it } from 'vitest';
import worker from '../../public/sw.js?raw';
import manifestSource from '../../public/manifest.webmanifest?raw';

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
      name: 'Tiamud',
      start_url: '/',
      scope: '/',
      display: 'standalone',
    });
  });
});
