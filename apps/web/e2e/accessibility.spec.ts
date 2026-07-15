import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

async function installMockWorld(page: import('@playwright/test').Page): Promise<void> {
  await page.route('**/api/characters', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ characters: [{ id: 7, name: 'Aria' }] }),
    });
  });
  await page.addInitScript(() => {
    localStorage.setItem('tiamud.access.v1', JSON.stringify({
      account: { id: 1, name: 'player' },
      token: 'test-token',
      expiresAt: Date.now() + 3_600_000,
      characterId: 7,
    }));

    class MockWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = MockWebSocket.CONNECTING;

      constructor(readonly url: string, readonly protocols?: string | string[]) {
        super();
        queueMicrotask(() => {
          this.readyState = MockWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type?: string };
        if (message.type !== 'client.hello') return;
        const frames = [
          { v: 1, type: 'server.welcome', seq: 1, data: { serverName: 'Tiamud', motd: 'Welcome.' } },
          {
            v: 1, type: 'scene.snapshot', seq: 2, data: {
              viewId: 'o1111111111111111', revision: 1, title: 'Lantern Square',
              prose: ['Rain shines on the stones.'], exits: [], items: [], npcs: [],
              weather: { condition: 'rainy', day: 1, hour: 2, nextChangeInTicks: 3 },
            },
          },
          {
            v: 1, type: 'player.snapshot', seq: 3, data: {
              revision: 1,
              character: {
                name: 'Aria', level: 1, experience: 0, nextLevelExperience: 100,
                health: 20, maxHealth: 20, strength: 5, defence: 2,
                effects: [], recoveryInTicks: null,
              },
              inventory: [{
                id: 'o2222222222222222', label: 'blue lantern', description: 'Cold blue light.',
                actions: [{ label: 'Drop', command: 'drop o2222222222222222' }],
              }],
              equipment: [],
              map: {
                currentRoomId: 'o1111111111111111',
                rooms: [{ id: 'o1111111111111111', title: 'Lantern Square', current: true }],
                paths: [{ from: 'o1111111111111111', direction: 'north' }],
              },
              quests: [{
                id: 'first-hunt', label: 'First Hunt', summary: 'Defeat one hostile creature.',
                status: 'available', current: 0, target: 1,
                actions: [{ label: 'Accept', command: 'quest accept first-hunt' }],
              }],
              partyInvites: [],
              economy: { coins: 40, commodities: [], shopAvailable: false, shop: [] },
              social: {
                online: [
                  { id: 'o3333333333333333', name: 'Aria', selfCharacter: true },
                  { id: 'o4444444444444444', name: 'Bram', selfCharacter: false },
                ],
              },
            },
          },
        ];
        for (const frame of frames) {
          this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(frame) }));
        }
      }

      close(): void {
        this.readyState = MockWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { value: MockWebSocket, configurable: true });
  });
}

test('landing experience passes WCAG A/AA automated checks', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1, name: 'Tiamud' })).toBeVisible();
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});

test('preferences are keyboard-operable, persistent, and prevent artwork requests', async ({ page }) => {
  await page.goto('/');
  await page.keyboard.press('Tab');
  const focused = page.locator(':focus');
  await expect(focused).toBeVisible();

  await page.getByText('Preferences', { exact: true }).click();
  await page.getByLabel('Save data').check();
  await expect(page.locator('picture.welcome-art')).toHaveCount(0);
  await page.reload();
  await expect(page.getByLabel('Save data')).toBeChecked();
  await expect(page.locator('picture.welcome-art')).toHaveCount(0);
});

test('screen-reader mode suppresses images and motion without accessibility violations', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('tiamud.preferences.v1', JSON.stringify({ screenReader: true }));
  });
  const imageRequests: string[] = [];
  page.on('request', (request) => {
    if (request.resourceType() === 'image') imageRequests.push(request.url());
  });

  await page.goto('/');
  await expect(page.getByLabel('Screen-reader mode')).toBeChecked();
  await expect(page.locator('html')).toHaveAttribute('data-reader', 'true');
  await expect(page.locator('html')).toHaveAttribute('data-motion', 'reduce');
  await expect(page.locator('picture.welcome-art')).toHaveCount(0);
  expect(imageRequests).toEqual([]);

  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});

test('narrow mobile layout has no horizontal overflow and keeps controls reachable', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Tiamud' })).toBeVisible();
  const dimensions = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  await expect(page.getByRole('button', { name: /Create account/i })).toBeVisible();
});

test('authenticated mobile play exposes complete accessible panels and a fixed command', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 });
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Aria' })).toBeVisible();
  await expect(page.getByText('blue lantern', { exact: true })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Inventory' })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Discovered map' })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Quests' })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Chat and adventurers' })).toBeVisible();

  const command = page.getByLabel('What do you do?');
  const box = await command.boundingBox();
  expect(box).not.toBeNull();
  expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(740);
  expect(await command.evaluate((input) => getComputedStyle(input.closest('form')!).position)).toBe('fixed');

  const dimensions = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});
