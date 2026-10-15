import { describe, expect, it } from 'vitest';
import type {
  LiveServerMessage,
  MailListData,
  PlayerStateSnapshot,
  SceneSnapshot,
} from '../protocol';
import {
  clearedPanels,
  confirmedActionPatch,
  reduceWorldMessage,
  submittedActionPatch,
} from './world-views';

const scene = {
  viewId: 'view-1', revision: 3, title: 'Square', items: [{ id: 'i1' }], npcs: [{ id: 'n1' }],
  exits: [], prose: ['Old.'], image: { alt: 'x' },
} as unknown as SceneSnapshot;
const player = { revision: 4, inventory: [{ id: 'chest' }], equipment: [] } as unknown as PlayerStateSnapshot;
const message = (type: string, data: unknown) =>
  ({ v: 1, type, seq: 1, data }) as unknown as LiveServerMessage;

describe('world view reducer', () => {
  it('applies scene patches only to the same view at a newer or equal revision', () => {
    expect(reduceWorldMessage({ scene }, message('scene.patch', {
      viewId: 'view-2', revision: 9, prose: ['New.'],
    }))).toEqual({});
    expect(reduceWorldMessage({ scene }, message('scene.patch', {
      viewId: 'view-1', revision: 2, prose: ['New.'],
    }))).toEqual({});
    const patched = reduceWorldMessage({ scene }, message('scene.patch', {
      viewId: 'view-1', revision: 3, prose: ['New.'], image: null,
    }));
    expect(patched.scene?.prose).toEqual(['New.']);
    expect(patched.scene?.image).toBeUndefined();
  });

  it('upserts and removes entities by kind', () => {
    const upsert = reduceWorldMessage({ scene }, message('entity.upsert', {
      viewId: 'view-1', revision: 4, kind: 'npc', id: 'n2', entity: { id: 'n2' },
    }));
    expect(upsert.scene?.npcs.map((npc) => npc.id)).toEqual(['n1', 'n2']);
    const removed = reduceWorldMessage({ scene }, message('entity.remove', {
      viewId: 'view-1', revision: 4, kind: 'item', id: 'i1',
    }));
    expect(removed.scene?.items).toEqual([]);
    expect(reduceWorldMessage({ scene }, message('entity.remove', {
      viewId: 'view-1', revision: 1, kind: 'item', id: 'i1',
    }))).toEqual({});
  });

  it('drops stale reward odds and container views when inventory changes', () => {
    const odds = { selector: 'gone' } as never;
    const patch = reduceWorldMessage({ player, rewardOdds: odds, containerContents: {} as never },
      message('inventory.patch', {
        revision: 5, inventory: [], equipment: [], inventoryCapacity: {},
      }));
    expect(patch.player?.revision).toBe(5);
    expect('containerContents' in patch && patch.containerContents === undefined).toBe(true);
    expect('rewardOdds' in patch).toBe(true);
    expect(reduceWorldMessage({ player }, message('inventory.patch', { revision: 3 }))).toEqual({});
  });

  it('marks an opened mail as read and decrements unread once', () => {
    const mailList = {
      unreadCount: 2,
      messages: [{ id: 'm1', read: false }, { id: 'm2', read: false }],
    } as unknown as MailListData;
    const patch = reduceWorldMessage({ mailList }, message('mail.message', { id: 'm1' }));
    expect(patch.mailList?.unreadCount).toBe(1);
    expect(patch.mailList?.messages[0].read).toBe(true);
  });

  it('ignores protocol-only and presentation-only messages', () => {
    for (const type of ['server.welcome', 'server.notice', 'media.play', 'chat.message', 'action.result']) {
      expect(reduceWorldMessage({ scene, player }, message(type, {}))).toEqual({});
    }
  });

  it('describes submit-time invalidation and the panel reset set', () => {
    expect(submittedActionPatch('item.drop')).toEqual({ containerContents: undefined });
    expect(submittedActionPatch('trade.offer')).toEqual({ tradeContents: undefined });
    expect(submittedActionPatch('trade.contents')).toEqual({});
    expect(submittedActionPatch('reward.open')).toEqual({ rewardOdds: undefined });
    expect(Object.keys(clearedPanels())).toHaveLength(8);
    expect('scene' in clearedPanels()).toBe(false);
  });

  it('confirms a board post by reloading the list', () => {
    expect(confirmedActionPatch({}, { action: 'board.post' })).toEqual({ boardList: undefined });
    expect(confirmedActionPatch({}, { action: 'unknown' })).toEqual({});
  });
});
