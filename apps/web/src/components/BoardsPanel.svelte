<script lang="ts">
  import type {
    ActionPerformFields,
    ActionResultData,
    BoardCatalogData,
    BoardEntryData,
    BoardKey,
    BoardListData,
    ConnectionState,
  } from '../lib/protocol';
  import { mailDate, mailVisibleText } from '../lib/dashboard/mail';

  interface Props {
    boardCatalog?: BoardCatalogData;
    boardList?: BoardListData;
    boardEntry?: BoardEntryData;
    actionResult?: ActionResultData;
    connectionState?: ConnectionState;
    onaction: (
      action: string,
      fields: ActionPerformFields | undefined,
      label: string,
    ) => string | undefined;
  }

  let {
    boardCatalog, boardList, boardEntry, actionResult, connectionState = 'connecting', onaction,
  }: Props = $props();
  let boardSelection = $state<BoardKey>('general');
  let boardSubject = $state('');
  let boardBody = $state('');
  let boardReportReason = $state('');
  let pendingBoardPostRequest = $state<string>();
  let pendingBoardReportRequest = $state<string>();
  const selectedBoard = $derived(
    boardCatalog?.boards.find((board) => board.key === boardSelection),
  );

  $effect(() => {
    const result = actionResult;
    if (!result?.requestId) return;
    if (result.requestId === pendingBoardPostRequest) {
      if (result.ok) {
        boardSubject = '';
        boardBody = '';
      }
      pendingBoardPostRequest = undefined;
    }
    if (result.requestId === pendingBoardReportRequest) {
      if (result.ok) boardReportReason = '';
      pendingBoardReportRequest = undefined;
    }
  });

  $effect(() => {
    if (connectionState === 'connected') return;
    pendingBoardPostRequest = undefined;
    pendingBoardReportRequest = undefined;
  });

  function loadBoard(page = 1): void {
    onaction('board.list', { board: boardSelection, page }, `board ${boardSelection} page ${page}`);
  }

  function publishBoardPost(): void {
    if (pendingBoardPostRequest || !selectedBoard?.canPost || !mailVisibleText(boardSubject, 128) ||
      boardSubject.includes('|') || !mailVisibleText(boardBody, 4_096)) return;
    const requestId = onaction(
      'board.post',
      { board: boardSelection, subject: boardSubject, body: boardBody },
      `post to board ${boardSelection}`,
    );
    if (requestId) pendingBoardPostRequest = requestId;
  }

  function reportBoardEntry(): void {
    if (pendingBoardReportRequest || !boardEntry?.canReport ||
      !mailVisibleText(boardReportReason, 2_048)) return;
    const requestId = onaction(
      'board.report',
      { postId: boardEntry.postId, reason: boardReportReason },
      `report board post ${boardEntry.postId}`,
    );
    if (requestId) pendingBoardReportRequest = requestId;
  }
</script>

<details class="dashboard-panel board-panel">
  <summary>
    Tablones
    <span>{boardList ? `${boardList.totalPosts}/${boardList.capacity}` : 'not loaded'}</span>
  </summary>
  <div class="panel-body">
    <div class="panel-heading-row">
      <h3>Community boards</h3>
      <button
        type="button"
        onclick={() => onaction('board.catalog', undefined, 'board catalog')}
      >Load boards</button>
    </div>
    {#if boardCatalog}
      <p>Posts are public, text-only and retained for {boardCatalog.retentionDays} days.</p>
      <div class="inline-form board-selector">
        <label for="board-selection">Board</label>
        <select id="board-selection" bind:value={boardSelection}>
          {#each boardCatalog.boards as board (board.key)}
            <option value={board.key}>{board.label}</option>
          {/each}
        </select>
        <p>{selectedBoard?.description}</p>
        <button type="button" onclick={() => loadBoard()}>Open board</button>
      </div>

      {#if boardList?.board === boardSelection}
        {#if boardList.posts.length > 0}
          <ul class="dashboard-list" aria-label={`${selectedBoard?.label ?? boardSelection} posts`}>
            {#each boardList.posts as post (post.postId)}
              <li>
                <div>
                  <strong>{post.subject}</strong>
                  <p>By {post.author} · {mailDate(post.createdAt)}</p>
                  <small>Expires {mailDate(post.expiresAt)}</small>
                </div>
                <div class="dashboard-actions">
                  <button
                    type="button"
                    onclick={() => onaction('board.read', { postId: post.postId }, `read board post ${post.postId}`)}
                  >Read</button>
                  {#if post.canDelete}
                    <button
                      type="button"
                      onclick={() => onaction('board.delete', { postId: post.postId }, `delete board post ${post.postId}`)}
                    >Delete</button>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="empty-copy">There are no posts on this page.</p>
        {/if}
        <div class="dashboard-actions" aria-label="Board pages">
          <button
            type="button"
            disabled={boardList.page <= 1}
            onclick={() => loadBoard(boardList.page - 1)}
          >Previous</button>
          <span>Page {boardList.page} / {boardList.totalPages}</span>
          <button
            type="button"
            disabled={boardList.page >= boardList.totalPages}
            onclick={() => loadBoard(boardList.page + 1)}
          >Next</button>
        </div>
      {/if}

      {#if boardEntry?.board === boardSelection}
        <article class="board-entry" aria-labelledby="open-board-subject">
          <h3 id="open-board-subject">{boardEntry.subject}</h3>
          <p>By {boardEntry.author} · {mailDate(boardEntry.createdAt)}</p>
          <p class="board-body">{boardEntry.body}</p>
          <div class="dashboard-actions">
            {#if boardEntry.canDelete}
              <button
                type="button"
                onclick={() => onaction('board.delete', { postId: boardEntry.postId }, `delete board post ${boardEntry.postId}`)}
              >Delete this post</button>
            {/if}
          </div>
          {#if boardEntry.canReport}
            <form class="inline-form" onsubmit={(event) => { event.preventDefault(); reportBoardEntry(); }}>
              <label for="board-report-reason">Report this post</label>
              <textarea
                id="board-report-reason"
                maxlength="2048"
                rows="3"
                bind:value={boardReportReason}
              ></textarea>
              <button
                type="submit"
                disabled={Boolean(pendingBoardReportRequest) ||
                  !mailVisibleText(boardReportReason, 2_048)}
              >Send report</button>
            </form>
          {/if}
        </article>
      {/if}

      <form class="inline-form" onsubmit={(event) => { event.preventDefault(); publishBoardPost(); }}>
        <h3>Publish to {selectedBoard?.label ?? boardSelection}</h3>
        <label for="board-subject">Subject</label>
        <input id="board-subject" maxlength="128" bind:value={boardSubject} />
        <label for="board-body">Post</label>
        <textarea id="board-body" maxlength="4096" rows="7" bind:value={boardBody}></textarea>
        {#if selectedBoard && !selectedBoard.canPost}
          <small role="status">Posting is unavailable while this character is muted or banned.</small>
        {:else}
          <small>No Markdown, HTML, links, images or attachments are interpreted.</small>
        {/if}
        <button
          type="submit"
          disabled={Boolean(pendingBoardPostRequest) || !selectedBoard?.canPost ||
            !mailVisibleText(boardSubject, 128) ||
            boardSubject.includes('|') || !mailVisibleText(boardBody, 4_096)}
        >Publish post</button>
      </form>
    {:else}
      <p class="empty-copy">Load the bounded public board catalog when you need it.</p>
    {/if}
  </div>
</details>
