import { describe, expect, it } from 'vitest';
import sharedFixture from '../../e2e/fixtures/story-shared.json';
import {
  createStoryInvite,
  createStoryJoin,
  createStoryKick,
  createStoryMode,
  createStoryShare,
  createStoryVote,
  parseServerMessage,
} from './protocol';
import {
  parseStoryInvitation,
  parseStoryLeft,
  parseStoryLobby,
  parseStoryTally,
} from './story-protocol';

const CHOICE = 'c1111111111111111';
const OTHER = 'c2222222222222222';

const seat = (name: string, role = 'player', extra: Record<string, unknown> = {}) => ({
  name, role, connected: true, turn: false, ...extra,
});

const lobby = (overrides: Record<string, unknown> = {}) => ({
  mode: 'vote',
  voteSeconds: 60,
  you: 'host',
  code: 'ABCD2345',
  seats: [seat('Aria', 'host'), seat('Bruno')],
  spectators: 1,
  maxSeats: 8,
  maxSpectators: 32,
  ...overrides,
});

const tally = (overrides: Record<string, unknown> = {}) => ({
  viewRev: 3,
  mode: 'vote',
  choices: [{ choiceId: CHOICE, votes: 2 }],
  voted: 2,
  eligible: 3,
  remainingSeconds: 41,
  yourVote: CHOICE,
  ...overrides,
});

describe('shared story protocol', () => {
  it('accepts every real server frame of the two-player fixture', () => {
    let seq = 0;
    let frames = 0;
    for (const step of sharedFixture.steps) {
      for (const frame of [...step.frames.A, ...step.frames.B]) {
        seq += 1;
        frames += 1;
        const raw = JSON.stringify({ v: 1, type: frame.type, seq, data: frame.data });
        expect(parseServerMessage(raw)?.type, frame.type).toBe(frame.type);
      }
    }
    expect(frames).toBeGreaterThan(40);
    expect(sharedFixture.code).toBe('ABCD2345');
  });

  it('validates lobbies strictly', () => {
    expect(parseStoryLobby(lobby())).toEqual(lobby());
    const { code: _code, ...guest } = lobby({ you: 'player' });
    expect(parseStoryLobby(guest)).toBeDefined();
    for (const data of [
      lobby({ you: 'player' }),
      lobby({ code: 'abcd2345' }),
      lobby({ code: 'ABCD234O' }),
      lobby({ mode: 'chaos' }),
      lobby({ voteSeconds: 29 }),
      lobby({ voteSeconds: 121 }),
      lobby({ seats: [seat('Bruno'), seat('Aria', 'host')] }),
      lobby({ seats: [seat('Aria', 'host'), seat('Bruno', 'host')] }),
      lobby({ seats: [seat('Aria', 'spectator')] }),
      lobby({ seats: [seat('Aria', 'host', { turn: true })] }),
      lobby({ mode: 'turns', seats: [seat('Aria', 'host', { turn: true }), seat('B', 'player', { turn: true })] }),
      lobby({ seats: Array.from({ length: 9 }, (_, index) => seat(`p${index}`, index === 0 ? 'host' : 'player')) }),
      lobby({ maxSeats: 1 }),
      lobby({ spectators: 33 }),
      lobby({ seats: [seat('x'.repeat(65), 'host')] }),
      lobby({ seats: [{ ...seat('Aria', 'host'), account: 1 }] }),
      { ...lobby(), extra: true },
    ]) expect(parseStoryLobby(data as Record<string, unknown>)).toBeUndefined();
    expect(parseStoryLobby(lobby({ mode: 'turns', seats: [seat('Aria', 'host', { turn: true })] })))
      .toBeDefined();
  });

  it('validates tallies without voter identities', () => {
    expect(parseStoryTally(tally())).toEqual(tally());
    expect(parseStoryTally(tally({ choices: [], voted: 0, yourVote: undefined, remainingSeconds: undefined })))
      .toEqual({ viewRev: 3, mode: 'vote', choices: [], voted: 0, eligible: 3 });
    for (const data of [
      tally({ voted: 3 }),
      tally({ yourVote: OTHER }),
      tally({ yourVote: 'c1' }),
      tally({ choices: [{ choiceId: CHOICE, votes: 1 }, { choiceId: CHOICE, votes: 1 }] }),
      tally({ choices: [{ choiceId: CHOICE, votes: 0 }], voted: 0, yourVote: undefined }),
      tally({ choices: [{ choiceId: CHOICE, votes: 2, voters: ['Aria'] }] }),
      tally({ remainingSeconds: 121 }),
      tally({ viewRev: 0 }),
      tally({ eligible: 9 }),
      { ...tally(), voters: ['Aria'] },
    ]) expect(parseStoryTally(data as Record<string, unknown>)).toBeUndefined();
  });

  it('validates invitations and the new leave reasons', () => {
    const invitation = { host: 'Aria', storyId: 'cripta', title: 'Cripta', code: 'ABCD2345' };
    expect(parseStoryInvitation(invitation)).toEqual(invitation);
    for (const data of [
      { ...invitation, code: 'ABC' },
      { ...invitation, storyId: 'Cripta!' },
      { ...invitation, host: '' },
      { ...invitation, extra: 1 },
    ]) expect(parseStoryInvitation(data)).toBeUndefined();
    expect(parseStoryLeft({ reason: 'kicked' })).toEqual({ reason: 'kicked' });
    expect(parseStoryLeft({ reason: 'ended' })).toEqual({ reason: 'ended' });
    expect(parseStoryLeft({ reason: 'banned' })).toBeUndefined();
  });

  it('builds client messages with exact shapes and bounds', () => {
    expect(createStoryVote(CHOICE, 2).data).toEqual({ choiceId: CHOICE, viewRev: 2 });
    expect(createStoryVote(undefined, 2).data).toEqual({ viewRev: 2 });
    expect(() => createStoryVote('c1', 2)).toThrow();
    expect(() => createStoryVote(CHOICE, 0)).toThrow();
    expect(createStoryShare()).toEqual({ v: 1, type: 'story.share', data: {} });
    expect(createStoryJoin(' abcd2345 ', true).data).toEqual({ code: 'ABCD2345', spectate: true });
    expect(() => createStoryJoin('ABCD-345', false)).toThrow();
    expect(createStoryInvite(' Bruno ').data).toEqual({ name: 'Bruno' });
    expect(createStoryKick('Bruno').type).toBe('story.kick');
    expect(() => createStoryInvite('   ')).toThrow();
    expect(() => createStoryInvite('a\u0007')).toThrow();
    expect(() => createStoryKick('ñ'.repeat(33))).toThrow();
    expect(createStoryMode('turns').data).toEqual({ mode: 'turns' });
    expect(createStoryMode('vote', 45).data).toEqual({ mode: 'vote', voteSeconds: 45 });
    expect(() => createStoryMode('vote', 10)).toThrow();
    expect(() => createStoryMode('chaos' as never)).toThrow();
  });
});
