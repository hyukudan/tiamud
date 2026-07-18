import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import GroupLootPanel from './GroupLootPanel.svelte';

describe('GroupLootPanel', () => {
  it('renders the distribution-mode controls when no roll is loaded', () => {
    const { body } = render(GroupLootPanel, {
      props: { oncommand: () => undefined },
    });

    expect(body).toContain('Group loot');
    expect(body).toContain('Round-robin');
    expect(body).toContain('Need / Greed');
    expect(body).toContain('Master-loot');
    expect(body).toContain('Changing the distribution mode requires the party leader.');
    expect(body).toContain('not loaded');
  });

  it('projects a parsed need/greed roll', () => {
    const lootRoll = [
      'Group loot for a rain wolf (mode: need/greed):',
      '- charged blue lantern (ref o1111111111111111): 1 need, 0 greed, 0 pass, 0 pending',
    ].join('\n');
    const { body } = render(GroupLootPanel, {
      props: { lootRoll, oncommand: () => undefined },
    });

    expect(body).toContain('charged blue lantern');
    expect(body).toContain('1 need');
    expect(body).toContain('Need / Greed');
  });
});
