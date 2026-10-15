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
    callbacks().onMessage(message('story.entered', { storyId: 'lluvia-de-neon', title: 'Lluvia' }, 4));
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
});
