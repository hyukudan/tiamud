<script lang="ts">
  import type { ActionPerformFields, ActionResultData, ConnectionState, MailListData, MailMessageData } from '../lib/protocol';
  import { mailDate, mailVisibleText } from '../lib/dashboard/mail';

  interface Props {
    mailList?: MailListData;
    mailMessage?: MailMessageData;
    actionResult?: ActionResultData;
    connectionState?: ConnectionState;
    onaction: (
      action: string,
      fields: ActionPerformFields | undefined,
      label: string,
    ) => string | undefined;
  }

  let { mailList, mailMessage, actionResult, connectionState = 'connecting', onaction }: Props = $props();
  let mailTarget = $state('');
  let mailSubject = $state('');
  let mailBody = $state('');
  let mailReportReason = $state('');
  let pendingSend = $state<{ id: string; subject: string; body: string }>();
  let pendingReport = $state<{ id: string; reason: string }>();

  $effect(() => {
    if (!actionResult?.requestId) return;
    if (actionResult.requestId === pendingSend?.id) {
      if (actionResult.ok) {
        if (mailSubject === pendingSend.subject) mailSubject = '';
        if (mailBody === pendingSend.body) mailBody = '';
      }
      pendingSend = undefined;
    }
    if (actionResult.requestId === pendingReport?.id) {
      if (actionResult.ok && mailReportReason === pendingReport.reason) mailReportReason = '';
      pendingReport = undefined;
    }
  });

  $effect(() => {
    if (connectionState === 'connected') return;
    pendingSend = undefined;
    pendingReport = undefined;
  });


  function sendMail(): void {
    if (pendingSend || connectionState !== 'connected' || !mailVisibleText(mailTarget, 64) || mailTarget.includes('|') ||
      !mailVisibleText(mailSubject, 128) || mailSubject.includes('|') ||
      !mailVisibleText(mailBody, 2_048)) return;
    const id = onaction(
      'mail.send',
      { target: mailTarget, subject: mailSubject, body: mailBody },
      `mail to ${mailTarget}`,
    );
    if (id) pendingSend = { id, subject: mailSubject, body: mailBody };
  }

  function reportOpenMail(): void {
    if (pendingReport || connectionState !== 'connected' || !mailMessage ||
      !mailVisibleText(mailReportReason, 2_048)) return;
    const id = onaction(
      'mail.report',
      { mailId: mailMessage.id, reason: mailReportReason },
      `report mail ${mailMessage.id}`,
    );
    if (id) pendingReport = { id, reason: mailReportReason };
  }
</script>

<details class="dashboard-panel">
  <summary>
    Offline mail
    <span>{mailList ? `${mailList.unreadCount} unread · ${mailList.totalMessages}/${mailList.capacity}` : 'not loaded'}</span>
  </summary>
  <div class="panel-body">
    <div class="panel-heading-row">
      <h3>Inbox</h3>
      <button type="button" onclick={() => onaction('mail.list', undefined, 'mail inbox')}>Refresh</button>
    </div>
    {#if mailList}
      {#if mailList.messages.length > 0}
        <ul class="dashboard-list" aria-label="Offline mail inbox">
          {#each mailList.messages as message (message.id)}
            <li>
              <div>
                <strong>{message.read ? '' : 'Unread · '}{message.subject}</strong>
                <p>From {message.sender} · {mailDate(message.createdAt)}</p>
                <small>Expires {mailDate(message.expiresAt)}</small>
              </div>
              <div class="dashboard-actions">
                <button
                  type="button"
                  onclick={() => onaction('mail.read', { mailId: message.id }, `read mail ${message.id}`)}
                >Read</button>
                <button
                  type="button"
                  onclick={() => onaction('mail.delete', { mailId: message.id }, `delete mail ${message.id}`)}
                >Delete</button>
              </div>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty-copy">Your inbox is empty.</p>
      {/if}
      {#if mailList.totalPages > 1}
        <div class="dashboard-actions" aria-label="Mail pages">
          <button
            type="button"
            disabled={mailList.page <= 1}
            onclick={() => onaction('mail.list', { page: mailList.page - 1 }, 'previous mail page')}
          >Previous</button>
          <span>Page {mailList.page} / {mailList.totalPages}</span>
          <button
            type="button"
            disabled={mailList.page >= mailList.totalPages}
            onclick={() => onaction('mail.list', { page: mailList.page + 1 }, 'next mail page')}
          >Next</button>
        </div>
      {/if}
    {:else}
      <p class="empty-copy">Load your bounded, 30-day inbox when you need it.</p>
    {/if}

    {#if mailMessage}
      <article aria-labelledby="open-mail-subject">
        <h3 id="open-mail-subject">{mailMessage.subject}</h3>
        <p>From {mailMessage.sender} · {mailDate(mailMessage.createdAt)}</p>
        <p class="mail-body">{mailMessage.body}</p>
        <form class="inline-form" onsubmit={(event) => { event.preventDefault(); reportOpenMail(); }}>
          <label for="mail-report-reason">Report this message</label>
          <textarea id="mail-report-reason" maxlength="2048" rows="3" bind:value={mailReportReason}></textarea>
          <button type="submit" disabled={connectionState !== 'connected' || Boolean(pendingReport) || !mailVisibleText(mailReportReason, 2_048)}>Send report</button>
        </form>
      </article>
    {/if}

    <form class="inline-form" onsubmit={(event) => { event.preventDefault(); sendMail(); }}>
      <h3>Send offline mail</h3>
      <label for="mail-target">Exact character name</label>
      <input id="mail-target" maxlength="64" bind:value={mailTarget} />
      <label for="mail-subject">Subject</label>
      <input id="mail-subject" maxlength="128" bind:value={mailSubject} />
      <label for="mail-body">Message</label>
      <textarea id="mail-body" maxlength="2048" rows="5" bind:value={mailBody}></textarea>
      <small>Mail expires after 30 days. Delivery acknowledgements do not reveal ignore or mailbox state.</small>
      <button
        type="submit"
        disabled={connectionState !== 'connected' || Boolean(pendingSend) || !mailVisibleText(mailTarget, 64) || mailTarget.includes('|') ||
          !mailVisibleText(mailSubject, 128) || mailSubject.includes('|') ||
          !mailVisibleText(mailBody, 2_048)}
      >Send mail</button>
    </form>
  </div>
</details>
