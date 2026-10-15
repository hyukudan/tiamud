//! Visibility, detection, spatial voice, emotes and telepathy.

use super::*;

impl World {
    pub(crate) fn resolve_visible_npc(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .as_str();
        let observer = self.observer(session)?;
        let query = query.trim().to_ascii_lowercase();
        let visible = self
            .npcs
            .values()
            .map(|npc| {
                let present = npc.alive() && npc.spawn_room_id == room_id;
                self.npc_visible_to(session, &npc.stable_id, room_id)
                    .map(|visible| (present && visible).then_some(npc))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if let Some(npc) = visible
            .iter()
            .find(|npc| opaque_id(observer.salt, "npc", &npc.stable_id) == query)
        {
            return Ok(npc.stable_id.clone());
        }
        let exact = visible
            .iter()
            .filter(|npc| npc.name.eq_ignore_ascii_case(&query))
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [npc] => return Ok(npc.stable_id.clone()),
            [_, ..] => return Err(WorldError::AmbiguousNpc),
            [] => {}
        }
        let prefix = visible
            .iter()
            .filter(|npc| {
                let name = npc.name.to_ascii_lowercase();
                name.starts_with(&query)
                    || name.split_whitespace().any(|word| word.starts_with(&query))
            })
            .collect::<Vec<_>>();
        match prefix.as_slice() {
            [npc] => Ok(npc.stable_id.clone()),
            [] => Err(WorldError::NpcNotFound),
            [_, ..] => Err(WorldError::AmbiguousNpc),
        }
    }

    pub(crate) fn item_visible_to(
        &self,
        session: SessionId,
        item_id: &ItemInstanceId,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        if !self.item_instances.contains_key(item_id) {
            return Err(WorldError::InvalidEntity);
        }
        self.can_detect(
            session,
            room_id,
            self.hidden_items
                .get(item_id.as_str())
                .copied()
                .unwrap_or(0),
        )
    }

    pub(crate) fn npc_visible_to(
        &self,
        session: SessionId,
        npc_id: &str,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        self.can_detect(
            session,
            room_id,
            self.hidden_npcs.get(npc_id).copied().unwrap_or(0),
        )
    }

    pub(crate) fn exit_visible_to(
        &self,
        session: SessionId,
        room_id: &str,
        direction: Direction,
    ) -> Result<bool, WorldError> {
        self.can_detect(
            session,
            room_id,
            self.secret_exits
                .get(&(room_id.to_owned(), direction))
                .copied()
                .unwrap_or(0),
        )
    }

    pub(crate) fn can_detect(
        &self,
        session: SessionId,
        room_id: &str,
        difficulty: u16,
    ) -> Result<bool, WorldError> {
        let observer = self.observer(session)?;
        if observer.role == AccountRole::Admin {
            return Ok(true);
        }
        let character = self.character(session)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let senses = perception_senses(gameplay);
        let tremorsense = senses.contains(&Sense::Tremorsense);
        let light = self
            .room_light
            .get(room_id)
            .copied()
            .unwrap_or(LightLevel::Bright);
        let usable_sense = tremorsense
            || match light {
                LightLevel::Bright | LightLevel::Dim => senses.contains(&Sense::Vision),
                LightLevel::Dark => senses.contains(&Sense::Darkvision),
            };
        if !usable_sense {
            return Ok(false);
        }
        if difficulty == 0 {
            return Ok(true);
        }
        let mut score = gameplay.skills.get("perception").copied().unwrap_or(0);
        if light == LightLevel::Dim && !tremorsense {
            score = score.saturating_sub(10);
        }
        Ok(score >= difficulty)
    }

    pub fn can_receive_visual_scene(&self, session: SessionId) -> Result<bool, WorldError> {
        if self.observer(session)?.role == AccountRole::Admin {
            return Ok(true);
        }
        let character = self.character(session)?;
        let room_id = &room_checked(&self.entities, character.location)?.stable_id;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let senses = perception_senses(gameplay);
        Ok(
            match self
                .room_light
                .get(room_id)
                .copied()
                .unwrap_or(LightLevel::Bright)
            {
                LightLevel::Bright | LightLevel::Dim => senses.contains(&Sense::Vision),
                LightLevel::Dark => senses.contains(&Sense::Darkvision),
            },
        )
    }

    pub(crate) fn can_receive_audio_scene(&self, session: SessionId) -> Result<bool, WorldError> {
        if self.observer(session)?.role == AccountRole::Admin {
            return Ok(true);
        }
        let character = self.character(session)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        Ok(perception_senses(gameplay).contains(&Sense::Hearing))
    }

    pub fn connected_sessions(&self) -> Vec<SessionId> {
        let mut sessions = self.sessions.keys().copied().collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.0);
        sessions
    }

    /// Server-internal presence data used to route social messages. Transports
    /// must project this into public names/text and never serialize the durable
    /// account or character identifiers directly.
    pub fn connected_characters(&self) -> Result<Vec<ConnectedCharacter>, WorldError> {
        self.connected_sessions()
            .into_iter()
            .map(|session_id| {
                let character = self.character(session_id)?;
                let room_id = room_checked(&self.entities, character.location)?
                    .stable_id
                    .clone();
                Ok(ConnectedCharacter {
                    session_id,
                    character_id: character.id,
                    account_id: character.account_id,
                    name: character.name.clone(),
                    room_id,
                })
            })
            .collect()
    }

    /// Returns the connected, hearing audience in the actor's current room.
    ///
    /// Deafness affects only reception: a deaf actor may still speak and gets
    /// the same private acknowledgement as any other actor.
    pub fn local_voice_audience(
        &self,
        session: SessionId,
    ) -> Result<Vec<ConnectedCharacter>, WorldError> {
        self.voice_audience(session, 0)
    }

    /// Returns the connected, hearing audience reached by a bounded yell.
    pub fn yell_audience(&self, session: SessionId) -> Result<Vec<ConnectedCharacter>, WorldError> {
        self.voice_audience(session, YELL_MAX_ROOM_HOPS)
    }

    /// Returns the connected, hearing audience reached by a bounded shout.
    pub fn shout_audience(
        &self,
        session: SessionId,
    ) -> Result<Vec<ConnectedCharacter>, WorldError> {
        self.voice_audience(session, SHOUT_MAX_ROOM_HOPS)
    }

    pub(crate) fn voice_audience(
        &self,
        session: SessionId,
        max_hops: u8,
    ) -> Result<Vec<ConnectedCharacter>, WorldError> {
        debug_assert!(max_hops <= SHOUT_MAX_ROOM_HOPS);
        let origin = self.character(session)?.location;
        room_checked(&self.entities, origin)?;
        let mut reachable = HashSet::from([origin]);
        let mut pending = VecDeque::from([(origin, 0_u8)]);

        while let Some((room_id, hops)) = pending.pop_front() {
            if hops >= max_hops {
                continue;
            }
            let room = room_checked(&self.entities, room_id)?;
            for (direction, target) in &room.exits {
                // Secret-exit authorship is authoritative here. Acoustic
                // propagation never uses observer perception and therefore
                // cannot reveal or traverse a secret connection.
                if self
                    .secret_exits
                    .contains_key(&(room.stable_id.clone(), *direction))
                {
                    continue;
                }
                let Some(target) = target.resident_target() else {
                    continue;
                };
                room_checked(&self.entities, target)?;
                if reachable.insert(target) {
                    if reachable.len() > MAX_SPATIAL_VOICE_ROOMS {
                        return Err(WorldError::InvalidEntity);
                    }
                    pending.push_back((target, hops + 1));
                }
            }
        }

        let reachable_stable_ids = reachable
            .into_iter()
            .map(|room_id| room_checked(&self.entities, room_id).map(|room| room.stable_id.clone()))
            .collect::<Result<HashSet<_>, _>>()?;
        self.connected_characters()?
            .into_iter()
            .filter(|candidate| reachable_stable_ids.contains(&candidate.room_id))
            .filter_map(
                |candidate| match self.character_can_hear_voice(candidate.character_id) {
                    Ok(true) => Some(Ok(candidate)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            .collect()
    }

    pub(crate) fn character_can_hear_voice(
        &self,
        character_id: CharacterId,
    ) -> Result<bool, WorldError> {
        let gameplay = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        Ok(perception_senses(gameplay).contains(&Sense::Hearing))
    }

    /// Resolves one authored local emote without mutating or persisting state.
    ///
    /// The returned room audience is a transient routing plan. A target must
    /// be a distinct, connected character in the actor's current room and is
    /// selected only by an exact, case-insensitive, unambiguous name.
    pub fn emote_preflight(
        &self,
        session: SessionId,
        command: &str,
        target: Option<&str>,
    ) -> Result<SocialEmotePlan, WorldError> {
        if !emote::valid_command(command)
            || target.is_some_and(|target| !emote::valid_target_query(target))
        {
            return Err(WorldError::EmoteUnavailable);
        }
        let definition = self
            .social_emotes
            .get(command)
            .ok_or(WorldError::EmoteUnavailable)?;
        match (definition.targeting, target) {
            (SocialEmoteTargeting::None, Some(_))
            | (SocialEmoteTargeting::RequiredCharacter, None) => {
                return Err(WorldError::EmoteTargetUnavailable);
            }
            (SocialEmoteTargeting::None | SocialEmoteTargeting::OptionalCharacter, None)
            | (
                SocialEmoteTargeting::OptionalCharacter | SocialEmoteTargeting::RequiredCharacter,
                Some(_),
            ) => {}
        }

        let actor = self.character(session)?;
        if !emote::valid_character_name(&actor.name) {
            return Err(WorldError::EmoteUnavailable);
        }
        let room_id = room_checked(&self.entities, actor.location)?
            .stable_id
            .clone();
        let connected = self.connected_characters()?;
        let target = if let Some(query) = target {
            let matches = connected
                .iter()
                .filter(|candidate| {
                    candidate.session_id != session
                        && candidate.room_id == room_id
                        && candidate.name.eq_ignore_ascii_case(query)
                })
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [target] if emote::valid_character_name(&target.name) => Some((*target).clone()),
                _ => return Err(WorldError::EmoteTargetUnavailable),
            }
        } else {
            None
        };
        let target_session_id = target.as_ref().map(|target| target.session_id);
        Ok(SocialEmotePlan {
            emote_id: definition.id.clone(),
            actor_session_id: session,
            actor_character_id: actor.id,
            actor_name: actor.name.clone(),
            target_session_id,
            target_character_id: target.as_ref().map(|target| target.character_id),
            target_name: target.as_ref().map(|target| target.name.clone()),
            room_id: room_id.clone(),
            other_session_ids: emote::other_sessions(
                &connected,
                &room_id,
                session,
                target_session_id,
            ),
        })
    }

    /// Renders a preflighted emote into bounded plain text per perspective.
    ///
    /// The plan is revalidated against current connections and room placement,
    /// so a stale or caller-forged destination cannot create cross-room output.
    pub fn render_emote(&self, plan: &SocialEmotePlan) -> Result<SocialEmoteRender, WorldError> {
        let definition = self
            .social_emotes
            .values()
            .find(|definition| definition.id == plan.emote_id)
            .ok_or(WorldError::EmoteUnavailable)?;
        let actor = self.character(plan.actor_session_id)?;
        let actor_room_id = room_checked(&self.entities, actor.location)?
            .stable_id
            .as_str();
        if actor.id != plan.actor_character_id
            || actor.name != plan.actor_name
            || actor_room_id != plan.room_id
        {
            return Err(WorldError::EmoteUnavailable);
        }
        let target_fields_present = usize::from(plan.target_session_id.is_some())
            + usize::from(plan.target_character_id.is_some())
            + usize::from(plan.target_name.is_some());
        if !matches!(target_fields_present, 0 | 3) {
            return Err(WorldError::EmoteTargetUnavailable);
        }
        let target_session_id = match (
            plan.target_session_id,
            plan.target_character_id,
            plan.target_name.as_deref(),
        ) {
            (Some(target_session_id), Some(target_character_id), Some(target_name)) => {
                let target = self.character(target_session_id)?;
                let target_room_id = room_checked(&self.entities, target.location)?
                    .stable_id
                    .as_str();
                if target_session_id == plan.actor_session_id
                    || target.id != target_character_id
                    || target.name != target_name
                    || target_room_id != plan.room_id
                {
                    return Err(WorldError::EmoteTargetUnavailable);
                }
                Some(target_session_id)
            }
            (None, None, None) => None,
            _ => return Err(WorldError::EmoteTargetUnavailable),
        };
        match (definition.targeting, target_session_id) {
            (SocialEmoteTargeting::None, None)
            | (SocialEmoteTargeting::OptionalCharacter, _)
            | (SocialEmoteTargeting::RequiredCharacter, Some(_)) => {}
            (SocialEmoteTargeting::None, Some(_))
            | (SocialEmoteTargeting::RequiredCharacter, None) => {
                return Err(WorldError::EmoteTargetUnavailable);
            }
        }
        let connected = self.connected_characters()?;
        if emote::other_sessions(
            &connected,
            &plan.room_id,
            plan.actor_session_id,
            target_session_id,
        ) != plan.other_session_ids
        {
            return Err(WorldError::EmoteUnavailable);
        }
        emote::render(definition, plan)
    }

    /// Validates one telepathy attempt without mutating resources or timers.
    ///
    /// The server still resolves the online destination and applies social
    /// policy. Core owns the authored ability, sender state, room interference
    /// and bounded input contract.
    pub fn telepathy_preflight(
        &self,
        session: SessionId,
        ability_id: &str,
        target: &str,
        message: &str,
    ) -> Result<TelepathyPlan, WorldError> {
        if !valid_telepathy_fields(ability_id, target, message) {
            return Err(WorldError::InvalidTelepathyRequest);
        }
        let sender = self.character(session)?;
        let room_id = room_checked(&self.entities, sender.location)?
            .stable_id
            .clone();
        let state = self
            .character_gameplay
            .get(&sender.id)
            .ok_or(WorldError::InvalidEntity)?;
        let ability = self
            .abilities
            .get(ability_id)
            .filter(|_| state.ability_ids.iter().any(|known| known == ability_id))
            .ok_or(WorldError::AbilityNotKnown)?;
        let communication = ability
            .communication
            .as_ref()
            .ok_or(WorldError::AbilityUnavailable)?;
        if state.vitals.health == 0
            || state.recovery_at_tick.is_some()
            || state.ability_global_ready_tick > self.logical_tick
            || state
                .statuses
                .iter()
                .any(|status| status.kind == ActiveStatusKind::Stun)
            || communication.blocked_room_ids.contains(&room_id)
            || state
                .ability_cooldowns
                .get(ability_id)
                .is_some_and(|ready| *ready > self.logical_tick)
        {
            return Err(WorldError::AbilityUnavailable);
        }
        if ability.resource_costs.iter().any(|(resource_id, cost)| {
            state
                .resources
                .get(resource_id)
                .is_none_or(|pool| pool.current < *cost)
        }) {
            return Err(WorldError::InsufficientResource);
        }
        Ok(TelepathyPlan {
            sender_character_id: sender.id,
            sender_name: sender.name.clone(),
            sender_room_id: room_id,
            ability_id: ability.id.clone(),
            ability_name: ability.name.clone(),
        })
    }

    /// Returns private routing eligibility for one already-resolved online
    /// destination. A single false value intentionally hides whether room
    /// interference, defeat/recovery or stun caused the rejection.
    pub fn telepathy_receptive(
        &self,
        target_session: SessionId,
        ability_id: &str,
    ) -> Result<bool, WorldError> {
        if ability_id.len() > MAX_TELEPATHY_ABILITY_ID_BYTES || !valid_definition_id(ability_id) {
            return Err(WorldError::AbilityNotKnown);
        }
        let communication = self
            .abilities
            .get(ability_id)
            .and_then(|ability| ability.communication.as_ref())
            .ok_or(WorldError::AbilityNotKnown)?;
        let target = self.character(target_session)?;
        let target_room_id = room_checked(&self.entities, target.location)?
            .stable_id
            .as_str();
        let state = self
            .character_gameplay
            .get(&target.id)
            .ok_or(WorldError::InvalidEntity)?;
        Ok(state.vitals.health > 0
            && state.recovery_at_tick.is_none()
            && !state
                .statuses
                .iter()
                .any(|status| status.kind == ActiveStatusKind::Stun)
            && !communication.blocked_room_ids.contains(target_room_id))
    }

    /// Atomically consumes one validated telepathy ability use.
    ///
    /// No destination, message body or `DomainEvent` is retained. The world
    /// actor persists the changed character/runtime before social delivery.
    pub fn apply_telepathy(
        &mut self,
        session: SessionId,
        ability_id: &str,
        target: &str,
        message: &str,
    ) -> Result<TelepathyPlan, WorldError> {
        let checkpoint = self.checkpoint();
        match self.apply_telepathy_inner(session, ability_id, target, message) {
            Ok(plan) => Ok(plan),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn apply_telepathy_inner(
        &mut self,
        session: SessionId,
        ability_id: &str,
        target: &str,
        message: &str,
    ) -> Result<TelepathyPlan, WorldError> {
        let plan = self.telepathy_preflight(session, ability_id, target, message)?;
        let ability = self
            .abilities
            .get(&plan.ability_id)
            .cloned()
            .ok_or(WorldError::AbilityNotKnown)?;
        let state = self
            .character_gameplay
            .get_mut(&plan.sender_character_id)
            .ok_or(WorldError::InvalidEntity)?;
        for (resource_id, cost) in &ability.resource_costs {
            let pool = state
                .resources
                .get_mut(resource_id)
                .ok_or(WorldError::InsufficientResource)?;
            pool.current = pool
                .current
                .checked_sub(*cost)
                .ok_or(WorldError::InsufficientResource)?;
        }
        state.ability_cooldowns.insert(
            ability.id,
            self.logical_tick
                .saturating_add(u64::from(ability.cooldown_ticks)),
        );
        state.ability_global_ready_tick = self.logical_tick.saturating_add(1);
        self.mark_authorized_script_change(session)?;
        Ok(plan)
    }
}

pub(crate) fn valid_telepathy_fields(ability_id: &str, target: &str, message: &str) -> bool {
    ability_id.len() <= MAX_TELEPATHY_ABILITY_ID_BYTES
        && valid_definition_id(ability_id)
        && target == target.trim()
        && message == message.trim()
        && safe_definition_text(target, MAX_TELEPATHY_TARGET_BYTES)
        && safe_definition_text(message, MAX_TELEPATHY_MESSAGE_BYTES)
}
