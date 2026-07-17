use super::{CharacterId, ConnectedCharacter, SessionId, WorldError, valid_definition_id};
use serde::{Deserialize, Serialize};

pub const MAX_SOCIAL_EMOTES: usize = 32;
pub const MAX_SOCIAL_EMOTE_ID_BYTES: usize = 64;
pub const MAX_SOCIAL_EMOTE_COMMAND_BYTES: usize = 32;
pub const MAX_SOCIAL_EMOTE_NAME_BYTES: usize = 64;
pub const MAX_SOCIAL_EMOTE_TARGET_BYTES: usize = 64;
pub const MAX_SOCIAL_EMOTE_RENDER_BYTES: usize = 512;

const ACTOR_PLACEHOLDER: &str = "{actor}";
const TARGET_PLACEHOLDER: &str = "{target}";
const MAX_CHARACTER_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialEmoteAudience {
    Room,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialEmoteTargeting {
    None,
    OptionalCharacter,
    RequiredCharacter,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteUntargetedTemplates {
    pub actor: String,
    pub others: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteTargetedTemplates {
    pub actor: String,
    pub target: String,
    pub others: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteTemplates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untargeted: Option<SocialEmoteUntargetedTemplates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted: Option<SocialEmoteTargetedTemplates>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteDefinition {
    pub id: String,
    pub command: String,
    pub name: String,
    pub audience: SocialEmoteAudience,
    pub targeting: SocialEmoteTargeting,
    pub templates: SocialEmoteTemplates,
}

/// Bounded authored metadata suitable for command discovery.
///
/// Templates are deliberately excluded: clients never render authoritative
/// social text or receive an authoring surface through this view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialEmoteView {
    pub id: String,
    pub command: String,
    pub name: String,
    pub audience: SocialEmoteAudience,
    pub targeting: SocialEmoteTargeting,
}

/// Transient, server-internal routing result for one local authored emote.
///
/// It is intentionally not serializable or part of a runtime snapshot. The
/// world actor may use the session audience to apply social policy before
/// delivering the separately rendered strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocialEmotePlan {
    pub emote_id: String,
    pub actor_session_id: SessionId,
    pub actor_character_id: CharacterId,
    pub actor_name: String,
    pub target_session_id: Option<SessionId>,
    pub target_character_id: Option<CharacterId>,
    pub target_name: Option<String>,
    pub room_id: String,
    pub other_session_ids: Vec<SessionId>,
}

/// Perspective-specific, bounded plain text for one preflighted emote.
///
/// The target text is present exactly when the plan has a distinct target.
/// This value is transient and intentionally has no serialization contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocialEmoteRender {
    pub actor: String,
    pub target: Option<String>,
    pub others: String,
}

pub(crate) fn validate_definition(definition: &SocialEmoteDefinition) -> bool {
    definition.id.len() <= MAX_SOCIAL_EMOTE_ID_BYTES
        && valid_definition_id(&definition.id)
        && valid_command(&definition.command)
        && safe_plain_text(&definition.name, MAX_SOCIAL_EMOTE_NAME_BYTES)
        && matches!(definition.audience, SocialEmoteAudience::Room)
        && match definition.targeting {
            SocialEmoteTargeting::None => {
                definition.templates.targeted.is_none()
                    && definition
                        .templates
                        .untargeted
                        .as_ref()
                        .is_some_and(|templates| {
                            validate_template(&templates.actor, 0, 0)
                                && validate_template(&templates.others, 1, 0)
                        })
            }
            SocialEmoteTargeting::OptionalCharacter => {
                definition
                    .templates
                    .untargeted
                    .as_ref()
                    .is_some_and(|templates| {
                        validate_template(&templates.actor, 0, 0)
                            && validate_template(&templates.others, 1, 0)
                    })
                    && definition
                        .templates
                        .targeted
                        .as_ref()
                        .is_some_and(|templates| {
                            validate_template(&templates.actor, 0, 1)
                                && validate_template(&templates.target, 1, 0)
                                && validate_template(&templates.others, 1, 1)
                        })
            }
            SocialEmoteTargeting::RequiredCharacter => {
                definition.templates.untargeted.is_none()
                    && definition
                        .templates
                        .targeted
                        .as_ref()
                        .is_some_and(|templates| {
                            validate_template(&templates.actor, 0, 1)
                                && validate_template(&templates.target, 1, 0)
                                && validate_template(&templates.others, 1, 1)
                        })
            }
        }
}

pub(crate) fn valid_command(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_SOCIAL_EMOTE_COMMAND_BYTES
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

pub(crate) fn valid_target_query(value: &str) -> bool {
    value == value.trim() && safe_literal(value, MAX_SOCIAL_EMOTE_TARGET_BYTES)
}

pub(crate) fn valid_character_name(value: &str) -> bool {
    safe_literal(value, MAX_CHARACTER_NAME_BYTES)
}

pub(crate) fn render(
    definition: &SocialEmoteDefinition,
    plan: &SocialEmotePlan,
) -> Result<SocialEmoteRender, WorldError> {
    if !valid_character_name(&plan.actor_name) {
        return Err(WorldError::EmoteUnavailable);
    }
    let (actor, target, others) = match plan.target_name.as_deref() {
        None => {
            let templates = definition
                .templates
                .untargeted
                .as_ref()
                .ok_or(WorldError::EmoteTargetUnavailable)?;
            (
                templates.actor.clone(),
                None,
                substitute(&templates.others, &plan.actor_name, None)?,
            )
        }
        Some(target_name) => {
            if !valid_character_name(target_name) {
                return Err(WorldError::EmoteTargetUnavailable);
            }
            let templates = definition
                .templates
                .targeted
                .as_ref()
                .ok_or(WorldError::EmoteTargetUnavailable)?;
            (
                substitute(&templates.actor, &plan.actor_name, Some(target_name))?,
                Some(substitute(
                    &templates.target,
                    &plan.actor_name,
                    Some(target_name),
                )?),
                substitute(&templates.others, &plan.actor_name, Some(target_name))?,
            )
        }
    };
    if actor.len() > MAX_SOCIAL_EMOTE_RENDER_BYTES
        || target
            .as_ref()
            .is_some_and(|text| text.len() > MAX_SOCIAL_EMOTE_RENDER_BYTES)
        || others.len() > MAX_SOCIAL_EMOTE_RENDER_BYTES
    {
        return Err(WorldError::EmoteUnavailable);
    }
    Ok(SocialEmoteRender {
        actor,
        target,
        others,
    })
}

pub(crate) fn view(definition: &SocialEmoteDefinition) -> SocialEmoteView {
    SocialEmoteView {
        id: definition.id.clone(),
        command: definition.command.clone(),
        name: definition.name.clone(),
        audience: definition.audience,
        targeting: definition.targeting,
    }
}

pub(crate) fn other_sessions(
    connected: &[ConnectedCharacter],
    room_id: &str,
    actor: SessionId,
    target: Option<SessionId>,
) -> Vec<SessionId> {
    let mut sessions = connected
        .iter()
        .filter(|character| {
            character.room_id == room_id
                && character.session_id != actor
                && Some(character.session_id) != target
        })
        .map(|character| character.session_id)
        .collect::<Vec<_>>();
    sessions.sort_by_key(|session| session.0);
    sessions
}

fn safe_literal(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && value == trimmed
        && value.len() <= maximum_bytes
        && !value
            .chars()
            .any(|character| character.is_control() || character == '\u{7f}')
}

fn safe_plain_text(value: &str, maximum_bytes: usize) -> bool {
    safe_literal(value, maximum_bytes)
        && !value.bytes().any(|byte| matches!(byte, b'<' | b'>' | b'&'))
}

fn validate_template(value: &str, expected_actor: usize, expected_target: usize) -> bool {
    if !safe_literal(value, MAX_SOCIAL_EMOTE_RENDER_BYTES)
        || value.bytes().any(|byte| matches!(byte, b'<' | b'>' | b'&'))
        || value.matches(ACTOR_PLACEHOLDER).count() != expected_actor
        || value.matches(TARGET_PLACEHOLDER).count() != expected_target
    {
        return false;
    }
    let remainder = value
        .replace(ACTOR_PLACEHOLDER, "")
        .replace(TARGET_PLACEHOLDER, "");
    if remainder.contains(['{', '}']) {
        return false;
    }
    let worst_case_bytes = remainder
        .len()
        .saturating_add(expected_actor.saturating_mul(MAX_CHARACTER_NAME_BYTES))
        .saturating_add(expected_target.saturating_mul(MAX_CHARACTER_NAME_BYTES));
    worst_case_bytes <= MAX_SOCIAL_EMOTE_RENDER_BYTES
}

fn substitute(template: &str, actor: &str, target: Option<&str>) -> Result<String, WorldError> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        output.push_str(&remaining[..open]);
        let token = &remaining[open..];
        if let Some(rest) = token.strip_prefix(ACTOR_PLACEHOLDER) {
            output.push_str(actor);
            remaining = rest;
        } else if let Some(rest) = token.strip_prefix(TARGET_PLACEHOLDER) {
            output.push_str(target.ok_or(WorldError::EmoteTargetUnavailable)?);
            remaining = rest;
        } else {
            return Err(WorldError::EmoteUnavailable);
        }
    }
    output.push_str(remaining);
    Ok(output)
}
