//! Give recipients and confirmed two-party trades: reservation,
//! settlement, status and state-transition validation.

use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TradeId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradePhase {
    Requested,
    Negotiating,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TradeOfferKey {
    Fungible { prototype_id: String },
    Unique { item_id: ItemInstanceId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeOfferLine {
    pub key: TradeOfferKey,
    pub item_ids: Vec<ItemInstanceId>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeOffer {
    pub lines: Vec<TradeOfferLine>,
    pub coins: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeConfirmation {
    pub state_revision: u64,
    pub basis_sha256: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeSnapshot {
    pub id: TradeId,
    pub requester: CharacterId,
    pub recipient: CharacterId,
    pub room_id: String,
    pub phase: TradePhase,
    pub created_tick: u64,
    pub opened_tick: Option<u64>,
    pub expires_at_tick: u64,
    pub state_revision: u64,
    pub offers: BTreeMap<CharacterId, TradeOffer>,
    pub confirmations: BTreeMap<CharacterId, TradeConfirmation>,
}

impl TradeSnapshot {
    pub fn participants(&self) -> [CharacterId; 2] {
        [self.requester, self.recipient]
    }

    pub fn includes(&self, character_id: CharacterId) -> bool {
        self.requester == character_id || self.recipient == character_id
    }

    pub fn counterpart(&self, character_id: CharacterId) -> Option<CharacterId> {
        if self.requester == character_id {
            Some(self.recipient)
        } else if self.recipient == character_id {
            Some(self.requester)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeCancellationReason {
    Declined,
    CancelledByParticipant,
    Expired,
    ParticipantsSeparated,
    CombatStarted,
    ParticipantDefeated,
    ParticipantDisconnected,
    SocialPolicyChanged,
    InvalidReservation,
    CounterExhausted,
    Restart,
}

impl World {
    pub(crate) fn apply_trade(
        &mut self,
        session: SessionId,
        action: TradeAction,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.apply_trade_inner(session, action) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn apply_trade_inner(
        &mut self,
        session: SessionId,
        action: TradeAction,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let actor_id = character.id;
        match action {
            TradeAction::Request(query) => {
                if self.trades.len() >= MAX_LIVE_TRADES
                    || self.trade_for_character(actor_id).is_some()
                    || self
                        .last_trade_request_ticks
                        .get(&actor_id)
                        .is_some_and(|last_tick| {
                            last_tick
                                .checked_add(TRADE_REQUEST_RATE_TICKS)
                                .is_none_or(|next_tick| self.logical_tick < next_tick)
                        })
                {
                    return Err(WorldError::TradeUnavailable);
                }
                let recipient = self.resolve_trade_recipient(session, &query)?;
                if self.trade_for_character(recipient.character_id).is_some()
                    || self.character_in_combat(actor_id)
                    || self.character_in_combat(recipient.character_id)
                    || self.character_defeated(actor_id)
                    || self.character_defeated(recipient.character_id)
                {
                    return Err(WorldError::TradeUnavailable);
                }
                let room_id = room_checked(&self.entities, character.location)?
                    .stable_id
                    .clone();
                let id = self.next_trade_id;
                self.next_trade_id = TradeId(
                    id.0.checked_add(1)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                );
                let trade = TradeSnapshot {
                    id,
                    requester: actor_id,
                    recipient: recipient.character_id,
                    room_id,
                    phase: TradePhase::Requested,
                    created_tick: self.logical_tick,
                    opened_tick: None,
                    expires_at_tick: self
                        .logical_tick
                        .checked_add(TRADE_REQUEST_TICKS)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                    state_revision: 1,
                    offers: BTreeMap::new(),
                    confirmations: BTreeMap::new(),
                };
                self.last_trade_request_ticks
                    .insert(actor_id, self.logical_tick);
                self.trades.insert(id, trade.clone());
                self.finish_trade_action(
                    session,
                    format!("You ask {} to trade.\r\n", recipient.name),
                    vec![DomainEvent::TradeRequested { trade }],
                )
            }
            TradeAction::Accept => {
                let previous = self
                    .trade_for_character(actor_id)
                    .filter(|trade| {
                        trade.phase == TradePhase::Requested && trade.recipient == actor_id
                    })
                    .cloned()
                    .ok_or(WorldError::TradeUnavailable)?;
                self.ensure_trade_available(&previous)?;
                let mut current = previous.clone();
                current.phase = TradePhase::Negotiating;
                current.opened_tick = Some(self.logical_tick);
                current.expires_at_tick = self
                    .logical_tick
                    .checked_add(TRADE_NEGOTIATION_TICKS)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                current.state_revision = current
                    .state_revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                current.offers = BTreeMap::from([
                    (current.requester, TradeOffer::default()),
                    (current.recipient, TradeOffer::default()),
                ]);
                self.trades.insert(current.id, current.clone());
                self.finish_trade_action(
                    session,
                    "You accept the trade request.\r\n".into(),
                    vec![DomainEvent::TradeOpened { previous, current }],
                )
            }
            TradeAction::Decline => {
                let trade = self
                    .trade_for_character(actor_id)
                    .filter(|trade| {
                        trade.phase == TradePhase::Requested && trade.recipient == actor_id
                    })
                    .cloned()
                    .ok_or(WorldError::TradeUnavailable)?;
                self.trades.remove(&trade.id);
                self.finish_trade_action(
                    session,
                    "You decline the trade request.\r\n".into(),
                    vec![DomainEvent::TradeCancelled {
                        previous_trade: trade,
                        reason: TradeCancellationReason::Declined,
                    }],
                )
            }
            TradeAction::Cancel => {
                let trade = self
                    .trade_for_character(actor_id)
                    .cloned()
                    .ok_or(WorldError::TradeUnavailable)?;
                self.trades.remove(&trade.id);
                self.finish_trade_action(
                    session,
                    "You cancel the trade.\r\n".into(),
                    vec![DomainEvent::TradeCancelled {
                        previous_trade: trade,
                        reason: TradeCancellationReason::CancelledByParticipant,
                    }],
                )
            }
            TradeAction::OfferCoins(coins) => {
                if coins > MAX_TRADE_COINS
                    || self
                        .character_gameplay
                        .get(&actor_id)
                        .is_none_or(|gameplay| gameplay.coins < coins)
                {
                    return Err(WorldError::InsufficientFunds);
                }
                let previous = self.negotiating_trade(actor_id)?.clone();
                self.ensure_trade_available(&previous)?;
                let mut current = previous.clone();
                let offer = current
                    .offers
                    .get_mut(&actor_id)
                    .ok_or(WorldError::TradeUnavailable)?;
                if offer.coins == coins {
                    return self.trade_status_outcome(session);
                }
                offer.coins = coins;
                self.publish_trade_offer_change(session, previous, current)
            }
            TradeAction::OfferItem(selection) => {
                if selection.quantity == 0 || selection.quantity > MAX_TRADE_ITEM_QUANTITY {
                    return Err(WorldError::TradeUnavailable);
                }
                let previous = self.negotiating_trade(actor_id)?.clone();
                self.ensure_trade_available(&previous)?;
                let mut all_carried = self.item_candidates(session, ItemScope::Carried, None)?;
                let own_reserved = previous
                    .offers
                    .get(&actor_id)
                    .into_iter()
                    .flat_map(|offer| &offer.lines)
                    .flat_map(|line| &line.item_ids)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let all_reserved = self.reserved_item_ids()?;
                all_carried.retain(|id| !all_reserved.contains(id) || own_reserved.contains(id));
                let representative = self
                    .resolve_item_selection(
                        session,
                        &ItemSelection::one(&selection.query),
                        all_carried.clone(),
                    )?
                    .into_iter()
                    .next()
                    .ok_or(WorldError::TradeUnavailable)?;
                let instance = self
                    .item_instances
                    .get(&representative)
                    .ok_or(WorldError::InvalidEntity)?;
                let prototype = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let key = match prototype.stack_policy {
                    ItemStackPolicy::Fungible => TradeOfferKey::Fungible {
                        prototype_id: instance.prototype_id.clone(),
                    },
                    ItemStackPolicy::Unique => TradeOfferKey::Unique {
                        item_id: representative.clone(),
                    },
                };
                if prototype.stack_policy == ItemStackPolicy::Unique && selection.quantity != 1 {
                    return Err(WorldError::TradeUnavailable);
                }
                let mut current = previous.clone();
                let offer = current
                    .offers
                    .get_mut(&actor_id)
                    .ok_or(WorldError::TradeUnavailable)?;
                let line_index = offer.lines.iter().position(|line| line.key == key);
                let mut ids = line_index
                    .map(|index| offer.lines[index].item_ids.clone())
                    .unwrap_or_default();
                let desired = usize::from(selection.quantity);
                if ids.len() < desired {
                    let mut candidates = all_carried
                        .into_iter()
                        .filter(|id| {
                            self.item_instances.get(id).is_some_and(|candidate| {
                                candidate.prototype_id == instance.prototype_id
                                    && !all_reserved.contains(id)
                            })
                        })
                        .collect::<Vec<_>>();
                    candidates.sort();
                    for candidate in candidates {
                        if ids.len() == desired {
                            break;
                        }
                        if !ids.contains(&candidate) {
                            ids.push(candidate);
                        }
                    }
                } else {
                    ids.truncate(desired);
                }
                ids.sort();
                if ids.len() != desired {
                    return Err(WorldError::TradeUnavailable);
                }
                let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
                if ids.iter().any(|id| {
                    self.item_location(id) != Some(&SharedItemLocation::Carried(actor_id))
                        || !graph.subtree_is_transferable(id).unwrap_or(false)
                }) {
                    return Err(WorldError::ItemTransferForbidden);
                }
                let new_line = TradeOfferLine { key, item_ids: ids };
                if let Some(index) = line_index {
                    if offer.lines[index] == new_line {
                        return self.trade_status_outcome(session);
                    }
                    offer.lines[index] = new_line;
                } else {
                    if offer.lines.len() >= MAX_TRADE_LINES_PER_PARTICIPANT {
                        return Err(WorldError::TradeUnavailable);
                    }
                    offer.lines.push(new_line);
                    offer.lines.sort_by(|left, right| left.key.cmp(&right.key));
                }
                let total_roots = current
                    .offers
                    .values()
                    .flat_map(|offer| &offer.lines)
                    .map(|line| line.item_ids.len())
                    .sum::<usize>();
                if total_roots > MAX_TRADE_ROOTS {
                    return Err(WorldError::TradeUnavailable);
                }
                self.publish_trade_offer_change(session, previous, current)
            }
            TradeAction::RemoveItem(query) => {
                let previous = self.negotiating_trade(actor_id)?.clone();
                let observer = self.observer(session)?;
                let mut current = previous.clone();
                let offer = current
                    .offers
                    .get_mut(&actor_id)
                    .ok_or(WorldError::TradeUnavailable)?;
                let matches = offer
                    .lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| {
                        let Some(first) = line.item_ids.first() else {
                            return false;
                        };
                        let label_matches = self.item_prototype_for(first).is_ok_and(|prototype| {
                            prototype.name.eq_ignore_ascii_case(query.trim())
                        });
                        label_matches
                            || opaque_id(observer.salt, "item", first.as_str()) == query.trim()
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                let [index] = matches.as_slice() else {
                    return Err(WorldError::TradeUnavailable);
                };
                offer.lines.remove(*index);
                self.publish_trade_offer_change(session, previous, current)
            }
            TradeAction::Confirm { state_revision } => {
                let previous = self.negotiating_trade(actor_id)?.clone();
                self.ensure_trade_available(&previous)?;
                if previous.state_revision != state_revision {
                    return Err(WorldError::StaleTradeRevision);
                }
                if !trade_has_assets(&previous) {
                    return Err(WorldError::TradeUnavailable);
                }
                let basis_sha256 = self.trade_basis_sha256(&previous)?;
                let confirmation = TradeConfirmation {
                    state_revision,
                    basis_sha256,
                };
                if previous.confirmations.get(&actor_id) == Some(&confirmation) {
                    return self.trade_status_outcome(session);
                }
                let mut current = previous.clone();
                current.confirmations.insert(actor_id, confirmation.clone());
                if current.confirmations.len() == 2 {
                    if current
                        .confirmations
                        .values()
                        .any(|candidate| candidate != &confirmation)
                    {
                        return Err(WorldError::StaleTradeRevision);
                    }
                    self.complete_trade(session, current)
                } else {
                    self.trades.insert(current.id, current);
                    self.finish_trade_action(
                        session,
                        "You confirm the current trade terms.\r\n".into(),
                        vec![DomainEvent::TradeConfirmed {
                            participant: actor_id,
                            confirmation,
                        }],
                    )
                }
            }
            TradeAction::Status => self.trade_status_outcome(session),
            TradeAction::Contents { container, page } => {
                self.inspect_trade_container(session, &container, page)
            }
        }
    }

    pub(crate) fn negotiating_trade(
        &self,
        character_id: CharacterId,
    ) -> Result<&TradeSnapshot, WorldError> {
        self.trade_for_character(character_id)
            .filter(|trade| trade.phase == TradePhase::Negotiating)
            .ok_or(WorldError::TradeUnavailable)
    }

    pub(crate) fn character_in_combat(&self, character_id: CharacterId) -> bool {
        self.npcs
            .values()
            .any(|npc| npc.engaged_character == Some(character_id))
    }

    pub(crate) fn character_defeated(&self, character_id: CharacterId) -> bool {
        self.character_gameplay
            .get(&character_id)
            .is_none_or(|gameplay| gameplay.recovery_at_tick.is_some())
    }

    pub(crate) fn ensure_trade_available(&self, trade: &TradeSnapshot) -> Result<(), WorldError> {
        if self.logical_tick >= trade.expires_at_tick
            || self.character_in_combat(trade.requester)
            || self.character_in_combat(trade.recipient)
            || self.character_defeated(trade.requester)
            || self.character_defeated(trade.recipient)
        {
            return Err(WorldError::TradeUnavailable);
        }
        let rooms = trade
            .participants()
            .into_iter()
            .map(|participant| {
                self.session_for_character(participant)
                    .ok_or(WorldError::TradeUnavailable)
                    .and_then(|session| self.character(session))
                    .and_then(|character| room_checked(&self.entities, character.location))
                    .map(|room| room.stable_id.as_str())
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        if rooms.iter().any(|room| *room != trade.room_id) {
            return Err(WorldError::TradeUnavailable);
        }
        Ok(())
    }

    pub(crate) fn publish_trade_offer_change(
        &mut self,
        session: SessionId,
        previous: TradeSnapshot,
        mut current: TradeSnapshot,
    ) -> Result<ActionOutcome, WorldError> {
        current.state_revision = current
            .state_revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        current.confirmations.clear();
        let mut candidate_trades = self.trades.clone();
        candidate_trades.insert(current.id, current.clone());
        validate_trade_state(
            self.next_trade_id,
            &candidate_trades,
            &self.item_instances,
            &self.item_prototypes,
            &self.stable_rooms,
        )?;
        self.trades.insert(current.id, current.clone());
        self.finish_trade_action(
            session,
            "You update your trade offer.\r\n".into(),
            vec![DomainEvent::TradeOfferSet { previous, current }],
        )
    }

    pub(crate) fn complete_trade(
        &mut self,
        session: SessionId,
        trade: TradeSnapshot,
    ) -> Result<ActionOutcome, WorldError> {
        self.ensure_trade_available(&trade)?;
        let offer_a = trade
            .offers
            .get(&trade.requester)
            .ok_or(WorldError::TradeUnavailable)?;
        let offer_b = trade
            .offers
            .get(&trade.recipient)
            .ok_or(WorldError::TradeUnavailable)?;
        let roots_a = offer_a
            .lines
            .iter()
            .flat_map(|line| &line.item_ids)
            .cloned()
            .collect::<Vec<_>>();
        let roots_b = offer_b
            .lines
            .iter()
            .flat_map(|line| &line.item_ids)
            .cloned()
            .collect::<Vec<_>>();
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        if roots_a.iter().any(|root| {
            self.item_location(root) != Some(&SharedItemLocation::Carried(trade.requester))
                || !graph.subtree_is_transferable(root).unwrap_or(false)
        }) || roots_b.iter().any(|root| {
            self.item_location(root) != Some(&SharedItemLocation::Carried(trade.recipient))
                || !graph.subtree_is_transferable(root).unwrap_or(false)
        }) {
            return Err(WorldError::TradeUnavailable);
        }
        let mut prospective = self.item_instances.clone();
        for root in &roots_a {
            prospective
                .get_mut(root)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(trade.recipient);
        }
        for root in &roots_b {
            prospective
                .get_mut(root)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(trade.requester);
        }
        self.validate_prospective_item_locations(&prospective, [trade.requester, trade.recipient])?;
        let coins_a = self
            .character_gameplay
            .get(&trade.requester)
            .ok_or(WorldError::InvalidEntity)?
            .coins;
        let coins_b = self
            .character_gameplay
            .get(&trade.recipient)
            .ok_or(WorldError::InvalidEntity)?
            .coins;
        let final_a = coins_a
            .checked_sub(offer_a.coins)
            .and_then(|coins| coins.checked_add(offer_b.coins))
            .ok_or(WorldError::InsufficientFunds)?;
        let final_b = coins_b
            .checked_sub(offer_b.coins)
            .and_then(|coins| coins.checked_add(offer_a.coins))
            .ok_or(WorldError::InsufficientFunds)?;
        self.item_instances = prospective;
        self.character_gameplay
            .get_mut(&trade.requester)
            .ok_or(WorldError::InvalidEntity)?
            .coins = final_a;
        self.character_gameplay
            .get_mut(&trade.recipient)
            .ok_or(WorldError::InvalidEntity)?
            .coins = final_b;
        self.trades.remove(&trade.id);
        self.finish_trade_action(
            session,
            "The trade completes atomically.\r\n".into(),
            vec![DomainEvent::TradeCompleted {
                previous_trade: trade,
            }],
        )
    }

    pub(crate) fn finish_trade_action(
        &mut self,
        session: SessionId,
        output: String,
        events: Vec<DomainEvent>,
    ) -> Result<ActionOutcome, WorldError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        self.bump_view_revisions_for_events(&events)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: events,
        })
    }

    pub(crate) fn trade_status_outcome(
        &self,
        session: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        let viewer = self.character(session)?;
        let durable_trade = self
            .trade_for_character(viewer.id)
            .ok_or(WorldError::TradeUnavailable)?;
        let counterparty_id = durable_trade
            .counterpart(viewer.id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        let counterparty_name = self
            .connected_character_name(counterparty_id)
            .unwrap_or_else(|| "Disconnected character".into());
        let player = self.player_state_view(session)?;
        let trade = player.trade.as_ref().ok_or(WorldError::TradeUnavailable)?;
        let offer_text = |label: &str, offer: &TradeOfferView| {
            let mut output = format!("{label}: {} coin", offer.coins);
            if offer.lines.is_empty() {
                output.push_str("; no items");
            } else {
                for line in &offer.lines {
                    output.push_str(&format!(
                        "; {} × {} [ref {}{}]",
                        line.quantity,
                        line.label,
                        line.target,
                        if line.container { "; container" } else { "" }
                    ));
                }
            }
            output.push_str(".\r\n");
            output
        };
        let mut output = format!(
            "Trade {} with {}: {:?}; revision {}; expires in {} ticks.\r\nYour confirmation: {}; counterpart confirmation: {}.\r\n",
            trade.id,
            counterparty_name,
            trade.phase,
            trade.state_revision,
            trade.expires_in_ticks,
            if trade.own_confirmed { "yes" } else { "no" },
            if trade.counterparty_confirmed {
                "yes"
            } else {
                "no"
            },
        );
        output.push_str(&offer_text("Your offer", &trade.own_offer));
        output.push_str(&offer_text("Counterpart offer", &trade.counterparty_offer));
        output.push_str(&format!(
            "Coins: {} total; {} reserved; {} spendable.\r\n",
            player.economy.coins, player.economy.reserved_coins, player.economy.spendable_coins,
        ));
        output.push_str("Inventory available for trade:\r\n");
        if player.inventory.is_empty() {
            output.push_str("- none\r\n");
        } else {
            for item in player
                .inventory
                .iter()
                .take(MAX_TRADE_STATUS_INVENTORY_GROUPS)
            {
                output.push_str(&format!(
                    "- {}: {} total; {} available; {} reserved [ref {}].\r\n",
                    item.label,
                    item.quantity,
                    item.available_quantity,
                    item.reserved_for_trade,
                    item.id,
                ));
            }
            let omitted = player
                .inventory
                .len()
                .saturating_sub(MAX_TRADE_STATUS_INVENTORY_GROUPS);
            if omitted > 0 {
                output.push_str(&format!(
                    "- {omitted} more inventory groups omitted; use inventory to choose an opaque reference. Use trade contents <offered-container> <page> to inspect an offered container.\r\n"
                ));
            }
        }
        if output.len() > MAX_TRADE_STATUS_TEXT_BYTES {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    pub(crate) fn inspect_trade_container(
        &self,
        session: SessionId,
        container: &str,
        page: u16,
    ) -> Result<ActionOutcome, WorldError> {
        if page == 0 {
            return Err(WorldError::TradeUnavailable);
        }
        let viewer = self.character(session)?;
        let trade = self.negotiating_trade(viewer.id)?;
        let observer = self.observer(session)?;
        let query = container.trim();
        let matches = trade
            .offers
            .values()
            .flat_map(|offer| &offer.lines)
            .flat_map(|line| &line.item_ids)
            .filter(|item_id| {
                self.item_prototype_for(item_id).is_ok_and(|prototype| {
                    prototype.container.is_some()
                        && (prototype.name.eq_ignore_ascii_case(query)
                            || opaque_id(observer.salt, "item", item_id.as_str()) == query)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let [container_id] = matches.as_slice() else {
            return Err(WorldError::TradeUnavailable);
        };
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let direct_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Contained(parent) if parent == container_id)
                    .then_some(item_id)
            })
            .collect::<Vec<_>>();
        let groups = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            direct_ids.into_iter(),
        )?;
        let total_pages = groups.len().max(1).div_ceil(CONTAINER_PAGE_SIZE);
        let page_index = page_index(page);
        if page_index >= total_pages {
            return Err(WorldError::TradeUnavailable);
        }
        let items = groups
            .into_iter()
            .skip(page_index * CONTAINER_PAGE_SIZE)
            .take(CONTAINER_PAGE_SIZE)
            .map(|group| {
                let first = group
                    .members
                    .first()
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                let prototype = self.item_prototype_for(first)?;
                let quantity = u16::try_from(group.members.len())
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                Ok(PlayerItemView {
                    id: opaque_id(observer.salt, "item", first.as_str()),
                    label: self.item_effective_name(first)?,
                    description: prototype.description.clone(),
                    rarity: prototype.rarity.as_str().into(),
                    item_level: prototype.item_level,
                    weight_grams: prototype.weight_grams,
                    quantity,
                    available_quantity: 0,
                    reserved_for_trade: quantity,
                    stack_slots: stack_slots(prototype, group.members.len())?,
                    total_weight_grams: group.members.iter().try_fold(
                        0_u64,
                        |total, item_id| {
                            total
                                .checked_add(graph.subtree_weight(item_id)?)
                                .ok_or(WorldError::InvalidEntity)
                        },
                    )?,
                    transfer_policy: prototype.transfer_policy,
                    bound_to_you: false,
                    can_give: false,
                    can_trade: false,
                    affix: self.item_affix_view(first),
                    container_capacity: prototype
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                first,
                                &self.item_instances,
                                &self.item_prototypes,
                                &graph,
                            )
                        })
                        .transpose()?,
                    actions: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let capacity = container_usage(
            container_id,
            &self.item_instances,
            &self.item_prototypes,
            &graph,
        )?;
        let total_pages =
            u16::try_from(total_pages).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
        let mut output = format!(
            "Offered container contents, page {page}/{total_pages}: {}/{} slots; {}/{} g.\r\n",
            capacity.used_slots,
            capacity.max_slots,
            capacity.used_weight_grams,
            capacity.max_weight_grams
        );
        if items.is_empty() {
            output.push_str("- empty\r\n");
        } else {
            for item in &items {
                output.push_str(&format!(
                    "- {} × {} [{}; level {}; {} g total; ref {}].\r\n",
                    item.quantity,
                    item.label,
                    item.rarity,
                    item.item_level,
                    item.total_weight_grams,
                    item.id,
                ));
            }
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: Some(TradeContentsView {
                trade_id: opaque_id(observer.salt, "trade", &trade.id.0.to_string()),
                container: opaque_id(observer.salt, "item", container_id.as_str()),
                items,
                page,
                total_pages,
                capacity: capacity.clone(),
            }),
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    pub(crate) fn trade_basis_sha256(&self, trade: &TradeSnapshot) -> Result<[u8; 32], WorldError> {
        let mut digest = Sha256::new();
        digest.update(b"tiamud.trade.basis.v2\0");
        canonical_u64(&mut digest, trade.id.0);
        canonical_i64(&mut digest, trade.requester.0);
        canonical_i64(&mut digest, trade.recipient.0);
        canonical_str(&mut digest, &trade.room_id)?;
        canonical_u64(&mut digest, trade.state_revision);
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        for participant in trade.participants() {
            canonical_i64(&mut digest, participant.0);
            let owned_items = self
                .item_instances
                .iter()
                .filter_map(|(item_id, instance)| {
                    item_effective_owner(&self.item_instances, item_id)
                        .map(|owner| (owner == Some(participant)).then_some((item_id, instance)))
                        .transpose()
                })
                .collect::<Result<Vec<_>, WorldError>>()?;
            canonical_u64(
                &mut digest,
                u64::try_from(owned_items.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
            );
            for (item_id, instance) in owned_items {
                let prototype = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                canonical_str(&mut digest, item_id.as_str())?;
                canonical_str(&mut digest, &instance.prototype_id)?;
                canonical_item_location(&mut digest, &instance.location)?;
                canonical_item_binding(&mut digest, instance.binding);
                canonical_item_affixes(&mut digest, &instance.item_affixes)?;
                digest.update([match prototype.transfer_policy {
                    ItemTransferPolicy::Transferable => 0,
                    ItemTransferPolicy::CharacterBound => 1,
                    ItemTransferPolicy::NonTransferable => 2,
                }]);
                digest.update([match prototype.stack_policy {
                    ItemStackPolicy::Unique => 0,
                    ItemStackPolicy::Fungible => 1,
                }]);
                canonical_u64(&mut digest, u64::from(prototype.weight_grams));
                canonical_u64(&mut digest, u64::from(prototype.slot_cost));
                match &prototype.container {
                    Some(capacity) => {
                        digest.update([1]);
                        canonical_u64(&mut digest, u64::from(capacity.max_slots));
                        canonical_u64(&mut digest, u64::from(capacity.max_weight_grams));
                    }
                    None => digest.update([0]),
                }
            }
            let offer = trade
                .offers
                .get(&participant)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            canonical_u64(&mut digest, offer.coins);
            canonical_u64(
                &mut digest,
                u64::try_from(offer.lines.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
            );
            for line in &offer.lines {
                canonical_trade_key(&mut digest, &line.key)?;
                canonical_u64(
                    &mut digest,
                    u64::try_from(line.item_ids.len())
                        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
                );
                for root in &line.item_ids {
                    for item_id in graph.subtree_ids(root)? {
                        let instance = self
                            .item_instances
                            .get(&item_id)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                        let prototype = self
                            .item_prototypes
                            .get(&instance.prototype_id)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                        canonical_str(&mut digest, item_id.as_str())?;
                        canonical_str(&mut digest, &instance.prototype_id)?;
                        digest.update([prototype.transfer_policy as u8]);
                        canonical_item_binding(&mut digest, instance.binding);
                        canonical_item_affixes(&mut digest, &instance.item_affixes)?;
                        canonical_item_location(&mut digest, &instance.location)?;
                    }
                }
            }
            let gameplay = self
                .character_gameplay
                .get(&participant)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            canonical_u64(&mut digest, gameplay.coins);
            canonical_u64(
                &mut digest,
                u64::from(gameplay.inventory_capacity.max_slots),
            );
            canonical_u64(
                &mut digest,
                u64::from(gameplay.inventory_capacity.max_weight_grams),
            );
            canonical_u64(
                &mut digest,
                u64::try_from(gameplay.equipment.len())
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
            );
            for (slot, item_id) in &gameplay.equipment {
                digest.update([*slot as u8]);
                canonical_str(&mut digest, item_id.as_str())?;
            }
        }
        Ok(digest.finalize().into())
    }
}

impl World {
    /// Resolve a connected, co-located give recipient using only the acting
    /// session's opaque character selectors or one exact, unambiguous name.
    /// The returned durable IDs are server-internal and must not be serialized.
    pub fn resolve_give_recipient(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<ConnectedCharacter, WorldError> {
        let giver = self.character(session)?;
        let giver_room = room_checked(&self.entities, giver.location)?
            .stable_id
            .as_str();
        let salt = self.observer(session)?.salt;
        let query = query.trim();
        if query.is_empty() {
            return Err(WorldError::GiveTargetUnavailable);
        }
        // Names are compared first and cheaply. A query without the exact
        // opaque-selector shape can never equal a selector, so hashing every
        // connected character's selector happens only for selector-shaped
        // queries. Ambiguity (name and selector, or two names) still fails.
        let selector_query = is_opaque_id_shaped(query);
        let matches = self
            .connected_characters()?
            .into_iter()
            .filter(|candidate| {
                candidate.name.eq_ignore_ascii_case(query)
                    || (selector_query
                        && opaque_id(salt, "character", &candidate.character_id.0.to_string())
                            == query)
            })
            .collect::<Vec<_>>();
        let Ok([recipient]) = <[ConnectedCharacter; 1]>::try_from(matches) else {
            return Err(WorldError::GiveTargetUnavailable);
        };
        if recipient.character_id == giver.id || recipient.room_id != giver_room {
            return Err(WorldError::GiveTargetUnavailable);
        }
        Ok(recipient)
    }

    pub fn resolve_trade_recipient(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<ConnectedCharacter, WorldError> {
        self.resolve_give_recipient(session, query)
            .map_err(|_| WorldError::TradeTargetUnavailable)
    }

    pub fn trade_for_character(&self, character_id: CharacterId) -> Option<&TradeSnapshot> {
        self.trades
            .values()
            .find(|trade| trade.includes(character_id))
    }

    pub fn trade_participants(&self) -> BTreeSet<CharacterId> {
        self.trades
            .values()
            .flat_map(|trade| trade.participants())
            .collect()
    }

    pub fn reserved_item_ids(&self) -> Result<BTreeSet<ItemInstanceId>, WorldError> {
        let graph = build_item_subgraph_index(
            &self.item_instances,
            &self.item_prototypes,
            self.trades
                .values()
                .flat_map(|trade| trade.offers.values())
                .flat_map(|offer| &offer.lines)
                .flat_map(|line| &line.item_ids),
        )?;
        reserved_item_ids_with_graph(&self.trades, &graph)
    }

    pub fn item_reserved_for_trade(&self, item_id: &ItemInstanceId) -> Result<bool, WorldError> {
        Ok(self.reserved_item_ids()?.contains(item_id))
    }

    pub(crate) fn ensure_items_unreserved<'a>(
        &self,
        item_ids: impl IntoIterator<Item = &'a ItemInstanceId>,
    ) -> Result<(), WorldError> {
        let reserved = self.reserved_item_ids()?;
        if item_ids
            .into_iter()
            .any(|item_id| reserved.contains(item_id))
        {
            Err(WorldError::ItemReservedForTrade)
        } else {
            Ok(())
        }
    }

    pub fn reserved_coins(&self, character_id: CharacterId) -> u64 {
        self.trade_for_character(character_id)
            .and_then(|trade| trade.offers.get(&character_id))
            .map_or(0, |offer| offer.coins)
    }

    pub fn spendable_coins(&self, character_id: CharacterId) -> Result<u64, WorldError> {
        self.character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .coins
            .checked_sub(self.reserved_coins(character_id))
            .ok_or(WorldError::InvalidRuntimeSnapshot)
    }

    pub fn cancel_trade_for_character(
        &mut self,
        character_id: CharacterId,
        reason: TradeCancellationReason,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let Some(trade) = self.trade_for_character(character_id).cloned() else {
            return Ok(Vec::new());
        };
        self.trades.remove(&trade.id);
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        Ok(vec![DomainEvent::TradeCancelled {
            previous_trade: trade,
            reason,
        }])
    }

    /// Removes every trade whose logical deadline has been reached. The actor
    /// calls this before timer/combat/script hooks so an expired reservation
    /// can never survive into another tick's mutations. This deliberately does
    /// not run basis invalidation; the post-hook reconciliation owns that
    /// separate comparison.
    pub fn cancel_expired_trades(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        let expired = self
            .trades
            .iter()
            .filter(|(_, trade)| self.logical_tick >= trade.expires_at_tick)
            .map(|(trade_id, trade)| (*trade_id, trade.clone()))
            .collect::<Vec<_>>();
        if expired.is_empty() {
            return Ok(Vec::new());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        for (trade_id, _) in &expired {
            self.trades.remove(trade_id);
        }
        self.revision = revision;
        Ok(expired
            .into_iter()
            .map(|(_, previous_trade)| DomainEvent::TradeCancelled {
                previous_trade,
                reason: TradeCancellationReason::Expired,
            })
            .collect())
    }
}

impl World {
    pub fn cancel_trades(
        &mut self,
        trade_ids: &BTreeSet<TradeId>,
        reason: TradeCancellationReason,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let mut events = Vec::new();
        for trade_id in trade_ids {
            if let Some(previous_trade) = self.trades.remove(trade_id) {
                events.push(DomainEvent::TradeCancelled {
                    previous_trade,
                    reason,
                });
            }
        }
        if !events.is_empty() {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(WorldError::InvalidDomainTransition)?;
        }
        Ok(events)
    }

    pub fn cancel_all_trades(
        &mut self,
        reason: TradeCancellationReason,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let trades = std::mem::take(&mut *self.trades);
        if trades.is_empty() {
            return Ok(Vec::new());
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        Ok(trades
            .into_values()
            .map(|previous_trade| DomainEvent::TradeCancelled {
                previous_trade,
                reason,
            })
            .collect())
    }

    pub fn validate_trade_participant_gameplay(
        &self,
        participants: &BTreeMap<CharacterId, CharacterGameplaySnapshot>,
    ) -> Result<(), WorldError> {
        for character_id in self.trade_participants() {
            let gameplay = participants
                .get(&character_id)
                .or_else(|| self.character_gameplay.get(&character_id))
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            if gameplay.coins < self.reserved_coins(character_id) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            self.validate_character_gameplay_snapshot(character_id, gameplay)
                .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
        }
        Ok(())
    }

    pub fn reconcile_trades(
        &mut self,
        before: &WorldCheckpoint,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let mut events = Vec::new();
        let mut cancelled = BTreeSet::new();
        let mut invalidated = false;
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let before_items_by_owner = items_by_effective_owner(&before.item_instances)?;
        let current_items_by_owner = items_by_effective_owner(&self.item_instances)?;
        let trade_ids = self.trades.keys().copied().collect::<Vec<_>>();
        for trade_id in trade_ids {
            let Some(trade) = self.trades.get(&trade_id).cloned() else {
                continue;
            };
            let requester_session = self.session_for_character(trade.requester);
            let recipient_session = self.session_for_character(trade.recipient);
            let disconnected = requester_session.is_none() || recipient_session.is_none();
            let room_for = |world: &Self, character_id| {
                world
                    .session_for_character(character_id)
                    .and_then(|session| world.character(session).ok())
                    .and_then(|character| room_checked(&world.entities, character.location).ok())
                    .map(|room| room.stable_id.clone())
            };
            let separated = !disconnected
                && (room_for(self, trade.requester) != Some(trade.room_id.clone())
                    || room_for(self, trade.recipient) != Some(trade.room_id.clone()));
            let defeated = trade.participants().into_iter().any(|participant| {
                self.character_gameplay
                    .get(&participant)
                    .is_some_and(|gameplay| gameplay.recovery_at_tick.is_some())
            });
            let combat = self.npcs.values().any(|npc| {
                npc.engaged_character
                    .is_some_and(|participant| trade.includes(participant))
            });
            let reservation_valid = validate_trade_state_with_graph(
                self.next_trade_id,
                &BTreeMap::from([(trade.id, trade.clone())]),
                &self.item_instances,
                &self.item_prototypes,
                &self.stable_rooms,
                &graph,
            )
            .is_ok()
                && trade.participants().into_iter().all(|participant| {
                    self.character_gameplay
                        .get(&participant)
                        .is_some_and(|gameplay| gameplay.coins >= self.reserved_coins(participant))
                });
            let reason = if self.logical_tick >= trade.expires_at_tick {
                Some(TradeCancellationReason::Expired)
            } else if disconnected {
                Some(TradeCancellationReason::ParticipantDisconnected)
            } else if separated {
                Some(TradeCancellationReason::ParticipantsSeparated)
            } else if defeated {
                Some(TradeCancellationReason::ParticipantDefeated)
            } else if combat {
                Some(TradeCancellationReason::CombatStarted)
            } else if !reservation_valid {
                Some(TradeCancellationReason::InvalidReservation)
            } else {
                None
            };
            if let Some(reason) = reason {
                self.trades.remove(&trade.id);
                events.push(DomainEvent::TradeCancelled {
                    previous_trade: trade,
                    reason,
                });
                cancelled.insert(trade_id);
                continue;
            }

            // A trade command publishes its own single state revision. For any
            // other mutation, compare the material authorities retained in the
            // checkpoint and invalidate confirmations exactly once.
            if before.trades.get(&trade_id) == Some(&trade) {
                let participant_gameplay_changed = trade.participants().into_iter().any(|id| {
                    before.character_gameplay.get(&id) != self.character_gameplay.get(&id)
                });
                let participant_location_changed = trade.participants().into_iter().any(|id| {
                    character_room_id(&before.entities, id) != character_room_id(&self.entities, id)
                });
                if participant_gameplay_changed
                    || participant_location_changed
                    || trade.participants().into_iter().any(|participant| {
                        before_items_by_owner.get(&participant)
                            != current_items_by_owner.get(&participant)
                    })
                {
                    let previous = trade.clone();
                    let mut current = trade;
                    current.state_revision = current
                        .state_revision
                        .checked_add(1)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    current.confirmations.clear();
                    self.trades.insert(trade_id, current.clone());
                    events.push(DomainEvent::TradeInvalidated { previous, current });
                    invalidated = true;
                }
            }
        }
        if !cancelled.is_empty() || invalidated {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(WorldError::InvalidDomainTransition)?;
        }
        Ok(events)
    }
}

impl World {
    pub(crate) fn trade_view_for(
        &self,
        session: SessionId,
    ) -> Result<Option<TradeView>, WorldError> {
        let viewer = self.character(session)?;
        let Some(trade) = self.trade_for_character(viewer.id) else {
            return Ok(None);
        };
        let observer = self.observer(session)?;
        let counterpart = trade
            .counterpart(viewer.id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        let counterparty = opaque_id(observer.salt, "character", &counterpart.0.to_string());
        let offer_view = |participant: CharacterId| -> Result<TradeOfferView, WorldError> {
            let offer = trade.offers.get(&participant).cloned().unwrap_or_default();
            let lines = offer
                .lines
                .iter()
                .map(|line| {
                    let first = line
                        .item_ids
                        .first()
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    let prototype = self.item_prototype_for(first)?;
                    Ok(TradeOfferLineView {
                        target: opaque_id(observer.salt, "item", first.as_str()),
                        label: prototype.name.clone(),
                        quantity: u16::try_from(line.item_ids.len())
                            .map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
                        container: prototype.container.is_some(),
                    })
                })
                .collect::<Result<Vec<_>, WorldError>>()?;
            Ok(TradeOfferView {
                coins: offer.coins,
                lines,
            })
        };
        let empty = trade
            .offers
            .values()
            .all(|offer| offer.coins == 0 && offer.lines.is_empty());
        Ok(Some(TradeView {
            id: opaque_id(observer.salt, "trade", &trade.id.0.to_string()),
            phase: trade.phase,
            counterparty,
            expires_in_ticks: trade.expires_at_tick.saturating_sub(self.logical_tick),
            state_revision: trade.state_revision,
            own_confirmed: trade.confirmations.contains_key(&viewer.id),
            counterparty_confirmed: trade.confirmations.contains_key(&counterpart),
            own_offer: offer_view(viewer.id)?,
            counterparty_offer: offer_view(counterpart)?,
            can_accept: trade.phase == TradePhase::Requested && trade.recipient == viewer.id,
            can_confirm: trade.phase == TradePhase::Negotiating && !empty,
        }))
    }
}

pub(crate) fn same_trade_identity_and_lifetime(
    left: &TradeSnapshot,
    right: &TradeSnapshot,
) -> bool {
    left.id == right.id
        && left.requester == right.requester
        && left.recipient == right.recipient
        && left.room_id == right.room_id
        && left.created_tick == right.created_tick
        && left.opened_tick == right.opened_tick
        && left.expires_at_tick == right.expires_at_tick
}

pub(crate) fn next_trade_revision(previous: &TradeSnapshot) -> Option<u64> {
    previous.state_revision.checked_add(1)
}

pub(crate) fn valid_trade_open_transition(
    previous: &TradeSnapshot,
    current: &TradeSnapshot,
) -> bool {
    let expected_offers = BTreeMap::from([
        (previous.requester, TradeOffer::default()),
        (previous.recipient, TradeOffer::default()),
    ]);
    previous.phase == TradePhase::Requested
        && previous.opened_tick.is_none()
        && previous.offers.is_empty()
        && previous.confirmations.is_empty()
        && current.id == previous.id
        && current.requester == previous.requester
        && current.recipient == previous.recipient
        && current.room_id == previous.room_id
        && current.created_tick == previous.created_tick
        && current.phase == TradePhase::Negotiating
        && current
            .opened_tick
            .is_some_and(|opened_tick| opened_tick < previous.expires_at_tick)
        && current.state_revision == next_trade_revision(previous).unwrap_or(0)
        && current.offers == expected_offers
        && current.confirmations.is_empty()
}

pub(crate) fn trade_has_assets(trade: &TradeSnapshot) -> bool {
    trade
        .offers
        .values()
        .any(|offer| offer.coins > 0 || !offer.lines.is_empty())
}

pub(crate) fn one_offer_assignment_changed(previous: &TradeOffer, current: &TradeOffer) -> bool {
    if previous.coins != current.coins {
        return previous.lines == current.lines;
    }
    if previous.lines == current.lines {
        return false;
    }
    let previous_by_key = previous
        .lines
        .iter()
        .map(|line| (&line.key, line))
        .collect::<BTreeMap<_, _>>();
    let current_by_key = current
        .lines
        .iter()
        .map(|line| (&line.key, line))
        .collect::<BTreeMap<_, _>>();
    let changed_keys = previous_by_key
        .keys()
        .chain(current_by_key.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| previous_by_key.get(**key) != current_by_key.get(**key))
        .collect::<Vec<_>>();
    let [key] = changed_keys.as_slice() else {
        return false;
    };
    match (previous_by_key.get(**key), current_by_key.get(**key)) {
        (None, Some(_)) | (Some(_), None) => true,
        (Some(previous_line), Some(current_line)) => {
            previous_line.key == current_line.key
                && ((previous_line.item_ids.len() < current_line.item_ids.len()
                    && previous_line
                        .item_ids
                        .iter()
                        .all(|item_id| current_line.item_ids.contains(item_id)))
                    || (current_line.item_ids.len() < previous_line.item_ids.len()
                        && current_line.item_ids
                            == previous_line.item_ids[..current_line.item_ids.len()]))
        }
        (None, None) => false,
    }
}

pub(crate) fn valid_trade_offer_transition(
    previous: &TradeSnapshot,
    current: &TradeSnapshot,
) -> bool {
    if previous.phase != TradePhase::Negotiating
        || current.phase != TradePhase::Negotiating
        || !same_trade_identity_and_lifetime(previous, current)
        || current.state_revision != next_trade_revision(previous).unwrap_or(0)
        || !current.confirmations.is_empty()
    {
        return false;
    }
    let changed = previous
        .participants()
        .into_iter()
        .filter(|participant| previous.offers.get(participant) != current.offers.get(participant))
        .collect::<Vec<_>>();
    let [participant] = changed.as_slice() else {
        return false;
    };
    previous
        .offers
        .get(participant)
        .zip(current.offers.get(participant))
        .is_some_and(|(before, after)| one_offer_assignment_changed(before, after))
}

pub(crate) fn valid_trade_invalidation_transition(
    previous: &TradeSnapshot,
    current: &TradeSnapshot,
) -> bool {
    let Some(next_revision) = next_trade_revision(previous) else {
        return false;
    };
    let mut expected = previous.clone();
    expected.state_revision = next_revision;
    expected.confirmations.clear();
    current == &expected
}

pub(crate) fn portable_subtree_ids(
    root: &ItemInstanceId,
    children: &BTreeMap<ItemInstanceId, Vec<ItemInstanceId>>,
) -> Result<Vec<ItemInstanceId>, WorldError> {
    let mut result = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(item_id) = pending.pop() {
        if result.len() >= MAX_ITEM_INSTANCES {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        if let Some(direct) = children.get(&item_id) {
            pending.extend(direct.iter().rev().cloned());
        }
        result.push(item_id);
    }
    Ok(result)
}

pub(crate) fn validate_portable_trade_state(
    snapshot: &WorldRuntimeSnapshot,
    children: &BTreeMap<ItemInstanceId, Vec<ItemInstanceId>>,
) -> Result<(), WorldError> {
    if snapshot.next_trade_id.0 == 0 || snapshot.trades.len() > MAX_LIVE_TRADES {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let mut participants = BTreeSet::new();
    let mut reserved_roots = BTreeSet::new();
    let mut reserved_subtrees = BTreeSet::new();
    for (trade_id, trade) in &snapshot.trades {
        if *trade_id != trade.id
            || trade_id.0 == 0
            || trade_id.0 >= snapshot.next_trade_id.0
            || trade.requester.0 <= 0
            || trade.recipient.0 <= 0
            || trade.requester == trade.recipient
            || !valid_definition_id(&trade.room_id)
            || trade.state_revision == 0
            || trade.created_tick >= trade.expires_at_tick
            || !participants.insert(trade.requester)
            || !participants.insert(trade.recipient)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        match trade.phase {
            TradePhase::Requested => {
                if trade.opened_tick.is_some()
                    || !trade.offers.is_empty()
                    || !trade.confirmations.is_empty()
                    || trade.expires_at_tick
                        != trade
                            .created_tick
                            .checked_add(TRADE_REQUEST_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            TradePhase::Negotiating => {
                let opened = trade
                    .opened_tick
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                if opened < trade.created_tick
                    || trade.expires_at_tick
                        != opened
                            .checked_add(TRADE_NEGOTIATION_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                    || trade.offers.len() != 2
                    || !trade.offers.contains_key(&trade.requester)
                    || !trade.offers.contains_key(&trade.recipient)
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
        }
        if trade.confirmations.keys().any(|participant| {
            !trade.includes(*participant)
                || trade
                    .confirmations
                    .get(participant)
                    .is_none_or(|confirmation| confirmation.state_revision != trade.state_revision)
        }) || (trade.confirmations.len() == 2
            && trade
                .confirmations
                .values()
                .map(|confirmation| confirmation.basis_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != 1)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let mut trade_roots = 0_usize;
        for (owner, offer) in &trade.offers {
            if !trade.includes(*owner)
                || offer.coins > MAX_TRADE_COINS
                || offer.lines.len() > MAX_TRADE_LINES_PER_PARTICIPANT
                || !offer.lines.windows(2).all(|pair| pair[0].key < pair[1].key)
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            for line in &offer.lines {
                if line.item_ids.is_empty()
                    || line.item_ids.len() > usize::from(MAX_TRADE_ITEM_QUANTITY)
                    || !line.item_ids.windows(2).all(|pair| pair[0] < pair[1])
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                trade_roots = trade_roots
                    .checked_add(line.item_ids.len())
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                for root in &line.item_ids {
                    if !valid_item_instance_id(root.as_str())
                        || !reserved_roots.insert(root.clone())
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let instance = snapshot
                        .item_instances
                        .get(root)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if instance.location
                        != (RuntimeItemLocation::Carried {
                            character_id: *owner,
                        })
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let key_valid = match &line.key {
                        TradeOfferKey::Fungible { prototype_id } => {
                            valid_definition_id(prototype_id)
                                && &instance.prototype_id == prototype_id
                        }
                        TradeOfferKey::Unique { item_id } => {
                            item_id == root && line.item_ids.len() == 1
                        }
                    };
                    if !key_valid {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    for member in portable_subtree_ids(root, children)? {
                        if !reserved_subtrees.insert(member) {
                            return Err(WorldError::InvalidRuntimeSnapshot);
                        }
                    }
                }
            }
        }
        if trade_roots > MAX_TRADE_ROOTS {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    if reserved_roots.len() > MAX_RESERVED_TRADE_ROOTS {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let mut ground_subtrees = BTreeSet::new();
    for root in snapshot.ground_loot.keys() {
        for member in portable_subtree_ids(root, children)? {
            if !ground_subtrees.insert(member.clone()) || reserved_subtrees.contains(&member) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_trade_state(
    next_trade_id: TradeId,
    trades: &BTreeMap<TradeId, TradeSnapshot>,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    room_ids: &HashMap<String, EntityId>,
) -> Result<(), WorldError> {
    let graph = build_item_graph_index(instances, prototypes)
        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    validate_trade_state_with_graph(
        next_trade_id,
        trades,
        instances,
        prototypes,
        room_ids,
        &graph,
    )
}

pub(crate) fn validate_trade_state_with_graph(
    next_trade_id: TradeId,
    trades: &BTreeMap<TradeId, TradeSnapshot>,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    room_ids: &HashMap<String, EntityId>,
    graph: &ItemGraphIndex,
) -> Result<(), WorldError> {
    if next_trade_id.0 == 0 || trades.len() > MAX_LIVE_TRADES {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let mut participants = BTreeSet::new();
    let mut reserved_roots = BTreeSet::new();
    let mut reserved_subtrees = BTreeSet::new();
    for (id, trade) in trades {
        if *id != trade.id
            || id.0 == 0
            || id.0 >= next_trade_id.0
            || trade.requester.0 <= 0
            || trade.recipient.0 <= 0
            || trade.requester == trade.recipient
            || !room_ids.contains_key(&trade.room_id)
            || trade.state_revision == 0
            || trade.created_tick >= trade.expires_at_tick
            || !participants.insert(trade.requester)
            || !participants.insert(trade.recipient)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        match trade.phase {
            TradePhase::Requested => {
                if trade.opened_tick.is_some()
                    || !trade.offers.is_empty()
                    || !trade.confirmations.is_empty()
                    || trade.expires_at_tick
                        != trade
                            .created_tick
                            .checked_add(TRADE_REQUEST_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            TradePhase::Negotiating => {
                let opened = trade
                    .opened_tick
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                if opened < trade.created_tick
                    || trade.expires_at_tick
                        != opened
                            .checked_add(TRADE_NEGOTIATION_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                    || trade.offers.len() != 2
                    || !trade.offers.contains_key(&trade.requester)
                    || !trade.offers.contains_key(&trade.recipient)
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
        }
        if trade.confirmations.keys().any(|participant| {
            !trade.includes(*participant)
                || trade
                    .confirmations
                    .get(participant)
                    .is_none_or(|confirmation| confirmation.state_revision != trade.state_revision)
        }) || (trade.confirmations.len() == 2
            && trade
                .confirmations
                .values()
                .map(|confirmation| confirmation.basis_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != 1)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let mut trade_roots = 0usize;
        for (owner, offer) in &trade.offers {
            if !trade.includes(*owner)
                || offer.coins > MAX_TRADE_COINS
                || offer.lines.len() > MAX_TRADE_LINES_PER_PARTICIPANT
                || !offer.lines.windows(2).all(|pair| pair[0].key < pair[1].key)
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            for line in &offer.lines {
                if line.item_ids.is_empty()
                    || line.item_ids.len() > usize::from(MAX_TRADE_ITEM_QUANTITY)
                    || !line.item_ids.windows(2).all(|pair| pair[0] < pair[1])
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                trade_roots = trade_roots
                    .checked_add(line.item_ids.len())
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                for root in &line.item_ids {
                    if !valid_item_instance_id(root.as_str())
                        || !reserved_roots.insert(root.clone())
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let instance = instances
                        .get(root)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    let prototype = prototypes
                        .get(&instance.prototype_id)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if instance.location != SharedItemLocation::Carried(*owner)
                        || !graph
                            .subtree_is_transferable(root)
                            .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let key_valid = match &line.key {
                        TradeOfferKey::Fungible { prototype_id } => {
                            prototype.stack_policy == ItemStackPolicy::Fungible
                                && &instance.prototype_id == prototype_id
                        }
                        TradeOfferKey::Unique { item_id } => {
                            prototype.stack_policy == ItemStackPolicy::Unique
                                && item_id == root
                                && line.item_ids.len() == 1
                        }
                    };
                    if !key_valid {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    for member in graph
                        .subtree_ids(root)
                        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                    {
                        if !reserved_subtrees.insert(member) {
                            return Err(WorldError::InvalidRuntimeSnapshot);
                        }
                    }
                }
            }
        }
        if trade_roots > MAX_TRADE_ROOTS {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    if reserved_roots.len() > MAX_RESERVED_TRADE_ROOTS {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    Ok(())
}
