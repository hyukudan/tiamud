//! Canonical hashing and bounded text validation helpers.

use super::*;

pub(crate) fn canonical_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_be_bytes());
}

pub(crate) fn canonical_i64(digest: &mut Sha256, value: i64) {
    digest.update(value.to_be_bytes());
}

pub(crate) fn canonical_str(digest: &mut Sha256, value: &str) -> Result<(), WorldError> {
    let length = u32::try_from(value.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    digest.update(length.to_be_bytes());
    digest.update(value.as_bytes());
    Ok(())
}

pub(crate) fn canonical_trade_key(
    digest: &mut Sha256,
    key: &TradeOfferKey,
) -> Result<(), WorldError> {
    match key {
        TradeOfferKey::Fungible { prototype_id } => {
            digest.update([0]);
            canonical_str(digest, prototype_id)
        }
        TradeOfferKey::Unique { item_id } => {
            digest.update([1]);
            canonical_str(digest, item_id.as_str())
        }
    }
}

pub(crate) fn canonical_item_binding(digest: &mut Sha256, binding: ItemBinding) {
    match binding {
        ItemBinding::Unbound => digest.update([0]),
        ItemBinding::Character { character_id } => {
            digest.update([1]);
            canonical_i64(digest, character_id.0);
        }
    }
}

pub(crate) fn canonical_item_affixes(
    digest: &mut Sha256,
    affixes: &[AppliedItemAffix],
) -> Result<(), WorldError> {
    canonical_u64(
        digest,
        u64::try_from(affixes.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
    );
    for affix in affixes {
        canonical_str(digest, &affix.affix_id)?;
        canonical_str(digest, &affix.definition_digest_sha256)?;
        canonical_str(digest, &affix.pool_id)?;
        canonical_str(digest, &affix.pool_digest_sha256)?;
        canonical_str(digest, &affix.record_digest_sha256)?;
        canonical_str(digest, &affix.label)?;
        digest.update([match affix.position {
            AffixPosition::Prefix => 0,
            AffixPosition::Suffix => 1,
        }]);
        digest.update(affix.attack_bonus_delta.to_be_bytes());
        digest.update(affix.heal_amount_delta.to_be_bytes());
    }
    Ok(())
}

pub(crate) fn canonical_item_location(
    digest: &mut Sha256,
    location: &SharedItemLocation,
) -> Result<(), WorldError> {
    match location {
        SharedItemLocation::Staged => digest.update([0]),
        SharedItemLocation::Carried(character_id) => {
            digest.update([1]);
            canonical_i64(digest, character_id.0);
        }
        SharedItemLocation::Room(room_id) => {
            digest.update([2]);
            canonical_str(digest, room_id)?;
        }
        SharedItemLocation::Equipped(character_id, slot) => {
            digest.update([3, *slot as u8]);
            canonical_i64(digest, character_id.0);
        }
        SharedItemLocation::HeldByNpc(npc_id) => {
            digest.update([4]);
            canonical_str(digest, npc_id)?;
        }
        SharedItemLocation::Contained(container_id) => {
            digest.update([5]);
            canonical_str(digest, container_id.as_str())?;
        }
        SharedItemLocation::Consumed => digest.update([6]),
    }
    Ok(())
}

pub(crate) fn safe_definition_text(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= maximum_bytes
        && !trimmed
            .chars()
            .any(|character| character.is_control() || character == '\u{7f}')
}

pub(crate) fn validate_area_load_request(
    request: &AreaLoadRequest,
    generation_id: &AreaGenerationId,
) -> Result<(), WorldError> {
    if &request.generation_id != generation_id
        || !valid_definition_id(&request.source_room_id)
        || !valid_definition_id(&request.target_room_id)
        || !valid_definition_id(&request.target_area_id)
        || request.minimum_target_revision == 0
    {
        return Err(WorldError::InvalidAreaLifecycle);
    }
    Ok(())
}

pub fn valid_mail_public_id(value: &str) -> bool {
    value.len() == MAIL_PUBLIC_ID_BYTES
        && value.starts_with('m')
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

pub fn valid_mail_visible_text(value: &str, maximum_bytes: usize) -> bool {
    value == value.trim()
        && !value.is_empty()
        && value.len() <= maximum_bytes
        && !value
            .chars()
            .any(|character| character.is_control() || is_bidi_formatting(character))
}

pub fn valid_mail_target(value: &str) -> bool {
    valid_mail_visible_text(value, MAX_MAIL_TARGET_BYTES) && !value.contains('|')
}

pub fn valid_mail_subject(value: &str) -> bool {
    valid_mail_visible_text(value, MAX_MAIL_SUBJECT_BYTES) && !value.contains('|')
}

pub fn valid_board_public_id(value: &str) -> bool {
    value.len() == BOARD_PUBLIC_ID_BYTES
        && value.starts_with('b')
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

pub fn valid_board_visible_text(value: &str, maximum_bytes: usize) -> bool {
    value == value.trim()
        && !value.is_empty()
        && value.len() <= maximum_bytes
        && !value
            .chars()
            .any(|character| character.is_control() || is_bidi_formatting(character))
}

pub fn valid_board_subject(value: &str) -> bool {
    valid_board_visible_text(value, MAX_BOARD_SUBJECT_BYTES) && !value.contains('|')
}

pub(crate) fn is_bidi_formatting(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

pub(crate) fn valid_digest_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn canonical_sha256(value: &impl Serialize) -> Result<String, WorldError> {
    let bytes = serde_json::to_vec(value).map_err(|_| WorldError::InvalidGameplayDefinition)?;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").map_err(|_| WorldError::InvalidGameplayDefinition)?;
    }
    Ok(encoded)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppliedItemAffixRecord<'a> {
    pub(crate) affix_id: &'a str,
    pub(crate) definition_digest_sha256: &'a str,
    pub(crate) pool_id: &'a str,
    pub(crate) pool_digest_sha256: &'a str,
    pub(crate) label: &'a str,
    pub(crate) position: AffixPosition,
    pub(crate) attack_bonus_delta: i32,
    pub(crate) heal_amount_delta: i32,
}

pub(crate) fn applied_affix_record_sha256(affix: &AppliedItemAffix) -> Result<String, WorldError> {
    canonical_sha256(&AppliedItemAffixRecord {
        affix_id: &affix.affix_id,
        definition_digest_sha256: &affix.definition_digest_sha256,
        pool_id: &affix.pool_id,
        pool_digest_sha256: &affix.pool_digest_sha256,
        label: &affix.label,
        position: affix.position,
        attack_bonus_delta: affix.attack_bonus_delta,
        heal_amount_delta: affix.heal_amount_delta,
    })
}

pub(crate) fn affix_pool_applies(
    pool_id: &str,
    prototype: &ItemPrototype,
    pools: &BTreeMap<String, AffixPool>,
    affixes: &BTreeMap<String, AffixDefinition>,
) -> bool {
    prototype.stack_policy == ItemStackPolicy::Unique
        && prototype.max_stack_units == 1
        && prototype.sealed_loot_table_id.is_none()
        && pools.get(pool_id).is_some_and(|pool| {
            pool.entries.iter().all(|entry| match entry.kind {
                AffixPoolEntryKind::Nothing => true,
                AffixPoolEntryKind::Affix => entry
                    .affix_id
                    .as_ref()
                    .and_then(|id| affixes.get(id))
                    .is_some_and(|affix| {
                        (affix.attack_bonus_delta == 0 || prototype.equipment_slot.is_some())
                            && (affix.heal_amount_delta == 0 || prototype.consumable)
                            && prototype
                                .attack_bonus
                                .checked_add(affix.attack_bonus_delta)
                                .is_some_and(|value| (0..=MAX_STAT_VALUE_I32).contains(&value))
                            && prototype
                                .heal_amount
                                .checked_add(affix.heal_amount_delta)
                                .is_some_and(|value| (0..=MAX_STAT_VALUE_I32).contains(&value))
                    }),
            })
        })
}

pub(crate) fn safe_definition_multiline_text(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= maximum_bytes
        && !trimmed.chars().any(|character| {
            (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
                || character == '\u{7f}'
        })
}
