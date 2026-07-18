<script lang="ts">
  import {
    greedCommand,
    LOOT_MODE_LABELS,
    lootMasterCommand,
    lootMasterMemberValid,
    lootModeCommand,
    lootRollCommand,
    needCommand,
    parseLootRoll,
    passCommand,
    rootIsMaster,
    rootIsNeedGreed,
  } from '../lib/dashboard/loot';

  interface Props {
    lootRoll?: string;
    oncommand: (command: string) => void;
  }

  let { lootRoll, oncommand }: Props = $props();
  let lootMasterMember = $state('');
  const lootView = $derived(lootRoll ? parseLootRoll(lootRoll) : undefined);
</script>

<details class="dashboard-panel">
  <summary>
    Group loot
    <span>{lootView ? LOOT_MODE_LABELS[lootView.mode] : 'not loaded'}</span>
  </summary>
  <div class="panel-body">
    <div class="panel-heading-row">
      <h3>Group loot</h3>
      <button type="button" onclick={() => oncommand(lootRollCommand())}>Refresh</button>
    </div>
    <div class="dashboard-actions" aria-label="Loot distribution mode">
      <button
        type="button"
        aria-pressed={lootView?.mode === 'round_robin'}
        onclick={() => oncommand(lootModeCommand('round_robin'))}
      >Round-robin</button>
      <button
        type="button"
        aria-pressed={lootView?.mode === 'need_greed'}
        onclick={() => oncommand(lootModeCommand('need_greed'))}
      >Need / Greed</button>
      <button
        type="button"
        aria-pressed={lootView?.mode === 'master_loot'}
        onclick={() => oncommand(lootModeCommand('master_loot'))}
      >Master-loot</button>
    </div>
    <p class="empty-copy">Changing the distribution mode requires the party leader.</p>
    {#if !lootView}
      <p class="empty-copy">Run &ldquo;loot roll&rdquo; to see the current distribution.</p>
    {:else if lootView.empty}
      <p class="empty-copy">No carried loot to divide for {lootView.npcName}.</p>
    {:else}
      {#if lootView.mode === 'master_loot'}
        <div class="inline-form">
          <label for="loot-master-member">Assign to</label>
          <input id="loot-master-member" maxlength="64" bind:value={lootMasterMember} />
        </div>
        <p class="empty-copy">Assigning items requires the party leader.</p>
      {/if}
      <ul class="dashboard-list" aria-label={`Group loot for ${lootView.npcName}`}>
        {#each lootView.roots as root (root.ref)}
          <li>
            <strong>{root.name}</strong>
            {#if lootView.mode === 'need_greed' && rootIsNeedGreed(root)}
              <p>{root.need} need · {root.greed} greed · {root.pass} pass · {root.pending} pending</p>
              <div class="dashboard-actions">
                <button type="button" onclick={() => oncommand(needCommand(root.ref))}>Need</button>
                <button type="button" onclick={() => oncommand(greedCommand(root.ref))}>Greed</button>
                <button type="button" onclick={() => oncommand(passCommand(root.ref))}>Pass</button>
              </div>
            {:else if lootView.mode === 'master_loot' && rootIsMaster(root)}
              <p>{root.assignedTo ? `Assigned to ${root.assignedTo}` : 'Unassigned'}</p>
              <div class="dashboard-actions">
                <button
                  type="button"
                  disabled={!lootMasterMemberValid(lootMasterMember)}
                  onclick={() => oncommand(lootMasterCommand(root.ref, lootMasterMember))}
                >Assign</button>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</details>
