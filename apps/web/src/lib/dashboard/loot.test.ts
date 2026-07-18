import { describe, expect, it } from 'vitest';
import {
  greedCommand,
  isLootRollProjection,
  LOOT_MODE_LABELS,
  lootMasterCommand,
  lootMasterMemberValid,
  lootModeCommand,
  lootRollCommand,
  needCommand,
  parseLootRoll,
  passCommand,
} from './loot';

describe('isLootRollProjection', () => {
  it('recognizes headers for every distribution mode', () => {
    expect(isLootRollProjection('Group loot for a rusty goblin (mode: round-robin):')).toBe(true);
    expect(isLootRollProjection('Group loot for a rusty goblin (mode: need/greed):')).toBe(true);
    expect(isLootRollProjection('Group loot for a rusty goblin (mode: master-loot):')).toBe(true);
  });

  it('rejects the mode-change acknowledgement', () => {
    expect(isLootRollProjection('Group loot mode set to need/greed.')).toBe(false);
  });

  it('rejects declare and assign acknowledgements', () => {
    expect(isLootRollProjection('You declare need on a rusty dagger.')).toBe(false);
    expect(isLootRollProjection('You assign a rusty dagger to Alice.')).toBe(false);
  });

  it('rejects empty strings and arbitrary text', () => {
    expect(isLootRollProjection('')).toBe(false);
    expect(isLootRollProjection('   ')).toBe(false);
    expect(isLootRollProjection('You see nothing special.')).toBe(false);
  });
});

describe('parseLootRoll', () => {
  const needGreedMessage = [
    'Group loot for a rusty goblin (mode: need/greed):',
    '- a rusty dagger (ref o0123456789abcdef): 1 need, 2 greed, 0 pass, 3 pending',
    '- a small shield (ref ofedcba9876543210): 0 need, 0 greed, 4 pass, 0 pending',
  ].join('\r\n');

  it('parses need/greed roots with counts, using CRLF line endings', () => {
    const view = parseLootRoll(needGreedMessage);
    expect(view).toBeDefined();
    expect(view?.npcName).toBe('a rusty goblin');
    expect(view?.mode).toBe('need_greed');
    expect(view?.empty).toBe(false);
    expect(view?.roots).toHaveLength(2);
    expect(view?.roots[0]).toEqual({
      ref: 'o0123456789abcdef',
      name: 'a rusty dagger',
      need: 1,
      greed: 2,
      pass: 0,
      pending: 3,
    });
    expect(view?.roots[1]).toEqual({
      ref: 'ofedcba9876543210',
      name: 'a small shield',
      need: 0,
      greed: 0,
      pass: 4,
      pending: 0,
    });
  });

  it('parses the same message using plain LF line endings', () => {
    const lfMessage = needGreedMessage.replace(/\r\n/g, '\n');
    const view = parseLootRoll(lfMessage);
    expect(view?.roots).toHaveLength(2);
    expect(view?.roots[0]).toMatchObject({ ref: 'o0123456789abcdef', need: 1, greed: 2 });
  });

  it('parses master-loot roots, both assigned and unassigned', () => {
    const message = [
      'Group loot for a rusty goblin (mode: master-loot):',
      '- a rusty dagger (ref o0123456789abcdef): assigned to Alice Longname',
      '- a small shield (ref ofedcba9876543210): unassigned',
    ].join('\n');
    const view = parseLootRoll(message);
    expect(view?.mode).toBe('master_loot');
    expect(view?.roots[0]).toEqual({
      ref: 'o0123456789abcdef',
      name: 'a rusty dagger',
      assignedTo: 'Alice Longname',
    });
    expect(view?.roots[1]).toEqual({
      ref: 'ofedcba9876543210',
      name: 'a small shield',
      assignedTo: undefined,
    });
  });

  it('parses round-robin roots as plain name/ref pairs', () => {
    const message = [
      'Group loot for a rusty goblin (mode: round-robin):',
      '- a rusty dagger (ref o0123456789abcdef)',
      '- a small shield (ref ofedcba9876543210)',
    ].join('\n');
    const view = parseLootRoll(message);
    expect(view?.mode).toBe('round_robin');
    expect(view?.roots).toEqual([
      { ref: 'o0123456789abcdef', name: 'a rusty dagger' },
      { ref: 'ofedcba9876543210', name: 'a small shield' },
    ]);
  });

  it('marks the view empty when there is no carried loot', () => {
    const message = 'Group loot for a rusty goblin (mode: need/greed):\n- (no carried loot to divide)';
    const view = parseLootRoll(message);
    expect(view?.empty).toBe(true);
    expect(view?.roots).toEqual([]);
    expect(view?.npcName).toBe('a rusty goblin');
  });

  it('returns undefined for non-projection text', () => {
    expect(parseLootRoll('Group loot mode set to need/greed.')).toBeUndefined();
    expect(parseLootRoll('You declare need on a rusty dagger.')).toBeUndefined();
    expect(parseLootRoll('')).toBeUndefined();
    expect(parseLootRoll('You see nothing special.')).toBeUndefined();
  });

  it('preserves names with spaces and extracts refs without leaking other ids', () => {
    const message = [
      'Group loot for the ancient stone golem (mode: need/greed):',
      '- a heavy iron warhammer (ref o00000000000000ab): 2 need, 1 greed, 0 pass, 1 pending',
    ].join('\n');
    const view = parseLootRoll(message);
    expect(view?.npcName).toBe('the ancient stone golem');
    expect(view?.roots[0].name).toBe('a heavy iron warhammer');
    expect(view?.roots[0].ref).toBe('o00000000000000ab');
    expect(JSON.stringify(view)).not.toMatch(/"id"|instance-|prototype-/);
  });
});

describe('LOOT_MODE_LABELS', () => {
  it('provides a human label for every mode', () => {
    expect(LOOT_MODE_LABELS.round_robin).toBe('Round-robin');
    expect(LOOT_MODE_LABELS.need_greed).toBe('Need / Greed');
    expect(LOOT_MODE_LABELS.master_loot).toBe('Master-loot');
  });
});

describe('command builders', () => {
  it('produce exact command text', () => {
    expect(lootRollCommand()).toBe('loot roll');
    expect(lootModeCommand('round_robin')).toBe('loot mode round_robin');
    expect(lootModeCommand('need_greed')).toBe('loot mode need_greed');
    expect(lootModeCommand('master_loot')).toBe('loot mode master_loot');
    expect(needCommand('o0123456789abcdef')).toBe('need o0123456789abcdef');
    expect(greedCommand('o0123456789abcdef')).toBe('greed o0123456789abcdef');
    expect(passCommand('o0123456789abcdef')).toBe('pass o0123456789abcdef');
  });

  it('lootMasterCommand trims the member name', () => {
    expect(lootMasterCommand('o0123456789abcdef', '  Alice  ')).toBe('loot master o0123456789abcdef to Alice');
    expect(lootMasterCommand('o0123456789abcdef', 'Alice')).toBe('loot master o0123456789abcdef to Alice');
  });
});

describe('lootMasterMemberValid', () => {
  it('rejects empty or whitespace-only member names', () => {
    expect(lootMasterMemberValid('')).toBe(false);
    expect(lootMasterMemberValid('   ')).toBe(false);
  });

  it('accepts names up to 64 characters and rejects longer ones', () => {
    expect(lootMasterMemberValid('a'.repeat(64))).toBe(true);
    expect(lootMasterMemberValid('a'.repeat(65))).toBe(false);
  });

  it('rejects names containing newlines or control characters', () => {
    expect(lootMasterMemberValid('Alice\n')).toBe(false);
    expect(lootMasterMemberValid('Al\tice')).toBe(false);
  });
});
