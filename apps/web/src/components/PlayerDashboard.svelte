<script lang="ts">
  import type {
    ActionPerformFields,
    CommunityChannel,
    PlayerStateSnapshot,
    SceneAction,
  } from '../lib/protocol';

  interface Props {
    value?: PlayerStateSnapshot;
    abilityTargets?: Array<{ id: string; label: string }>;
    oncommand: (command: string) => void;
    onaction: (action: string, fields: ActionPerformFields | undefined, label: string) => void;
    onability: (abilityId: string, target: string | undefined, label: string) => void;
    onpromote: (classId: string, label: string) => void;
    ontalent: (graphId: string, nodeId: string, label: string) => void;
  }

  let {
    value, abilityTargets = [], oncommand, onaction, onability, onpromote, ontalent,
  }: Props = $props();
  let socialTarget = $state('');
  let chatText = $state('');
  let partyTarget = $state('');
  let partyMessage = $state('');
  let replyMessage = $state('');
  let communityChannel = $state<CommunityChannel>('general');
  let communityMessage = $state('');
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

  const humanizeId = (id: string): string => id.split(/[_:.-]+/).filter(Boolean)
    .map((part) => (part[0]?.toUpperCase() ?? '') + part.slice(1))
    .join(' ');

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
      onaction(structured, target ? { target } : undefined, action.command);
      return;
    }
    if (verb === 'quest' && tail.length >= 2) {
      onaction(`quest.${tail[0]}`, { target: tail.slice(1).join(' ') }, action.command);
      return;
    }
    if (verb === 'party' && tail.length >= 1) {
      const [partyAction = '', ...partyTail] = tail;
      const argument = partyTail.join(' ');
      onaction(
        `party.${partyAction}`,
        argument ? (partyAction === 'say' ? { message: argument } : { target: argument }) : undefined,
        action.command,
      );
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

  function sendReply(): void {
    const message = replyMessage.trim();
    if (!message) return;
    onaction('social.reply', { message }, 'reply');
    replyMessage = '';
  }

  function channelAction(action: 'join' | 'leave'): void {
    const actionName = action === 'join' ? 'social.channel.join' : 'social.channel.leave';
    onaction(actionName, { channel: communityChannel }, `${action} #${communityChannel}`);
  }

  function sendChannelMessage(): void {
    const message = communityMessage.trim();
    if (!message) return;
    onaction(
      'social.channel.say',
      { channel: communityChannel, message },
      `#${communityChannel} message`,
    );
    communityMessage = '';
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
          {value.character.lineage.label} · {value.character.activeClass.label} · Level {value.character.level}
          {#if value.character.nextLevelExperience > 0}
            · {value.character.experience} / {value.character.nextLevelExperience} XP
          {:else}
            · {value.character.experience} XP · maximum level
          {/if}
        </p>
        <p class="character-origin">
          {value.character.background?.label ?? 'No background'}
          {#if value.character.baseClass.id !== value.character.activeClass.id}
            · began as {value.character.baseClass.label}
          {/if}
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

    <details class="dashboard-panel progression-panel" open>
      <summary>Progression <span>{value.character.talents.length} paths</span></summary>
      <div class="panel-body">
        <section aria-labelledby="class-history-title">
          <h3 id="class-history-title">Class evolution</h3>
          <ol class="class-history">
            {#each value.character.classHistory as characterClass (characterClass.id)}
              <li class:current={characterClass.id === value.character.activeClass.id}>
                <strong>{characterClass.label}</strong>
                {#if characterClass.id === value.character.activeClass.id}<span>Current</span>{/if}
                <p>{characterClass.description}</p>
              </li>
            {/each}
          </ol>
        </section>

        {#if Object.keys(value.character.progressionPoints).length > 0}
          <dl class="progression-points" aria-label="Available progression points">
            {#each Object.entries(value.character.progressionPoints) as [poolId, amount] (poolId)}
              <div><dt>{humanizeId(poolId)}</dt><dd>{amount}</dd></div>
            {/each}
          </dl>
        {/if}

        {#if value.character.promotions.length > 0}
          <section aria-labelledby="promotions-title">
            <h3 id="promotions-title">Advanced classes</h3>
            <ul class="progression-list promotion-list">
              {#each value.character.promotions as promotion (promotion.class.id)}
                <li>
                  <div>
                    <h4 id={`promotion-${promotion.class.id}`}>{promotion.class.label}</h4>
                    <p>{promotion.class.description}</p>
                    {#if promotion.requirements.length > 0}
                      <ul class="requirement-list" aria-label={`${promotion.class.label} requirements`}>
                        {#each promotion.requirements as requirement, index (`${requirement.text}-${index}`)}
                          <li class:met={requirement.met}>
                            <span aria-hidden="true">{requirement.met ? '✓' : '○'}</span>
                            {requirement.text}
                          </li>
                        {/each}
                      </ul>
                    {/if}
                  </div>
                  <button
                    type="button"
                    disabled={!promotion.available}
                    aria-describedby={`promotion-${promotion.class.id}`}
                    onclick={() => onpromote(
                      promotion.class.id,
                      promotion.command || `promote ${promotion.class.id}`,
                    )}
                  >Promote to {promotion.class.label}</button>
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        {#if value.character.talents.length > 0}
          <section aria-labelledby="talent-paths-title">
            <h3 id="talent-paths-title">Talent paths</h3>
            {#each value.character.talents as graph (graph.id)}
              <article class="talent-graph" aria-labelledby={`talent-graph-${graph.id}`}>
                <div class="talent-graph-heading">
                  <div>
                    <h4 id={`talent-graph-${graph.id}`}>{graph.label}</h4>
                    <p>{graph.description}</p>
                  </div>
                  <strong>{graph.availablePoints} points</strong>
                </div>
                <ul class="progression-list talent-list">
                  {#each graph.nodes as node (node.id)}
                    <li>
                      <div>
                        <h5 id={`talent-${graph.id}-${node.id}`}>{node.label}</h5>
                        <p>{node.description}</p>
                        <p class="talent-rank">Rank {node.rank} / {node.maxRank}{#if node.nextCost !== undefined} · next costs {node.nextCost}{/if}</p>
                        {#if node.requirements.length > 0}
                          <ul class="requirement-list" aria-label={`${node.label} requirements`}>
                            {#each node.requirements as requirement, index (`${requirement.text}-${index}`)}
                              <li class:met={requirement.met}>
                                <span aria-hidden="true">{requirement.met ? '✓' : '○'}</span>
                                {requirement.text}
                              </li>
                            {/each}
                          </ul>
                        {/if}
                      </div>
                      <button
                        type="button"
                        disabled={!node.available || node.nextCost === undefined}
                        aria-describedby={`talent-${graph.id}-${node.id}`}
                        onclick={() => ontalent(
                          graph.id,
                          node.id,
                          node.command || `talent learn ${graph.id} ${node.id}`,
                        )}
                      >{node.rank === 0 ? 'Learn' : 'Improve'} {node.label}</button>
                    </li>
                  {/each}
                </ul>
              </article>
            {/each}
          </section>
        {/if}
      </div>
    </details>

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
      <summary>
        Inventory
        <span>
          {value.inventoryCapacity.usedSlots}/{value.inventoryCapacity.maxSlots} slots ·
          {(value.inventoryCapacity.usedWeightGrams / 1000).toFixed(1)}/
          {(value.inventoryCapacity.maxWeightGrams / 1000).toFixed(1)} kg
        </span>
      </summary>
      <div class="panel-body">
        {#if value.inventory.length === 0 && value.equipment.length === 0}
          <p class="empty-copy">You are carrying nothing.</p>
        {/if}
        {#if value.inventory.length > 0}
          <h3>Carried</h3>
          <ul class="dashboard-list inventory-list">
            {#each value.inventory as item (item.id)}
              <li>
                <div>
                  <strong>{item.label}</strong>
                  <p>{item.rarity} · level {item.itemLevel} · {(item.weightGrams / 1000).toFixed(1)} kg</p>
                  <p>{item.description}</p>
                </div>
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
                <span>
                  <small>{item.slot}</small> <strong>{item.label}</strong>
                  · {item.rarity} · level {item.itemLevel} · {(item.weightGrams / 1000).toFixed(1)} kg
                </span>
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
            {#if !value.party.actions.some((action) => action.command.trim().toLowerCase() === 'party disband')}
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
                  <button type="button" onclick={() => onaction('social.ignore', { target: adventurer.name }, `ignore ${adventurer.name}`)}>Ignore</button>
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
        <form class="inline-form" onsubmit={(event) => { event.preventDefault(); sendReply(); }}>
          <label for="reply-text">Reply to your latest direct message</label>
          <div><input id="reply-text" maxlength="512" bind:value={replyMessage} /><button type="submit" disabled={!replyMessage.trim()}>Reply</button></div>
        </form>
        <section class="channel-controls" aria-labelledby="channel-controls-title">
          <div class="panel-heading-row">
            <h3 id="channel-controls-title">World channels</h3>
            <button type="button" onclick={() => onaction('social.channel.list', undefined, 'list channels')}>Refresh channels</button>
          </div>
          <div class="inline-form">
            <label for="community-channel">Channel</label>
            <select id="community-channel" bind:value={communityChannel}>
              <option value="general">General</option>
              <option value="newbie">Newbie</option>
              <option value="staff">Staff (moderators only)</option>
            </select>
            <div class="dashboard-actions">
              <button type="button" onclick={() => channelAction('join')}>Join channel</button>
              <button type="button" onclick={() => channelAction('leave')}>Leave channel</button>
            </div>
          </div>
          <form class="inline-form" onsubmit={(event) => { event.preventDefault(); sendChannelMessage(); }}>
            <label for="channel-message">Message to #{communityChannel}</label>
            <div><input id="channel-message" maxlength="512" bind:value={communityMessage} /><button type="submit" disabled={!communityMessage.trim()}>Send to channel</button></div>
          </form>
        </section>
        <form class="inline-form" onsubmit={(event) => event.preventDefault()}>
          <label for="ignore-target">Manage ignore list</label>
          <div>
            <input id="ignore-target" maxlength="64" bind:value={socialTarget} />
            <button type="button" disabled={!socialTarget.trim()} onclick={() => onaction('social.ignore', { target: socialTarget.trim() }, `ignore ${socialTarget.trim()}`)}>Ignore</button>
            <button type="button" disabled={!socialTarget.trim()} onclick={() => onaction('social.unignore', { target: socialTarget.trim() }, `unignore ${socialTarget.trim()}`)}>Unignore</button>
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
