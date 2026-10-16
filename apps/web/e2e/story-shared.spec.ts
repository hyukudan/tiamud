import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Browser, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Two browser contexts play one shared story (ADR 0033). The frames are the
// real server frames recorded per player by `browser_shared_fixture_is_current`
// in tiamud-server; a broker in this test process relays each client message
// in order and delivers the recorded frames of that step to both pages.
type Actor = 'A' | 'B';
interface Frame { type: string; data: Record<string, unknown> }
interface SharedFixture {
  storyId: string;
  title: string;
  code: string;
  steps: {
    actor: Actor;
    input: { type: string; data: Record<string, unknown> };
    label: string | null;
    frames: Record<Actor, Frame[]>;
  }[];
}

const fixture = JSON.parse(readFileSync(
  new URL('./fixtures/story-shared.json', import.meta.url), 'utf8',
)) as SharedFixture;

const canonical = (value: unknown): string => JSON.stringify(value, (_key, inner: unknown) =>
  inner && typeof inner === 'object' && !Array.isArray(inner)
    ? Object.fromEntries(
        Object.entries(inner as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)),
      )
    : inner);

const escapeRegExp = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

class Broker {
  index = 0;
  readonly rejected: string[] = [];
  readonly pages = new Map<Actor, Page>();
  #chain: Promise<void> = Promise.resolve();

  relay(actor: Actor, raw: string): Promise<void> {
    this.#chain = this.#chain.then(() => this.#handle(actor, raw));
    return this.#chain;
  }

  async #deliver(actor: Actor, frames: Frame[], requestId?: string): Promise<void> {
    const page = this.pages.get(actor);
    if (!page || frames.length === 0) return;
    const resolved = frames.map((frame) => frame.type === 'action.result' && requestId
      ? { ...frame, data: { ...frame.data, requestId } }
      : frame);
    await page.evaluate((batch) => {
      (window as Window & { tiamudDeliver?: (frames: unknown[]) => void }).tiamudDeliver?.(batch);
    }, resolved);
  }

  async #handle(actor: Actor, raw: string): Promise<void> {
    const message = JSON.parse(raw) as { type: string; data: Record<string, unknown> };
    if (message.type !== 'command.submit' && !message.type.startsWith('story.')) return;
    const wire = message.type === 'command.submit'
      ? { type: message.type, data: { command: message.data.command } }
      : { type: message.type, data: message.data };
    const step = fixture.steps[this.index];
    if (!step || step.actor !== actor || canonical(step.input) !== canonical(wire)) {
      this.rejected.push(`${actor}: ${canonical(wire)}`);
      await this.#deliver(actor, [{
        type: 'story.error', data: { code: 'unknown_choice', message: 'Esa opción no está disponible.' },
      }]);
      return;
    }
    this.index += 1;
    const requestId = typeof message.data.requestId === 'string' ? message.data.requestId : undefined;
    await this.#deliver('A', step.frames.A, actor === 'A' ? requestId : undefined);
    await this.#deliver('B', step.frames.B, actor === 'B' ? requestId : undefined);
  }
}

async function openPlayer(browser: Browser, broker: Broker, actor: Actor, name: string): Promise<Page> {
  const context = await browser.newContext();
  const page = await context.newPage();
  broker.pages.set(actor, page);
  await page.route('**/api/characters', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ characters: [{ id: actor === 'A' ? 7 : 8, name }] }),
    });
  });
  await page.exposeFunction('tiamudRelay', (raw: string) => broker.relay(actor, raw));
  await page.addInitScript(({ actor: who }) => {
    localStorage.setItem('tiamud.access.v1', JSON.stringify({
      account: { id: who === 'A' ? 1 : 2, name: who === 'A' ? 'aria' : 'bruno' },
      token: 'test-token',
      expiresAt: Date.now() + 3_600_000,
      characterId: who === 'A' ? 7 : 8,
    }));
    const relay = (window as unknown as { tiamudRelay: (raw: string) => Promise<void> }).tiamudRelay;

    class SharedWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = SharedWebSocket.CONNECTING;
      sequence = 0;

      constructor() {
        super();
        (window as Window & { tiamudDeliver?: (frames: unknown[]) => void }).tiamudDeliver =
          (frames) => {
            for (const frame of frames as { type: string; data: unknown }[]) {
              this.emitFrame(frame.type, frame.data);
            }
          };
        queueMicrotask(() => {
          this.readyState = SharedWebSocket.OPEN;
          this.dispatchEvent(new Event('open'));
        });
      }

      emitFrame(type: string, data: unknown): void {
        this.sequence += 1;
        const frame = JSON.stringify({ v: 1, type, seq: this.sequence, data });
        queueMicrotask(() => this.dispatchEvent(new MessageEvent('message', { data: frame })));
      }

      send(raw: string): void {
        const message = JSON.parse(raw) as { type: string };
        if (message.type === 'client.hello') {
          this.emitFrame('server.welcome', { serverName: 'Tiamud', motd: 'Welcome.' });
          this.emitFrame('scene.snapshot', {
            viewId: 'o1111111111111111', revision: 1, title: 'Lantern Square',
            prose: ['Rain shines on the stones.'], exits: [], items: [], npcs: [],
            weather: { condition: 'rainy', day: 1, hour: 2, nextChangeInTicks: 3 },
          });
          return;
        }
        void relay(raw);
      }

      close(): void {
        this.readyState = SharedWebSocket.CLOSED;
        this.dispatchEvent(new Event('close'));
      }
    }

    Object.defineProperty(window, 'WebSocket', { value: SharedWebSocket, configurable: true });
  }, { actor });
  await page.goto('/');
  await page.getByRole('button', { name: new RegExp(`Continue as ${name}`, 'i') }).click();
  await expect(page.getByRole('heading', { name: 'Lantern Square' })).toBeVisible();
  return page;
}

async function type(page: Page, command: string): Promise<void> {
  const input = page.getByLabel('What do you do?');
  await input.fill(command);
  await input.press('Enter');
}

test('two players share a story, vote every choice and reach an ending together', async ({ browser }) => {
  const broker = new Broker();
  const aria = await openPlayer(browser, broker, 'A', 'Aria');
  const bruno = await openPlayer(browser, broker, 'B', 'Bruno');

  await type(aria, `historia compartir ${fixture.storyId}`);
  const hostPanel = aria.locator('section.story-panel');
  await expect(hostPanel.getByRole('heading', { level: 1, name: fixture.title })).toBeVisible();
  const lobby = hostPanel.getByRole('region', { name: 'Shared story' });
  await expect(lobby).toContainText(`Join code: ${fixture.code}`);
  await expect(lobby).toContainText('you are the host');

  await type(bruno, `historia unirse ${fixture.code}`);
  const guestPanel = bruno.locator('section.story-panel');
  await expect(guestPanel.getByRole('region', { name: 'Shared story' })).toContainText('you are the player');
  await expect(guestPanel).not.toContainText(fixture.code);
  await expect(lobby.getByRole('list', { name: 'Seats' }).getByRole('listitem')).toHaveCount(2);
  await expect(hostPanel.getByRole('button', { name: 'Undo' })).toBeDisabled();

  const axe = await new AxeBuilder({ page: aria })
    .include('section.story-panel')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(axe.violations).toEqual([]);

  const voteSteps = fixture.steps.slice(2);
  for (const [index, step] of voteSteps.entries()) {
    const page = step.actor === 'A' ? aria : bruno;
    const other = step.actor === 'A' ? bruno : aria;
    const panel = page.locator('section.story-panel');
    const button = panel.getByRole('list', { name: 'Choices' })
      .getByRole('button', { name: new RegExp(`^${escapeRegExp(step.label ?? '')}`) });
    await expect(button).toBeEnabled();
    if (index === 0) {
      // Keyboard: the first vote by number.
      await panel.getByRole('heading', { level: 1 }).focus();
      await page.keyboard.press('1');
    } else {
      await button.click();
    }
    if (step.actor === 'A') {
      // The other player sees the public tally before voting.
      await expect(other.locator('section.story-panel .story-tally')).toContainText('Votes: 1 of 2.');
      await expect(button).toHaveAttribute('aria-pressed', 'true');
      await expect(button).toContainText('1 vote');
    }
  }

  await expect.poll(() => broker.index).toBe(fixture.steps.length);
  for (const page of [aria, bruno]) {
    const panel = page.locator('section.story-panel');
    await expect(panel.getByRole('status').filter({ hasText: 'The end.' })).toBeVisible();
    await expect(panel.getByRole('list', { name: 'Choices' })).toHaveCount(0);
  }
  expect(broker.rejected).toEqual([]);
});
