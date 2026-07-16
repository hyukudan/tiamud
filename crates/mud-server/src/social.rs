use crate::{
    delivery::DeliveryRouter,
    persistence::{
        CharacterStore, MAX_REPORT_REASON_BYTES, MAX_SOCIAL_REASON_BYTES, ModerationAction,
        SocialRelationKind, StoreError,
    },
    world_actor::GameError,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tiamud_core::{CharacterId, ConnectedCharacter, SessionId, SocialAction, World};

pub const MAX_CHAT_MESSAGE_BYTES: usize = 512;
pub const MAX_SOCIAL_TARGET_BYTES: usize = 64;
pub const CHAT_BURST: u32 = 4;
pub const CHAT_MESSAGES_PER_SECOND: u32 = 1;
pub const MIN_MODERATION_DURATION_SECONDS: u64 = 60;
pub const MAX_MODERATION_DURATION_SECONDS: u64 = 365 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SocialChannel {
    Say,
    Tell,
}

/// This is the complete client-visible social projection. Durable IDs,
/// account role, block state, reports, moderation reasons and audit data are
/// intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialMessage {
    pub channel: SocialChannel,
    pub sender: String,
    pub text: String,
}

#[derive(Debug)]
struct ChatBucket {
    tokens: f64,
    updated_at: Instant,
}

#[derive(Debug)]
pub struct SocialRuntime {
    buckets: HashMap<CharacterId, ChatBucket>,
}

#[derive(Debug)]
pub struct SocialOutcome {
    pub output: String,
}

impl SocialRuntime {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    pub fn disconnected(&mut self, character_id: CharacterId) {
        self.buckets.remove(&character_id);
    }

    fn chat_allowed(&mut self, character_id: CharacterId, now: Instant) -> bool {
        let bucket = self.buckets.entry(character_id).or_insert(ChatBucket {
            tokens: f64::from(CHAT_BURST),
            updated_at: now,
        });
        let elapsed = now
            .saturating_duration_since(bucket.updated_at)
            .as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * f64::from(CHAT_MESSAGES_PER_SECOND))
            .min(f64::from(CHAT_BURST));
        bucket.updated_at = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }
}

pub async fn apply_social(
    runtime: &mut SocialRuntime,
    delivery: &mut DeliveryRouter,
    world: &mut World,
    store: &dyn CharacterStore,
    session: SessionId,
    action: SocialAction,
) -> Result<SocialOutcome, GameError> {
    let actor = world.character_snapshot(session)?;
    match action {
        SocialAction::Say(message) => {
            let message = normalise_text(&message, MAX_CHAT_MESSAGE_BYTES)?;
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let connected = world.connected_characters()?;
            let actor_presence = connected
                .iter()
                .find(|candidate| candidate.session_id == session)
                .ok_or(tiamud_core::WorldError::UnknownSession)?;
            let recipients = connected.iter().filter(|candidate| {
                candidate.character_id != actor.id && candidate.room_id == actor_presence.room_id
            });
            deliver_allowed(
                delivery,
                store,
                actor.id,
                recipients,
                SocialMessage {
                    channel: SocialChannel::Say,
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("You say, \"{message}\"\r\n"),
            })
        }
        SocialAction::Tell { target, message } => {
            let target = normalise_text(&target, MAX_SOCIAL_TARGET_BYTES)?;
            let message = normalise_text(&message, MAX_CHAT_MESSAGE_BYTES)?;
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let connected = world.connected_characters()?;
            let recipient = resolve_connected(&connected, &target)?;
            if recipient.character_id == actor.id {
                return Err(StoreError::SocialSelfTarget.into());
            }
            // A blocked or ignored delivery is acknowledged identically to a
            // delivered tell so policy state cannot be probed remotely.
            deliver_allowed(
                delivery,
                store,
                actor.id,
                std::iter::once(recipient),
                SocialMessage {
                    channel: SocialChannel::Tell,
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("You tell {}, \"{message}\"\r\n", recipient.name),
            })
        }
        SocialAction::Who => {
            let mut names = world
                .connected_characters()?
                .into_iter()
                .map(|character| character.name)
                .collect::<Vec<_>>();
            names.sort_by_key(|name| name.to_ascii_lowercase());
            names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
            Ok(SocialOutcome {
                output: format!(
                    "Adventurers online ({}):\r\n{}\r\n",
                    names.len(),
                    names
                        .into_iter()
                        .map(|name| format!("- {name}"))
                        .collect::<Vec<_>>()
                        .join("\r\n")
                ),
            })
        }
        SocialAction::Help(topic) => Ok(SocialOutcome {
            output: help_text(topic.as_deref()),
        }),
        SocialAction::Ignore(target) => {
            change_relation(
                store,
                actor.id,
                &target,
                SocialRelationKind::Ignore,
                true,
                "ignored",
            )
            .await
        }
        SocialAction::Unignore(target) => {
            change_relation(
                store,
                actor.id,
                &target,
                SocialRelationKind::Ignore,
                false,
                "unignored",
            )
            .await
        }
        SocialAction::Block(target) => {
            change_relation(
                store,
                actor.id,
                &target,
                SocialRelationKind::Block,
                true,
                "blocked",
            )
            .await
        }
        SocialAction::Unblock(target) => {
            change_relation(
                store,
                actor.id,
                &target,
                SocialRelationKind::Block,
                false,
                "unblocked",
            )
            .await
        }
        SocialAction::Report { target, reason } => {
            let reason = normalise_text(&reason, MAX_REPORT_REASON_BYTES)?;
            let target = store.social_target(&target).await?;
            store
                .report_character(actor.id, target.character_id, &reason)
                .await?;
            Ok(SocialOutcome {
                output: "Your report has been recorded for moderator review.\r\n".to_owned(),
            })
        }
        SocialAction::Mute {
            target,
            duration_seconds,
            reason,
        } => {
            apply_moderation(
                runtime,
                world,
                store,
                actor.id,
                &target,
                duration_seconds,
                reason,
                false,
            )
            .await
        }
        SocialAction::Ban {
            target,
            duration_seconds,
            reason,
        } => {
            apply_moderation(
                runtime,
                world,
                store,
                actor.id,
                &target,
                duration_seconds,
                reason,
                true,
            )
            .await
        }
        SocialAction::Unmute(target) => remove_moderation(store, actor.id, &target, false).await,
        SocialAction::Unban(target) => remove_moderation(store, actor.id, &target, true).await,
        SocialAction::SetRole { target, role } => {
            let target = store.social_target(&target).await?;
            store
                .moderate_character(
                    actor.id,
                    target.character_id,
                    ModerationAction::SetRole(role),
                )
                .await?;
            Ok(SocialOutcome {
                output: format!("{} now has the {role:?} role.\r\n", target.character_name),
            })
        }
    }
}

async fn change_relation(
    store: &dyn CharacterStore,
    actor: CharacterId,
    target_name: &str,
    relation: SocialRelationKind,
    enabled: bool,
    verb: &str,
) -> Result<SocialOutcome, GameError> {
    let target = store.social_target(target_name).await?;
    store
        .set_social_relation(actor, target.character_id, relation, enabled)
        .await?;
    Ok(SocialOutcome {
        output: format!("{} has been {verb}.\r\n", target.character_name),
    })
}

#[allow(clippy::too_many_arguments)]
async fn apply_moderation(
    runtime: &mut SocialRuntime,
    world: &mut World,
    store: &dyn CharacterStore,
    actor: CharacterId,
    target_name: &str,
    duration_seconds: Option<u64>,
    reason: String,
    is_ban: bool,
) -> Result<SocialOutcome, GameError> {
    let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
    let expires_at = moderation_expiry(duration_seconds)?;
    let target = store.social_target(target_name).await?;
    let moderation = if is_ban {
        ModerationAction::Ban { expires_at, reason }
    } else {
        ModerationAction::Mute { expires_at, reason }
    };
    store
        .moderate_character(actor, target.character_id, moderation)
        .await?;
    if is_ban {
        for connected in world.connected_characters()? {
            if connected.account_id == target.account_id {
                runtime.disconnected(connected.character_id);
                world.disconnect(connected.session_id);
            }
        }
    }
    Ok(SocialOutcome {
        output: format!(
            "{} has been {}.\r\n",
            target.character_name,
            if is_ban { "banned" } else { "muted" }
        ),
    })
}

async fn remove_moderation(
    store: &dyn CharacterStore,
    actor: CharacterId,
    target_name: &str,
    is_unban: bool,
) -> Result<SocialOutcome, GameError> {
    let target = store.social_target(target_name).await?;
    store
        .moderate_character(
            actor,
            target.character_id,
            if is_unban {
                ModerationAction::Unban
            } else {
                ModerationAction::Unmute
            },
        )
        .await?;
    Ok(SocialOutcome {
        output: format!(
            "{} has been {}.\r\n",
            target.character_name,
            if is_unban { "unbanned" } else { "unmuted" }
        ),
    })
}

async fn ensure_chat_allowed(
    runtime: &mut SocialRuntime,
    store: &dyn CharacterStore,
    actor: CharacterId,
) -> Result<(), StoreError> {
    if store.social_profile(actor).await?.muted {
        return Err(StoreError::ChatMuted);
    }
    if !runtime.chat_allowed(actor, Instant::now()) {
        return Err(StoreError::ChatRateLimited);
    }
    Ok(())
}

async fn deliver_allowed<'a>(
    delivery: &mut DeliveryRouter,
    store: &dyn CharacterStore,
    sender: CharacterId,
    recipients: impl Iterator<Item = &'a ConnectedCharacter>,
    message: SocialMessage,
) -> Result<(), StoreError> {
    for recipient in recipients {
        if store
            .social_delivery_allowed(sender, recipient.character_id)
            .await?
        {
            delivery.send_social(recipient.session_id, message.clone());
        }
    }
    Ok(())
}

fn resolve_connected<'a>(
    connected: &'a [ConnectedCharacter],
    target: &str,
) -> Result<&'a ConnectedCharacter, StoreError> {
    let matches = connected
        .iter()
        .filter(|candidate| candidate.name.eq_ignore_ascii_case(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(*target),
        [] => Err(StoreError::SocialTargetNotFound),
        [_, ..] => Err(StoreError::AmbiguousSocialTarget),
    }
}

fn normalise_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(value.to_owned())
    }
}

fn moderation_expiry(duration_seconds: Option<u64>) -> Result<Option<i64>, StoreError> {
    let Some(duration_seconds) = duration_seconds else {
        return Ok(None);
    };
    if !(MIN_MODERATION_DURATION_SECONDS..=MAX_MODERATION_DURATION_SECONDS)
        .contains(&duration_seconds)
    {
        return Err(StoreError::InvalidSocialInput);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.checked_add(duration_seconds)
        .and_then(|expires| i64::try_from(expires).ok())
        .map(Some)
        .ok_or(StoreError::InvalidSocialInput)
}

fn help_text(topic: Option<&str>) -> String {
    match topic.map(str::trim).filter(|topic| !topic.is_empty()) {
        Some(topic) if topic.eq_ignore_ascii_case("moderation") => concat!(
            "Moderation: mute/unmute, ban/unban and role require the appropriate role.\r\n",
            "Durations use m, h or d; use permanent for no expiry.\r\n"
        )
        .to_owned(),
        Some(topic) if topic.eq_ignore_ascii_case("social") => concat!(
            "Social: say <message>, tell <name> <message>, who, ignore/unignore, ",
            "block/unblock and report <name> <reason>.\r\n"
        )
        .to_owned(),
        Some(_) => "No help entry matches that topic. Try: help social.\r\n".to_owned(),
        None => concat!(
            "Movement: north/east/south/west/up/down, look.\r\n",
            "Items: inventory, take, drop, examine, use, equip, unequip.\r\n",
            "Social: say, tell, who, ignore, unignore, block, unblock, report.\r\n",
            "More: help social.\r\n"
        )
        .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tiamud_core::{AccountId, AccountRole};

    #[test]
    fn projected_chat_contains_no_durable_or_moderation_state() {
        let message = SocialMessage {
            channel: SocialChannel::Tell,
            sender: "Aria".into(),
            text: "Meet at the gate.".into(),
        };
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(
            value,
            json!({"channel": "tell", "sender": "Aria", "text": "Meet at the gate."})
        );
        let encoded = value.to_string();
        for secret in [
            format!("{:?}", CharacterId(77)),
            format!("{:?}", AccountId(8)),
            format!("{:?}", AccountRole::Admin),
            "blocked".to_owned(),
        ] {
            assert!(!encoded.contains(&secret));
        }
    }

    #[test]
    fn chat_bucket_has_an_explicit_burst_and_refill() {
        let mut runtime = SocialRuntime::new();
        let character = CharacterId(1);
        let start = Instant::now();
        for _ in 0..CHAT_BURST {
            assert!(runtime.chat_allowed(character, start));
        }
        assert!(!runtime.chat_allowed(character, start));
        assert!(runtime.chat_allowed(character, start + Duration::from_secs(1)));
        runtime.disconnected(character);
        assert!(runtime.chat_allowed(character, start + Duration::from_secs(1)));
    }

    #[test]
    fn moderation_duration_is_strictly_bounded() {
        assert!(moderation_expiry(Some(59)).is_err());
        assert!(moderation_expiry(Some(60)).unwrap().is_some());
        assert!(moderation_expiry(Some(MAX_MODERATION_DURATION_SECONDS + 1)).is_err());
        assert_eq!(moderation_expiry(None).unwrap(), None);
    }
}
