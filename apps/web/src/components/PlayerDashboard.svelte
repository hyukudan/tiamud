<script lang="ts">
  import type {
    ActionPerformFields,
    ActionResultData,
    BoardCatalogData,
    BoardEntryData,
    BoardKey,
    BoardListData,
    CommunityChannel,
    ConnectionState,
    ContainerContentsData,
    ContainerItemSnapshot,
    MailListData,
    MailMessageData,
    PlayerItemSnapshot,
    PlayerStateSnapshot,
    RewardOddsData,
    SceneAction,
    TradeContentsData,
  } from '../lib/protocol';

  interface Props {
    value?: PlayerStateSnapshot;
    containerContents?: ContainerContentsData;
    tradeContents?: TradeContentsData;
    rewardOdds?: RewardOddsData;
    mailList?: MailListData;
    mailMessage?: MailMessageData;
    boardCatalog?: BoardCatalogData;
    boardList?: BoardListData;
    boardEntry?: BoardEntryData;
    actionResult?: ActionResultData;
    connectionState?: ConnectionState;
    abilityTargets?: Array<{ id: string; label: string }>;
    oncommand: (command: string) => void;
    onaction: (
      action: string,
      fields: ActionPerformFields | undefined,
      label: string,
    ) => string | undefined;
    onability: (abilityId: string, target: string | undefined, label: string) => void;
    onpromote: (classId: string, label: string) => void;
    ontalent: (graphId: string, nodeId: string, label: string) => void;
  }

  let {
    value, containerContents, tradeContents, rewardOdds, mailList, mailMessage, boardCatalog,
    boardList, boardEntry, actionResult, connectionState = 'connecting', abilityTargets = [],
    oncommand, onaction, onability, onpromote, ontalent,
  }: Props = $props();
  let socialTarget = $state('');
  let chatText = $state('');
  let partyTarget = $state('');
  let partyMessage = $state('');
  let replyMessage = $state('');
  let communityChannel = $state<CommunityChannel>('general');
  let communityMessage = $state('');
  let abilityTargetById = $state<Record<string, string>>({});
  let telepathyTargetById = $state<Record<string, string>>({});
  let telepathyMessageById = $state<Record<string, string>>({});
  let emoteCommand = $state('');
  let emoteTarget = $state('');
  let mailTarget = $state('');
  let mailSubject = $state('');
  let mailBody = $state('');
  let mailReportReason = $state('');
  let boardSelection = $state<BoardKey>('general');
  let boardSubject = $state('');
  let boardBody = $state('');
  let boardReportReason = $state('');
  let pendingBoardPostRequest = $state<string>();
  let pendingBoardReportRequest = $state<string>();
  let itemQuantity = $state(1);
  let destinationContainer = $state('');
  let giveRecipient = $state('');
  let tradeRecipient = $state('');
  let tradeCoins = $state(0);
  let tradeOfferQuantity = $state(1);
  let lastTradeRevision = $state(0);
  const carriedContainers = $derived(value?.inventory.filter((item) => item.containerCapacity) ?? []);
  const nearbyRecipients = $derived(
    value?.social.online.filter((character) => character.nearby && !character.selfCharacter) ?? [],
  );
  const telepathyRecipients = $derived(
    value?.social.online.filter((character) => !character.selfCharacter) ?? [],
  );
  const giveRecipientIsNearby = $derived(
    nearbyRecipients.some((character) => character.id === giveRecipient),
  );
  const tradeRecipientIsNearby = $derived(
    nearbyRecipients.some((character) => character.id === tradeRecipient),
  );
  const tradeCounterpartyName = $derived(
    value?.trade
      ? (value.social.online.find((character) => character.id === value?.trade?.counterparty)?.name ??
        'Nearby adventurer')
      : '',
  );
  const additionalAttributes = $derived(
    value?.character.attributes.filter((attribute) => attribute.id !== 'strength' && attribute.id !== 'defence') ?? [],
  );
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
    if (verb.toLowerCase() === 'open' && tail.length === 1) {
      onaction('reward.open', { target: tail[0] }, action.command);
      return;
    }
    if (verb.toLowerCase() === 'reward' && tail[0]?.toLowerCase() === 'odds' && tail[1]) {
      const page = Number.parseInt(tail[2] ?? '1', 10);
      onaction(
        'reward.odds',
        { target: tail[1], page: Number.isSafeInteger(page) && page > 0 ? page : 1 },
        action.command,
      );
      return;
    }
    const structured = ({
      examine: 'item.examine',
      drop: 'item.drop',
      use: 'item.use',
      equip: 'item.equip',
      unequip: 'item.unequip',
      contents: 'item.contents',
      buy: 'economy.buy',
      sell: 'economy.sell',
    } as Record<string, string>)[verb.toLowerCase()];
    if (structured) {
      onaction(
        structured,
        structured === 'item.contents'
          ? (target ? { container: target, page: 1 } : undefined)
          : (target ? { target } : undefined),
        action.command,
      );
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

  function boundedItemQuantity(maximum: number): number {
    return Math.max(1, Math.min(256, maximum, Math.trunc(itemQuantity) || 1));
  }

  function boundedTradeQuantity(maximum: number): number {
    return Math.max(1, Math.min(128, maximum, Math.trunc(tradeOfferQuantity) || 1));
  }

  $effect(() => {
    const trade = value?.trade;
    if (trade && trade.stateRevision !== lastTradeRevision) {
      lastTradeRevision = trade.stateRevision;
      tradeCoins = trade.ownOffer.coins;
    } else if (!trade) {
      lastTradeRevision = 0;
      tradeCoins = 0;
    }
  });

  function requestTrade(): void {
    if (!tradeRecipientIsNearby) return;
    const recipient = nearbyRecipients.find((character) => character.id === tradeRecipient);
    if (!recipient) return;
    onaction('trade.request', { recipient: recipient.id }, `trade request ${recipient.name}`);
  }

  function tradeAction(action: 'accept' | 'decline' | 'status' | 'cancel'): void {
    const trade = value?.trade;
    if (!trade) return;
    onaction(`trade.${action}`, { tradeId: trade.id }, `trade ${action}`);
  }

  function setTradeCoins(): void {
    const trade = value?.trade;
    if (!trade) return;
    const coins = Math.max(0, Math.min(
      1_000_000_000,
      value?.economy.coins ?? 0,
      Math.trunc(tradeCoins) || 0,
    ));
    tradeCoins = coins;
    onaction('trade.coins.set', { tradeId: trade.id, coins }, `offer ${coins} coins`);
  }

  function offerTradeItem(item: PlayerItemSnapshot): void {
    const trade = value?.trade;
    if (!trade || !item.canTrade || item.availableQuantity === 0) return;
    const quantity = boundedTradeQuantity(item.availableQuantity);
    onaction(
      'trade.item.set',
      { tradeId: trade.id, target: item.id, quantity },
      `offer ${quantity} ${item.label}`,
    );
  }

  function removeTradeItem(target: string, label: string): void {
    const trade = value?.trade;
    if (!trade) return;
    onaction('trade.item.remove', { tradeId: trade.id, target }, `remove ${label} from trade`);
  }

  function inspectTradeContainer(target: string, page = 1): void {
    const trade = value?.trade;
    if (!trade) return;
    onaction(
      'trade.contents',
      { tradeId: trade.id, target, page },
      `trade contents page ${page}`,
    );
  }

  function confirmTrade(): void {
    const trade = value?.trade;
    if (!trade || !trade.canConfirm) return;
    onaction(
      'trade.confirm',
      { tradeId: trade.id, stateRevision: trade.stateRevision },
      `trade confirm revision ${trade.stateRevision}`,
    );
  }

  function putItem(item: PlayerItemSnapshot): void {
    if (!destinationContainer || destinationContainer === item.id) return;
    const quantity = boundedItemQuantity(item.quantity);
    onaction(
      'item.put',
      { target: item.id, container: destinationContainer, quantity },
      `put ${quantity} ${item.label}`,
    );
  }

  function getItem(item: ContainerItemSnapshot): void {
    if (!containerContents) return;
    const quantity = boundedItemQuantity(item.quantity);
    onaction(
      'item.get',
      { target: item.id, container: containerContents.container, quantity },
      `get ${quantity} ${item.label}`,
    );
  }

  function giveItem(item: PlayerItemSnapshot): void {
    if (!giveRecipient) return;
    const recipient = nearbyRecipients.find((character) => character.id === giveRecipient);
    if (!recipient) return;
    const quantity = boundedItemQuantity(item.quantity);
    onaction(
      'item.give',
      { target: item.id, recipient: recipient.id, quantity },
      `give ${quantity} ${item.label} to ${recipient.name}`,
    );
  }

  const transferPolicyLabel = (
    item: Pick<PlayerItemSnapshot, 'transferPolicy' | 'boundToYou'>,
  ): string => {
    if (item.transferPolicy === 'transferable') return 'Transferable';
    if (item.transferPolicy === 'non_transferable') return 'Non-transferable';
    return item.boundToYou ? 'Bound to this character' : 'Binds on first acquisition';
  };

  function openContainer(container: string, page = 1): void {
    onaction('item.contents', { container, page }, `contents page ${page}`);
  }

  function requestRewardOdds(selector: string, page: number): void {
    onaction('reward.odds', { target: selector, page }, `reward odds ${selector} ${page}`);
  }

  function exactProbability(numerator: number, denominator: number): string {
    return `${numerator}/${denominator} (${((numerator * 100) / denominator).toFixed(2)}%)`;
  }

  function signedDelta(value: number): string {
    return `${value >= 0 ? '+' : ''}${value}`;
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

  const targetingLabel = (
    ability: PlayerStateSnapshot['character']['abilities'][number],
  ): string => ability.purpose === 'telepathy' ? 'Direct online message' : ({
      self: 'Self',
      // The current scene contract has no authorized ally target IDs. The wire
      // value stays additive for a future client, but v1 executes it as self.
      optionalTarget: 'Self',
      explicitTarget: 'Choose a target',
    })[ability.targeting];

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

  function sendTelepathy(ability: PlayerStateSnapshot['character']['abilities'][number]): void {
    const target = (telepathyTargetById[ability.id] ?? '').trim();
    const message = (telepathyMessageById[ability.id] ?? '').trim();
    if (!target || !message) return;
    onaction(
      'social.telepathy',
      { abilityId: ability.id, target, message },
      `telepathy with ${ability.name} to ${target}`,
    );
    telepathyMessageById = { ...telepathyMessageById, [ability.id]: '' };
  }

  function sendEmote(): void {
    const emote = emoteCommand.trim();
    const target = emoteTarget.trim();
    if (!/^[a-z][a-z0-9-]{0,31}$/.test(emote)) return;
    onaction(
      'social.emote',
      target ? { emote, target } : { emote },
      target ? `emote ${emote} to ${target}` : `emote ${emote}`,
    );
    emoteTarget = '';
  }

  const mailVisibleText = (value: string, maximum: number): boolean => {
    const text = value.trim();
    return text === value && text.length > 0 && new TextEncoder().encode(text).length <= maximum &&
      !/[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/.test(text);
  };

  function sendMail(): void {
    if (!mailVisibleText(mailTarget, 64) || mailTarget.includes('|') ||
      !mailVisibleText(mailSubject, 128) || mailSubject.includes('|') ||
      !mailVisibleText(mailBody, 2_048)) return;
    onaction(
      'mail.send',
      { target: mailTarget, subject: mailSubject, body: mailBody },
      `mail to ${mailTarget}`,
    );
    mailSubject = '';
    mailBody = '';
  }

  function reportOpenMail(): void {
    if (!mailMessage || !mailVisibleText(mailReportReason, 2_048)) return;
    onaction(
      'mail.report',
      { mailId: mailMessage.id, reason: mailReportReason },
      `report mail ${mailMessage.id}`,
    );
    mailReportReason = '';
  }

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

  const mailDate = (unixSeconds: number): string => new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(unixSeconds * 1_000));
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
                  <span>{targetingLabel(ability)}</span>
                </div>
                <p id={`ability-description-${ability.id}`}>{ability.description}</p>
                <ul class="ability-meta" aria-label={`${ability.name} requirements`}>
                  {#each ability.resourceCosts as cost (`${cost.resourceId}-${cost.amount}`)}
                    <li>{resourceCostLabel(cost)}</li>
                  {/each}
                  {#if ability.cooldownRemaining > 0}<li>{ability.cooldownRemaining} ticks cooldown</li>{/if}
                </ul>
                {#if ability.purpose === 'telepathy'}
                  <form
                    class="inline-form telepathy-form"
                    onsubmit={(event) => { event.preventDefault(); sendTelepathy(ability); }}
                  >
                    <label for={`telepathy-target-${ability.id}`}>Mind to reach with {ability.name}</label>
                    <input
                      id={`telepathy-target-${ability.id}`}
                      list={`telepathy-recipients-${ability.id}`}
                      maxlength="64"
                      value={telepathyTargetById[ability.id] ?? ''}
                      oninput={(event) => {
                        telepathyTargetById = {
                          ...telepathyTargetById,
                          [ability.id]: event.currentTarget.value,
                        };
                      }}
                    />
                    <datalist id={`telepathy-recipients-${ability.id}`}>
                      {#each telepathyRecipients as recipient (recipient.id)}
                        <option value={recipient.name}></option>
                      {/each}
                    </datalist>
                    <label for={`telepathy-message-${ability.id}`}>Private thought</label>
                    <input
                      id={`telepathy-message-${ability.id}`}
                      maxlength="512"
                      value={telepathyMessageById[ability.id] ?? ''}
                      oninput={(event) => {
                        telepathyMessageById = {
                          ...telepathyMessageById,
                          [ability.id]: event.currentTarget.value,
                        };
                      }}
                    />
                    <button
                      type="submit"
                      aria-describedby={`ability-description-${ability.id}`}
                      disabled={!ability.usable || ability.cooldownRemaining > 0 ||
                        !(telepathyTargetById[ability.id] ?? '').trim() ||
                        !(telepathyMessageById[ability.id] ?? '').trim()}
                    >Reach with {ability.name}</button>
                  </form>
                {:else if ability.targeting === 'explicitTarget'}
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
                {#if ability.purpose === 'gameplay'}
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
                {:else if !ability.usable}
                  <p class="ability-unavailable" role="status">Telepathy is unavailable right now.</p>
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
        <div class="field-grid inventory-transfer-controls">
          <label>
            Quantity (maximum 256)
            <input type="number" min="1" max="256" step="1" bind:value={itemQuantity} />
          </label>
          {#if value.trade?.phase === 'negotiating'}
            <label>
              Trade quantity (maximum 128)
              <input type="number" min="1" max="128" step="1" bind:value={tradeOfferQuantity} />
            </label>
          {/if}
          {#if carriedContainers.length > 0}
            <label>
              Destination container
              <select bind:value={destinationContainer}>
                <option value="">Choose a container</option>
                {#each carriedContainers as container (container.id)}
                  <option value={container.id}>{container.label}</option>
                {/each}
              </select>
            </label>
          {/if}
          {#if nearbyRecipients.length > 0}
            <label>
              Nearby recipient
              <select bind:value={giveRecipient}>
                <option value="">Choose a character</option>
                {#each nearbyRecipients as character (character.id)}
                  <option value={character.id}>{character.name}</option>
                {/each}
              </select>
            </label>
          {/if}
        </div>
        {#if value.inventory.length === 0 && value.equipment.length === 0}
          <p class="empty-copy">You are carrying nothing.</p>
        {/if}
        {#if value.inventory.length > 0}
          <h3>Carried</h3>
          <ul class="dashboard-list inventory-list">
            {#each value.inventory as item (item.id)}
              <li>
                <div>
                  <strong>{item.label} ×{item.quantity}</strong>
                  <p>
                    Available {item.availableQuantity} · reserved for trade {item.reservedForTrade}
                  </p>
                  <p>
                    {item.rarity} · level {item.itemLevel} ·
                    {transferPolicyLabel(item)} ·
                    {(item.weightGrams / 1000).toFixed(1)} kg each ·
                    {(item.totalWeightGrams / 1000).toFixed(1)} kg total ·
                    {item.stackSlots} {item.stackSlots === 1 ? 'stack slot' : 'stack slots'}
                  </p>
                  {#if item.containerCapacity}
                    <p>
                      Container: {item.containerCapacity.usedSlots}/{item.containerCapacity.maxSlots} slots ·
                      {(item.containerCapacity.usedWeightGrams / 1000).toFixed(1)}/
                      {(item.containerCapacity.maxWeightGrams / 1000).toFixed(1)} kg
                    </p>
                  {/if}
                  <p>{item.description}</p>
                  {#if item.affix}
                    <p>
                      Modifier: {item.affix.label} ({item.affix.position}) ·
                      attack {signedDelta(item.affix.attackBonusDelta)} ·
                      healing {signedDelta(item.affix.healAmountDelta)}
                    </p>
                  {/if}
                </div>
                <div class="dashboard-actions">
                  {#each item.actions as action (action.command)}
                    <button type="button" onclick={() => perform(action)}>{action.label}</button>
                  {/each}
                  {#if item.canGive}
                    <button
                      type="button"
                      disabled={!giveRecipientIsNearby}
                      onclick={() => giveItem(item)}
                    >Give</button>
                  {/if}
                  {#if value.trade?.phase === 'negotiating' && item.canTrade}
                    <button type="button" onclick={() => offerTradeItem(item)}>Offer in trade</button>
                  {/if}
                  {#if item.containerCapacity && !item.actions.some((action) => action.command.startsWith('contents '))}
                    <button type="button" onclick={() => openContainer(item.id)}>Contents</button>
                  {/if}
                  {#if carriedContainers.length > 0}
                    <button
                      type="button"
                      disabled={!destinationContainer || destinationContainer === item.id}
                      onclick={() => putItem(item)}
                    >Put</button>
                  {/if}
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
                  <small>{item.slot}</small> <strong>{item.label} ×{item.quantity}</strong>
                  · {item.rarity} · level {item.itemLevel} ·
                  {transferPolicyLabel(item)} ·
                  {(item.weightGrams / 1000).toFixed(1)} kg each ·
                  {(item.totalWeightGrams / 1000).toFixed(1)} kg total ·
                  {item.stackSlots} {item.stackSlots === 1 ? 'stack slot' : 'stack slots'}
                </span>
                {#if item.containerCapacity}
                  <p>
                    Container: {item.containerCapacity.usedSlots}/{item.containerCapacity.maxSlots} slots ·
                    {(item.containerCapacity.usedWeightGrams / 1000).toFixed(1)}/
                    {(item.containerCapacity.maxWeightGrams / 1000).toFixed(1)} kg
                  </p>
                {/if}
                {#if item.affix}
                  <p>
                    Modifier: {item.affix.label} ({item.affix.position}) ·
                    attack {signedDelta(item.affix.attackBonusDelta)} ·
                    healing {signedDelta(item.affix.healAmountDelta)}
                  </p>
                {/if}
                <div class="dashboard-actions">
                  {#each item.actions as action (action.command)}
                    <button type="button" onclick={() => perform(action)}>{action.label}</button>
                  {/each}
                  {#if item.containerCapacity}
                    <button type="button" onclick={() => openContainer(item.id)}>Contents</button>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
        {#if containerContents}
          <section aria-labelledby="container-contents-title">
            <h3 id="container-contents-title">Container contents</h3>
            <p>
              Page {containerContents.page} of {containerContents.totalPages} ·
              {containerContents.capacity.usedSlots}/{containerContents.capacity.maxSlots} slots ·
              {(containerContents.capacity.usedWeightGrams / 1000).toFixed(1)}/
              {(containerContents.capacity.maxWeightGrams / 1000).toFixed(1)} kg
            </p>
            {#if containerContents.items.length === 0}
              <p class="empty-copy">This container is empty.</p>
            {:else}
              <ul class="dashboard-list inventory-list">
                {#each containerContents.items as item (item.id)}
                  <li>
                    <span>
                      <strong>{item.label} ×{item.quantity}</strong>
                      · {item.rarity} · level {item.itemLevel} · {transferPolicyLabel(item)} ·
                      {(item.weightGrams / 1000).toFixed(1)} kg each ·
                      {(item.totalWeightGrams / 1000).toFixed(1)} kg total ·
                      {item.stackSlots} {item.stackSlots === 1 ? 'stack slot' : 'stack slots'}
                    </span>
                    {#if item.affix}
                      <p>
                        Modifier: {item.affix.label} ({item.affix.position}) ·
                        attack {signedDelta(item.affix.attackBonusDelta)} ·
                        healing {signedDelta(item.affix.healAmountDelta)}
                      </p>
                    {/if}
                    <div class="dashboard-actions">
                      <button type="button" onclick={() => getItem(item)}>Get</button>
                      {#if item.containerCapacity}
                        <button type="button" onclick={() => openContainer(item.id)}>Open</button>
                      {/if}
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
            <div class="dashboard-actions" aria-label="Container pages">
              <button
                type="button"
                disabled={containerContents.page <= 1}
                onclick={() => openContainer(containerContents.container, containerContents.page - 1)}
              >Previous</button>
              <button
                type="button"
                disabled={containerContents.page >= containerContents.totalPages}
                onclick={() => openContainer(containerContents.container, containerContents.page + 1)}
              >Next</button>
            </div>
          </section>
        {/if}
        {#if rewardOdds}
          <section aria-labelledby="reward-odds-title">
            <h3 id="reward-odds-title">Sealed reward odds</h3>
            <p>
              Page {rewardOdds.page} of {rewardOdds.totalPages}. Fractions below are exact;
              percentages are display-only.
            </p>
            {#if rewardOdds.guaranteed.length > 0}
              <h4>Guaranteed</h4>
              <ul class="compact-list">
                {#each rewardOdds.guaranteed as row, index (`${row.label}-${index}`)}
                  <li>
                    <strong>{row.label}</strong> ×{row.quantity}
                    {#if row.affixPoolLabel} · modifier draw: {row.affixPoolLabel}{/if}
                  </li>
                {/each}
              </ul>
            {/if}
            {#if rewardOdds.weighted.length > 0}
              <h4>Weighted outcomes</h4>
              <ul class="compact-list">
                {#each rewardOdds.weighted as row, index (`${row.poolLabel}-${row.outcomeLabel}-${index}`)}
                  <li>
                    <strong>{row.outcomeLabel}</strong> · {row.poolLabel} · {row.rolls}
                    {row.rolls === 1 ? 'draw' : 'draws'} · quantity {row.minQuantity}–{row.maxQuantity} ·
                    exact chance {exactProbability(row.probabilityNumerator, row.probabilityDenominator)}
                    {#if row.affixPoolLabel} · modifier draw: {row.affixPoolLabel}{/if}
                  </li>
                {/each}
              </ul>
            {/if}
            {#if rewardOdds.affixes.length > 0}
              <h4>Conditional modifiers</h4>
              <ul class="compact-list">
                {#each rewardOdds.affixes as row, index (`${row.parentOutcomeLabel}-${row.affixLabel}-${index}`)}
                  <li>
                    <strong>{row.parentOutcomeLabel}</strong> → {row.affixLabel}
                    {#if row.position} ({row.position}){/if} ·
                    attack {signedDelta(row.attackBonusDelta)} ·
                    healing {signedDelta(row.healAmountDelta)} ·
                    exact conditional chance
                    {exactProbability(
                      row.conditionalProbabilityNumerator,
                      row.conditionalProbabilityDenominator,
                    )}
                  </li>
                {/each}
              </ul>
            {/if}
            <div class="dashboard-actions" aria-label="Sealed reward odds pages">
              <button
                type="button"
                disabled={rewardOdds.page <= 1}
                onclick={() => requestRewardOdds(rewardOdds.selector, rewardOdds.page - 1)}
              >Previous</button>
              <button
                type="button"
                disabled={rewardOdds.page >= rewardOdds.totalPages}
                onclick={() => requestRewardOdds(rewardOdds.selector, rewardOdds.page + 1)}
              >Next</button>
            </div>
          </section>
        {/if}
      </div>
    </details>

    <details class="dashboard-panel" open={Boolean(value.trade)}>
      <summary>
        Confirmed trade
        <span>{value.trade ? `${value.trade.expiresInTicks} ticks left` : 'No active trade'}</span>
      </summary>
      <div class="panel-body">
        {#if !value.trade}
          <p class="empty-copy">
            Start a private exchange with one nearby adventurer. Nothing moves until both sides
            confirm the same revision.
          </p>
          {#if nearbyRecipients.length > 0}
            <label for="trade-recipient">Nearby adventurer</label>
            <select id="trade-recipient" bind:value={tradeRecipient}>
              <option value="">Choose a character</option>
              {#each nearbyRecipients as character (character.id)}
                <option value={character.id}>{character.name}</option>
              {/each}
            </select>
            <button type="button" disabled={!tradeRecipientIsNearby} onclick={requestTrade}>
              Request trade
            </button>
          {:else}
            <p>No other connected character is nearby.</p>
          {/if}
        {:else if value.trade.phase === 'requested'}
          <h3>Trade request with {tradeCounterpartyName}</h3>
          <p role="status">This request expires in {value.trade.expiresInTicks} world ticks.</p>
          <div class="dashboard-actions">
            {#if value.trade.canAccept}
              <button type="button" onclick={() => tradeAction('accept')}>Accept and negotiate</button>
              <button type="button" onclick={() => tradeAction('decline')}>Decline</button>
            {:else}
              <button type="button" onclick={() => tradeAction('cancel')}>Cancel request</button>
            {/if}
            <button type="button" onclick={() => tradeAction('status')}>Refresh status</button>
          </div>
        {:else}
          <div class="panel-heading-row">
            <div>
              <h3>Exchange with {tradeCounterpartyName}</h3>
              <p>
                Revision {value.trade.stateRevision} · {value.trade.expiresInTicks} ticks left
              </p>
            </div>
            <button type="button" onclick={() => tradeAction('status')}>Refresh status</button>
          </div>

          <div class="field-grid">
            <section aria-labelledby="own-trade-offer">
              <h4 id="own-trade-offer">Your offer</h4>
              <p>
                {value.trade.ownOffer.coins} coins ·
                {value.trade.ownConfirmed ? 'confirmed' : 'not confirmed'}
              </p>
              {#if value.trade.ownOffer.lines.length === 0}
                <p class="empty-copy">No items offered.</p>
              {:else}
                <ul class="dashboard-list">
                  {#each value.trade.ownOffer.lines as line (line.target)}
                    <li>
                      <span><strong>{line.label}</strong> ×{line.quantity}</span>
                      <div class="dashboard-actions">
                        {#if line.container}
                          <button type="button" onclick={() => inspectTradeContainer(line.target)}>
                            Inspect contents
                          </button>
                        {/if}
                        <button type="button" onclick={() => removeTradeItem(line.target, line.label)}>
                          Remove
                        </button>
                      </div>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
            <section aria-labelledby="counterparty-trade-offer">
              <h4 id="counterparty-trade-offer">{tradeCounterpartyName}'s offer</h4>
              <p>
                {value.trade.counterpartyOffer.coins} coins ·
                {value.trade.counterpartyConfirmed ? 'confirmed' : 'not confirmed'}
              </p>
              {#if value.trade.counterpartyOffer.lines.length === 0}
                <p class="empty-copy">No items offered.</p>
              {:else}
                <ul class="dashboard-list">
                  {#each value.trade.counterpartyOffer.lines as line (line.target)}
                    <li>
                      <span><strong>{line.label}</strong> ×{line.quantity}</span>
                      {#if line.container}
                        <div class="dashboard-actions">
                          <button type="button" onclick={() => inspectTradeContainer(line.target)}>
                            Inspect offered contents
                          </button>
                        </div>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          </div>

          <form class="inline-form" onsubmit={(event) => { event.preventDefault(); setTradeCoins(); }}>
            <label for="trade-coins">
              Coins to offer (spendable now: {value.economy.spendableCoins})
            </label>
            <div>
              <input
                id="trade-coins"
                type="number"
                min="0"
                max={Math.min(1_000_000_000, value.economy.coins)}
                step="1"
                bind:value={tradeCoins}
              />
              <button type="submit">Set coin offer</button>
            </div>
          </form>

          <aside class="notice" aria-labelledby="trade-confirmation-warning">
            <h4 id="trade-confirmation-warning">Review before confirming</h4>
            <p>
              Confirming approves revision {value.trade.stateRevision} only. Any change to items,
              coins, inventory or capacity clears both confirmations. The exchange completes only
              after the other participant confirms that same revision.
            </p>
          </aside>
          <div class="dashboard-actions">
            <button
              type="button"
              disabled={!value.trade.canConfirm || value.trade.ownConfirmed}
              onclick={confirmTrade}
            >Confirm revision {value.trade.stateRevision}</button>
            <button type="button" onclick={() => tradeAction('cancel')}>Cancel trade</button>
          </div>

          {#if tradeContents && tradeContents.tradeId === value.trade.id}
            <section aria-labelledby="trade-contents-title">
              <h4 id="trade-contents-title">Offered container contents</h4>
              <p>
                Page {tradeContents.page} of {tradeContents.totalPages} ·
                {tradeContents.capacity.usedSlots}/{tradeContents.capacity.maxSlots} slots ·
                {(tradeContents.capacity.usedWeightGrams / 1000).toFixed(1)}/
                {(tradeContents.capacity.maxWeightGrams / 1000).toFixed(1)} kg
              </p>
              {#if tradeContents.items.length === 0}
                <p class="empty-copy">This offered container is empty.</p>
              {:else}
                <ul class="dashboard-list">
                  {#each tradeContents.items as item (item.id)}
                    <li>
                      <span>
                        <strong>{item.label}</strong> ×{item.quantity} · {item.rarity} ·
                        {(item.totalWeightGrams / 1000).toFixed(1)} kg total
                      </span>
                    </li>
                  {/each}
                </ul>
              {/if}
              <div class="dashboard-actions" aria-label="Offered container pages">
                <button
                  type="button"
                  disabled={tradeContents.page <= 1}
                  onclick={() => inspectTradeContainer(tradeContents.container, tradeContents.page - 1)}
                >Previous</button>
                <button
                  type="button"
                  disabled={tradeContents.page >= tradeContents.totalPages}
                  onclick={() => inspectTradeContainer(tradeContents.container, tradeContents.page + 1)}
                >Next</button>
              </div>
            </section>
          {/if}
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
              <button type="submit" disabled={!mailVisibleText(mailReportReason, 2_048)}>Send report</button>
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
            disabled={!mailVisibleText(mailTarget, 64) || mailTarget.includes('|') ||
              !mailVisibleText(mailSubject, 128) || mailSubject.includes('|') ||
              !mailVisibleText(mailBody, 2_048)}
          >Send mail</button>
        </form>
      </div>
    </details>

    <details class="dashboard-panel">
      <summary>Coin and market <span>{value.economy.spendableCoins} spendable</span></summary>
      <div class="panel-body">
        <p class="balance"><strong>{value.economy.coins}</strong> coin total</p>
        <p>
          {value.economy.reservedCoins} reserved for trade ·
          {value.economy.spendableCoins} spendable
        </p>
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
        <section class="channel-controls" aria-labelledby="emote-controls-title">
          <div class="panel-heading-row">
            <h3 id="emote-controls-title">Authored emotes</h3>
            <button type="button" onclick={() => onaction('social.emote.list', undefined, 'list emotes')}>List emotes</button>
          </div>
          <form class="inline-form" onsubmit={(event) => { event.preventDefault(); sendEmote(); }}>
            <label for="emote-command">Emote command</label>
            <input
              id="emote-command"
              maxlength="32"
              pattern="[a-z][a-z0-9-]*"
              bind:value={emoteCommand}
              placeholder="smile"
              aria-describedby="emote-help"
            />
            <label for="emote-target">Nearby character <span>(optional)</span></label>
            <select id="emote-target" bind:value={emoteTarget}>
              <option value="">No target</option>
              {#each nearbyRecipients as recipient (recipient.id)}
                <option value={recipient.name}>{recipient.name}</option>
              {/each}
            </select>
            <small id="emote-help">The world supplies the safe wording; emotes do not accept free text.</small>
            <button type="submit" disabled={!/^[a-z][a-z0-9-]{0,31}$/.test(emoteCommand.trim())}>Perform emote</button>
          </form>
        </section>
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
