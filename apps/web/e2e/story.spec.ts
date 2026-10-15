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
          this.emitFrame('story.error', { code: 'unknown_choice', message: 'Esa opción no está disponible.' });
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

test('the sample Spanish gamebook is playable to an ending with keyboard and buttons', async ({ page }) => {
  const story = load('story-cripta.json');
  const panel = await enterStory(page, story);
  await expect(panel.getByRole('button', { name: /Tira un dado/ })).toBeVisible();
  await expectNoViolations(page);

  // The first choice by keyboard shortcut, the rest by activating buttons.
  await panel.getByRole('heading', { level: 1 }).focus();
  await page.keyboard.press('1');
  await expect(panel).toContainText('Tirada 1d6+6');
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

test('the verb-menu adventure plays verbs, targets and dialogue to its ending', async ({ page }) => {
  const story = load('story-lluvia.json');
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
