const CACHE_VERSION = 'tiamud-shell-v1';
const CACHE_PREFIX = 'tiamud-shell-';
const MAX_ENTRIES = 32;

self.addEventListener('install', () => self.skipWaiting());

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    const names = await caches.keys();
    await Promise.all(
      names.filter((name) => name.startsWith(CACHE_PREFIX) && name !== CACHE_VERSION)
        .map((name) => caches.delete(name)),
    );
    await self.clients.claim();
  })());
});

async function trim(cache) {
  const keys = await cache.keys();
  await Promise.all(keys.slice(0, Math.max(0, keys.length - MAX_ENTRIES)).map((key) => cache.delete(key)));
}

self.addEventListener('fetch', (event) => {
  const { request } = event;
  const url = new URL(request.url);
  if (
    request.method !== 'GET' ||
    url.origin !== self.location.origin ||
    url.pathname.startsWith('/api/') ||
    url.pathname === '/ws' ||
    request.destination === 'image' ||
    request.destination === 'audio'
  ) {
    return;
  }

  const cacheable = request.mode === 'navigate' ||
    request.destination === 'script' ||
    request.destination === 'style' ||
    request.destination === 'font';
  if (!cacheable) return;

  event.respondWith((async () => {
    let response;
    try {
      response = await fetch(request);
    } catch (error) {
      try {
        const cache = await caches.open(CACHE_VERSION);
        const cached = await cache.match(request);
        if (cached) return cached;
      } catch {
        // Storage may be disabled; preserve the original network failure.
      }
      throw error;
    }
    if (response.ok) {
      try {
        const cache = await caches.open(CACHE_VERSION);
        await cache.put(request, response.clone());
        await trim(cache);
      } catch {
        // Quota, privacy settings or cache eviction must not hide a live
        // response or substitute an older client after a successful update.
      }
    }
    return response;
  })());
});

self.addEventListener('message', (event) => {
  if (event.data === 'CLEAR_CACHE') {
    event.waitUntil(caches.delete(CACHE_VERSION));
  }
});
