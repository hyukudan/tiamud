import { describe, expect, it } from 'vitest';
import {
  exactProbability,
  humanizeId,
  resourceCostLabel,
  signedDelta,
  statusLabel,
} from './format';

describe('statusLabel', () => {
  it('maps every quest status to its label', () => {
    expect(statusLabel('available')).toBe('Available');
    expect(statusLabel('active')).toBe('Active');
    expect(statusLabel('readyToClaim')).toBe('Reward ready');
    expect(statusLabel('completed')).toBe('Completed');
  });
});

describe('humanizeId', () => {
  it('returns an empty string for empty or separator-only input', () => {
    expect(humanizeId('')).toBe('');
    expect(humanizeId('___')).toBe('');
    expect(humanizeId(':.-')).toBe('');
  });

  it('splits on separators and capitalizes the first letter of each part', () => {
    expect(humanizeId('foo_bar')).toBe('Foo Bar');
    expect(humanizeId('lantern:square')).toBe('Lantern Square');
    expect(humanizeId('a.b-c_d')).toBe('A B C D');
  });

  it('only capitalizes the first character, leaving camelCase tokens intact', () => {
    expect(humanizeId('camelCase')).toBe('CamelCase');
    expect(humanizeId('already Spaced')).toBe('Already Spaced');
  });

  it('collapses consecutive separators without emitting empty parts', () => {
    expect(humanizeId('foo__bar')).toBe('Foo Bar');
    expect(humanizeId('_leading')).toBe('Leading');
  });
});

describe('signedDelta', () => {
  it('prefixes non-negative values with a plus sign', () => {
    expect(signedDelta(5)).toBe('+5');
    expect(signedDelta(0)).toBe('+0');
  });

  it('leaves negative values with their own minus sign', () => {
    expect(signedDelta(-3)).toBe('-3');
  });
});

describe('exactProbability', () => {
  it('renders the exact fraction and a two-decimal percentage', () => {
    expect(exactProbability(1, 4)).toBe('1/4 (25.00%)');
    expect(exactProbability(1, 3)).toBe('1/3 (33.33%)');
    expect(exactProbability(0, 5)).toBe('0/5 (0.00%)');
    expect(exactProbability(5, 5)).toBe('5/5 (100.00%)');
  });
});

describe('resourceCostLabel', () => {
  it('joins the amount and resource label', () => {
    expect(resourceCostLabel({ resourceId: 'mana', label: 'Mana', amount: 2 })).toBe('2 Mana');
    expect(resourceCostLabel({ resourceId: 'focus', label: 'Focus', amount: 0 })).toBe('0 Focus');
  });
});
