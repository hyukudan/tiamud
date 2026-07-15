<script lang="ts">
  import type { PlayerStateSnapshot, SceneAction } from '../lib/protocol';

  interface Props {
    value?: PlayerStateSnapshot;
    oncommand: (command: string) => void;
    onaction: (action: string, target: string | undefined, label: string) => void;
  }

  let { value, oncommand, onaction }: Props = $props();
  let socialTarget = $state('');
  let chatText = $state('');
  let partyTarget = $state('');

  const statusLabel = (status: PlayerStateSnapshot['quests'][number]['status']): string => ({
    available: 'Available',
    active: 'Active',
    readyToClaim: 'Reward ready',
    completed: 'Completed',
  })[status];

  function perform(action: SceneAction): void {
    const [verb = '', ...tail] = action.command.trim().split(/\s+/);
    const target = tail.join(' ') || undefined;
    const structured = ({
      examine: 'item.examine',
      drop: 'item.drop',
      use: 'item.use',
      equip: 'item.equip',
      unequip: 'item.unequip',
      buy: 'economy.buy',
      sell: 'economy.sell',
    } as Record<string, string>)[verb.toLowerCase()];
    if (structured) {
      onaction(structured, target, action.command);
      return;
    }
    if (verb === 'quest' && tail.length >= 2) {
      onaction(`quest.${tail[0]}`, tail.slice(1).join(' '), action.command);
      return;
    }
    if (verb === 'party' && tail.length >= 1) {
      onaction(`party.${tail[0]}`, tail.slice(1).join(' ') || undefined, action.command);
      return;
    }
    oncommand(action.command);
  }

  function sendChat(): void {
    const text = chatText.trim();
    if (!text) return;
    const target = socialTarget.trim();
    oncommand(target ? `tell ${target} ${text}` : `say ${text}`);
    chatText = '';
  }

  function invite(): void {
    const target = partyTarget.trim();
    if (!target) return;
    onaction('party.invite', target, `party invite ${target}`);
    partyTarget = '';
  }
</script>

<aside class="player-dashboard" aria-label="Character and world panels">
  {#if value}
    <section class="character-card" aria-labelledby="character-state-title">
      <div>
        <p class="eyebrow">Character</p>
        <h2 id="character-state-title">{value.character.name}</h2>
        <p>Level {value.character.level} · {value.character.experience} / {value.character.nextLevelExperience} XP</p>
      </div>
      <div class="health-readout">
        <label for="player-health">Health</label>
        <meter id="player-health" min="0" max={value.character.maxHealth} value={value.character.health}>
          {value.character.health} / {value.character.maxHealth}
        </meter>
        <strong>{value.character.health} / {value.character.maxHealth}</strong>
      </div>
      <dl class="attributes">
        <div><dt>Strength</dt><dd>{value.character.strength}</dd></div>
        <div><dt>Defence</dt><dd>{value.character.defence}</dd></div>
      </dl>
      {#if value.character.skills.length > 0}
        <dl class="attributes" aria-label="Skills">
          {#each value.character.skills as skill (skill.id)}
            <div><dt>{skill.label}</dt><dd>{skill.rank}</dd></div>
          {/each}
        </dl>
      {/if}
      {#if value.character.recoveryInTicks !== undefined}
        <p class="recovery" role="status">Recovering in {value.character.recoveryInTicks} world ticks.</p>
      {/if}
      {#if value.character.effects.length > 0}
        <ul class="compact-list" aria-label="Active effects">
          {#each value.character.effects as effect}
            <li>{effect.kind} +{effect.magnitude} ({effect.remainingTicks} ticks)</li>
          {/each}
        </ul>
      {/if}
    </section>

    <details class="dashboard-panel" open>
      <summary>Inventory <span>{value.inventory.length}</span></summary>
      <div class="panel-body">
        {#if value.inventory.length === 0 && value.equipment.length === 0}
          <p class="empty-copy">You are carrying nothing.</p>
        {/if}
        {#if value.inventory.length > 0}
          <h3>Carried</h3>
          <ul class="dashboard-list inventory-list">
            {#each value.inventory as item (item.id)}
              <li>
                <div><strong>{item.label}</strong><p>{item.description}</p></div>
                <div class="dashboard-actions">
                  {#each item.actions as action (action.command)}
                    <button type="button" onclick={() => perform(action)}>{action.label}</button>
                  {/each}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
        {#if value.equipment.length > 0}
          <h3>Equipped</h3>
          <ul class="dashboard-list">
            {#each value.equipment as item (item.slot)}
              <li>
                <span><small>{item.slot}</small> <strong>{item.label}</strong></span>
                <div class="dashboard-actions">
                  {#each item.actions as action (action.command)}
                    <button type="button" onclick={() => perform(action)}>{action.label}</button>
                  {/each}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </details>

    <details class="dashboard-panel">
      <summary>Discovered map <span>{value.map.rooms.length}</span></summary>
      <div class="panel-body">
        <ul class="map-list">
          {#each value.map.rooms as room (room.id)}
            <li class:current={room.current}>
              <strong>{room.title}</strong>
              {#if room.current}<span>Current</span>{/if}
              <ul aria-label={`Known paths from ${room.title}`}>
                {#each value.map.paths.filter((path) => path.from === room.id) as path}
                  <li>{path.direction} → {path.to ? (value.map.rooms.find((known) => known.id === path.to)?.title ?? 'Known place') : 'Undiscovered'}</li>
                {/each}
              </ul>
            </li>
          {/each}
        </ul>
      </div>
    </details>

    <details class="dashboard-panel">
      <summary>Quests <span>{value.quests.length}</span></summary>
      <div class="panel-body">
        <ul class="dashboard-list">
          {#each value.quests as quest (quest.id)}
            <li>
              <div>
                <strong>{quest.label}</strong>
                <p>{quest.summary}</p>
                <p><span class="status-pill">{statusLabel(quest.status)}</span> {quest.current} / {quest.target}</p>
              </div>
              <div class="dashboard-actions">
                {#each quest.actions as action (action.command)}
                  <button type="button" onclick={() => perform(action)}>{action.label}</button>
                {/each}
              </div>
            </li>
          {/each}
        </ul>
      </div>
    </details>

    <details class="dashboard-panel">
      <summary>Party <span>{value.party?.members.length ?? 0}</span></summary>
      <div class="panel-body">
        {#if value.party}
          <ul class="dashboard-list">
            {#each value.party.members as member (member.id)}
              <li><span><strong>{member.name}</strong>{member.leader ? ' · leader' : ''}</span><small>{member.online ? 'Online' : 'Away'}</small></li>
            {/each}
          </ul>
          <div class="dashboard-actions">
            {#each value.party.actions as action (action.command)}
              <button type="button" onclick={() => perform(action)}>{action.label}</button>
            {/each}
          </div>
          {#if value.party.leader}
            <form class="inline-form" onsubmit={(event) => { event.preventDefault(); invite(); }}>
              <label for="party-target">Invite adventurer</label>
              <div><input id="party-target" maxlength="64" bind:value={partyTarget} /><button type="submit" disabled={!partyTarget.trim()}>Invite</button></div>
            </form>
          {/if}
        {:else}
          <p class="empty-copy">You are travelling alone.</p>
          <button type="button" onclick={() => onaction('party.create', undefined, 'party create')}>Create party</button>
        {/if}
        {#if value.partyInvites.length > 0}
          <h3>Invitations</h3>
          <ul class="dashboard-list">
            {#each value.partyInvites as invite (invite.leaderName)}
              <li>
                <span>From <strong>{invite.leaderName}</strong> · {invite.expiresInTicks} ticks</span>
                <div class="dashboard-actions">
                  {#each invite.actions as action (action.command)}
                    <button type="button" onclick={() => perform(action)}>{action.label}</button>
                  {/each}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </details>

    <details class="dashboard-panel">
      <summary>Coin and market <span>{value.economy.coins} coin</span></summary>
      <div class="panel-body">
        <p class="balance"><strong>{value.economy.coins}</strong> coin</p>
        {#if value.economy.commodities.length > 0}
          <ul class="compact-list" aria-label="Owned market goods">
            {#each value.economy.commodities as commodity (commodity.id)}
              <li>{commodity.label} × {commodity.quantity}</li>
            {/each}
          </ul>
        {/if}
        {#if value.economy.shopAvailable}
          <h3>Lantern Market</h3>
          <ul class="dashboard-list">
            {#each value.economy.shop as item (item.id)}
              <li>
                <div><strong>{item.label}</strong><p>Buy {item.buyPrice} · sell {item.sellPrice} · stock {item.stock} · owned {item.owned}</p></div>
                <div class="dashboard-actions">
                  {#each item.actions as action (action.command)}
                    <button type="button" onclick={() => perform(action)}>{action.label}</button>
                  {/each}
                </div>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="empty-copy">Trading is available in Lantern Square.</p>
        {/if}
      </div>
    </details>

    <details class="dashboard-panel" open>
      <summary>Chat and adventurers <span>{value.social.online.length}</span></summary>
      <div class="panel-body">
        <div class="panel-heading-row">
          <h3>Online now</h3>
          <button type="button" onclick={() => onaction('social.who', undefined, 'who')}>Refresh</button>
        </div>
        <ul class="who-list">
          {#each value.social.online as adventurer (adventurer.id)}
            <li>
              <span>{adventurer.name}{adventurer.selfCharacter ? ' (you)' : ''}</span>
              {#if !adventurer.selfCharacter}
                <div class="dashboard-actions">
                  <button type="button" onclick={() => (socialTarget = adventurer.name)}>Tell</button>
                  <button type="button" onclick={() => onaction('social.ignore', adventurer.name, `ignore ${adventurer.name}`)}>Ignore</button>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
        <form class="inline-form" onsubmit={(event) => { event.preventDefault(); sendChat(); }}>
          <label for="chat-target">Recipient <span>(blank means say)</span></label>
          <input id="chat-target" maxlength="64" bind:value={socialTarget} placeholder="Everyone nearby" />
          <label for="chat-text">Message</label>
          <div><input id="chat-text" maxlength="512" bind:value={chatText} /><button type="submit" disabled={!chatText.trim()}>Send</button></div>
        </form>
        <form class="inline-form" onsubmit={(event) => event.preventDefault()}>
          <label for="ignore-target">Manage ignore list</label>
          <div>
            <input id="ignore-target" maxlength="64" bind:value={socialTarget} />
            <button type="button" disabled={!socialTarget.trim()} onclick={() => onaction('social.ignore', socialTarget.trim(), `ignore ${socialTarget.trim()}`)}>Ignore</button>
            <button type="button" disabled={!socialTarget.trim()} onclick={() => onaction('social.unignore', socialTarget.trim(), `unignore ${socialTarget.trim()}`)}>Unignore</button>
          </div>
        </form>
      </div>
    </details>
  {:else}
    <section class="character-card" aria-live="polite">
      <p>Character state will appear after the world accepts your session.</p>
    </section>
  {/if}
</aside>
