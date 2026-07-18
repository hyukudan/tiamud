<script lang="ts">
  import type {
    ActionPerformFields,
    PlayerStateSnapshot,
    SceneAction,
  } from '../lib/protocol';

  interface Props {
    party?: PlayerStateSnapshot['party'];
    partyInvites: PlayerStateSnapshot['partyInvites'];
    onaction: (
      action: string,
      fields: ActionPerformFields | undefined,
      label: string,
    ) => string | undefined;
    onperform: (action: SceneAction) => void;
  }

  let { party, partyInvites, onaction, onperform }: Props = $props();
  let partyTarget = $state('');
  let partyMessage = $state('');

  function invite(): void {
    const target = partyTarget.trim();
    if (!target) return;
    onaction('party.invite', { target }, `party invite ${target}`);
    partyTarget = '';
  }

  function partyTargetAction(action: 'revoke' | 'kick' | 'transfer'): void {
    const target = partyTarget.trim();
    if (!target) return;
    const actionName = {
      revoke: 'party.revoke',
      kick: 'party.kick',
      transfer: 'party.transfer',
    }[action];
    onaction(actionName, { target }, `party ${action} ${target}`);
  }

  function sendPartyMessage(): void {
    const message = partyMessage.trim();
    if (!message) return;
    onaction('party.say', { message }, 'party message');
    partyMessage = '';
  }
</script>

<details class="dashboard-panel">
  <summary>Party <span>{party?.members.length ?? 0}</span></summary>
  <div class="panel-body">
    {#if party}
      <ul class="dashboard-list">
        {#each party.members as member (member.id)}
          <li><span><strong>{member.name}</strong>{member.leader ? ' · leader' : ''}</span><small>{member.online ? 'Online' : 'Away'}</small></li>
        {/each}
      </ul>
      <div class="dashboard-actions">
        {#each party.actions as action (action.command)}
          <button type="button" onclick={() => onperform(action)}>{action.label}</button>
        {/each}
      </div>
      {#if party.leader}
        <form class="inline-form" onsubmit={(event) => { event.preventDefault(); invite(); }}>
          <label for="party-target">Invite or manage an adventurer</label>
          <input id="party-target" maxlength="64" bind:value={partyTarget} aria-describedby="party-target-help" />
          <small id="party-target-help">Enter the exact character name.</small>
          <div class="dashboard-actions">
            <button type="submit" disabled={!partyTarget.trim()}>Invite</button>
            <button type="button" disabled={!partyTarget.trim()} onclick={() => partyTargetAction('revoke')}>Revoke invitation</button>
            <button type="button" disabled={!partyTarget.trim()} onclick={() => partyTargetAction('kick')}>Remove from party</button>
            <button type="button" disabled={!partyTarget.trim()} onclick={() => partyTargetAction('transfer')}>Transfer leadership</button>
          </div>
        </form>
        {#if !party.actions.some((action) => action.command.trim().toLowerCase() === 'party disband')}
          <button type="button" onclick={() => onaction('party.disband', undefined, 'party disband')}>Disband party</button>
        {/if}
      {/if}
      <form class="inline-form" onsubmit={(event) => { event.preventDefault(); sendPartyMessage(); }}>
        <label for="party-message">Message to party</label>
        <div><input id="party-message" maxlength="512" bind:value={partyMessage} /><button type="submit" disabled={!partyMessage.trim()}>Send to party</button></div>
      </form>
    {:else}
      <p class="empty-copy">You are travelling alone.</p>
      <button type="button" onclick={() => onaction('party.create', undefined, 'party create')}>Create party</button>
    {/if}
    {#if partyInvites.length > 0}
      <h3>Invitations</h3>
      <ul class="dashboard-list">
        {#each partyInvites as invite (invite.leaderName)}
          <li>
            <span>From <strong>{invite.leaderName}</strong> · {invite.expiresInTicks} ticks</span>
            <div class="dashboard-actions">
              {#each invite.actions as action (action.command)}
                <button type="button" onclick={() => onperform(action)}>{action.label}</button>
              {/each}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</details>
