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

  it('offers to continue a compatible autosave and exposes the saved games control', () => {
    const slots = [{ slot: 0, updatedAt: 1_760_536_800, compatible: true }];
    const { body } = render(StoryPanel, {
      props: {
        ...props(view([scene()])),
        slots,
        offerContinue: true,
        notice: 'Saved to slot 2.',
        onsave: () => undefined,
        onrestore: () => undefined,
        ondeleteslot: () => undefined,
        onslots: () => undefined,
        oncontinue: () => undefined,
        ondismisscontinue: () => undefined,
      },
    });
    expect(body).toContain('You have an autosaved game from 2025-10-15 14:00 UTC.');
    expect(body).toContain('Continue saved game');
    expect(body).toContain('Start over');
    expect(body).toContain('Saved to slot 2.');
    expect(body).toContain('aria-expanded="false"');
    expect(body).toContain('aria-controls="story-saves"');
    // The panel opens on demand and is not rendered until then.
    expect(body).not.toContain('id="story-saves"');
  });

  it('hides slot controls without slot callbacks and the offer without an autosave', () => {
    const { body } = render(StoryPanel, {
      props: { ...props(view([scene()])), offerContinue: true, slots: [] },
    });
    expect(body).not.toContain('Saved games');
    expect(body).not.toContain('Continue saved game');
  });
});

describe('StoryPanel (shared story)', () => {
  const CHOICE = 'c0123456789abcdef';
  const OTHER = 'c1123456789abcdef';
  const lobby = (you: 'host' | 'player' | 'spectator', mode: 'vote' | 'host' | 'turns' = 'vote') => ({
    mode,
    voteSeconds: 60,
    you,
    ...(you === 'host' ? { code: 'ABCD2345' } : {}),
    seats: [
      { name: 'Aria', role: 'host' as const, connected: true, turn: mode === 'turns' },
      { name: 'Bruno', role: 'player' as const, connected: false, turn: false },
    ],
    spectators: 1,
    maxSeats: 8,
    maxSpectators: 32,
  });
  const shared = (you: 'host' | 'player' | 'spectator', mode: 'vote' | 'host' | 'turns' = 'vote') => ({
    ...props(view([
      scene(),
      { type: 'choice', id: CHOICE, label: 'Avanzar' },
      { type: 'choice', id: OTHER, label: 'Huir' },
    ], false)),
    lobby: lobby(you, mode),
    tally: {
      viewRev: 3, mode, choices: [{ choiceId: CHOICE, votes: 2 }], voted: 2, eligible: 2,
      remainingSeconds: 30, yourVote: CHOICE,
    },
    deadline: Date.now() + 30_000,
    votes: you !== 'spectator' && (mode === 'vote' || (mode === 'host' && you !== 'host')),
    onsave: () => undefined,
    onrestore: () => undefined,
    onshare: () => undefined,
    oninvite: () => true,
    onkick: () => undefined,
    onmode: () => undefined,
    onwithdraw: () => undefined,
  });

  it('shows seats, the code, vote counts, the countdown and host controls to the host', () => {
    const { body } = render(StoryPanel, { props: shared('host') });
    for (const text of [
      'Shared story', 'Majority vote · 60 s window · you are the host.', 'Join code:', 'ABCD2345',
      'aria-label="Seats"', 'Aria', 'disconnected, seat held', 'aria-label="Remove Bruno"',
      'Seats 2/8 · spectators 1/32', 'Votes: 2 of 2.', 'You voted.', '· 2 votes',
      'role="timer"', 'The vote closes in 30 s.', 'Withdraw my vote', 'aria-label="Host controls"',
      'Decision mode', 'Apply rules', 'aria-label="Invite a character"', 'Saved games',
      'Press 1–2 to vote.',
    ]) expect(body, text).toContain(text);
    expect(body).toContain('aria-pressed="true"');
    expect(body).toContain('aria-pressed="false"');
    expect(body).not.toContain('Share story');
    expect(body).not.toContain(CHOICE);
  });

  it('hides the code, host controls and saves from players', () => {
    const { body } = render(StoryPanel, { props: shared('player') });
    expect(body).toContain('you are the player.');
    for (const text of ['ABCD2345', 'Remove Bruno', 'Host controls', 'Saved games']) {
      expect(body, text).not.toContain(text);
    }
  });

  it('disables every action for spectators', () => {
    const { body } = render(StoryPanel, { props: shared('spectator') });
    expect(body).toContain('You are watching as a spectator.');
    expect(body).not.toContain('Press 1');
    const choices = body.slice(body.indexOf('story-choices'));
    expect(choices.match(/<button[^>]*disabled/g)?.length).toBeGreaterThanOrEqual(2);
  });

  it('names the turn holder in turns mode and offers sharing to a solo story', () => {
    const { body } = render(StoryPanel, { props: shared('host', 'turns') });
    expect(body).toContain('Turn: Aria.');
    expect(body).toContain('The turn passes in 30 s.');
    const solo = render(StoryPanel, {
      props: { ...props(view([scene()])), onshare: () => undefined },
    });
    expect(solo.body).toContain('Share story');
  });
});
