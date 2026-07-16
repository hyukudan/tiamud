<script lang="ts">
  import type { PlayerStateSnapshot, SceneAction } from '../lib/protocol';

  interface Props {
    value?: PlayerStateSnapshot;
    abilityTargets?: Array<{ id: string; label: string }>;
    oncommand: (command: string) => void;
    onaction: (action: string, target: string | undefined, label: string) => void;
    onability: (abilityId: string, target: string | undefined, label: string) => void;
  }

  let { value, abilityTargets = [], oncommand, onaction, onability }: Props = $props();
  let socialTarget = $state('');
  let chatText = $state('');
  let partyTarget = $state('');
  let abilityTargetById = $state<Record<string, string>>({});
  const additionalAttributes = $derived(
    value?.character.attributes.filter((attribute) => attribute.id !== 'strength' && attribute.id !== 'defence') ?? [],
  );

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

  const targetingLabel = (targeting: PlayerStateSnapshot['character']['abilities'][number]['targeting']): string => ({
    self: 'Self',
    // The current scene contract has no authorized ally target IDs. The wire
    // value stays additive for a future client, but v1 executes it as self.
    optionalTarget: 'Self',
    explicitTarget: 'Choose a target',
  })[targeting];

  function resourceCostLabel(
    cost: PlayerStateSnapshot['character']['abilities'][number]['resourceCosts'][number],
  ): string {
    return `${cost.amount} ${cost.label}`;
  }

  function abilityTarget(abilityId: string): string | undefined {
    const selected = abilityTargetById[abilityId];
    return selected && abilityTargets.some((target) => target.id === selected) ? selected : undefined;
  }

  function useAbility(ability: PlayerStateSnapshot['character']['abilities'][number]): void {
    const target = ability.targeting === 'explicitTarget'
      ? abilityTarget(ability.id)
      : undefined;
    onability(ability.id, target, `${ability.kind === 'spell' ? 'cast' : 'use'} ${ability.name}`);
  }
</script>

<aside class="player-dashboard" aria-label="Character and world panels">
  {#if value}
    <section class="character-card" aria-labelledby="character-state-title">
      <div>
        <p class="eyebrow">Character</p>
        <h2 id="character-state-title">{value.character.name}</h2>
        <p>
          {#if value.character.archetype}{value.character.archetype.label} · {/if}Level {value.character.level}
          · {value.character.experience} / {value.character.nextLevelExperience} XP
        </p>
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
      {#if additionalAttributes.length > 0}
        <dl class="attributes attribute-grid" aria-label="Attributes">
          {#each additionalAttributes as attribute (attribute.id)}
            <div><dt>{attribute.label}</dt><dd>{attribute.value}</dd></div>
          {/each}
        </dl>
      {/if}
      {#if value.character.resources.length > 0}
        <div class="resource-readouts" aria-label="Resources">
          {#each value.character.resources as resource (resource.id)}
            <div>
              <label for={`resource-${resource.id}`}>{resource.label}</label>
              <meter id={`resource-${resource.id}`} min="0" max={resource.max} value={resource.current}>
                {resource.current} / {resource.max}
              </meter>
              <strong>{resource.current} / {resource.max}</strong>
            </div>
          {/each}
        </div>
      {/if}
      {#if value.character.resistances.length > 0}
        <dl class="resistance-list" aria-label="Resistances">
          {#each value.character.resistances as resistance (resistance.damageType)}
            <div><dt>{resistance.damageType}</dt><dd>{resistance.percent > 0 ? '+' : ''}{resistance.percent}%</dd></div>
          {/each}
        </dl>
      {/if}
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
      {#if value.character.statuses.length > 0}
        <ul class="compact-list status-list" aria-label="Active statuses">
          {#each value.character.statuses as status}
            <li>
              <strong>{status.label}</strong> · {status.kind.replaceAll('_', ' ')}
              {#if status.magnitude !== 0} · {status.magnitude > 0 ? '+' : ''}{status.magnitude}{/if}
              · {status.remainingTicks} ticks
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if value.character.abilities.length > 0}
      <details class="dashboard-panel ability-panel" open>
        <summary>Abilities <span>{value.character.abilities.length}</span></summary>
        <div class="panel-body">
          <ul class="ability-list">
            {#each value.character.abilities as ability (ability.id)}
              <li>
                <div class="ability-heading">
                  <div>
                    <span class={`ability-kind ${ability.kind}`}>{ability.kind}</span>
                    <h3 id={`ability-${ability.id}`}>{ability.name}</h3>
                  </div>
                  <span>{targetingLabel(ability.targeting)}</span>
                </div>
                <p id={`ability-description-${ability.id}`}>{ability.description}</p>
                <ul class="ability-meta" aria-label={`${ability.name} requirements`}>
                  {#each ability.resourceCosts as cost (`${cost.resourceId}-${cost.amount}`)}
                    <li>{resourceCostLabel(cost)}</li>
                  {/each}
                  {#if ability.cooldownRemaining > 0}<li>{ability.cooldownRemaining} ticks cooldown</li>{/if}
                </ul>
                {#if ability.targeting === 'explicitTarget'}
                  <label for={`ability-target-${ability.id}`}>Target for {ability.name}</label>
                  <select
                    id={`ability-target-${ability.id}`}
                    value={abilityTarget(ability.id) ?? ''}
                    onchange={(event) => {
                      abilityTargetById = {
                        ...abilityTargetById,
                        [ability.id]: event.currentTarget.value,
                      };
                    }}
                  >
                    <option value="">Choose target</option>
                    {#each abilityTargets as target (target.id)}
                      <option value={target.id}>{target.label}</option>
                    {/each}
                  </select>
                {/if}
                <button
                  type="button"
                  aria-describedby={`ability-description-${ability.id}`}
                  disabled={!ability.usable || ability.cooldownRemaining > 0 ||
                    (ability.targeting === 'explicitTarget' && !abilityTarget(ability.id))}
                  onclick={() => useAbility(ability)}
                >{ability.kind === 'spell' ? 'Cast' : 'Use'} {ability.name}</button>
                {#if !ability.usable}
                  <p class="ability-unavailable" role="status">Unavailable right now.</p>
                {:else if ability.targeting === 'explicitTarget' && !abilityTarget(ability.id)}
                  <p class="ability-unavailable">Choose a visible target to use this ability.</p>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      </details>
    {/if}

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
