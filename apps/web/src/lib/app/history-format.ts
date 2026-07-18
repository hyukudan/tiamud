import type { ConnectionState, SocialMessageData } from '../protocol';

export function connectionLabelFor(state: ConnectionState): string {
  return state === 'connected'
    ? 'Connected'
    : state === 'connecting'
      ? 'Connecting…'
      : 'Disconnected — reconnecting…';
}

export function socialHistoryLabel(message: SocialMessageData): string {
  switch (message.channel) {
    case 'announcement':
      return `${message.sender} announces`;
    case 'say':
      return `${message.sender} says`;
    case 'yell':
      return `${message.sender} yells`;
    case 'shout':
      return `${message.sender} shouts`;
    case 'tell':
      return `${message.sender} tells you`;
    case 'telepathy':
      return `${message.sender} reaches your mind`;
    case 'emote':
      return 'Emote';
    case 'party':
      return `${message.sender} to the party`;
    default:
      return `${message.sender} in #${message.channel}`;
  }
}

export function itemActionName(commandValue: string): string | undefined {
  const normalized = commandValue.trim().toLowerCase();
  if (normalized.startsWith('reward odds ')) return 'reward.odds';
  const verb = normalized.split(/\s+/, 1)[0];
  if (verb === 'open') return 'reward.open';
  return verb && ['take', 'drop', 'examine', 'use', 'equip', 'unequip', 'contents'].includes(verb)
    ? `item.${verb}`
    : undefined;
}
