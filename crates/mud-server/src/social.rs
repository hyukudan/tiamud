use crate::{
    persistence::{
        CharacterStore, MAX_REPORT_REASON_BYTES, MAX_SOCIAL_REASON_BYTES, MailMessage, MailPage,
        MailSendResult, ModerationAction, SocialRelationKind, StaffAccess, StoreError,
    },
    world_actor::GameError,
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tiamud_core::{
    AccountId, CharacterId, ConnectedCharacter, MAIL_PUBLIC_ID_BYTES, MAX_MAIL_BODY_BYTES,
    MAX_MAIL_PAGE, MAX_MAIL_REPORT_REASON_BYTES, MAX_MAIL_SUBJECT_BYTES, MAX_MAIL_TARGET_BYTES,
    MailAction, SessionId, SocialAction, SocialChannelName, StaffCapability, StaffProfileKey,
    World, valid_mail_public_id, valid_mail_subject, valid_mail_target, valid_mail_visible_text,
};

pub const MAX_CHAT_MESSAGE_BYTES: usize = 512;
pub const MAX_SOCIAL_TARGET_BYTES: usize = 64;
pub const CHAT_BURST: u32 = 4;
pub const CHAT_MESSAGES_PER_SECOND: u32 = 1;
pub const MIN_MODERATION_DURATION_SECONDS: u64 = 60;
pub const MAX_MODERATION_DURATION_SECONDS: u64 = 365 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SocialChannel {
    Announcement,
    Say,
    Tell,
    Telepathy,
    Emote,
    Party,
    General,
    Newbie,
    Staff,
}

impl From<SocialChannelName> for SocialChannel {
    fn from(value: SocialChannelName) -> Self {
        match value {
            SocialChannelName::General => Self::General,
            SocialChannelName::Newbie => Self::Newbie,
            SocialChannelName::Staff => Self::Staff,
        }
    }
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
    last_tell_senders: HashMap<CharacterId, CharacterId>,
}

#[derive(Debug)]
pub struct SocialOutcome {
    pub output: String,
}

#[derive(Debug)]
pub struct PlannedSocialDelivery {
    pub session_id: SessionId,
    pub recipient_id: CharacterId,
    pub message: SocialMessage,
    pub reply_from: Option<CharacterId>,
}

#[derive(Debug)]
pub struct AppliedSocial {
    pub outcome: SocialOutcome,
    pub deliveries: Vec<PlannedSocialDelivery>,
    pub authority_update: Option<AccountId>,
    pub disconnects: Vec<(SessionId, CharacterId)>,
}

#[derive(Debug)]
pub struct AppliedMail {
    pub output: String,
    pub page: Option<MailPage>,
    pub message: Option<MailMessage>,
}

impl SocialRuntime {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            last_tell_senders: HashMap::new(),
        }
    }

    pub fn disconnected(&mut self, character_id: CharacterId) {
        self.buckets.remove(&character_id);
        self.last_tell_senders.remove(&character_id);
        self.last_tell_senders
            .retain(|_, sender| *sender != character_id);
    }

    pub fn record_delivered_tell(&mut self, recipient: CharacterId, sender: CharacterId) {
        self.last_tell_senders.insert(recipient, sender);
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

pub async fn apply_mail(
    runtime: &mut SocialRuntime,
    world: &World,
    store: &dyn CharacterStore,
    session: SessionId,
    action: MailAction,
) -> Result<AppliedMail, GameError> {
    let actor = world.character_snapshot(session)?;
    match action {
        MailAction::List { page } => {
            if !(1..=MAX_MAIL_PAGE).contains(&page) {
                return Err(StoreError::InvalidMailInput.into());
            }
            let page = store.mailbox_page(actor.id, u32::from(page)).await?;
            let output = render_mail_page(&page);
            Ok(AppliedMail {
                output,
                page: Some(page),
                message: None,
            })
        }
        MailAction::Read { mail_id } => {
            ensure_mail_id(&mail_id)?;
            let message = store.read_mail(actor.id, &mail_id).await?;
            let output = format!(
                "Mail {} from {}\r\nSubject: {}\r\nSent: {} · Expires: {}\r\n\r\n{}\r\n",
                message.summary.public_id,
                message.summary.sender_name,
                message.summary.subject,
                message.summary.created_at,
                message.summary.expires_at,
                message.body,
            );
            Ok(AppliedMail {
                output,
                page: None,
                message: Some(message),
            })
        }
        MailAction::Delete { mail_id } => {
            ensure_mail_id(&mail_id)?;
            store.delete_mail(actor.id, &mail_id).await?;
            Ok(AppliedMail {
                output: "Mail deleted.\r\n".to_owned(),
                page: None,
                message: None,
            })
        }
        MailAction::Send {
            target,
            subject,
            body,
        } => {
            let target = normalise_mail_text(&target, MAX_MAIL_TARGET_BYTES)?;
            let subject = normalise_mail_text(&subject, MAX_MAIL_SUBJECT_BYTES)?;
            let body = normalise_mail_text(&body, MAX_MAIL_BODY_BYTES)?;
            if !valid_mail_target(&target) || !valid_mail_subject(&subject) {
                return Err(StoreError::InvalidMailInput.into());
            }
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let public_id = random_mail_public_id();
            match store
                .send_mail(actor.id, &target, &public_id, &subject, &body)
                .await?
            {
                MailSendResult::Stored | MailSendResult::Suppressed => Ok(AppliedMail {
                    output: "Mail accepted.\r\n".to_owned(),
                    page: None,
                    message: None,
                }),
                MailSendResult::Unavailable => Err(GameError::MailRecipientUnavailable),
            }
        }
        MailAction::Report { mail_id, reason } => {
            ensure_mail_id(&mail_id)?;
            let reason = normalise_mail_text(&reason, MAX_MAIL_REPORT_REASON_BYTES)?;
            let report_id = store.report_mail(actor.id, &mail_id, &reason).await?;
            Ok(AppliedMail {
                output: format!("Mail reported for moderator review as report {report_id}.\r\n"),
                page: None,
                message: None,
            })
        }
    }
}

fn ensure_mail_id(value: &str) -> Result<(), StoreError> {
    valid_mail_public_id(value)
        .then_some(())
        .ok_or(StoreError::InvalidMailInput)
}

fn normalise_mail_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    valid_mail_visible_text(value, maximum)
        .then(|| value.to_owned())
        .ok_or(StoreError::InvalidMailInput)
}

fn random_mail_public_id() -> String {
    let mut random = [0_u8; 16];
    OsRng.fill_bytes(&mut random);
    let mut id = String::with_capacity(MAIL_PUBLIC_ID_BYTES);
    id.push('m');
    for byte in random {
        write!(&mut id, "{byte:02x}").expect("writing to a String cannot fail");
    }
    debug_assert!(valid_mail_public_id(&id));
    id
}

fn render_mail_page(page: &MailPage) -> String {
    let mut output = format!(
        "Mail page {}/{} ({} message(s), {} unread; capacity {}).\r\n",
        page.page, page.total_pages, page.total_messages, page.unread_count, page.capacity,
    );
    if page.messages.is_empty() {
        output.push_str("- No mail on this page.\r\n");
        return output;
    }
    for message in &page.messages {
        let state = if message.read { "read" } else { "unread" };
        writeln!(
            &mut output,
            "- {} [{state}] from {}: {} (sent {}, expires {})\r",
            message.public_id,
            message.sender_name,
            message.subject,
            message.created_at,
            message.expires_at,
        )
        .expect("writing to a String cannot fail");
    }
    output
}

pub async fn apply_social(
    runtime: &mut SocialRuntime,
    world: &mut World,
    store: &dyn CharacterStore,
    session: SessionId,
    actor_access: &StaffAccess,
    session_accesses: &HashMap<SessionId, StaffAccess>,
    action: SocialAction,
) -> Result<AppliedSocial, GameError> {
    let actor = world.character_snapshot(session)?;
    let mut deliveries = Vec::new();
    let mut authority_update = None;
    let mut disconnects = Vec::new();
    let outcome = match action {
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
            plan_allowed(
                &mut deliveries,
                store,
                actor.id,
                recipients,
                SocialMessage {
                    channel: SocialChannel::Say,
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
                None,
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("You say, \"{message}\"\r\n"),
            })
        }
        SocialAction::PartySay(message) => {
            let message = normalise_text(&message, MAX_CHAT_MESSAGE_BYTES)?;
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let members = world.connected_party_members(session)?;
            let recipients = members
                .iter()
                .filter(|candidate| candidate.character_id != actor.id);
            plan_allowed(
                &mut deliveries,
                store,
                actor.id,
                recipients,
                SocialMessage {
                    channel: SocialChannel::Party,
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
                None,
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("You tell your party, \"{message}\"\r\n"),
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
            plan_allowed(
                &mut deliveries,
                store,
                actor.id,
                std::iter::once(recipient),
                SocialMessage {
                    channel: SocialChannel::Tell,
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
                Some(actor.id),
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("You tell {}, \"{message}\"\r\n", recipient.name),
            })
        }
        SocialAction::Reply(message) => {
            let message = normalise_text(&message, MAX_CHAT_MESSAGE_BYTES)?;
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let recipient_id = runtime
                .last_tell_senders
                .get(&actor.id)
                .copied()
                .ok_or(StoreError::SocialTargetNotFound)?;
            let connected = world.connected_characters()?;
            let recipient = resolve_connected_id(&connected, recipient_id)?;
            plan_allowed(
                &mut deliveries,
                store,
                actor.id,
                std::iter::once(recipient),
                SocialMessage {
                    channel: SocialChannel::Tell,
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
                Some(actor.id),
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("You tell {}, \"{message}\"\r\n", recipient.name),
            })
        }
        SocialAction::ChannelList => {
            let joined = store
                .social_channel_subscriptions(actor.id)
                .await?
                .into_iter()
                .collect::<HashSet<_>>();
            let lines = SocialChannelName::ALL
                .into_iter()
                .map(|channel| {
                    let state = if !channel_allowed(Some(actor_access), channel) {
                        "unavailable"
                    } else if joined.contains(&channel) {
                        "joined"
                    } else {
                        "not joined"
                    };
                    format!("- {}: {state}", channel.as_str())
                })
                .collect::<Vec<_>>()
                .join("\r\n");
            Ok(SocialOutcome {
                output: format!("Channels:\r\n{lines}\r\n"),
            })
        }
        SocialAction::ChannelJoin(channel) => {
            ensure_channel_allowed(actor_access, channel)?;
            store
                .set_social_channel_subscription(actor.id, channel, true)
                .await?;
            Ok(SocialOutcome {
                output: format!("You join the {} channel.\r\n", channel.as_str()),
            })
        }
        SocialAction::ChannelLeave(channel) => {
            store
                .set_social_channel_subscription(actor.id, channel, false)
                .await?;
            Ok(SocialOutcome {
                output: format!("You leave the {} channel.\r\n", channel.as_str()),
            })
        }
        SocialAction::ChannelSay { channel, message } => {
            ensure_channel_allowed(actor_access, channel)?;
            let message = normalise_text(&message, MAX_CHAT_MESSAGE_BYTES)?;
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let joined = store.social_channel_subscriptions(actor.id).await?;
            if !joined.contains(&channel) {
                return Err(StoreError::SocialForbidden.into());
            }
            let subscribers = store
                .social_channel_subscribers(channel)
                .await?
                .into_iter()
                .collect::<HashSet<_>>();
            let connected = world.connected_characters()?;
            let recipients = connected.iter().filter(|candidate| {
                candidate.character_id != actor.id
                    && subscribers.contains(&candidate.character_id)
                    && channel_allowed(session_accesses.get(&candidate.session_id), channel)
            });
            plan_allowed(
                &mut deliveries,
                store,
                actor.id,
                recipients,
                SocialMessage {
                    channel: channel.into(),
                    sender: actor.name.clone(),
                    text: message.clone(),
                },
                None,
            )
            .await?;
            Ok(SocialOutcome {
                output: format!(
                    "You tell the {} channel, \"{message}\"\r\n",
                    channel.as_str()
                ),
            })
        }
        SocialAction::EmoteList => {
            let emotes = world.social_emotes();
            if emotes.is_empty() {
                Ok(SocialOutcome {
                    output: "No authored emotes are available.\r\n".to_owned(),
                })
            } else {
                let lines = emotes
                    .into_iter()
                    .map(|emote| {
                        let targeting = match emote.targeting {
                            tiamud_core::SocialEmoteTargeting::None => "no target",
                            tiamud_core::SocialEmoteTargeting::OptionalCharacter => {
                                "optional character"
                            }
                            tiamud_core::SocialEmoteTargeting::RequiredCharacter => {
                                "character required"
                            }
                        };
                        format!("- {}: {} ({targeting})", emote.command, emote.name)
                    })
                    .collect::<Vec<_>>()
                    .join("\r\n");
                Ok(SocialOutcome {
                    output: format!("Authored emotes:\r\n{lines}\r\n"),
                })
            }
        }
        SocialAction::Emote { command, target } => {
            let plan = world.emote_preflight(session, &command, target.as_deref())?;
            let rendered = world.render_emote(&plan)?;
            ensure_chat_allowed(runtime, store, actor.id).await?;

            let connected = world.connected_characters()?;
            if let (Some(target_session_id), Some(target_character_id), Some(target_text)) = (
                plan.target_session_id,
                plan.target_character_id,
                rendered.target,
            ) && let Some(recipient) = connected.iter().find(|candidate| {
                candidate.session_id == target_session_id
                    && candidate.character_id == target_character_id
            }) && store
                .social_delivery_allowed(actor.id, recipient.character_id)
                .await?
            {
                deliveries.push(PlannedSocialDelivery {
                    session_id: recipient.session_id,
                    recipient_id: recipient.character_id,
                    message: SocialMessage {
                        channel: SocialChannel::Emote,
                        sender: plan.actor_name.clone(),
                        text: target_text,
                    },
                    reply_from: None,
                });
            }

            let other_session_ids = plan.other_session_ids.into_iter().collect::<HashSet<_>>();
            plan_allowed(
                &mut deliveries,
                store,
                actor.id,
                connected
                    .iter()
                    .filter(|candidate| other_session_ids.contains(&candidate.session_id)),
                SocialMessage {
                    channel: SocialChannel::Emote,
                    sender: plan.actor_name,
                    text: rendered.others,
                },
                None,
            )
            .await?;
            Ok(SocialOutcome {
                output: format!("{}\r\n", rendered.actor),
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
                world,
                store,
                actor.id,
                actor_access,
                &target,
                duration_seconds,
                reason,
                false,
                &mut disconnects,
            )
            .await
        }
        SocialAction::Ban {
            target,
            duration_seconds,
            reason,
        } => {
            apply_moderation(
                world,
                store,
                actor.id,
                actor_access,
                &target,
                duration_seconds,
                reason,
                true,
                &mut disconnects,
            )
            .await
        }
        SocialAction::Unmute { target, reason } => {
            remove_moderation(store, actor.id, actor_access, &target, reason, false).await
        }
        SocialAction::Unban { target, reason } => {
            remove_moderation(store, actor.id, actor_access, &target, reason, true).await
        }
        SocialAction::SetRole {
            target,
            role,
            reason,
        } => {
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            let target = store.social_target(&target).await?;
            authorize_staff_target(
                store,
                actor_access,
                target.character_id,
                StaffCapability::StaffAssign,
            )
            .await?;
            store
                .moderate_character(
                    actor.id,
                    target.character_id,
                    ModerationAction::SetRole { role, reason },
                )
                .await?;
            authority_update = Some(target.account_id);
            Ok(SocialOutcome {
                output: format!("{} now has the {role:?} role.\r\n", target.character_name),
            })
        }
    }?;
    Ok(AppliedSocial {
        outcome,
        deliveries,
        authority_update,
        disconnects,
    })
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
    world: &mut World,
    store: &dyn CharacterStore,
    actor: CharacterId,
    actor_access: &StaffAccess,
    target_name: &str,
    duration_seconds: Option<u64>,
    reason: String,
    is_ban: bool,
    disconnects: &mut Vec<(SessionId, CharacterId)>,
) -> Result<SocialOutcome, GameError> {
    let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
    let expires_at = moderation_expiry(duration_seconds)?;
    let target = store.social_target(target_name).await?;
    let capability = if is_ban {
        StaffCapability::ModerationBan
    } else {
        StaffCapability::ModerationMute
    };
    authorize_staff_target(store, actor_access, target.character_id, capability).await?;
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
                disconnects.push((connected.session_id, connected.character_id));
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
    actor_access: &StaffAccess,
    target_name: &str,
    reason: String,
    is_unban: bool,
) -> Result<SocialOutcome, GameError> {
    let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
    let target = store.social_target(target_name).await?;
    let capability = if is_unban {
        StaffCapability::ModerationBan
    } else {
        StaffCapability::ModerationMute
    };
    authorize_staff_target(store, actor_access, target.character_id, capability).await?;
    store
        .moderate_character(
            actor,
            target.character_id,
            if is_unban {
                ModerationAction::Unban { reason }
            } else {
                ModerationAction::Unmute { reason }
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

pub(crate) async fn ensure_chat_allowed(
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

async fn plan_allowed<'a>(
    deliveries: &mut Vec<PlannedSocialDelivery>,
    store: &dyn CharacterStore,
    sender: CharacterId,
    recipients: impl Iterator<Item = &'a ConnectedCharacter>,
    message: SocialMessage,
    reply_from: Option<CharacterId>,
) -> Result<(), StoreError> {
    for recipient in recipients {
        if store
            .social_delivery_allowed(sender, recipient.character_id)
            .await?
        {
            deliveries.push(PlannedSocialDelivery {
                session_id: recipient.session_id,
                recipient_id: recipient.character_id,
                message: message.clone(),
                reply_from,
            });
        }
    }
    Ok(())
}

pub(crate) fn resolve_connected<'a>(
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

fn resolve_connected_id(
    connected: &[ConnectedCharacter],
    target: CharacterId,
) -> Result<&ConnectedCharacter, StoreError> {
    connected
        .iter()
        .find(|candidate| candidate.character_id == target)
        .ok_or(StoreError::SocialTargetNotFound)
}

fn channel_allowed(access: Option<&StaffAccess>, channel: SocialChannelName) -> bool {
    channel != SocialChannelName::Staff
        || access.is_some_and(|access| {
            matches!(
                access.profile,
                Some(
                    StaffProfileKey::Moderator
                        | StaffProfileKey::Administrator
                        | StaffProfileKey::Implementor
                )
            )
        })
}

fn ensure_channel_allowed(
    access: &StaffAccess,
    channel: SocialChannelName,
) -> Result<(), StoreError> {
    channel_allowed(Some(access), channel)
        .then_some(())
        .ok_or(StoreError::SocialForbidden)
}

async fn authorize_staff_target(
    store: &dyn CharacterStore,
    actor: &StaffAccess,
    target: CharacterId,
    capability: StaffCapability,
) -> Result<(), StoreError> {
    if !actor.capabilities.contains(&capability) {
        return Err(StoreError::SocialForbidden);
    }
    let target = store.staff_access(target).await?;
    let actor_rank = actor
        .profile
        .map(StaffProfileKey::authority_rank)
        .unwrap_or(0);
    let target_rank = target
        .profile
        .map(StaffProfileKey::authority_rank)
        .unwrap_or(0);
    if actor.account_id == target.account_id || actor_rank <= target_rank {
        Err(StoreError::SocialForbidden)
    } else {
        Ok(())
    }
}

pub(crate) fn normalise_text(value: &str, maximum: usize) -> Result<String, StoreError> {
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
            "Moderation: mute/unmute, ban/unban and role require the assigned capability.\r\n",
            "Every moderation action requires an explicit reason.\r\n",
            "Durations use m, h or d; use permanent for no expiry.\r\n"
        )
        .to_owned(),
        Some(topic) if topic.eq_ignore_ascii_case("social") => concat!(
            "Social: say <message>, tell <name> <message>, reply <message>, who.\r\n",
            "Telepathy: telepathy <ability-id> <name> <message> when the world grants it.\r\n",
            "Emotes: emotes lists authored gestures; emote <command> [character] performs one.\r\n",
            "Mail: mail list [page], mail read/delete <id>, mail send <name> | <subject> | <body>, mail report <id> | <reason>.\r\n",
            "Boards: board, board list/read/post/delete/report; posts are public and retained for 90 days.\r\n",
            "Party: party say/invite/accept/revoke/kick/transfer/disband/leave/status.\r\n",
            "Channels: channel list, channel join/leave <general|newbie|staff>, ",
            "channel say <channel> <message>. Staff requires a moderator, administrator or implementor profile.\r\n",
            "Safety: ignore/unignore, block/unblock and report <name> <reason>.\r\n"
        )
        .to_owned(),
        Some(topic) if topic.eq_ignore_ascii_case("trade") => concat!(
            "Trade: trade request <name>, trade accept, trade decline, trade status or trade cancel.\r\n",
            "Offer: trade offer item [quantity] <item>, trade remove item <item>, ",
            "trade offer coins <amount>.\r\n",
            "Review offered containers with trade contents <container> [page], then ",
            "trade confirm <state-revision>. Both participants must confirm the same revision.\r\n"
        )
        .to_owned(),
        Some(_) => "No help entry matches that topic. Try: help social or help trade.\r\n".to_owned(),
        None => concat!(
            "Movement: north/east/south/west/up/down, look.\r\n",
            "Items: inventory, take/drop/give [quantity], contents, put/get [quantity], examine, use, equip, unequip.\r\n",
            "Trade: request, offer, review and double-confirm a same-room exchange; help trade.\r\n",
            "Social: say, tell, reply, telepathy, emotes, mail, boards, party say, channels, who, ignore, block, report.\r\n",
            "More: help social, help trade.\r\n"
        )
        .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{collections::BTreeSet, time::Duration};
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

        assert_eq!(
            serde_json::to_value(SocialMessage {
                channel: SocialChannel::Telepathy,
                sender: "Aria".into(),
                text: "A private thought.".into(),
            })
            .unwrap(),
            json!({"channel": "telepathy", "sender": "Aria", "text": "A private thought."})
        );
        assert_eq!(
            serde_json::to_value(SocialMessage {
                channel: SocialChannel::Emote,
                sender: "Aria".into(),
                text: "Aria salutes Borin.".into(),
            })
            .unwrap(),
            json!({"channel": "emote", "sender": "Aria", "text": "Aria salutes Borin."})
        );
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

    #[test]
    fn trade_help_is_text_complete() {
        let help = help_text(Some("trade"));
        for command in [
            "trade request",
            "trade offer item",
            "trade offer coins",
            "trade contents",
            "trade confirm",
            "trade cancel",
        ] {
            assert!(help.contains(command), "missing {command:?} from {help:?}");
        }
    }

    #[test]
    fn helper_and_builder_profiles_cannot_enter_the_staff_channel() {
        for profile in [StaffProfileKey::Helper, StaffProfileKey::Builder] {
            let access = StaffAccess {
                account_id: AccountId(1),
                role: AccountRole::Admin,
                profile: Some(profile),
                lore_title: None,
                capabilities: BTreeSet::new(),
            };
            assert!(!channel_allowed(Some(&access), SocialChannelName::Staff));
        }

        let moderator = StaffAccess {
            account_id: AccountId(2),
            role: AccountRole::Player,
            profile: Some(StaffProfileKey::Moderator),
            lore_title: None,
            capabilities: BTreeSet::new(),
        };
        assert!(channel_allowed(Some(&moderator), SocialChannelName::Staff));
    }
}
