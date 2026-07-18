import { describe, expect, it } from 'vitest';
import {
  connectionLabelFor,
  itemActionName,
  socialHistoryLabel,
} from './history-format';
import type { SocialMessageData } from '../protocol';

const message = (
  channel: SocialMessageData['channel'],
  sender = 'Ada',
): SocialMessageData => ({ channel, sender, text: 'hello' });

describe('connectionLabelFor', () => {
  it('labels each connection state', () => {
    expect(connectionLabelFor('connected')).toBe('Connected');
    expect(connectionLabelFor('connecting')).toBe('Connecting…');
    expect(connectionLabelFor('disconnected')).toBe('Disconnected — reconnecting…');
  });
});

describe('socialHistoryLabel', () => {
  it('renders sender-scoped labels for directed channels', () => {
    expect(socialHistoryLabel(message('announcement'))).toBe('Ada announces');
    expect(socialHistoryLabel(message('say'))).toBe('Ada says');
    expect(socialHistoryLabel(message('yell'))).toBe('Ada yells');
    expect(socialHistoryLabel(message('shout'))).toBe('Ada shouts');
    expect(socialHistoryLabel(message('tell'))).toBe('Ada tells you');
    expect(socialHistoryLabel(message('telepathy'))).toBe('Ada reaches your mind');
    expect(socialHistoryLabel(message('party'))).toBe('Ada to the party');
  });

  it('renders a fixed label for emotes without exposing the sender', () => {
    expect(socialHistoryLabel(message('emote'))).toBe('Emote');
  });

  it('falls back to a channel-tagged label for community channels', () => {
    expect(socialHistoryLabel(message('general'))).toBe('Ada in #general');
    expect(socialHistoryLabel(message('newbie'))).toBe('Ada in #newbie');
  });
});

describe('itemActionName', () => {
  it('maps the reward odds prefix before generic verb parsing', () => {
    expect(itemActionName('reward odds lantern')).toBe('reward.odds');
    expect(itemActionName('  Reward Odds Lantern ')).toBe('reward.odds');
  });

  it('maps the open verb to reward.open', () => {
    expect(itemActionName('open sealed chest')).toBe('reward.open');
    expect(itemActionName('OPEN')).toBe('reward.open');
  });

  it('maps recognized item verbs to item.<verb>', () => {
    expect(itemActionName('take lantern')).toBe('item.take');
    expect(itemActionName('drop lantern')).toBe('item.drop');
    expect(itemActionName('examine lantern')).toBe('item.examine');
    expect(itemActionName('use lantern')).toBe('item.use');
    expect(itemActionName('equip sword')).toBe('item.equip');
    expect(itemActionName('unequip sword')).toBe('item.unequip');
    expect(itemActionName('contents pack')).toBe('item.contents');
  });

  it('returns undefined for unrecognized or empty commands', () => {
    expect(itemActionName('north')).toBeUndefined();
    expect(itemActionName('')).toBeUndefined();
    expect(itemActionName('   ')).toBeUndefined();
    expect(itemActionName('reward odds')).toBeUndefined();
  });
});
