import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

async function installMockWorld(
  page: import('@playwright/test').Page,
  sceneImage?: Record<string, unknown>,
): Promise<void> {
  await page.route('**/api/characters', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ characters: [{ id: 7, name: 'Aria' }] }),
    });
  });
  await page.route('**/api/character-options', async (route) => {
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({
      defaultLineageId: 'mortal', defaultBackgroundId: null, defaultBaseClassId: 'wanderer',
      lineages: [{ id: 'mortal', label: 'Mortal', description: 'Adaptable people.' }],
      backgrounds: [],
      baseClasses: [{ id: 'wanderer', label: 'Wanderer', description: 'A flexible explorer.' }],
    }) });
  });
  await page.addInitScript((initialSceneImage: Record<string, unknown> | null) => {
    const mockState = window as Window & {
      mockHellos?: unknown[];
      mockActions?: unknown[];
      mockSocketCount?: number;
      emitMockEvents?: (count: number) => void;
      emitMockTrade?: () => void;
      emitMockTradeStep?: (step: 'initial' | 'changed' | 'settled') => void;
      emitMockRewardOdds?: (page: 1 | 2) => void;
    };
    mockState.mockHellos = [];
    mockState.mockActions = [];
    mockState.mockSocketCount = 0;
    let activeSocket: MockWebSocket | undefined;
    let serverSequence = 3;
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
        activeSocket = this;
        mockState.mockSocketCount = (mockState.mockSocketCount ?? 0) + 1;
        queueMicrotask(() => {
          this.readyState = MockWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type?: string; data?: unknown };
        if (message.type === 'action.perform') {
          mockState.mockActions?.push(message);
          return;
        }
        if (message.type !== 'client.hello') return;
        mockState.mockHellos?.push(message);
        serverSequence = 3;
        const frames = [
          { v: 1, type: 'server.welcome', seq: 1, data: { serverName: 'Tiamud', motd: 'Welcome.' } },
          {
            v: 1, type: 'scene.snapshot', seq: 2, data: {
              viewId: 'o1111111111111111', revision: 1, title: 'Lantern Square',
              prose: ['Rain shines on the stones.'], exits: [], items: [
                {
                  id: 'o9999999999999999', label: 'wolf clasp', description: 'A rain-dark clasp.',
                  rarity: 'rare', itemLevel: 2, weightGrams: 100,
                  quantity: 1, stackSlots: 1, totalWeightGrams: 100,
                  transferPolicy: 'transferable', boundToYou: false,
                  loot: { access: 'yours', expiresInTicks: 12 },
                  actions: [
                    { label: 'Take', command: 'take o9999999999999999' },
                    { label: 'Pass', command: 'loot pass o9999999999999999' },
                  ],
                },
                {
                  id: 'oaaaaaaaaaaaaaaaa', label: 'road token', description: 'A stamped token.',
                  rarity: 'common', itemLevel: 1, weightGrams: 10,
                  quantity: 1, stackSlots: 1, totalWeightGrams: 10,
                  transferPolicy: 'transferable', boundToYou: false,
                  loot: { access: 'reserved' }, actions: [],
                },
              ],
              npcs: [{
                id: 'opaque-wolf', label: 'rain wolf', description: 'It watches the lanterns.',
                health: 12, maxHealth: 12,
                actions: [{ label: 'Attack', command: 'attack opaque-wolf' }],
              }],
              weather: { condition: 'rainy', day: 1, hour: 2, nextChangeInTicks: 3 },
              ...(initialSceneImage ? { image: initialSceneImage } : {}),
            },
          },
          {
            v: 1, type: 'player.snapshot', seq: 3, data: {
              revision: 1,
              character: {
                name: 'Aria', level: 1, experience: 0, nextLevelExperience: 100,
                health: 20, maxHealth: 20, strength: 5, defence: 2,
                lineage: { id: 'mortal', label: 'Mortal', description: 'Adaptable people.' },
                background: null,
                baseClass: { id: 'mystic', label: 'Mystic', description: 'A wielder of arcane power.' },
                activeClass: { id: 'mystic', label: 'Mystic', description: 'A wielder of arcane power.' },
                classHistory: [{ id: 'mystic', label: 'Mystic', description: 'A wielder of arcane power.' }],
                progressionPoints: { talent: 2 },
                talents: [{
                  id: 'mystic-path', label: 'Mystic Path', description: 'Shape power with precision.',
                  pointPoolId: 'talent', availablePoints: 2,
                  nodes: [{
                    id: 'steady-flame', label: 'Steady Flame', description: 'Keep a spell stable.',
                    rank: 0, maxRank: 2, nextCost: 1, available: true,
                    requirements: [{ met: true, text: 'Reach level 1' }],
                    command: 'talent learn mystic-path steady-flame',
                  }],
                }],
                promotions: [{
                  class: { id: 'stormcaller', label: 'Stormcaller', description: 'Call lightning from rain.' },
                  available: true, requirements: [{ met: true, text: 'Train as a Mystic' }],
                  command: 'promote stormcaller',
                }],
                attributes: [{ id: 'intellect', label: 'Intellect', value: 9 }],
                resources: [{ id: 'mana', label: 'Mana', current: 12, max: 20 }],
                abilities: [{
                  id: 'ember-bolt', name: 'Ember Bolt', description: 'Hurl a compact flame.', kind: 'spell',
                  resourceCosts: [{ resourceId: 'mana', label: 'Mana', amount: 4 }],
                  cooldownRemaining: 0, usable: true,
                  purpose: 'gameplay', targeting: 'explicitTarget',
                }, {
                  id: 'mind-thread', name: 'Mind Thread',
                  description: 'Carry a private thought to an online mind.', kind: 'utility',
                  resourceCosts: [{ resourceId: 'mana', label: 'Mana', amount: 2 }],
                  cooldownRemaining: 0, usable: true,
                  purpose: 'telepathy', targeting: 'optionalTarget',
                }],
                resistances: [{ damageType: 'fire', percent: 10 }],
                statuses: [],
                effects: [], recoveryInTicks: null,
              },
              inventory: [{
                id: 'o2222222222222222', label: 'blue lantern', description: 'Cold blue light.',
                rarity: 'uncommon', itemLevel: 3, weightGrams: 800,
                quantity: 1, availableQuantity: 1, reservedForTrade: 0,
                stackSlots: 1, totalWeightGrams: 800,
                transferPolicy: 'transferable', boundToYou: false,
                affix: {
                  label: 'charged', position: 'prefix', attackBonusDelta: 2, healAmountDelta: 0,
                },
                canGive: true, canTrade: true,
                actions: [
                  { label: 'Drop', command: 'drop o2222222222222222' },
                  { label: 'Abrir', command: 'open o2222222222222222' },
                  { label: 'Probabilidades', command: 'reward odds o2222222222222222 1' },
                ],
              }],
              equipment: [],
              inventoryCapacity: {
                usedSlots: 1, maxSlots: 24, usedWeightGrams: 800, maxWeightGrams: 30000,
              },
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
              party: {
                leader: true,
                members: [
                  { id: 'o3333333333333333', name: 'Aria', leader: true, online: true },
                  { id: 'o4444444444444444', name: 'Bram', leader: false, online: true },
                ],
                actions: [],
              },
              partyInvites: [],
              economy: {
                coins: 40, reservedCoins: 0, spendableCoins: 40,
                commodities: [], shopAvailable: false, shop: [],
              },
              social: {
                online: [
                  { id: 'o3333333333333333', name: 'Aria', selfCharacter: true, nearby: true },
                  { id: 'o4444444444444444', name: 'Bram', selfCharacter: false, nearby: true },
                  { id: 'o5555555555555555', name: 'Cato', selfCharacter: false, nearby: false },
                ],
              },
            },
          },
        ];
        const playerData = frames[2]?.data as Record<string, unknown>;
        const tradeInventory = (playerData.inventory as Array<Record<string, unknown>>).map((item) => ({
          ...item,
          availableQuantity: 0,
          reservedForTrade: 1,
          canGive: false,
          canTrade: false,
        }));
        const emitPlayer = (data: Record<string, unknown>) => {
          serverSequence += 1;
          activeSocket?.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({
            v: 1,
            type: 'player.snapshot',
            seq: serverSequence,
            data,
          }) }));
        };
        mockState.emitMockTradeStep = (step) => {
          if (step === 'settled') {
            emitPlayer({
              ...playerData,
              revision: 4,
              inventory: [{
                id: 'o8888888888888888', label: 'field satchel', description: 'A received satchel.',
                rarity: 'rare', itemLevel: 4, weightGrams: 500,
                quantity: 1, availableQuantity: 1, reservedForTrade: 0,
                stackSlots: 1, totalWeightGrams: 500,
                transferPolicy: 'transferable', boundToYou: false,
                canGive: true, canTrade: true,
                containerCapacity: {
                  usedSlots: 0, maxSlots: 8, usedWeightGrams: 0, maxWeightGrams: 5000,
                },
                actions: [{ label: 'Drop', command: 'drop o8888888888888888' }],
              }],
              inventoryCapacity: {
                usedSlots: 1, maxSlots: 24, usedWeightGrams: 500, maxWeightGrams: 29000,
              },
              economy: {
                ...(playerData.economy as Record<string, unknown>),
                coins: 39,
                reservedCoins: 0,
                spendableCoins: 39,
              },
            });
            return;
          }
          const changed = step === 'changed';
          emitPlayer({
            ...playerData,
            revision: changed ? 3 : 2,
            inventory: tradeInventory,
            inventoryCapacity: {
              ...(playerData.inventoryCapacity as Record<string, unknown>),
              maxWeightGrams: changed ? 29000 : 30000,
            },
            economy: {
              ...(playerData.economy as Record<string, unknown>),
              reservedCoins: 5,
              spendableCoins: 35,
            },
            trade: {
              id: 'o6666666666666666',
              phase: 'negotiating',
              counterparty: 'o4444444444444444',
              expiresInTicks: changed ? 75 : 80,
              stateRevision: changed ? 7 : 6,
              ownConfirmed: false,
              counterpartyConfirmed: changed,
              ownOffer: {
                coins: 5,
                lines: [{
                  target: 'o2222222222222222', label: 'blue lantern', quantity: 1,
                  container: false,
                }],
              },
              counterpartyOffer: {
                coins: changed ? 4 : 3,
                lines: [{
                  target: 'o7777777777777777', label: 'field satchel', quantity: 1,
                  container: true,
                }],
              },
              canAccept: false,
              canConfirm: true,
            },
          });
        };
        mockState.emitMockTrade = () => mockState.emitMockTradeStep?.('initial');
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
    mockState.emitMockEvents = (count: number) => {
      for (let index = 0; index < count; index += 1) {
        serverSequence += 1;
        activeSocket?.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({
          v: 1,
          type: 'message.append',
          seq: serverSequence,
          data: {
            revision: serverSequence,
            kind: 'gameplay',
            text: `World event ${serverSequence}`,
            actorId: 'o1111111111111111',
          },
        }) }));
      }
    };
    mockState.emitMockRewardOdds = (page) => {
      serverSequence += 1;
      const firstPage = page === 1;
      activeSocket?.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({
        v: 1,
        type: 'reward.odds',
        seq: serverSequence,
        data: {
          selector: 'o2222222222222222',
          page,
          totalPages: 2,
          guaranteed: firstPage ? [{
            label: 'three sparks', quantity: 3, affixPoolLabel: 'storm modifiers',
          }] : [{ label: 'healing tonic', quantity: 1 }],
          weighted: firstPage ? [{
            poolLabel: 'storm draw', rolls: 1, outcomeLabel: 'iron blade',
            minQuantity: 1, maxQuantity: 1,
            probabilityNumerator: 1, probabilityDenominator: 4,
            affixPoolLabel: 'storm modifiers',
          }, {
            poolLabel: 'storm draw', rolls: 1, outcomeLabel: 'nothing',
            minQuantity: 0, maxQuantity: 0,
            probabilityNumerator: 3, probabilityDenominator: 4,
          }] : [],
          affixes: firstPage ? [{
            parentOutcomeLabel: 'iron blade', affixLabel: 'charged', position: 'prefix',
            attackBonusDelta: 2, healAmountDelta: 0,
            conditionalProbabilityNumerator: 1, conditionalProbabilityDenominator: 5,
          }, {
            parentOutcomeLabel: 'iron blade', affixLabel: 'restoring', position: 'suffix',
            attackBonusDelta: 0, healAmountDelta: 3,
            conditionalProbabilityNumerator: 1, conditionalProbabilityDenominator: 10,
          }, {
            parentOutcomeLabel: 'iron blade', affixLabel: 'nothing',
            attackBonusDelta: 0, healAmountDelta: 0,
            conditionalProbabilityNumerator: 7, conditionalProbabilityDenominator: 10,
          }] : [],
        },
      }) }));
    };
  }, sceneImage ?? null);
}

async function installRejectedWorld(page: import('@playwright/test').Page): Promise<void> {
  await page.route('**/api/characters', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ characters: [{ id: 7, name: 'Aria' }] }),
    });
  });
  await page.route('**/api/character-options', async (route) => {
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({
      defaultLineageId: 'mortal', defaultBackgroundId: null, defaultBaseClassId: 'wanderer',
      lineages: [{ id: 'mortal', label: 'Mortal', description: 'Adaptable people.' }],
      backgrounds: [],
      baseClasses: [{ id: 'wanderer', label: 'Wanderer', description: 'A flexible explorer.' }],
    }) });
  });
  await page.addInitScript(() => {
    localStorage.setItem('tiamud.access.v1', JSON.stringify({
      account: { id: 1, name: 'player' },
      token: 'test-token',
      expiresAt: Date.now() + 3_600_000,
      characterId: 7,
    }));

    class RejectedWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = RejectedWebSocket.CONNECTING;

      constructor() {
        super();
        const state = window as Window & { rejectedSocketCount?: number };
        state.rejectedSocketCount = (state.rejectedSocketCount ?? 0) + 1;
        queueMicrotask(() => {
          this.readyState = RejectedWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type?: string };
        if (message.type !== 'client.hello') return;
        this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({
          v: 1,
          type: 'server.notice',
          seq: 1,
          data: {
            level: 'error',
            code: 'access.session_invalid',
            message: 'Your saved session is no longer valid.',
          },
        }) }));
      }

      close(): void {
        this.readyState = RejectedWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { value: RejectedWebSocket, configurable: true });
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

test('welcome remains usable when every artwork rendition fails', async ({ page }) => {
  await page.route('**/assets/**/scene-*', async (route) => {
    await route.fulfill({ status: 404, body: 'missing' });
  });
  await page.goto('/');

  await expect(page.getByRole('heading', { level: 1, name: 'Tiamud' })).toBeVisible();
  await expect(page.locator('picture.welcome-art')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Return to your story' })).toBeVisible();
});

test('character creation keeps world defaults distinct from explicit v3 choices', async ({ page }) => {
  const creationBodies: unknown[] = [];
  await page.route('**/api/auth/login', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        account: { id: 1, name: 'player' },
        token: 'test-token',
        expiresAt: Date.now() + 3_600_000,
      }),
    });
  });
  await page.route('**/api/character-options', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        defaultLineageId: 'mortal',
        defaultBackgroundId: 'scout',
        defaultBaseClassId: 'wanderer',
        lineages: [
          { id: 'mortal', label: 'Mortal', description: 'Adaptable people.' },
          { id: 'embermarked', label: 'Embermarked', description: 'Fire answers their blood.' },
        ],
        backgrounds: [
          { id: 'scout', label: 'Caravan Scout', description: 'Knows the road.' },
          { id: 'archive', label: 'Archive Apprentice', description: 'Studies old mysteries.' },
        ],
        baseClasses: [
          { id: 'wanderer', label: 'Wanderer', description: 'A flexible explorer.' },
          { id: 'arcanist', label: 'Arcanist', description: 'A wielder of focused arcane power.' },
        ],
      }),
    });
  });
  await page.route('**/api/characters', async (route) => {
    if (route.request().method() === 'POST') {
      const request = route.request().postDataJSON() as {
        name: string; lineageId?: string; backgroundId?: string | null; baseClassId?: string;
      };
      creationBodies.push(request);
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({ character: {
          id: 6 + creationBodies.length,
          name: request.name,
        } }),
      });
      return;
    }
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ characters: [] }) });
  });

  await page.goto('/');
  await page.getByLabel('Account name').fill('player');
  await page.getByLabel('Password').fill('long password');
  await page.getByRole('button', { name: 'Sign in' }).click();

  await expect(page.getByLabel('Lineage')).toHaveValue('');
  await expect(page.getByLabel('Background')).toHaveValue('@default');
  await expect(page.getByLabel('Base class')).toHaveValue('');
  await expect(page.getByText('Adaptable people.')).toBeVisible();
  await expect(page.getByText('Knows the road.')).toBeVisible();
  await expect(page.getByText('A flexible explorer.')).toBeVisible();
  await page.getByLabel('Create an adventurer').fill('Wren');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Enter as Wren' })).toBeVisible();

  await page.getByLabel('Lineage').selectOption('embermarked');
  await page.getByLabel('Background').selectOption('@none');
  await page.getByLabel('Base class').selectOption('arcanist');
  await expect(page.getByText('Fire answers their blood.')).toBeVisible();
  await expect(page.getByText('Begin without a background package. This differs from the world default.')).toBeVisible();
  await expect(page.getByText('A wielder of focused arcane power.')).toBeVisible();
  await page.getByLabel('Create an adventurer').fill('Lyra');
  await page.getByRole('button', { name: 'Create', exact: true }).click();

  await expect(page.getByRole('button', { name: 'Enter as Lyra' })).toBeVisible();
  expect(creationBodies).toEqual([
    { name: 'Wren' },
    { name: 'Lyra', lineageId: 'embermarked', backgroundId: null, baseClassId: 'arcanist' },
  ]);
});

test('welcome falls through to a raster fallback when AVIF alone fails', async ({ page }) => {
  await page.route('**/assets/**/*.avif', async (route) => {
    await route.fulfill({ status: 404, body: 'missing' });
  });
  await page.goto('/');

  const artwork = page.locator('picture.welcome-art');
  await expect(artwork).toHaveCount(1);
  const image = artwork.locator('img');
  await expect(image).toBeVisible();
  await expect.poll(() => image.evaluate((node: HTMLImageElement) => node.currentSrc)).not.toMatch(/\.avif(?:$|\?)/);
  await expect(image).toHaveAttribute('alt', /Lantern Square after rain/);
});

test('a manifest-only scene completes loading and renders its atmosphere', async ({ page }) => {
  const hash = 'a'.repeat(64);
  const imagePath = `/assets/${hash}/scene-640.png`;
  let manifestRequests = 0;
  await page.route(`**/assets/${hash}/manifest.json`, async (route) => {
    manifestRequests += 1;
    await new Promise((resolve) => setTimeout(resolve, 100));
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        schema: 2,
        revision: 1,
        assetId: `sha256:${hash}`,
        kind: 'atmosphere',
        alt: 'Lantern Square under a blue-glass light.',
        focalPoint: { xPerMille: 500, yPerMille: 500 },
        dominantColor: '#20251b',
        author: 'Tiamud test suite',
        license: 'CC-BY-4.0',
        provenance: 'Deterministic test fixture.',
        contentClassification: 'general',
        humanReviewedAt: '2026-07-16T00:00:00Z',
        source: { sha256: 'b'.repeat(64), width: 640, height: 280 },
        variants: [{
          format: 'png',
          mediaType: 'image/png',
          width: 640,
          height: 280,
          bytes: 68,
          sha256: 'c'.repeat(64),
          path: `${hash}/scene-640.png`,
        }],
      }),
    });
  });
  await page.route(`**${imagePath}`, async (route) => {
    await route.fulfill({
      contentType: 'image/png',
      body: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAHnOcQAAAAABJRU5ErkJggg==',
        'base64',
      ),
    });
  });
  await installMockWorld(page, {
    manifest: `/assets/${hash}/manifest.json`,
    alt: 'Rain shines beneath the lantern.',
  });
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  const image = page.locator('picture.atmosphere img');
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute('src', imagePath);
  await expect(image).toHaveAttribute('alt', 'Rain shines beneath the lantern.');
  expect(manifestRequests).toBe(1);
});

test('an authenticated entry rejection returns to login without reconnecting', async ({ page }) => {
  await installRejectedWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  await expect(page.getByRole('alert')).toContainText('Your saved session is no longer valid.');
  await expect(page.getByRole('heading', { name: 'Return to your story' })).toBeVisible();
  await page.waitForTimeout(1_100);
  expect(await page.evaluate(() =>
    (window as Window & { rejectedSocketCount?: number }).rejectedSocketCount,
  )).toBe(1);
  expect(await page.evaluate(() => localStorage.getItem('tiamud.access.v1'))).toBeNull();
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
  await expect(page.getByText(/^blue lantern ×1$/).first()).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Inventory' })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Discovered map' })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Quests' })).toBeVisible();
  await expect(page.locator('summary').filter({ hasText: 'Chat and adventurers' })).toBeVisible();

  const command = page.getByLabel('What do you do?');
  await expect(command).toBeFocused();
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

test('sealed-reward odds render every public row and paginate with structured actions', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  await expect(page.getByText(
    'Modifier: charged (prefix) · attack +2 · healing +0',
    { exact: true },
  )).toBeVisible();
  await page.getByRole('button', { name: 'Probabilidades' }).click();
  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toMatchObject({
    action: 'reward.odds', target: 'o2222222222222222', page: 1,
  });

  await page.evaluate(() => {
    (window as Window & { emitMockRewardOdds?: (page: 1 | 2) => void })
      .emitMockRewardOdds?.(1);
  });
  const panel = page.locator('section[aria-labelledby="reward-odds-title"]');
  await expect(panel).toContainText('Page 1 of 2');
  await expect(panel).toContainText('three sparks ×3 · modifier draw: storm modifiers');
  await expect(panel).toContainText('iron blade · storm draw · 1 draw · quantity 1–1');
  await expect(panel).toContainText('exact chance 1/4 (25.00%)');
  await expect(panel).toContainText('nothing · storm draw · 1 draw · quantity 0–0');
  await expect(panel).toContainText('exact chance 3/4 (75.00%)');
  await expect(panel).toContainText('iron blade → charged (prefix)');
  await expect(panel).toContainText('attack +2 · healing +0');
  await expect(panel).toContainText('exact conditional chance 1/5 (20.00%)');
  await expect(panel).toContainText('iron blade → restoring (suffix)');
  await expect(panel).toContainText('attack +0 · healing +3');
  await expect(panel).toContainText('exact conditional chance 1/10 (10.00%)');
  await expect(panel).toContainText('iron blade → nothing');
  await expect(panel).toContainText('exact conditional chance 7/10 (70.00%)');
  await expect(panel).not.toContainText('prototypeId');
  await expect(panel).not.toContainText('affixPoolId');
  await expect(panel).not.toContainText('digest');

  await panel.getByRole('button', { name: 'Next' }).click();
  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toMatchObject({
    action: 'reward.odds', target: 'o2222222222222222', page: 2,
  });
  await page.evaluate(() => {
    (window as Window & { emitMockRewardOdds?: (page: 1 | 2) => void })
      .emitMockRewardOdds?.(2);
  });
  await expect(panel).toContainText('Page 2 of 2');
  await expect(panel).toContainText('healing tonic ×1');
  await expect(panel.getByRole('button', { name: 'Next' })).toBeDisabled();
  await expect(panel.getByRole('button', { name: 'Previous' })).toBeEnabled();
  await panel.getByRole('button', { name: 'Previous' }).click();
  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toMatchObject({
    action: 'reward.odds', target: 'o2222222222222222', page: 1,
  });

  await page.getByRole('button', { name: 'Abrir' }).click();
  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toMatchObject({ action: 'reward.open', target: 'o2222222222222222' });

  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});

test('loot rights stay textual, private, and use structured status and pass actions', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  await expect(page.getByText('Reserved for you · 12 ticks remaining', { exact: true }))
    .toBeVisible();
  await expect(page.getByText('Reserved for another adventurer', { exact: true }))
    .toBeVisible();
  await page.getByRole('button', { name: 'Review loot rights' }).click();
  await page.getByRole('button', { name: 'Pass', exact: true }).click();

  const actions = await page.evaluate(() =>
    (window as Window & { mockActions?: Array<{ data?: Record<string, unknown> }> }).mockActions ?? [],
  );
  expect(actions.map((action) => action.data)).toEqual([
    expect.objectContaining({ action: 'loot.status' }),
    expect.objectContaining({ action: 'loot.pass', target: 'o9999999999999999' }),
  ]);
  expect(JSON.stringify(actions)).not.toContain('beneficiary');
  expect(JSON.stringify(actions)).not.toContain('partyId');

  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});

test('mocked confirmed-trade UI binds every mutation to opaque selectors and the reviewed revision', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await page.evaluate(() => {
    (window as Window & { emitMockTrade?: () => void }).emitMockTrade?.();
  });

  await expect(page.getByRole('heading', { name: 'Exchange with Bram' })).toBeVisible();
  await expect(page.getByText('Revision 6 · 80 ticks left', { exact: true })).toBeVisible();
  await expect(page.getByText('Review before confirming')).toBeVisible();
  await page.getByRole('button', { name: 'Inspect offered contents' }).click();
  await page.getByRole('button', { name: 'Confirm revision 6' }).click();
  await page.getByLabel(/Coins to offer/).fill('7');
  await page.getByRole('button', { name: 'Set coin offer' }).click();

  const actions = await page.evaluate(() =>
    (window as Window & { mockActions?: Array<{ data?: Record<string, unknown> }> }).mockActions ?? [],
  );
  expect(actions.map((action) => action.data)).toEqual(expect.arrayContaining([
    expect.objectContaining({
      action: 'trade.contents', tradeId: 'o6666666666666666',
      target: 'o7777777777777777', page: 1,
    }),
    expect.objectContaining({
      action: 'trade.confirm', tradeId: 'o6666666666666666', stateRevision: 6,
    }),
    expect.objectContaining({
      action: 'trade.coins.set', tradeId: 'o6666666666666666', coins: 7,
    }),
  ]));

  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});

test('mocked two-participant projections invalidate an old confirmation and settle after reconfirmation', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  const emitStep = (step: 'initial' | 'changed' | 'settled') => page.evaluate((next) => {
    (window as Window & {
      emitMockTradeStep?: (value: 'initial' | 'changed' | 'settled') => void;
    }).emitMockTradeStep?.(next);
  }, step);

  await emitStep('initial');
  await expect(page.getByText('Revision 6 · 80 ticks left', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Confirm revision 6' }).click();

  // Bram changes his offer and confirms the new base. Aria's previous
  // confirmation must disappear and cannot approve revision 7 implicitly.
  await emitStep('changed');
  await expect(page.getByText('Revision 7 · 75 ticks left', { exact: true })).toBeVisible();
  await expect(page.getByText('5 coins · not confirmed', { exact: true })).toBeVisible();
  await expect(page.getByText('4 coins · confirmed', { exact: true })).toBeVisible();
  await expect(page.getByText(/0\.8\/\s*29\.0 kg/)).toBeVisible();
  await page.getByRole('button', { name: 'Confirm revision 7' }).click();

  const confirmations = await page.evaluate(() =>
    ((window as Window & { mockActions?: Array<{ data?: Record<string, unknown> }> }).mockActions ?? [])
      .map((action) => action.data)
      .filter((data) => data?.action === 'trade.confirm'),
  );
  expect(confirmations).toEqual([
    expect.objectContaining({ tradeId: 'o6666666666666666', stateRevision: 6 }),
    expect.objectContaining({ tradeId: 'o6666666666666666', stateRevision: 7 }),
  ]);

  await emitStep('settled');
  await expect(page.getByText('No active trade', { exact: true })).toBeVisible();
  await expect(page.getByText('field satchel ×1', { exact: true })).toBeVisible();
  await page.locator('summary').filter({ hasText: 'Coin and market' }).click();
  await expect(page.getByText('39 coin total', { exact: true })).toBeVisible();
  await expect(page.getByText('0 reserved for trade · 39 spendable', { exact: true })).toBeVisible();
});

test('ability panel chooses a visible target and sends a structured ability action', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await expect(page.getByRole('heading', { name: 'Aria' })).toBeVisible();

  await page.getByLabel('Target for Ember Bolt').selectOption({ label: 'rain wolf' });
  await page.getByRole('button', { name: 'Cast Ember Bolt' }).click();

  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toMatchObject({
    action: 'combat.ability',
    abilityId: 'ember-bolt',
    target: 'opaque-wolf',
  });
});

test('telepathy ability reaches an online mind through an exact structured action', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  await page.getByLabel('Mind to reach with Mind Thread').fill('Cato');
  await page.getByLabel('Private thought').fill('Meet beyond the archive.');
  await page.getByRole('button', { name: 'Reach with Mind Thread' }).click();

  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toEqual({
    requestId: expect.any(String),
    action: 'social.telepathy',
    abilityId: 'mind-thread',
    target: 'Cato',
    message: 'Meet beyond the archive.',
  });
});

test('inventory gives only to a selected nearby opaque recipient', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  const give = page.getByRole('button', { name: 'Give', exact: true });
  await expect(give).toBeDisabled();
  const recipients = page.getByLabel('Nearby recipient');
  await expect(recipients.getByRole('option', { name: 'Cato' })).toHaveCount(0);
  await recipients.selectOption({ label: 'Bram' });
  await expect(give).toBeEnabled();
  await give.click();

  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.at(-1)?.data;
  })).toMatchObject({
    action: 'item.give',
    target: 'o2222222222222222',
    recipient: 'o4444444444444444',
    quantity: 1,
  });
});

test('progression controls send native promotion and talent actions', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await expect(page.getByRole('heading', { name: 'Stormcaller' })).toBeVisible();

  await page.getByRole('button', { name: 'Promote to Stormcaller' }).click();
  await page.getByRole('button', { name: 'Learn Steady Flame' }).click();

  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.map((entry) => entry.data);
  })).toEqual([
    expect.objectContaining({ action: 'progression.promote', classId: 'stormcaller' }),
    expect.objectContaining({
      action: 'progression.talent', graphId: 'mystic-path', nodeId: 'steady-flame',
    }),
  ]);
});

test('party and channel panels send structured social actions', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();

  await page.locator('summary').filter({ hasText: 'Party' }).click();
  await page.getByLabel('Message to party').fill('Hold the bridge.');
  await page.getByRole('button', { name: 'Send to party' }).click();
  await page.getByLabel('Invite or manage an adventurer').fill('Bram');
  await page.getByRole('button', { name: 'Revoke invitation' }).click();
  await page.getByRole('button', { name: 'Remove from party' }).click();
  await page.getByRole('button', { name: 'Transfer leadership' }).click();
  await page.getByRole('button', { name: 'Disband party' }).click();

  await page.getByLabel('Reply to your latest direct message').fill('On my way.');
  await page.getByRole('button', { name: 'Reply', exact: true }).click();
  await page.getByRole('button', { name: 'Refresh channels' }).click();
  await page.getByLabel('Channel', { exact: true }).selectOption('newbie');
  await page.getByRole('button', { name: 'Join channel' }).click();
  await page.getByRole('button', { name: 'Leave channel' }).click();
  await page.getByLabel('Message to #newbie').fill('Where is the market?');
  await page.getByRole('button', { name: 'Send to channel' }).click();

  await expect.poll(async () => page.evaluate(() => {
    const state = window as Window & { mockActions?: Array<{ data?: unknown }> };
    return state.mockActions?.map((entry) => entry.data);
  })).toEqual([
    expect.objectContaining({ action: 'party.say', message: 'Hold the bridge.' }),
    expect.objectContaining({ action: 'party.revoke', target: 'Bram' }),
    expect.objectContaining({ action: 'party.kick', target: 'Bram' }),
    expect.objectContaining({ action: 'party.transfer', target: 'Bram' }),
    expect.objectContaining({ action: 'party.disband' }),
    expect.objectContaining({ action: 'social.reply', message: 'On my way.' }),
    expect.objectContaining({ action: 'social.channel.list' }),
    expect.objectContaining({ action: 'social.channel.join', channel: 'newbie' }),
    expect.objectContaining({ action: 'social.channel.leave', channel: 'newbie' }),
    expect.objectContaining({
      action: 'social.channel.say', channel: 'newbie', message: 'Where is the market?',
    }),
  ]);
});

test('presentation preferences reconnect with a fresh capability hello', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();

  await page.getByText('Preferences', { exact: true }).click();
  await page.getByLabel('Images', { exact: true }).uncheck();
  await page.getByLabel('Audio (off by default)').check();

  await expect.poll(() => page.evaluate(() => {
    const hellos = (window as Window & { mockHellos?: Array<{
      data?: { capabilities?: { images?: string[]; audio?: boolean } };
    }> }).mockHellos ?? [];
    return hellos.at(-1)?.data?.capabilities;
  })).toEqual({ images: [], reducedMotion: false, audio: true });
  expect(await page.evaluate(() =>
    (window as Window & { mockSocketCount?: number }).mockSocketCount,
  )).toBeGreaterThanOrEqual(3);
});

test('scrollback stays put for readers away from the bottom and can jump to latest', async ({ page }) => {
  await installMockWorld(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();

  await page.evaluate(() => {
    (window as Window & { emitMockEvents?: (count: number) => void }).emitMockEvents?.(520);
  });
  const log = page.getByRole('log', { name: 'Command and world event history' });
  await expect(log.locator('li')).toHaveCount(500);
  await expect.poll(() => log.evaluate((element) =>
    Math.abs(element.scrollHeight - element.clientHeight - element.scrollTop),
  )).toBeLessThanOrEqual(32);

  await log.evaluate((element) => { element.scrollTop = 0; });
  await page.evaluate(() => {
    (window as Window & { emitMockEvents?: (count: number) => void }).emitMockEvents?.(1);
  });
  const latest = page.getByRole('button', { name: /Jump to latest/i });
  await expect(latest).toBeVisible();
  expect(await log.evaluate((element) => element.scrollTop)).toBeLessThan(32);

  await latest.click();
  await expect.poll(() => log.evaluate((element) =>
    Math.abs(element.scrollHeight - element.clientHeight - element.scrollTop),
  )).toBeLessThanOrEqual(32);
});
