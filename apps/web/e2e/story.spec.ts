import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Real `story.view` frames recorded by the Rust server (seeded) together with
// the exact client input that follows each view; regenerated and checked by
// `browser_e2e_fixtures_are_current` in tiamud-server.
interface StoryFixture {
  storyId: string;
  title: string;
  steps: {
    view: unknown[];
    label: string | null;
    input: { type: string; data: Record<string, unknown> } | null;
  }[];
}

const load = (name: string): StoryFixture => JSON.parse(
  readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8'),
) as StoryFixture;

async function installStoryServer(page: Page, story: StoryFixture): Promise<void> {
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
  await page.addInitScript((fixture: StoryFixture) => {
    const state = window as Window & { storyInputs?: number; storyRejected?: number };
    state.storyInputs = 0;
    state.storyRejected = 0;
    localStorage.setItem('tiamud.access.v1', JSON.stringify({
      account: { id: 1, name: 'player' },
      token: 'test-token',
      expiresAt: Date.now() + 3_600_000,
      characterId: 7,
    }));
    // Key-order independent comparison of two small JSON values.
    const canonical = (value: unknown): string => JSON.stringify(value, (_key, inner: unknown) =>
      inner && typeof inner === 'object' && !Array.isArray(inner)
        ? Object.fromEntries(
            Object.entries(inner as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)),
          )
        : inner);

    class StoryWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = StoryWebSocket.CONNECTING;
      sequence = 0;
      stepIndex = 0;

      constructor() {
        super();
        queueMicrotask(() => {
          this.readyState = StoryWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      emitFrame(type: string, data: unknown): void {
        this.sequence += 1;
        const frame = JSON.stringify({ v: 1, type, seq: this.sequence, data });
        queueMicrotask(() => this.dispatchEvent(new MessageEvent('message', { data: frame })));
      }

      sendView(): void {
        for (const part of fixture.steps[this.stepIndex]?.view ?? []) this.emitFrame('story.view', part);
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type: string; data: Record<string, unknown> };
        if (message.type === 'client.hello') {
          this.emitFrame('server.welcome', { serverName: 'Tiamud', motd: 'Welcome.' });
          this.emitFrame('scene.snapshot', {
            viewId: 'o1111111111111111', revision: 1, title: 'Lantern Square',
            prose: ['Rain shines on the stones.'], exits: [], items: [], npcs: [],
            weather: { condition: 'rainy', day: 1, hour: 2, nextChangeInTicks: 3 },
          });
          return;
        }
        if (message.type === 'command.submit') {
          const ok = message.data.command === `historia ${fixture.storyId}`;
          this.emitFrame('action.result', {
            requestId: message.data.requestId, ok,
            message: ok ? `Entras en «${fixture.title}».` : 'Unknown command.',
          });
          if (ok) {
            this.emitFrame('story.entered', { storyId: fixture.storyId, title: fixture.title });
            this.sendView();
          }
          return;
        }
        if (message.type === 'story.leave') {
          this.emitFrame('story.left', { reason: 'left' });
          return;
        }
        if (!message.type.startsWith('story.')) return;
        const expected = fixture.steps[this.stepIndex]?.input;
        if (!expected || canonical(expected) !== canonical({ type: message.type, data: message.data })) {
          state.storyRejected = (state.storyRejected ?? 0) + 1;
          this.emitFrame('story.error', { code: 'unknown_choice', message: 'That option is not available.' });
          return;
        }
        state.storyInputs = (state.storyInputs ?? 0) + 1;
        this.stepIndex += 1;
        this.sendView();
      }

      close(): void {
        this.readyState = StoryWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { value: StoryWebSocket, configurable: true });
  }, story);
}

async function enterStory(page: Page, story: StoryFixture) {
  await installStoryServer(page, story);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();
  const command = page.getByLabel('What do you do?');
  await command.fill(`historia ${story.storyId}`);
  await command.press('Enter');
  const panel = page.locator('section.story-panel');
  await expect(panel.getByRole('heading', { level: 1, name: story.title })).toBeVisible();
  return panel;
}

async function expectAccepted(page: Page, count: number): Promise<void> {
  const counts = await page.evaluate(() => {
    const state = window as Window & { storyInputs?: number; storyRejected?: number };
    return [state.storyInputs, state.storyRejected];
  });
  expect(counts).toEqual([count, 0]);
}

async function expectNoViolations(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page })
    .include('section.story-panel')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(results.violations).toEqual([]);
}

test('the sample gamebook is playable to an ending with keyboard and buttons', async ({ page }) => {
  const story = load('story-crypt.json');
  const panel = await enterStory(page, story);
  await expect(panel.getByRole('button', { name: /Roll one die/ })).toBeVisible();
  await expectNoViolations(page);

  // The first choice by keyboard shortcut, the rest by activating buttons.
  await panel.getByRole('heading', { level: 1 }).focus();
  await page.keyboard.press('1');
  await expect(panel).toContainText('Roll 1d6+6');
  for (const step of story.steps.slice(1, -1)) {
    const button = panel.getByRole('list', { name: 'Choices' })
      .getByRole('button', { name: step.label ?? '' });
    await expect(button).toBeEnabled();
    await button.click();
  }

  await expect(panel.getByRole('status')).toContainText('The end.');
  await expect(panel.getByRole('list', { name: 'Choices' })).toHaveCount(0);
  await expectAccepted(page, story.steps.length - 1);

  await panel.getByRole('button', { name: 'Leave story' }).click();
  await expect(page.locator('section.story-panel')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();
});

test('the parser adventure is playable to its ending by typing commands', async ({ page }) => {
  const story = load('story-lighthouse.json');
  const panel = await enterStory(page, story);
  await expect(panel.getByRole('textbox', { name: 'Your command' })).toBeVisible();
  const field = panel.locator('#story-command');
  await expect(panel.getByRole('log', { name: 'Story replies' }))
    .toContainText('The storm caught you');
  await expectNoViolations(page);

  const steps = story.steps.slice(0, -1);
  for (const [index, step] of steps.entries()) {
    const text = step.label ?? '';
    // Up recalls the previous command from the local history.
    if (index === 1) {
      await field.press('ArrowUp');
      await expect(field).toHaveValue(steps[0].label ?? '');
      await field.press('ArrowDown');
      await expect(field).toHaveValue('');
    }
    // The lamp is ambiguous: the last line answers the parser's question.
    if (index === steps.length - 1) {
      await expect(panel.getByRole('textbox', { name: 'Answer the question' })).toBeVisible();
    }
    await field.fill(text);
    await field.press('Enter');
    await expect(panel.locator('.story-command')).toHaveText(`> ${text}`);
    // Focus stays in the command field after each answer.
    if (index < steps.length - 1) await expect(field).toBeFocused();
  }
  await expect(panel.getByRole('log', { name: 'Story replies' }))
    .toContainText('You open the glass globe');
  await expect(panel.getByRole('status')).toContainText('Victory.');
  await expect(panel.getByRole('textbox')).toHaveCount(0);
  await expectAccepted(page, steps.length);
  await expectNoViolations(page);
});

test('the verb-menu adventure plays verbs, targets and dialogue to its ending', async ({ page }) => {
  const story = load('story-neon-rain.json');
  const panel = await enterStory(page, story);
  await expectNoViolations(page);

  for (const step of story.steps.slice(0, -1)) {
    const label = step.label ?? '';
    const [verb, target] = label.split(' → ');
    const button = target
      ? panel.getByRole('group', { name: verb }).getByRole('button', { name: target, exact: true })
      : panel.getByRole('list', { name: 'Dialogue options' }).getByRole('button', { name: label });
    await expect(button).toBeEnabled();
    await button.click();
  }

  await expect(panel.getByRole('status')).toContainText('Victory.');
  await expectAccepted(page, story.steps.length - 1);
});

/** Plays the recorded menu adventure up to the club (six actions). */
async function walkToTheClub(panel: ReturnType<Page['locator']>, story: StoryFixture): Promise<void> {
  for (const step of story.steps.slice(0, 6)) {
    const [verb, target] = (step.label ?? '').split(' → ');
    await panel.getByRole('group', { name: verb }).getByRole('button', { name: target, exact: true }).click();
  }
}

/** Alt text of the first background and portrait the server recorded. */
function recordedAlt(story: StoryFixture): { office: string; club: string; lola: string } {
  const items = (index: number) => story.steps[index].view
    .flatMap((part) => (part as { items: Record<string, unknown>[] }).items);
  const alt = (item: Record<string, unknown> | undefined, key: string) =>
    ((item?.[key] as { alt?: string } | undefined)?.alt) ?? '';
  return {
    office: alt(items(0).find((item) => item.type === 'scene'), 'background'),
    club: alt(items(6).find((item) => item.type === 'scene'), 'background'),
    lola: alt(items(6).find((item) => item.type === 'portrait'), 'image'),
  };
}

test('reviewed story backgrounds and portraits load with their alt text', async ({ page }) => {
  const story = load('story-neon-rain.json');
  const alt = recordedAlt(story);
  expect(alt.office).toContain("detective's office");
  const panel = await enterStory(page, story);

  const office = panel.getByRole('img', { name: alt.office });
  await expect(office).toBeVisible();
  // The browser decoded a real pipeline variant, not a broken image.
  await expect.poll(() => office.evaluate((image: HTMLImageElement) => image.naturalWidth))
    .toBeGreaterThan(0);
  expect(await office.evaluate((image: HTMLImageElement) => image.currentSrc))
    .toMatch(/\/assets\/[a-f0-9]{64}\/scene-(640|1280)\.(avif|webp|jpg)$/);
  await expectNoViolations(page);

  await walkToTheClub(panel, story);
  await expect(panel.getByRole('img', { name: alt.club })).toBeVisible();
  const portraits = panel.getByRole('list', { name: 'Portraits' });
  await expect(portraits.getByRole('img', { name: alt.lola })).toBeVisible();
  await expect(panel.getByText('Present: Lola')).toBeVisible();
  await expectNoViolations(page);
  await expectAccepted(page, 6);
});

test('missing story images degrade to their alt text and play goes on', async ({ page }) => {
  const story = load('story-neon-rain.json');
  const alt = recordedAlt(story);
  let blocked = 0;
  await page.route(/\/assets\/[a-f0-9]{64}\/manifest\.json$/, async (route) => {
    blocked += 1;
    await route.fulfill({ status: 404, body: '' });
  });
  const panel = await enterStory(page, story);
  await expect(panel.getByText(alt.office)).toBeVisible();
  await expect(panel.getByRole('img')).toHaveCount(0);
  await walkToTheClub(panel, story);
  await expect(panel.getByText(alt.club)).toBeVisible();
  await expect(panel.getByRole('img')).toHaveCount(0);
  await expect(panel.getByText('Present: Lola')).toBeVisible();
  await expectAccepted(page, 6);
  expect(blocked).toBeGreaterThan(0);
});

test('reader mode shows story alt text without downloading images', async ({ page }) => {
  const story = load('story-neon-rain.json');
  const alt = recordedAlt(story);
  const requested: string[] = [];
  page.on('request', (request) => {
    if (/\/assets\/[a-f0-9]{64}\//.test(request.url())) requested.push(request.url());
  });
  await page.addInitScript(() => {
    localStorage.setItem('tiamud.preferences.v1', JSON.stringify({ screenReader: true }));
  });
  const panel = await enterStory(page, story);
  await expect(panel.getByText(alt.office)).toBeVisible();
  await expect(panel.getByRole('img')).toHaveCount(0);
  await walkToTheClub(panel, story);
  await expect(panel.getByText(alt.club)).toBeVisible();
  expect(requested).toEqual([]);
});

// Real server payloads of the save → leave → re-enter → continue scenario,
// recorded by `browser_slot_fixture_is_current` in tiamud-server.
interface SlotFixture {
  storyId: string;
  title: string;
  enter: { view: unknown[]; slots: unknown };
  play: { input: { type: string; data: Record<string, unknown> }; label: string; view: unknown[] };
  save: { input: { type: string; data: Record<string, unknown> }; saved: unknown };
  slots: { input: { type: string; data: Record<string, unknown> }; slots: unknown };
  reenter: { view: unknown[]; slots: unknown };
  continue: { input: { type: string; data: Record<string, unknown> }; restored: unknown; view: unknown[] };
}

async function installSlotServer(page: Page, story: SlotFixture): Promise<void> {
  await page.route('**/api/characters', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ characters: [{ id: 7, name: 'Aria' }] }),
    });
  });
  await page.addInitScript((fixture: SlotFixture) => {
    const state = window as Window & { slotInputs?: string[]; slotRejected?: number };
    state.slotInputs = [];
    state.slotRejected = 0;
    localStorage.setItem('tiamud.access.v1', JSON.stringify({
      account: { id: 1, name: 'player' },
      token: 'test-token',
      expiresAt: Date.now() + 3_600_000,
      characterId: 7,
    }));
    const canonical = (value: unknown): string => JSON.stringify(value, (_key, inner: unknown) =>
      inner && typeof inner === 'object' && !Array.isArray(inner)
        ? Object.fromEntries(
            Object.entries(inner as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)),
          )
        : inner);

    class SlotWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = SlotWebSocket.CONNECTING;
      sequence = 0;
      entries = 0;

      constructor() {
        super();
        queueMicrotask(() => {
          this.readyState = SlotWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      emitFrame(type: string, data: unknown): void {
        this.sequence += 1;
        const frame = JSON.stringify({ v: 1, type, seq: this.sequence, data });
        queueMicrotask(() => this.dispatchEvent(new MessageEvent('message', { data: frame })));
      }

      emitView(parts: unknown[]): void {
        for (const part of parts) this.emitFrame('story.view', part);
      }

      accept(step: { type: string; data: Record<string, unknown> }, message: unknown): boolean {
        if (canonical(step) === canonical(message)) {
          state.slotInputs?.push(step.type);
          return true;
        }
        state.slotRejected = (state.slotRejected ?? 0) + 1;
        this.emitFrame('story.error', { code: 'unknown_choice', message: 'That option is not available.' });
        return false;
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type: string; data: Record<string, unknown> };
        const wire = { type: message.type, data: message.data };
        switch (message.type) {
          case 'client.hello':
            this.emitFrame('server.welcome', { serverName: 'Tiamud', motd: 'Welcome.' });
            this.emitFrame('scene.snapshot', {
              viewId: 'o1111111111111111', revision: 1, title: 'Lantern Square',
              prose: ['Rain shines on the stones.'], exits: [], items: [], npcs: [],
              weather: { condition: 'rainy', day: 1, hour: 2, nextChangeInTicks: 3 },
            });
            return;
          case 'command.submit': {
            const ok = message.data.command === `historia ${fixture.storyId}`;
            this.emitFrame('action.result', {
              requestId: message.data.requestId, ok,
              message: ok ? `Entras en «${fixture.title}».` : 'Unknown command.',
            });
            if (!ok) return;
            const entry = this.entries === 0 ? fixture.enter : fixture.reenter;
            this.entries += 1;
            this.emitFrame('story.entered', { storyId: fixture.storyId, title: fixture.title });
            this.emitView(entry.view);
            this.emitFrame('story.slots', entry.slots);
            return;
          }
          case 'story.choose':
            if (this.accept(fixture.play.input, wire)) this.emitView(fixture.play.view);
            return;
          case 'story.save':
            if (this.accept(fixture.save.input, wire)) this.emitFrame('story.saved', fixture.save.saved);
            return;
          case 'story.slots':
            if (this.accept(fixture.slots.input, wire)) this.emitFrame('story.slots', fixture.slots.slots);
            return;
          case 'story.continue':
            if (this.accept(fixture.continue.input, wire)) {
              this.emitFrame('story.restored', fixture.continue.restored);
              this.emitView(fixture.continue.view);
            }
            return;
          case 'story.leave':
            state.slotInputs?.push('story.leave');
            this.emitFrame('story.left', { reason: 'left' });
            return;
          default:
            if (message.type.startsWith('story.')) {
              state.slotRejected = (state.slotRejected ?? 0) + 1;
            }
        }
      }

      close(): void {
        this.readyState = SlotWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { value: SlotWebSocket, configurable: true });
  }, story);
}

test('a saved game survives leaving and continues after re-entering, by keyboard', async ({ page }) => {
  const story = JSON.parse(readFileSync(
    new URL('./fixtures/story-slots.json', import.meta.url), 'utf8',
  )) as SlotFixture;
  await installSlotServer(page, story);
  await page.goto('/');
  await page.getByRole('button', { name: /Continue as Aria/i }).click();
  const command = page.getByLabel('What do you do?');
  await command.fill(`historia ${story.storyId}`);
  await command.press('Enter');
  const panel = page.locator('section.story-panel');
  await expect(panel.getByRole('heading', { level: 1, name: story.title })).toBeVisible();
  // No autosave yet: no offer to continue.
  await expect(panel.getByRole('button', { name: 'Continue saved game' })).toHaveCount(0);

  await panel.getByRole('button', { name: story.play.label }).click();
  await expect(panel).toContainText('Take the torch');

  // Open the saved games panel from the keyboard; it lists the slots.
  const toggle = panel.getByRole('button', { name: 'Saved games', exact: true });
  await toggle.focus();
  await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const saves = panel.getByRole('region', { name: 'Saved games' });
  await expect(saves.getByRole('heading', { name: 'Saved games' })).toBeFocused();
  await expect(saves.getByRole('list', { name: 'Save slots' }).getByRole('listitem')).toHaveCount(10);
  await expectNoViolations(page);
  // Number keys do not choose story actions while the panel is open.
  await page.keyboard.press('1');

  await saves.getByRole('button', { name: 'Save to Slot 1' }).focus();
  await page.keyboard.press('Enter');
  await expect(panel.getByRole('status').filter({ hasText: 'Saved to slot 1.' })).toBeVisible();
  await expect(saves.getByRole('button', { name: 'Load Slot 1' })).toBeEnabled();
  await page.keyboard.press('Escape');
  await expect(saves).toHaveCount(0);
  await expect(toggle).toBeFocused();

  await panel.getByRole('button', { name: 'Leave story' }).click();
  await expect(page.locator('section.story-panel')).toHaveCount(0);

  // Re-entering starts fresh and offers the autosave written on leave.
  await command.fill(`historia ${story.storyId}`);
  await command.press('Enter');
  const again = page.locator('section.story-panel');
  const offer = again.getByRole('group', { name: /You have an autosaved game/ });
  await expect(offer).toBeVisible();
  await expect(again).toContainText('Roll one die');
  await offer.getByRole('button', { name: 'Continue saved game' }).click();
  await expect(again.getByRole('status').filter({ hasText: 'Continued from the autosave.' })).toBeVisible();
  await expect(again.getByRole('button', { name: 'Follow the passage north' })).toBeEnabled();
  await expect(offer).toHaveCount(0);

  const inputs = await page.evaluate(() => {
    const state = window as Window & { slotInputs?: string[]; slotRejected?: number };
    return [state.slotInputs, state.slotRejected];
  });
  expect(inputs).toEqual([
    ['story.choose', 'story.slots', 'story.save', 'story.leave', 'story.continue'], 0,
  ]);
});
