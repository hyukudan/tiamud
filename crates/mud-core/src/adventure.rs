//! Party, quest and economy actions.

use super::*;

impl World {
    pub(crate) fn apply_party(
        &mut self,
        session: SessionId,
        action: PartyAction,
    ) -> Result<ActionOutcome, WorldError> {
        let actor = self.character(session)?;
        let actor_id = actor.id;
        let actor_name = actor.name.clone();
        match action {
            PartyAction::Create => {
                if self.party_for_member(actor_id).is_some() {
                    return Err(WorldError::PartyUnavailable);
                }
                let party_id = self.adventure.next_party_id.max(1);
                self.adventure.next_party_id = party_id
                    .checked_add(1)
                    .ok_or(WorldError::PartyUnavailable)?;
                self.adventure.parties.insert(
                    party_id,
                    PartySnapshot {
                        id: party_id,
                        leader: actor_id,
                        members: vec![actor_id],
                        invitations: BTreeMap::new(),
                    },
                );
                let audience = self.party_member_audience(party_id, &[]);
                self.finish_adventure_action(
                    session,
                    "You create an adventuring party.\r\n".into(),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} creates an adventuring party."),
                    },
                )
            }
            PartyAction::Invite(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                let leader = self
                    .adventure
                    .parties
                    .get(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?
                    .leader;
                if leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                if target_id == actor_id || self.party_for_member(target_id).is_some() {
                    return Err(WorldError::PartyUnavailable);
                }
                let party = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if party.members.len() >= 6 || party.invitations.len() >= 12 {
                    return Err(WorldError::PartyUnavailable);
                }
                party
                    .invitations
                    .insert(target_id, self.logical_tick.saturating_add(20));
                let mut audience = vec![actor_id, target_id];
                audience.sort_unstable();
                audience.dedup();
                self.finish_adventure_action(
                    session,
                    format!("You invite {target} to your party.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} sends a party invitation."),
                    },
                )
            }
            PartyAction::Accept(leader) => {
                if self.party_for_member(actor_id).is_some() {
                    return Err(WorldError::PartyUnavailable);
                }
                let leader_id = self.resolve_connected_character(&leader)?;
                let party_id = self
                    .adventure
                    .parties
                    .iter()
                    .find_map(|(id, party)| (party.leader == leader_id).then_some(*id))
                    .ok_or(WorldError::PartyUnavailable)?;
                let party = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                let expiry = *party
                    .invitations
                    .get(&actor_id)
                    .ok_or(WorldError::PartyForbidden)?;
                if expiry < self.logical_tick || party.members.len() >= 6 {
                    return Err(WorldError::PartyUnavailable);
                }
                party.invitations.remove(&actor_id);
                party.members.push(actor_id);
                party.members.sort_unstable();
                let audience = self.party_member_audience(party_id, &[]);
                self.finish_adventure_action(
                    session,
                    format!("You join {leader}'s party.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} joins an adventuring party."),
                    },
                )
            }
            PartyAction::Revoke(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                let removed = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?
                    .invitations
                    .remove(&target_id)
                    .is_some();
                if !removed {
                    return Err(WorldError::PartyUnavailable);
                }
                let mut audience = vec![actor_id, target_id];
                audience.sort_unstable();
                audience.dedup();
                self.finish_adventure_action(
                    session,
                    format!("You revoke {target}'s party invitation.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} revokes a party invitation."),
                    },
                )
            }
            PartyAction::Kick(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                if target_id == actor_id
                    || !self.adventure.parties[&party_id]
                        .members
                        .contains(&target_id)
                {
                    return Err(WorldError::PartyUnavailable);
                }
                let party = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                party.members.retain(|member| *member != target_id);
                party.invitations.remove(&target_id);
                let audience = self.party_member_audience(party_id, &[target_id]);
                self.finish_adventure_action(
                    session,
                    format!("You remove {target} from your party.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} removes a companion from the party."),
                    },
                )
            }
            PartyAction::Transfer(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                if target_id == actor_id
                    || !self.adventure.parties[&party_id]
                        .members
                        .contains(&target_id)
                {
                    return Err(WorldError::PartyUnavailable);
                }
                self.adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?
                    .leader = target_id;
                let audience = self.party_member_audience(party_id, &[]);
                self.finish_adventure_action(
                    session,
                    format!("You transfer party leadership to {target}.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} transfers party leadership."),
                    },
                )
            }
            PartyAction::Disband => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let audience = self.party_event_audience(party_id, &[]);
                self.adventure.parties.remove(&party_id);
                self.finish_adventure_action(
                    session,
                    "You disband your party.\r\n".into(),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} disbands an adventuring party."),
                    },
                )
            }
            PartyAction::Leave => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                let audience = self.party_event_audience(party_id, &[actor_id]);
                let mut remove_party = false;
                {
                    let party = self
                        .adventure
                        .parties
                        .get_mut(&party_id)
                        .ok_or(WorldError::PartyUnavailable)?;
                    party.members.retain(|member| *member != actor_id);
                    party.invitations.remove(&actor_id);
                    if party.members.is_empty() {
                        remove_party = true;
                    } else if party.leader == actor_id {
                        party.leader = party.members[0];
                    }
                }
                if remove_party {
                    self.adventure.parties.remove(&party_id);
                }
                self.finish_adventure_action(
                    session,
                    "You leave your party.\r\n".into(),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} leaves an adventuring party."),
                    },
                )
            }
            PartyAction::Status => {
                let output = if let Some(party_id) = self.party_for_member(actor_id) {
                    let party = &self.adventure.parties[&party_id];
                    let names = party
                        .members
                        .iter()
                        .map(|member| {
                            self.connected_character_name(*member)
                                .unwrap_or_else(|| "an absent companion".into())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Party members: {names}.\r\n")
                } else {
                    "You are not in a party.\r\n".into()
                };
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
        }
    }

    pub(crate) fn apply_quest(
        &mut self,
        session: SessionId,
        action: QuestAction,
    ) -> Result<ActionOutcome, WorldError> {
        const QUEST_ID: &str = "first-hunt";
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        match action {
            QuestAction::List => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some("Available quest: first-hunt — defeat one hostile creature; reward 25 coin and 50 experience.\r\n".into()),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            QuestAction::Status => {
                let state = self
                    .character_gameplay
                    .get(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let output = match state.quests.get(QUEST_ID) {
                    None => "The first-hunt quest has not been accepted.\r\n".into(),
                    Some(progress) => format!(
                        "First Hunt: {:?} ({}/{}).\r\n",
                        progress.status, progress.current, progress.target
                    ),
                };
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
            QuestAction::Accept(query) => {
                if !quest_matches(&query) {
                    return Err(WorldError::QuestUnavailable);
                }
                let state = self
                    .character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                if state.quests.contains_key(QUEST_ID) || state.quests.len() >= 32 {
                    return Err(WorldError::QuestUnavailable);
                }
                state.quests.insert(
                    QUEST_ID.into(),
                    QuestProgress {
                        status: QuestStatus::Active,
                        current: 0,
                        target: 1,
                    },
                );
                self.finish_adventure_action(
                    session,
                    "You accept First Hunt.\r\n".into(),
                    AdventureEvent::QuestChanged {
                        character_id,
                        quest_id: QUEST_ID.into(),
                        summary: "First Hunt accepted.".into(),
                    },
                )
            }
            QuestAction::Complete(query) => {
                if !quest_matches(&query) {
                    return Err(WorldError::QuestUnavailable);
                }
                let rewarded_experience = {
                    let state = self
                        .character_gameplay
                        .get_mut(&character_id)
                        .ok_or(WorldError::InvalidEntity)?;
                    let progress = state
                        .quests
                        .get_mut(QUEST_ID)
                        .ok_or(WorldError::QuestUnavailable)?;
                    if progress.status != QuestStatus::ReadyToClaim {
                        return Err(WorldError::QuestUnavailable);
                    }
                    state.coins = state
                        .coins
                        .checked_add(25)
                        .ok_or(WorldError::QuestUnavailable)?;
                    state.experience = state
                        .experience
                        .checked_add(50)
                        .ok_or(WorldError::QuestUnavailable)?;
                    progress.status = QuestStatus::Completed;
                    state.experience
                };
                let mut progression_events = vec![DomainEvent::Gameplay {
                    event: GameplayEvent::ExperienceGained {
                        character_id,
                        character_name: character_name.clone(),
                        amount: 50,
                        total: rewarded_experience,
                        room_id: room_id.clone(),
                    },
                }];
                self.advance_character_progression(
                    character_id,
                    &character_name,
                    &room_id,
                    &mut progression_events,
                )?;
                let mut outcome = self.finish_adventure_action(
                    session,
                    "First Hunt complete: you receive 25 coin and 50 experience.\r\n".into(),
                    AdventureEvent::QuestChanged {
                        character_id,
                        quest_id: QUEST_ID.into(),
                        summary: "First Hunt completed and its reward claimed.".into(),
                    },
                )?;
                outcome.domain_events.extend(progression_events);
                Ok(outcome)
            }
        }
    }

    pub(crate) fn apply_economy(
        &mut self,
        session: SessionId,
        action: EconomyAction,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let at_shop = room_checked(&self.entities, character.location)?.stable_id
            == self.start_room_stable_id;
        match action {
            EconomyAction::Balance => {
                let coins = self
                    .character_gameplay
                    .get(&character_id)
                    .ok_or(WorldError::InvalidEntity)?
                    .coins;
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!(
                        "You have {coins} coin; {} reserved and {} spendable.\r\n",
                        self.reserved_coins(character_id),
                        self.spendable_coins(character_id)?
                    )),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            EconomyAction::Shop => {
                if !at_shop {
                    return Err(WorldError::ShopUnavailable);
                }
                let output = commodity_specs()
                    .iter()
                    .map(|(id, label, buy, sell)| {
                        let stock = self.adventure.shop_stock.get(*id).copied().unwrap_or(0);
                        format!("- {label}: buy {buy}, sell {sell}, stock {stock}\r\n")
                    })
                    .collect::<String>();
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!("Lantern Market:\r\n{output}")),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            EconomyAction::Buy(query) => {
                if !at_shop {
                    return Err(WorldError::ShopUnavailable);
                }
                let (commodity_id, label, price, _) = resolve_commodity(&query)?;
                if self.spendable_coins(character_id)? < price {
                    return Err(WorldError::InsufficientFunds);
                }
                let stock = self
                    .adventure
                    .shop_stock
                    .get_mut(commodity_id)
                    .ok_or(WorldError::CommodityNotFound)?;
                if *stock == 0 {
                    return Err(WorldError::ShopUnavailable);
                }
                let state = self
                    .character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                if state.coins < price {
                    return Err(WorldError::InsufficientFunds);
                }
                let owned = state.commodities.entry(commodity_id.into()).or_default();
                if *owned >= 99 {
                    return Err(WorldError::ShopUnavailable);
                }
                state.coins -= price;
                *owned += 1;
                *stock -= 1;
                self.finish_adventure_action(
                    session,
                    format!("You buy {label} for {price} coin.\r\n"),
                    AdventureEvent::EconomyChanged {
                        character_id,
                        commodity_id: Some(commodity_id.into()),
                        summary: format!("A market purchase of {label} is completed."),
                    },
                )
            }
            EconomyAction::Sell(query) => {
                if !at_shop {
                    return Err(WorldError::ShopUnavailable);
                }
                let (commodity_id, label, _, price) = resolve_commodity(&query)?;
                let state = self
                    .character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let owned = state
                    .commodities
                    .get_mut(commodity_id)
                    .ok_or(WorldError::CommodityNotOwned)?;
                if *owned == 0 {
                    return Err(WorldError::CommodityNotOwned);
                }
                state.coins = state
                    .coins
                    .checked_add(price)
                    .ok_or(WorldError::ShopUnavailable)?;
                *owned -= 1;
                let stock = self
                    .adventure
                    .shop_stock
                    .entry(commodity_id.into())
                    .or_default();
                *stock = stock.saturating_add(1).min(100);
                self.finish_adventure_action(
                    session,
                    format!("You sell {label} for {price} coin.\r\n"),
                    AdventureEvent::EconomyChanged {
                        character_id,
                        commodity_id: Some(commodity_id.into()),
                        summary: format!("A market sale of {label} is completed."),
                    },
                )
            }
        }
    }

    pub(crate) fn finish_adventure_action(
        &mut self,
        session: SessionId,
        output: String,
        event: AdventureEvent,
    ) -> Result<ActionOutcome, WorldError> {
        let event = DomainEvent::Adventure { event };
        // Only take a mutable (copy-on-write, dirtying) borrow when a stale
        // cursor actually exists.
        if self
            .party_loot_cursors
            .keys()
            .any(|party_id| !self.adventure.parties.contains_key(party_id))
        {
            self.party_loot_cursors
                .retain(|party_id, _| self.adventure.parties.contains_key(party_id));
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: vec![event],
        })
    }

    pub(crate) fn party_for_member(&self, character_id: CharacterId) -> Option<u64> {
        self.adventure
            .parties
            .iter()
            .find_map(|(id, party)| party.members.contains(&character_id).then_some(*id))
    }

    pub(crate) fn party_event_audience(
        &self,
        party_id: u64,
        additional: &[CharacterId],
    ) -> Vec<CharacterId> {
        let mut audience = self
            .adventure
            .parties
            .get(&party_id)
            .map(|party| {
                party
                    .members
                    .iter()
                    .chain(party.invitations.keys())
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        audience.extend_from_slice(additional);
        audience.sort_unstable();
        audience.dedup();
        audience
    }

    pub(crate) fn party_member_audience(
        &self,
        party_id: u64,
        additional: &[CharacterId],
    ) -> Vec<CharacterId> {
        let mut audience = self
            .adventure
            .parties
            .get(&party_id)
            .map(|party| party.members.clone())
            .unwrap_or_default();
        audience.extend_from_slice(additional);
        audience.sort_unstable();
        audience.dedup();
        audience
    }

    pub(crate) fn resolve_connected_character(
        &self,
        query: &str,
    ) -> Result<CharacterId, WorldError> {
        let query = query.trim();
        let matches = self
            .connected_sessions()
            .into_iter()
            .filter_map(|session| self.character(session).ok())
            .filter(|character| character.name.eq_ignore_ascii_case(query))
            .map(|character| character.id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [id] => Ok(*id),
            _ => Err(WorldError::PartyTargetNotFound),
        }
    }

    pub(crate) fn connected_character_name(&self, character_id: CharacterId) -> Option<String> {
        self.session_for_character(character_id)
            .and_then(|session| self.character(session).ok())
            .map(|character| character.name.clone())
    }
}

pub(crate) fn quest_matches(query: &str) -> bool {
    matches!(
        query.trim().to_ascii_lowercase().as_str(),
        "first-hunt" | "first hunt" | "hunt"
    )
}

pub(crate) fn commodity_specs() -> [(&'static str, &'static str, u64, u64); 2] {
    [
        ("tonic", "healing tonic", 12, 6),
        ("trail-ration", "trail ration", 8, 4),
    ]
}

pub(crate) fn resolve_commodity(
    query: &str,
) -> Result<(&'static str, &'static str, u64, u64), WorldError> {
    let query = query.trim().to_ascii_lowercase();
    commodity_specs()
        .into_iter()
        .find(|(id, label, _, _)| {
            *id == query || label.eq_ignore_ascii_case(&query) || label.starts_with(&query)
        })
        .ok_or(WorldError::CommodityNotFound)
}

pub(crate) fn weather_label(weather: WeatherKind) -> &'static str {
    match weather {
        WeatherKind::Clear => "clear",
        WeatherKind::Rain => "rainy",
        WeatherKind::Wind => "windy",
        WeatherKind::Fog => "foggy",
    }
}

pub(crate) fn skill_label(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
