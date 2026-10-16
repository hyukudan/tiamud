import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Real `story.catalog`, `story.entered` and Spanish `story.view` frames
// recorded by the Rust server (`browser_e2e_fixtures_are_current`), plus the
// recorded English menu adventure whose dialogue lines carry speaker
// portraits. The catalog holds a Spanish-only story and the two bilingual
// samples, which default to English.
interface CatalogFixture {
  catalog: unknown[];
  entered: { storyId: string; title: string; locale: string; locales: string[] };
  view: unknown[];
}

interface StoryFixture {
  storyId: string;
  title: string;
  steps: {
    view: unknown[];
    label: string | null;
    input: { type: string; data: Record<string, unknown> } | null;
  }[];
}

const catalog = JSON.parse(readFileSync(
  new URL('./fixtures/story-catalog.json', import.meta.url), 'utf8',
)) as CatalogFixture;
const lluvia = JSON.parse(readFileSync(
  new URL('./fixtures/story-neon-rain.json', import.meta.url), 'utf8',
)) as StoryFixture;

/**
 * A scripted server: `story.catalog` answers with the recorded catalog,
 * `historia neon-rain es` enters the recorded Spanish instance and
 * `historia neon-rain en` replays the recorded English adventure.
 */
async function installPickerServer(page: Page): Promise<void> {
  await page.route('**/api/characters', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ characters: [{ id: 7, name: 'Aria' }] }),
    });
  });
  await page.addInitScript((fixtures: { catalog: CatalogFixture; lluvia: StoryFixture }) => {
    const state = window as Window & { pickerCommands?: string[]; catalogRequests?: number };
    state.pickerCommands = [];
    state.catalogRequests = 0;
    localStorage.setItem('tiamud.access.v1', JSON.stringify({
      account: { id: 1, name: 'player' },
      token: 'test-token',
      expiresAt: Date.now() + 3_600_000,
      characterId: 7,
    }));

    class PickerWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = PickerWebSocket.CONNECTING;
      sequence = 0;
      step = 0;

      constructor() {
        super();
        queueMicrotask(() => {
          this.readyState = PickerWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      emitFrame(type: string, data: unknown): void {
        this.sequence += 1;
        const frame = JSON.stringify({ v: 1, type, seq: this.sequence, data });
        queueMicrotask(() => this.dispatchEvent(new MessageEvent('message', { data: frame })));
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type: string; data: Record<string, unknown> };
        switch (message.type) {
          case 'client.hello':
            this.emitFrame('server.welcome', { serverName: 'Tiamud', motd: 'Welcome.' });
            this.emitFrame('scene.snapshot', {
              viewId: 'o1111111111111111', revision: 1, title: 'Lantern Square',
              prose: ['Rain shines on the stones.'], exits: [], items: [], npcs: [],
              weather: { condition: 'rainy', day: 1, hour: 2, nextChangeInTicks: 3 },
            });
            return;
          case 'story.catalog':
            state.catalogRequests = (state.catalogRequests ?? 0) + 1;
            for (const part of fixtures.catalog.catalog) this.emitFrame('story.catalog', part);
            return;
          case 'command.submit': {
            const command = String(message.data.command);
            state.pickerCommands?.push(command);
            const english = command === 'historia neon-rain en';
            const spanish = command === 'historia neon-rain es';
            this.emitFrame('action.result', {
              requestId: message.data.requestId, ok: english || spanish,
              message: english ? 'You enter «Neon Rain».' : spanish ? 'Entras en «Lluvia de neón».' : 'Unknown command.',
            });
            if (spanish) {
              this.emitFrame('story.entered', fixtures.catalog.entered);
              for (const part of fixtures.catalog.view) this.emitFrame('story.view', part);
            } else if (english) {
              this.emitFrame('story.entered', {
                storyId: 'neon-rain', title: 'Neon Rain', locale: 'en', locales: ['en', 'es'],
              });
              for (const part of fixtures.lluvia.steps[0].view) this.emitFrame('story.view', part);
            }
            return;
          }
          case 'story.menu':
          case 'story.dialogue':
          case 'story.choose':
            this.step += 1;
            for (const part of fixtures.lluvia.steps[this.step]?.view ?? []) {
              this.emitFrame('story.view', part);
            }
            return;
          case 'story.leave':
            this.emitFrame('story.left', { reason: 'left' });
            return;
          default:
        }
      }

      close(): void {
        this.readyState = PickerWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { value: PickerWebSocket, configurable: true });
  }, { catalog, lluvia });
}

async function openPicker(page: Page) {
  await installPickerServer(page);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();
  const toggle = page.getByRole('button', { name: 'Stories', exact: true });
  await toggle.click();
  const picker = page.getByRole('region', { name: 'Stories' });
  await expect(picker.getByRole('heading', { name: 'Stories' })).toBeFocused();
  return { picker, toggle };
}

async function expectNoViolations(page: Page, selector: string): Promise<void> {
  const results = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(results.violations).toEqual([]);
}

test.describe('with an English browser', () => {
  test.use({ locale: 'en-US' });

  test('the picker preselects English and enters the story in it', async ({ page }) => {
    const { picker, toggle } = await openPicker(page);
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    const stories = picker.getByRole('list', { name: 'Available stories' }).getByRole('listitem');
    await expect(stories).toHaveCount(3);
    // A single-language story shows its language without a select.
    await expect(stories.nth(0)).toContainText('Breve');
    await expect(stories.nth(0).getByRole('combobox')).toHaveCount(0);
    await expect(stories.nth(1)).toContainText('Crypt of the Necromancer');
    const language = picker.getByRole('combobox', { name: 'Language for Neon Rain' });
    await expect(language).toHaveValue('en');
    await expectNoViolations(page, 'section.story-picker');

    await stories.nth(2).getByRole('button', { name: 'Play' }).click();
    const panel = page.locator('section.story-panel');
    await expect(panel.getByRole('heading', { level: 1, name: 'Neon Rain' })).toBeVisible();
    await expect(panel.getByRole('heading', { level: 1 })).toHaveAttribute('lang', 'en');
    await expect(panel.getByRole('group', { name: 'INVESTIGATE' })).toBeVisible();
    await expect(page.locator('section.story-picker')).toHaveCount(0);
    expect(await page.evaluate(() => (window as Window & { pickerCommands?: string[] }).pickerCommands))
      .toEqual(['historia neon-rain en']);
  });
});

test.describe('with a Spanish browser', () => {
  test.use({ locale: 'es-ES' });

  test('the picker preselects Spanish and enters the story in it', async ({ page }) => {
    const { picker } = await openPicker(page);
    const language = picker.getByRole('combobox', { name: 'Language for Neon Rain' });
    await expect(language).toHaveValue('es');
    await picker.getByRole('listitem').nth(2).getByRole('button', { name: 'Play' }).click();
    const panel = page.locator('section.story-panel');
    await expect(panel.getByRole('heading', { level: 1, name: 'Lluvia de neón' })).toBeVisible();
    await expect(panel.getByRole('heading', { level: 1 })).toHaveAttribute('lang', 'es');
    await expect(panel.getByRole('group', { name: 'INVESTIGAR' })).toBeVisible();
    expect(await page.evaluate(() => (window as Window & { pickerCommands?: string[] }).pickerCommands))
      .toEqual(['historia neon-rain es']);
  });
});

test.describe('with a French browser', () => {
  test.use({ locale: 'fr-FR' });

  test('the picker falls back to the story default, switches language by keyboard and closes with Escape', async ({ page }) => {
    const { picker, toggle } = await openPicker(page);
    const language = picker.getByRole('combobox', { name: 'Language for Neon Rain' });
    await expect(language).toHaveValue('en');
    await language.selectOption('es');
    await expect(language).toHaveValue('es');
    await language.selectOption('en');
    await page.keyboard.press('Escape');
    await expect(page.locator('section.story-picker')).toHaveCount(0);
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(await page.evaluate(() => (window as Window & { catalogRequests?: number }).catalogRequests))
      .toBe(1);
  });
});

/** Plays the recorded English adventure up to Lola's first dialogue line. */
async function talkToLola(page: Page, readerMode = false) {
  if (readerMode) {
    await page.addInitScript(() => {
      localStorage.setItem('tiamud.preferences.v1', JSON.stringify({ screenReader: true }));
    });
  }
  const { picker } = await openPicker(page);
  await picker.getByRole('combobox', { name: 'Language for Neon Rain' }).selectOption('en');
  await picker.getByRole('listitem').nth(2).getByRole('button', { name: 'Play' }).click();
  const panel = page.locator('section.story-panel');
  await expect(panel.getByRole('heading', { level: 1, name: 'Neon Rain' })).toBeVisible();
  for (const step of lluvia.steps.slice(0, 7)) {
    const [verb, target] = (step.label ?? '').split(' → ');
    await panel.getByRole('group', { name: verb }).getByRole('button', { name: target, exact: true }).click();
  }
  await expect(panel.getByRole('list', { name: 'Dialogue options' })).toBeVisible();
  return panel;
}

/** Alt text of the first dialogue line portrait the server recorded. */
function recordedLineAlt(): string {
  for (const step of lluvia.steps) {
    for (const part of step.view as { items: Record<string, unknown>[] }[]) {
      for (const item of part.items) {
        const portrait = item.portrait as { alt?: string } | undefined;
        if (item.type === 'block' && portrait?.alt) return portrait.alt;
      }
    }
  }
  return '';
}

test('dialogue lines show the speaker portrait beside the text', async ({ page }) => {
  const alt = recordedLineAlt();
  expect(alt).toContain('Lola');
  const panel = await talkToLola(page);
  const row = panel.locator('.story-line-row').first();
  const portrait = row.getByRole('img', { name: alt });
  await expect(portrait).toBeVisible();
  await expect.poll(() => portrait.evaluate((image: HTMLImageElement) => image.naturalWidth))
    .toBeGreaterThan(0);
  await expect(row).toContainText('Lola:');
  await expectNoViolations(page, 'section.story-panel');
});

test('reader mode keeps dialogue lines as text without downloading portraits', async ({ page }) => {
  const requested: string[] = [];
  page.on('request', (request) => {
    if (/\/assets\/[a-f0-9]{64}\//.test(request.url())) requested.push(request.url());
  });
  const panel = await talkToLola(page, true);
  await expect(panel.locator('.story-line-row')).toHaveCount(0);
  await expect(panel.locator('p.story-line').filter({ hasText: 'Lola:' }).first()).toBeVisible();
  await expect(panel.getByRole('img')).toHaveCount(0);
  expect(requested).toEqual([]);
});
