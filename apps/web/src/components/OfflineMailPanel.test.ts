import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import type { MailListData, MailMessageData } from '../lib/protocol';
import OfflineMailPanel from './OfflineMailPanel.svelte';

const mailList: MailListData = {
  page: 1,
  totalPages: 1,
  totalMessages: 1,
  unreadCount: 1,
  capacity: 64,
  messages: [{
    id: 'm0123456789abcdef0123456789abcdef',
    sender: 'Bram <&>',
    subject: 'The northern gate',
    createdAt: 1_700_000_000,
    expiresAt: 1_702_592_000,
    read: false,
  }],
};

const mailMessage: MailMessageData = {
  id: 'm0123456789abcdef0123456789abcdef',
  sender: 'Bram <&>',
  subject: 'The northern gate',
  body: 'Meet me by the lighthouse.',
  createdAt: 1_700_000_000,
  expiresAt: 1_702_592_000,
  read: true,
};

describe('OfflineMailPanel', () => {
  it('renders the inbox, open message and compose form', () => {
    const { body } = render(OfflineMailPanel, {
      props: { mailList, mailMessage, onaction: () => undefined },
    });

    for (const label of [
      'Offline mail', 'Inbox', 'The northern gate', 'Meet me by the lighthouse.',
      'Send offline mail', 'Report this message', 'Exact character name',
    ]) expect(body).toContain(label);
  });

  it('prompts to load the inbox when no data is present', () => {
    const { body } = render(OfflineMailPanel, {
      props: { onaction: () => undefined },
    });

    expect(body).toContain('Load your bounded, 30-day inbox when you need it.');
    expect(body).toContain('Send offline mail');
  });
});
