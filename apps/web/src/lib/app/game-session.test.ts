import { describe, expect, it, vi } from 'vitest';
import type {
  BoardEntryData,
  BoardListData,
  ContainerContentsData,
  LiveServerMessage,
  PlayerStateSnapshot,
  SceneSnapshot,
} from '../protocol';
import type { ConnectionCallbacks } from '../socket';
import { MAX_SCROLLBACK_ENTRIES } from '../scrollback';
import { GameSession, type HistoryView, type SessionConnection } from './game-session.svelte';

const scene = (revision = 1, items: Array<{ id: string }> = []) =>
  ({ viewId: 'view-1', revision, title: 'Lantern Square', items, npcs: [], exits: [], prose: [] }) as unknown as SceneSnapshot;
const player = (revision = 1) =>
  ({ revision, inventory: [], equipment: [], map: { currentRoomId: 'a', rooms: [], paths: [] } }) as unknown as PlayerStateSnapshot;
const container = { container: 'o1', items: [], page: 1, totalPages: 1 } as unknown as ContainerContentsData;
const boardList = {
  board: 'general', page: 1, totalPages: 1, totalPosts: 1, capacity: 1000,
  posts: [{ postId: 'p1', board: 'general', author: 'Aria', subject: 'Hi', canDelete: true, canReport: true }],
} as unknown as BoardListData;
const boardEntry = { ...boardList.posts[0], body: 'Hello' } as unknown as BoardEntryData;

function message(type: string, data: unknown, seq = 1): LiveServerMessage {
  return { v: 1, type, seq, data } as unknown as LiveServerMessage;
}

function harness() {
  let callbacks!: ConnectionCallbacks;
  const connection = {
    connect: vi.fn(),
    close: vi.fn(),
    submit: vi.fn(() => true),
    perform: vi.fn(() => true),
    performAbility: vi.fn(() => true),
    promote: vi.fn(() => true),
    acquireTalent: vi.fn(() => true),
    updateCapabilities: vi.fn(() => true),
    storyChoose: vi.fn(() => true),
    storyDialogue: vi.fn(() => true),
    storyMenu: vi.fn(() => true),
    storyUndo: vi.fn(() => true),
    storyLeave: vi.fn(() => true),
    storySave: vi.fn(() => true),
    storyRestore: vi.fn(() => true),
    storyDeleteSlot: vi.fn(() => true),
    storySlots: vi.fn(() => true),
    storyContinue: vi.fn(() => true),
    storyVote: vi.fn(() => true),
    storyShare: vi.fn(() => true),
    storyJoin: vi.fn(() => true),
    storyInvite: vi.fn(() => true),
    storyKick: vi.fn(() => true),
    storyMode: vi.fn(() => true),
    storyCommand: vi.fn(() => true),
    storyCatalog: vi.fn(() => true),
  } satisfies SessionConnection;
  const storage = new Map<string, string>();
  const historyView: HistoryView = {
    beforeReplace: vi.fn(() => true),
    afterReplace: vi.fn(),
    reset: vi.fn(),
  };
  const audio = { play: vi.fn(async () => true), stop: vi.fn() };
  const revoke = vi.fn(async () => undefined);
  const session = new GameSession({
    storage: () => ({
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => void storage.set(key, value),
      removeItem: (key) => void storage.delete(key),
    }),
    capabilities: () => ({ images: true, reducedMotion: false, audio: false }),
    audio,
    historyView,
    socketUrl: () => 'wss://tiamud.test/ws',
    createConnection: (_url, _auth, received) => {
      callbacks = received;
      return connection;
    },
    revoke,
  });
  session.enterWorld({
    account: { id: 1, name: 'player' },
    token: 'token',
    expiresAt: Date.now() / 1000 + 3600,
    characterId: 7,
    characterName: 'Aria',
  } as never);
  return { session, connection, callbacks: () => callbacks, historyView, audio, revoke, storage };
}

describe('GameSession', () => {
  it('connects once on entry and routes authoritative snapshots', () => {
    const { session, connection, callbacks } = harness();
    expect(connection.connect).toHaveBeenCalledOnce();
    expect(session.hasEntered).toBe(true);
    callbacks().onMessage(message('scene.snapshot', scene()));
    callbacks().onMessage(message('player.snapshot', player()));
    expect(session.scene?.title).toBe('Lantern Square');
    expect(session.player?.revision).toBe(1);
  });

  it('applies correlated recovery atomically and clears pull-only panels', () => {
    const { session, callbacks } = harness();
    callbacks().onMessage(message('container.contents', container));
    callbacks().onMessage(message('board.list', boardList));
    callbacks().onSync?.({ syncId: 's', afterSeq: 0, scene: scene(5), player: player(9) });
    expect(session.scene?.revision).toBe(5);
    expect(session.player?.revision).toBe(9);
    expect(session.containerContents).toBeUndefined();
    expect(session.boardList).toBeUndefined();
    expect(session.activeVisualCues).toEqual({});
  });

  it('never carries a container view across mutations, disconnects or sessions', async () => {
    const { session, callbacks } = harness();
    for (const action of ['item.put', 'item.get', 'item.contents', 'loot.pass']) {
      callbacks().onMessage(message('container.contents', container));
      session.submitAction(action, { target: 'o1' }, action);
      expect(session.containerContents).toBeUndefined();
    }
    callbacks().onMessage(message('container.contents', container));
    session.submitAction('loot.status', undefined, 'loot status');
    expect(session.containerContents).toBe(container);

    callbacks().onStateChange('disconnected');
    expect(session.containerContents).toBeUndefined();

    callbacks().onMessage(message('container.contents', container));
    session.returnToLogin('Bye');
    expect(session.containerContents).toBeUndefined();
    expect(session.hasEntered).toBe(false);
    expect(session.accessError).toBe('Bye');

    const second = harness();
    second.callbacks().onMessage(message('container.contents', container));
    await second.session.signOut();
    expect(second.session.containerContents).toBeUndefined();
    expect(second.connection.close).toHaveBeenCalled();
    expect(second.revoke).toHaveBeenCalledWith('token');
  });

  it('applies board mutations only after a correlated success', () => {
    const { session, connection, callbacks } = harness();
    callbacks().onMessage(message('board.list', boardList));
    callbacks().onMessage(message('board.entry', boardEntry));
    const id = session.submitAction('board.delete', { postId: 'p1' }, 'board delete');
    expect(id).toBeDefined();
    expect(connection.perform).toHaveBeenCalled();
    expect(session.boardEntry).toBeDefined();

    callbacks().onMessage(message('action.result', { requestId: 'other', ok: true, message: null }));
    expect(session.boardEntry).toBeDefined();
    callbacks().onMessage(message('action.result', { requestId: id, ok: false, message: 'No.' }));
    expect(session.boardEntry).toBeDefined();

    const retry = session.submitAction('board.report', { postId: 'p1' }, 'board report');
    callbacks().onMessage(message('action.result', { requestId: retry, ok: true, message: 'Reported.' }));
    expect(session.boardEntry?.canReport).toBe(false);
    expect(session.boardList?.posts[0].canReport).toBe(false);
    expect(session.history.at(-1)).toMatchObject({ state: 'complete', response: 'Reported.' });
  });

  it('bounds pending structured confirmations', () => {
    const { session } = harness();
    for (let index = 0; index < MAX_SCROLLBACK_ENTRIES + 25; index += 1) {
      session.submitAction('board.post', { board: 'general' } as never, 'post');
    }
    expect(session.pendingStructuredActionCount).toBe(MAX_SCROLLBACK_ENTRIES);
  });

  it('marks unconfirmed commands as interrupted when the connection drops', () => {
    const { session, callbacks } = harness();
    session.submitCommand('look');
    expect(session.history.at(-1)?.state).toBe('pending');
    callbacks().onStateChange('disconnected');
    expect(session.connectionState).toBe('disconnected');
    expect(session.history.at(-1)).toMatchObject({
      state: 'failed',
      response: 'The connection closed before the server confirmed this action.',
    });
  });

  it('records unsent commands as failed and remembers them for history navigation', () => {
    const { session, connection, storage } = harness();
    connection.submit.mockReturnValueOnce(false);
    session.command = '  north ';
    session.submitCommand();
    expect(session.history.at(-1)).toMatchObject({ command: 'north', state: 'failed' });
    expect(session.command).toBe('');
    expect(session.rememberedCommands.at(-1)).toBe('north');
    expect([...storage.values()].some((value) => value.includes('north'))).toBe(true);

    const event = { key: 'ArrowUp', preventDefault: vi.fn() } as unknown as KeyboardEvent;
    session.navigateCommandHistory(event);
    expect(session.command).toBe('north');
  });

  it('appends world and chat events to the bounded history through the view', () => {
    const { session, callbacks, historyView } = harness();
    callbacks().onMessage(message('message.append', {
      revision: 1, kind: 'characterMoved', text: 'Bram arrives.', actorId: 'o2',
    }, 4));
    callbacks().onMessage(message('chat.message', {
      channel: 'say', sender: 'Bram', text: 'Hello.',
    }, 5));
    expect(session.history.map((entry) => entry.command)).toEqual(['World event', 'Bram says']);
    expect(historyView.afterReplace).toHaveBeenCalledWith(true, 1);
  });

  it('routes media cues to the optional audio sink only', () => {
    const { audio, callbacks } = harness();
    callbacks().onMessage(message('media.play', { channel: 'music', url: '/a', loop: true }));
    callbacks().onMessage(message('media.stop', { channel: 'music' }));
    expect(audio.play).toHaveBeenCalledWith('music', '/a', true);
    expect(audio.stop).toHaveBeenCalledWith('music');
  });

  it('returns to login without connecting for an expired selection', () => {
    let created = 0;
    const session = new GameSession({
      storage: () => undefined,
      capabilities: () => ({ images: true, reducedMotion: false, audio: false }),
      createConnection: () => {
        created += 1;
        return {} as SessionConnection;
      },
    });
    expect(session.enterWorld({
      account: { id: 1, name: 'player' }, token: 't', expiresAt: 1, characterId: 7, characterName: 'Aria',
    } as never)).toBe(false);
    expect(created).toBe(0);
    expect(session.accessError).toBe('Your session has expired. Sign in again.');
  });

  it('assembles story views, routes inputs by kind and plays reviewed cues', () => {
    const { session, connection, callbacks, audio } = harness();
    const VERB = 'c1111111111111111';
    const TARGET = 'c2222222222222222';
    const OPTION = 'c3333333333333333';
    const url = `/assets/${'b'.repeat(64)}/audio-x.ogg`;
    const scene = { type: 'scene', title: 'Lluvia', text: 'Llueve.', score: 0, ending: null };
    callbacks().onMessage(message('story.entered', {
      storyId: 'lluvia-de-neon', title: 'Neon Rain', locale: 'en', locales: ['es', 'en'],
    }, 4));
    // The panel renders story text in the instance language.
    expect(session.storyLocale).toBe('en');
    expect(session.storyLocales).toEqual(['es', 'en']);
    callbacks().onMessage(message('story.view', {
      viewRev: 1, part: 0, parts: 2, turn: 0, canUndo: false,
      items: [scene, { type: 'cue', action: 'play', channel: 'music', url, loop: true }],
    }, 5));
    expect(session.story).toBeUndefined();
    callbacks().onMessage(message('story.view', {
      viewRev: 1, part: 1, parts: 2, turn: 0, canUndo: false,
      items: [
        { type: 'verb', id: VERB, label: 'MIRAR', needsTarget: true },
        { type: 'target', verb: VERB, id: TARGET, label: 'Ventana' },
        { type: 'dialogueOption', id: OPTION, label: 'Hola' },
      ],
    }, 6));
    expect(session.story?.items).toHaveLength(5);
    expect(audio.play).toHaveBeenCalledWith('music', url, true);

    session.storyMenu(VERB);
    expect(connection.storyMenu).not.toHaveBeenCalled();
    session.storyMenu(VERB, TARGET);
    expect(connection.storyMenu).toHaveBeenCalledWith(VERB, TARGET, 1);
    expect(session.storyPending).toBe(true);
    session.chooseStory(OPTION);
    expect(connection.storyDialogue).not.toHaveBeenCalled();
    callbacks().onMessage(message('story.error', { code: 'stale_view', message: 'Vista anterior.' }, 7));
    expect(session.storyPending).toBe(false);
    expect(session.storyError?.code).toBe('stale_view');
    session.chooseStory(OPTION);
    expect(connection.storyDialogue).toHaveBeenCalledWith(OPTION, 1);
    session.chooseStory('c9999999999999999');
    expect(connection.storyChoose).not.toHaveBeenCalled();

    callbacks().onMessage(message('story.left', { reason: 'left' }, 8));
    expect(session.story).toBeUndefined();
    expect(session.history.at(-1)?.response).toContain('return to the world');

    // A new connection starts outside the story until the server re-sends it.
    callbacks().onMessage(message('story.view', {
      viewRev: 2, part: 0, parts: 1, turn: 1, canUndo: true, items: [scene],
    }, 9));
    expect(session.story).toBeDefined();
    callbacks().onMessage(message('server.welcome', { serverName: 'Tiamud', motd: 'Hi' }, 1));
    expect(session.story).toBeUndefined();
  });

  it('opens the story picker from the catalog and enters in the chosen language', () => {
    const { session, connection, callbacks } = harness();
    session.openStoryCatalog();
    expect(connection.storyCatalog).toHaveBeenCalledOnce();
    expect(session.storyCatalogPending).toBe(true);
    // A second request waits for the first answer.
    session.openStoryCatalog();
    expect(connection.storyCatalog).toHaveBeenCalledOnce();
    const lluvia = { storyId: 'lluvia-de-neon', title: 'Lluvia de neón', locales: ['es', 'en'] };
    callbacks().onMessage(message('story.catalog', {
      part: 0, parts: 2, stories: [{ storyId: 'cripta', title: 'Cripta', locales: ['es'] }],
    }, 2));
    expect(session.storyCatalog).toBeUndefined();
    callbacks().onMessage(message('story.catalog', { part: 1, parts: 2, stories: [lluvia] }, 3));
    expect(session.storyCatalogPending).toBe(false);
    expect(session.storyCatalog?.map((story) => story.storyId)).toEqual(['cripta', 'lluvia-de-neon']);

    // Only an offered language is sent, through the shared text command.
    session.enterStory('lluvia-de-neon', 'fr');
    session.enterStory('desconocida', 'es');
    expect(connection.submit).not.toHaveBeenCalled();
    session.enterStory('lluvia-de-neon', 'en');
    expect(connection.submit).toHaveBeenCalledWith('historia lluvia-de-neon en', expect.any(String));
    callbacks().onMessage(message('story.entered', {
      storyId: 'lluvia-de-neon', title: 'Neon Rain', locale: 'en', locales: ['es', 'en'],
    }, 4));
    expect(session.storyCatalog).toBeUndefined();
    expect(session.storyLocale).toBe('en');

    // Outside a story a story error becomes a notice (nothing else shows it).
    const other = harness();
    other.session.openStoryCatalog();
    other.callbacks().onMessage(message('story.error', {
      code: 'disabled', message: 'Las historias no están disponibles en este servidor.',
    }, 2));
    expect(other.session.storyCatalogPending).toBe(false);
    expect(other.session.notice).toEqual({
      level: 'error', message: 'Las historias no están disponibles en este servidor.',
    });
    other.session.closeStoryCatalog();
    expect(other.session.storyCatalog).toBeUndefined();
  });

  it('sends parser commands only while the view shows a prompt', () => {
    const { session, connection, callbacks } = harness();
    const scene = { type: 'scene', title: 'Faro', text: 'Playa', score: 0, ending: null };
    callbacks().onMessage(message('story.entered', { storyId: 'faro-de-punta-gris', title: 'Faro' }, 2));
    callbacks().onMessage(message('story.view', {
      viewRev: 1, part: 0, parts: 1, turn: 0, canUndo: false, items: [scene],
    }, 3));
    expect(session.commandStory('mira')).toBe(false);
    callbacks().onMessage(message('story.view', {
      viewRev: 2, part: 0, parts: 1, turn: 0, canUndo: false,
      items: [{ ...scene, prompt: { text: '>', question: false } }],
    }, 4));
    expect(session.commandStory('   ')).toBe(false);
    expect(session.commandStory('a'.repeat(257))).toBe(false);
    expect(connection.storyCommand).not.toHaveBeenCalled();
    expect(session.commandStory('  coge la medalla ')).toBe(true);
    expect(connection.storyCommand).toHaveBeenCalledWith('coge la medalla', 2);
    expect(session.storyPending).toBe(true);
    // One line at a time: nothing else is sent until the answer arrives.
    expect(session.commandStory('n')).toBe(false);
    callbacks().onMessage(message('story.error', { code: 'rate_limited', message: 'Espera.' }, 5));
    expect(session.storyPending).toBe(false);
    expect(session.commandStory('n')).toBe(true);
  });

  it('offers to continue, saves, lists and restores story slots', () => {
    const { session, connection, callbacks } = harness();
    const scene = { type: 'scene', title: 'Cripta', text: 'Oscuro.', score: 0, ending: null };
    const view = (viewRev: number, turn: number) => message('story.view', {
      viewRev, part: 0, parts: 1, turn, canUndo: false, items: [scene],
    }, 10 + viewRev);
    callbacks().onMessage(message('story.entered', { storyId: 'cripta', title: 'Cripta' }, 4));
    callbacks().onMessage(view(1, 0));
    // Nothing is sent before the story exists or with invalid slots.
    session.saveStory(0);
    session.saveStory(10);
    expect(connection.storySave).not.toHaveBeenCalled();
    session.continueStory();
    expect(connection.storyContinue).not.toHaveBeenCalled();

    callbacks().onMessage(message('story.slots', {
      slots: [{ slot: 0, updatedAt: 100, compatible: true }], offerContinue: true,
    }, 5));
    expect(session.storyOfferContinue).toBe(true);
    expect(session.storySlots).toHaveLength(1);

    session.saveStory(2);
    expect(connection.storySave).toHaveBeenCalledWith(2);
    expect(session.storyPending).toBe(true);
    session.saveStory(3);
    expect(connection.storySave).toHaveBeenCalledTimes(1);
    callbacks().onMessage(message('story.saved', { slot: 2, updatedAt: 200 }, 6));
    expect(session.storyPending).toBe(false);
    expect(session.storyNotice).toBe('Saved to slot 2.');
    expect(session.storySlots?.map((slot) => slot.slot)).toEqual([0, 2]);

    session.continueStory();
    expect(connection.storyContinue).toHaveBeenCalledTimes(1);
    callbacks().onMessage(message('story.restored', { slot: 0 }, 7));
    expect(session.storyOfferContinue).toBe(false);
    expect(session.storyPending).toBe(true);
    callbacks().onMessage(view(2, 3));
    expect(session.storyPending).toBe(false);
    expect(session.story?.turn).toBe(3);
    expect(session.storyNotice).toBe('Continued from the autosave.');

    session.refreshStorySlots();
    expect(connection.storySlots).toHaveBeenCalledTimes(1);
    callbacks().onMessage(message('story.error', { code: 'rate_limited', message: 'Espera.' }, 8));
    expect(session.storyPending).toBe(false);
    session.deleteStorySlot(2);
    expect(connection.storyDeleteSlot).toHaveBeenCalledWith(2);
    callbacks().onMessage(message('story.slots', { slots: [], offerContinue: false }, 9));
    expect(session.storySlots).toEqual([]);
    session.restoreStory(4);
    expect(connection.storyRestore).toHaveBeenCalledWith(4);

    callbacks().onMessage(message('story.left', { reason: 'evicted' }, 20));
    expect(session.storySlots).toBeUndefined();
    expect(session.history.at(-1)?.response).toContain('autosave');
  });

  it('routes shared-story choices by decision mode and role', () => {
    const { session, connection, callbacks } = harness();
    const CHOICE = 'c1111111111111111';
    const VERB = 'c2222222222222222';
    const TARGET = 'c3333333333333333';
    const lobby = (you: string, mode: string) => ({
      mode, voteSeconds: 60, you,
      seats: [
        { name: 'Aria', role: 'host', connected: true, turn: false },
        { name: 'Bruno', role: 'player', connected: true, turn: false },
      ],
      spectators: 0, maxSeats: 8, maxSpectators: 32,
    });
    callbacks().onMessage(message('story.entered', { storyId: 'cripta', title: 'Cripta' }, 1));
    callbacks().onMessage(message('story.view', {
      viewRev: 2, part: 0, parts: 1, turn: 0, canUndo: false,
      items: [
        { type: 'scene', title: 'Cripta', text: 'Oscuro.', score: 0, ending: null },
        { type: 'choice', id: CHOICE, label: 'Avanzar' },
        { type: 'verb', id: VERB, label: 'MIRAR', needsTarget: true },
        { type: 'target', verb: VERB, id: TARGET, label: 'Puerta' },
      ],
    }, 2));
    // Solo: an ordinary choice; sharing is offered.
    expect(session.storyVotes).toBe(false);
    session.shareStory();
    expect(connection.storyShare).toHaveBeenCalledTimes(1);
    callbacks().onMessage(message('story.lobby', lobby('player', 'vote'), 3));
    expect(session.storyPending).toBe(false);
    expect(session.storyVotes).toBe(true);
    session.chooseStory(CHOICE);
    expect(connection.storyVote).toHaveBeenLastCalledWith(CHOICE, 2);
    expect(connection.storyChoose).not.toHaveBeenCalled();
    expect(session.storyPending).toBe(true);
    // The tally acknowledges the vote and starts the countdown.
    callbacks().onMessage(message('story.tally', {
      viewRev: 2, mode: 'vote', choices: [{ choiceId: CHOICE, votes: 1 }], voted: 1, eligible: 2,
      remainingSeconds: 30, yourVote: CHOICE,
    }, 4));
    expect(session.storyPending).toBe(false);
    expect(session.storyTally?.yourVote).toBe(CHOICE);
    expect(session.storyDeadline).toBeGreaterThan(Date.now() + 29_000);
    session.storyMenu(VERB, TARGET);
    expect(connection.storyVote).toHaveBeenLastCalledWith(TARGET, 2);
    callbacks().onMessage(message('story.error', { code: 'stale_view', message: 'x' }, 5));
    session.withdrawStoryVote();
    expect(connection.storyVote).toHaveBeenLastCalledWith(undefined, 2);
    callbacks().onMessage(message('story.error', { code: 'stale_view', message: 'x' }, 6));
    // Host mode: the host acts, guests vote advisorily; spectators do nothing.
    callbacks().onMessage(message('story.lobby', lobby('host', 'host'), 7));
    session.chooseStory(CHOICE);
    expect(connection.storyChoose).toHaveBeenCalledWith(CHOICE, 2);
    callbacks().onMessage(message('story.error', { code: 'stale_view', message: 'x' }, 8));
    expect(session.inviteToStory('  Bruno ')).toBe(true);
    expect(connection.storyInvite).toHaveBeenCalledWith('  Bruno ');
    expect(session.storyNotice).toContain('Bruno');
    session.kickFromStory('Bruno');
    expect(connection.storyKick).toHaveBeenCalledWith('Bruno');
    session.setStoryMode('turns', 45);
    expect(connection.storyMode).toHaveBeenCalledWith('turns', 45);
    callbacks().onMessage(message('story.lobby', lobby('spectator', 'vote'), 9));
    const votes = connection.storyVote.mock.calls.length;
    session.chooseStory(CHOICE);
    session.storyMenu(VERB, TARGET);
    expect(connection.storyVote.mock.calls.length).toBe(votes);
    expect(session.inviteToStory('Bruno')).toBe(false);
    // Leaving reasons of shared stories.
    callbacks().onMessage(message('story.left', { reason: 'kicked' }, 10));
    expect(session.story).toBeUndefined();
    expect(session.storyLobby).toBeUndefined();
    expect(session.history.at(-1)?.response).toContain('host removed you');
  });

  it('records invitations and joins by code', () => {
    const { session, connection, callbacks } = harness();
    callbacks().onMessage(message('story.invitation', {
      host: 'Aria', storyId: 'cripta', title: 'Cripta', code: 'ABCD2345',
    }, 1));
    expect(session.storyInvitation?.code).toBe('ABCD2345');
    expect(session.history.at(-1)?.response).toContain('historia unirse ABCD2345');
    session.joinStory('ABCD2345', true);
    expect(connection.storyJoin).toHaveBeenCalledWith('ABCD2345', true);
    session.dismissStoryInvitation();
    expect(session.storyInvitation).toBeUndefined();
    callbacks().onMessage(message('story.entered', { storyId: 'cripta', title: 'Cripta' }, 2));
    callbacks().onMessage(message('story.left', { reason: 'ended' }, 3));
    expect(session.history.at(-1)?.response).toContain('no seated player remained');
  });
});
