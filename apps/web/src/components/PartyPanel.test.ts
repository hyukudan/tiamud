import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { PlayerStateSnapshot } from '../lib/protocol';
import PartyPanel from './PartyPanel.svelte';

const party: NonNullable<PlayerStateSnapshot['party']> = {
  leader: true,
  members: [{ id: 'o4444444444444444', name: 'Aria', leader: true, online: true }],
  actions: [{ label: 'Leave party', command: 'party leave' }],
};

describe('PartyPanel', () => {
  it('renders leader management controls for a led party', () => {
    const { body } = render(PartyPanel, {
      props: { party, partyInvites: [], onaction: () => undefined, onperform: () => undefined },
    });

    for (const label of [
      'Party', 'Aria', 'Leave party', 'Invite or manage an adventurer', 'Invite',
      'Revoke invitation', 'Remove from party', 'Transfer leadership', 'Disband party',
      'Message to party',
    ]) expect(body).toContain(label);
  });

  it('offers to create a party while travelling alone', () => {
    const { body } = render(PartyPanel, {
      props: { party: undefined, partyInvites: [], onaction: () => undefined, onperform: () => undefined },
    });

    expect(body).toContain('You are travelling alone.');
    expect(body).toContain('Create party');
  });

  it('lists pending invitations', () => {
    const { body } = render(PartyPanel, {
      props: {
        party: undefined,
        partyInvites: [{
          leaderName: 'Nara',
          expiresInTicks: 20,
          actions: [{ label: 'Accept', command: 'party accept Nara' }],
        }],
        onaction: () => undefined,
        onperform: () => undefined,
      },
    });

    expect(body).toContain('Invitations');
    expect(body).toContain('Nara');
    expect(body).toContain('Accept');
  });
});
