<script lang="ts">
  import type { StoryDecisionMode, StoryLobby, StoryTally } from '../lib/generated/protocol';

  interface Props {
    lobby: StoryLobby;
    tally?: StoryTally;
    /** `Date.now()` milliseconds when the running window closes. */
    deadline?: number;
    connected: boolean;
    oninvite?: (name: string) => boolean;
    onkick?: (name: string) => void;
    onmode?: (mode: StoryDecisionMode, voteSeconds?: number) => void;
    onwithdraw?: () => void;
  }

  let {
    lobby,
    tally,
    deadline,
    connected,
    oninvite,
    onkick,
    onmode,
    onwithdraw,
  }: Props = $props();

  const MODE_LABELS: Record<StoryDecisionMode, string> = {
    vote: 'Majority vote',
    host: 'Host decides',
    turns: 'Round-robin turns',
  };
  const ROLE_LABELS = { host: 'host', player: 'player', spectator: 'spectator' } as const;

  const isHost = $derived(lobby.you === 'host');
  const turnHolder = $derived(lobby.seats.find((seat) => seat.turn)?.name);

  let inviteName = $state('');
  // Host controls start from the current rules and follow later changes.
  let chosenMode = $state<StoryDecisionMode>('vote');
  let chosenSeconds = $state(60);
  let syncedRules = '';
  $effect(() => {
    const rules = `${lobby.mode}:${lobby.voteSeconds}`;
    if (rules === syncedRules) return;
    syncedRules = rules;
    chosenMode = lobby.mode;
    chosenSeconds = lobby.voteSeconds;
  });

  // Countdown of the vote or turn window, refreshed every second. The timer
  // element is not a live region: only vote counts are announced.
  let clock = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => (clock = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  const secondsLeft = $derived(
    deadline === undefined ? undefined : Math.max(0, Math.ceil((deadline - clock) / 1000)),
  );

  function submitInvite(event: SubmitEvent): void {
    event.preventDefault();
    if (inviteName.trim().length === 0) return;
    if (oninvite?.(inviteName)) inviteName = '';
  }

  function submitMode(event: SubmitEvent): void {
    event.preventDefault();
    onmode?.(chosenMode, chosenSeconds === lobby.voteSeconds ? undefined : chosenSeconds);
  }
</script>

<section class="story-lobby" aria-labelledby="story-lobby-title">
  <h2 id="story-lobby-title">Shared story</h2>
  <p>
    {MODE_LABELS[lobby.mode]} · {lobby.voteSeconds} s window · you are the {ROLE_LABELS[lobby.you]}.
  </p>
  {#if lobby.code}
    <p>Join code: <strong class="story-code">{lobby.code}</strong> (others type <kbd>historia unirse {lobby.code}</kbd>).</p>
  {/if}

  <ul class="story-seats" aria-label="Seats">
    {#each lobby.seats as seat (seat.name)}
      <li>
        <span>{seat.name}</span>
        {#if seat.role === 'host'}<span class="story-tag">host</span>{/if}
        {#if !seat.connected}<span class="story-tag">disconnected, seat held</span>{/if}
        {#if seat.turn}<span class="story-tag">their turn</span>{/if}
        {#if isHost && seat.role !== 'host' && onkick}
          <button type="button" disabled={!connected} aria-label={`Remove ${seat.name}`} onclick={() => onkick?.(seat.name)}>Remove</button>
        {/if}
      </li>
    {/each}
  </ul>
  <p>Seats {lobby.seats.length}/{lobby.maxSeats} · spectators {lobby.spectators}/{lobby.maxSpectators}</p>

  {#if tally && lobby.mode !== 'turns'}
    <p class="story-tally" aria-live="polite">
      {#if lobby.mode === 'vote'}
        Votes: {tally.voted} of {tally.eligible}.
      {:else if tally.voted > 0}
        Advisory votes: {tally.voted}. The host decides.
      {/if}
      {#if tally.yourVote}You voted.{/if}
    </p>
  {/if}
  {#if lobby.mode === 'turns' && turnHolder}
    <p class="story-tally" aria-live="polite">Turn: {turnHolder}.</p>
  {/if}
  {#if secondsLeft !== undefined}
    <p class="story-timer" role="timer" aria-label="Time left to decide">
      {lobby.mode === 'turns' ? 'The turn passes' : 'The vote closes'} in {secondsLeft} s.
    </p>
  {/if}
  {#if tally?.yourVote && onwithdraw}
    <button type="button" disabled={!connected} onclick={onwithdraw}>Withdraw my vote</button>
  {/if}

  {#if isHost}
    <form class="story-host-controls" aria-label="Host controls" onsubmit={submitMode}>
      <label>Decision mode
        <select bind:value={chosenMode}>
          <option value="vote">{MODE_LABELS.vote}</option>
          <option value="host">{MODE_LABELS.host}</option>
          <option value="turns">{MODE_LABELS.turns}</option>
        </select>
      </label>
      <label>Window (seconds)
        <input type="number" min="30" max="120" step="1" bind:value={chosenSeconds} />
      </label>
      <button type="submit" disabled={!connected}>Apply rules</button>
    </form>
    <form class="story-host-controls" aria-label="Invite a character" onsubmit={submitInvite}>
      <label>Character name
        <input type="text" maxlength="64" autocomplete="off" bind:value={inviteName} />
      </label>
      <button type="submit" disabled={!connected || inviteName.trim().length === 0}>Invite</button>
    </form>
  {/if}
</section>

<style>
  .story-lobby {
    display: grid;
    gap: 0.5rem;
    padding: 0.75rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
  }

  .story-lobby p,
  .story-lobby h2 {
    margin: 0;
  }

  .story-seats {
    display: grid;
    gap: 0.3rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .story-seats li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }

  .story-tag {
    font-size: 0.875rem;
    font-style: italic;
  }

  .story-code {
    letter-spacing: 0.1em;
  }

  .story-host-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 0.5rem;
  }

  .story-host-controls label {
    display: grid;
    gap: 0.2rem;
  }

  .story-lobby button {
    padding: 0.4rem 0.7rem;
    border: 1px solid #82744f;
    border-radius: 0.2rem;
    background: #2f2b1f;
    color: #f3ecd9;
    cursor: pointer;
  }
</style>
