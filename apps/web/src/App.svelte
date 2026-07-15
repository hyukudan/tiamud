<script lang="ts">
  import { onMount } from 'svelte';
  import type {
    ActionResultData,
    ConnectionState,
    SceneSnapshot,
    ServerMessage,
    ServerNoticeData,
  } from './lib/protocol';
  import { MudConnection, websocketUrl } from './lib/socket';

  interface HistoryEntry {
    id: string;
    command: string;
    state: 'pending' | 'complete' | 'failed';
    response?: string;
  }

  let connectionState = $state<ConnectionState>('connecting');
  let serverName = $state('Mud');
  let scene = $state<SceneSnapshot>();
  let notice = $state<ServerNoticeData>();
  let command = $state('');
  let history = $state<HistoryEntry[]>([]);
  let connection: MudConnection | undefined;

  const connectionLabel = $derived(
    connectionState === 'connected'
      ? 'Connected'
      : connectionState === 'connecting'
        ? 'Connecting…'
        : 'Disconnected — reconnecting…',
  );

  function handleActionResult(result: ActionResultData): void {
    if (!result.requestId) {
      if (result.message) {
        notice = { level: result.ok ? 'info' : 'error', message: result.message };
      }
      return;
    }

    history = history.map((entry) =>
      entry.id === result.requestId
        ? {
            ...entry,
            state: result.ok ? 'complete' : 'failed',
            response: result.message,
          }
        : entry,
    );
  }

  function handleMessage(message: ServerMessage): void {
    switch (message.type) {
      case 'server.welcome':
        serverName = message.data.serverName ?? serverName;
        if (message.data.motd) {
          notice = { level: 'info', message: message.data.motd };
        }
        break;
      case 'scene.snapshot':
        scene = message.data;
        break;
      case 'action.result':
        handleActionResult(message.data);
        break;
      case 'server.notice':
        notice = message.data;
        break;
    }
  }

  function submitCommand(value = command): void {
    const submitted = value.trim();
    if (!submitted) {
      return;
    }

    const id = crypto.randomUUID();
    const sent = connection?.submit(submitted, id) ?? false;

    history = [
      ...history.slice(-49),
      {
        id,
        command: submitted,
        state: sent ? 'pending' : 'failed',
        response: sent ? undefined : 'The command was not sent because the server is unavailable.',
      },
    ];
    command = '';
  }

  onMount(() => {
    connection = new MudConnection(websocketUrl(window.location), {
      onMessage: handleMessage,
      onStateChange: (state) => {
        connectionState = state;
      },
      onProtocolError: () => {
        notice = { level: 'warning', message: 'The server sent a message this client could not read.' };
      },
    });
    connection.connect();

    return () => connection?.close();
  });
</script>

<svelte:head>
  <title>{serverName}</title>
</svelte:head>

<div class="shell">
  <header class="topbar">
    <a class="brand" href="/" aria-label={`${serverName} home`}>{serverName}</a>
    <p class:connected={connectionState === 'connected'} class="connection" role="status">
      <span aria-hidden="true"></span>{connectionLabel}
    </p>
  </header>

  {#if notice}
    <aside class:warning={notice.level === 'warning'} class:error={notice.level === 'error'} class="notice">
      <p role={notice.level === 'error' ? 'alert' : 'status'}>{notice.message}</p>
      <button aria-label="Dismiss notice" onclick={() => (notice = undefined)}>×</button>
    </aside>
  {/if}

  <main>
    <section class="scene" aria-labelledby="scene-title">
      {#if scene}
        {#if scene.image}
          <picture class="atmosphere">
            {#if scene.image.avif}<source srcset={scene.image.avif} type="image/avif" />{/if}
            {#if scene.image.webp}<source srcset={scene.image.webp} type="image/webp" />{/if}
            <img
              src={scene.image.jpeg ?? scene.image.webp ?? scene.image.avif}
              alt={scene.image.alt}
              width={scene.image.width}
              height={scene.image.height}
              fetchpriority="high"
            />
          </picture>
        {/if}

        <div class="narrative">
          <p class="eyebrow">Current location</p>
          <h1 id="scene-title">{scene.title}</h1>
          {#each scene.prose as paragraph}
            <p>{paragraph}</p>
          {/each}

          {#if scene.exits.length > 0}
            <nav class="exits" aria-label="Exits">
              <h2>Ways onward</h2>
              <ul>
                {#each scene.exits as exit (exit.id)}
                  <li><button onclick={() => submitCommand(exit.command)}>{exit.label}</button></li>
                {/each}
              </ul>
            </nav>
          {/if}
        </div>
      {:else}
        <div class="empty-scene">
          <p class="eyebrow">Entering the world</p>
          <h1 id="scene-title">Awaiting the first scene…</h1>
          <p>The world will appear here when the server is ready.</p>
        </div>
      {/if}
    </section>

    <section class="console" aria-labelledby="console-title">
      <div class="console-heading">
        <h2 id="console-title">Command history</h2>
        <span>{history.length} / 50</span>
      </div>

      <ol class="history" aria-live="polite" aria-relevant="additions text">
        {#if history.length === 0}
          <li class="history-empty">Try <kbd>look</kbd> to examine your surroundings.</li>
        {:else}
          {#each history as entry (entry.id)}
            <li class:failed={entry.state === 'failed'}>
              <div><span aria-hidden="true">›</span> <code>{entry.command}</code></div>
              {#if entry.response}<p>{entry.response}</p>{/if}
              {#if entry.state === 'pending'}<span class="sr-only">Waiting for the server.</span>{/if}
            </li>
          {/each}
        {/if}
      </ol>

      <form
        class="command-form"
        onsubmit={(event) => {
          event.preventDefault();
          submitCommand();
        }}
      >
        <label for="command">What do you do?</label>
        <div>
          <span aria-hidden="true">›</span>
          <input
            id="command"
            bind:value={command}
            autocomplete="off"
            autocapitalize="none"
            enterkeyhint="send"
            placeholder="look, north, say hello…"
          />
          <button type="submit" disabled={!command.trim()}>Send</button>
        </div>
      </form>
    </section>
  </main>
</div>
