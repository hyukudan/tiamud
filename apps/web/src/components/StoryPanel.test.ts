import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { AssembledStoryView } from '../lib/story-protocol';
import StoryPanel from './StoryPanel.svelte';

const VERB = 'c1111111111111111';
const LOOK = 'c4444444444444444';
const TARGET = 'c2222222222222222';

const view = (items: AssembledStoryView['items'], canUndo = true): AssembledStoryView => ({
  viewRev: 3,
  turn: 2,
  canUndo,
  items,
});

const scene = (ending: 'victory' | null = null): AssembledStoryView['items'][number] => ({
  type: 'scene',
  title: 'Lluvia de neón',
  text: 'La oficina.\nLlueve.',
  score: 10,
  ending: ending ? { kind: ending } : null,
});

const props = (story: AssembledStoryView, connected = true) => ({
  view: story,
  connected,
  pending: false,
  onchoose: () => undefined,
  onmenu: () => undefined,
  onundo: () => undefined,
  onleave: () => undefined,
});

describe('StoryPanel', () => {
  it('renders prose, events, speakers, stats and numbered choices as buttons', () => {
    const { body } = render(StoryPanel, {
      props: props(view([
        scene(),
        { type: 'block', kind: 'event', text: 'Tirada 1d6: [4] → total 4' },
        { type: 'block', kind: 'line', speaker: 'Lola', text: '¿Café?' },
        { type: 'stat', label: 'Destreza', value: '9' },
        { type: 'item', name: 'Antorcha' },
        { type: 'choice', id: 'c0123456789abcdef', label: 'Avanzar' },
      ])),
    });
    for (const text of [
      'Lluvia de neón', 'La oficina.', 'Llueve.', 'Tirada 1d6', 'Lola:', '¿Café?', 'Destreza',
      'Antorcha', 'Avanzar', 'Undo', 'Leave story', 'Press 1–1 to choose.',
    ]) expect(body).toContain(text);
    expect(body).toContain('aria-live="polite"');
    expect(body).toContain('lang="es"');
    expect(body).not.toContain('c0123456789abcdef');
  });

  it('groups menu verbs with their targets and offers verbs without targets', () => {
    const { body } = render(StoryPanel, {
      props: props(view([
        scene(),
        { type: 'verb', id: VERB, label: 'MIRAR', needsTarget: true },
        { type: 'target', verb: VERB, id: TARGET, label: 'Ventana' },
        { type: 'verb', id: LOOK, label: 'ESPERAR', needsTarget: false },
      ])),
    });
    expect(body).toContain('<legend>MIRAR</legend>');
    expect(body).toContain('Ventana');
    expect(body).toContain('ESPERAR');
    expect(body).toContain('aria-label="Actions"');
  });

  it('labels dialogue options and disables input while disconnected', () => {
    const { body } = render(StoryPanel, {
      props: props(view([
        scene(),
        { type: 'dialogueOption', id: TARGET, label: 'Despedirte' },
      ], false), false),
    });
    expect(body).toContain('aria-label="Dialogue options"');
    expect(body).toContain('Despedirte');
    expect(body.match(/disabled/g)?.length ?? 0).toBeGreaterThanOrEqual(3);
  });

  it('announces an ending with the final score', () => {
    const { body } = render(StoryPanel, { props: props(view([scene('victory')])) });
    expect(body).toContain('Victory.');
    expect(body).toContain('Final score: 10.');
    expect(body).not.toContain('to choose');
  });
});
