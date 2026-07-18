import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { BoardCatalogData, BoardEntryData, BoardListData } from '../lib/protocol';
import BoardsPanel from './BoardsPanel.svelte';

const boardCatalog: BoardCatalogData = {
  boards: [
    { key: 'general', label: 'General', description: 'Conversación comunitaria deliberada.', canPost: true },
    { key: 'newbie', label: 'Personas nuevas', description: 'Preguntas y orientación para empezar.', canPost: false },
  ],
  retentionDays: 90,
};

const boardList: BoardListData = {
  board: 'general',
  page: 1,
  totalPages: 1,
  totalPosts: 1,
  capacity: 1000,
  posts: [{
    postId: 'b0123456789abcdef0123456789abcdef',
    board: 'general',
    author: 'Nara <&>',
    subject: 'Festival de faroles',
    createdAt: 1_700_000_000,
    expiresAt: 1_707_776_000,
    canDelete: false,
    canReport: true,
  }],
};

const boardEntry: BoardEntryData = {
  postId: 'b0123456789abcdef0123456789abcdef',
  board: 'general',
  author: 'Nara <&>',
  subject: 'Festival de faroles',
  body: 'Nos vemos | en la plaza. <&>',
  createdAt: 1_700_000_000,
  expiresAt: 1_707_776_000,
  canDelete: false,
  canReport: true,
};

describe('BoardsPanel', () => {
  it('renders the catalog, listing, open entry and publish form', () => {
    const { body } = render(BoardsPanel, {
      props: {
        boardCatalog,
        boardList,
        boardEntry,
        connectionState: 'connected',
        onaction: () => undefined,
      },
    });

    for (const label of [
      'Tablones', 'Community boards', 'Festival de faroles', 'Nos vemos | en la plaza.',
      'Report this post', 'Publish to General',
    ]) expect(body).toContain(label);
  });

  it('prompts to load the catalog when none is present', () => {
    const { body } = render(BoardsPanel, {
      props: { onaction: () => undefined },
    });

    expect(body).toContain('Load the bounded public board catalog when you need it.');
    expect(body).toContain('Load boards');
  });
});
