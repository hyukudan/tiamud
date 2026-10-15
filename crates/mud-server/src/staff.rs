use crate::{
    persistence::{
        CharacterStore, MAX_SOCIAL_REASON_BYTES, StaffAccess, StaffAuditEntry, StaffAuditRecord,
        StaffReport, StoreError,
    },
    world_actor::{GameError, reconcile_world_trades},
};
use serde_json::{Value, json};
use tiamud_core::{
    AccountId, AccountRole, ConnectedCharacter, DomainEvent, SessionId, StaffAction,
    StaffCapability, StaffProfileKey, World,
};

const MAX_STAFF_TARGET_BYTES: usize = 64;
const MAX_STAFF_MESSAGE_BYTES: usize = 512;
const MAX_STAFF_TITLE_BYTES: usize = 128;
/// Leaves ample room for the surrounding protocol envelope and authoritative
/// state in the same bounded transport response.
const MAX_STAFF_OUTPUT_BYTES: usize = 8 * 1024;

#[derive(Debug)]
pub struct AppliedStaff {
    pub output: String,
    pub domain_events: Vec<DomainEvent>,
    pub refresh_account: Option<AccountId>,
    pub announcement: Option<String>,
}

/// Converts canonical staff authority into the legacy presentation role used
/// by concealment and scripted presentation. The role is derived cache state;
/// it never authorizes a staff command.
pub fn derived_role(access: &StaffAccess) -> AccountRole {
    match access.profile {
        Some(StaffProfileKey::Administrator | StaffProfileKey::Implementor) => AccountRole::Admin,
        Some(StaffProfileKey::Moderator) => AccountRole::Moderator,
        Some(StaffProfileKey::Helper | StaffProfileKey::Builder) | None => AccountRole::Player,
    }
}

pub async fn apply_staff(
    world: &mut World,
    store: &dyn CharacterStore,
    session: SessionId,
    action: StaffAction,
) -> Result<AppliedStaff, GameError> {
    let actor = world.character_snapshot(session)?;
    let access = store.staff_access(actor.id).await?;

    match action {
        StaffAction::Status => Ok(AppliedStaff {
            output: status_output(&access),
            domain_events: Vec::new(),
            refresh_account: None,
            announcement: None,
        }),
        StaffAction::Reports { reason } => {
            ensure_capability(&access, StaffCapability::ReportsView)?;
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            let reports = store
                .staff_reports(audit_record(
                    &access,
                    None,
                    StaffCapability::ReportsView,
                    "reports_view",
                    reason,
                    json!({"actor_name": actor.name}),
                ))
                .await?;
            Ok(read_only(reports_output(&reports)))
        }
        StaffAction::ResolveReport {
            report_id,
            resolution,
            reason,
        } => {
            ensure_capability(&access, StaffCapability::ReportsResolve)?;
            if report_id <= 0 {
                return Err(StoreError::StaffReportUnavailable(report_id).into());
            }
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            let audit = audit_record(
                &access,
                None,
                StaffCapability::ReportsResolve,
                "report_resolve",
                reason.clone(),
                json!({
                    "actor_name": actor.name,
                    "report_id": report_id,
                    "resolution": resolution.as_str(),
                }),
            );
            store
                .resolve_staff_report(report_id, resolution, &reason, audit)
                .await?;
            Ok(read_only(format!(
                "Report #{report_id} resolved as {}.\r\n",
                resolution.as_str()
            )))
        }
        StaffAction::Inspect { target, reason } => {
            let target = connected_target(world, &target)?;
            let reason = authorize_target(
                store,
                &access,
                &target,
                StaffCapability::StaffInspect,
                reason,
            )
            .await?;
            let output = world.staff_inspection_as_plain_text(target.session_id)?;
            store
                .append_staff_audit(audit_record(
                    &access,
                    Some(target.account_id),
                    StaffCapability::StaffInspect,
                    "staff_inspect",
                    reason,
                    staff_target_details(&actor.name, &target.name),
                ))
                .await?;
            Ok(read_only(bounded_output(output)))
        }
        StaffAction::Where { target, reason } => {
            let target = connected_target(world, &target)?;
            let reason =
                authorize_target(store, &access, &target, StaffCapability::StaffWhere, reason)
                    .await?;
            let output = world.staff_location_as_plain_text(target.session_id)?;
            store
                .append_staff_audit(audit_record(
                    &access,
                    Some(target.account_id),
                    StaffCapability::StaffWhere,
                    "staff_where",
                    reason,
                    staff_target_details(&actor.name, &target.name),
                ))
                .await?;
            Ok(read_only(bounded_output(output)))
        }
        StaffAction::Goto { target, reason } => {
            mutate_target(
                world,
                store,
                session,
                &access,
                &actor.name,
                target,
                reason,
                StaffCapability::StaffGoto,
                "staff_goto",
                |world, target| world.staff_goto(session, target),
                |target| format!("You travel to {}.\r\n", target.name),
            )
            .await
        }
        StaffAction::Transfer { target, reason } => {
            mutate_target(
                world,
                store,
                session,
                &access,
                &actor.name,
                target,
                reason,
                StaffCapability::StaffTransfer,
                "staff_transfer",
                |world, target| world.staff_transfer(session, target),
                |target| format!("{} has been transferred to you.\r\n", target.name),
            )
            .await
        }
        StaffAction::Restore { target, reason } => {
            mutate_target(
                world,
                store,
                session,
                &access,
                &actor.name,
                target,
                reason,
                StaffCapability::StaffRestore,
                "staff_restore",
                |world, target| world.staff_restore(session, target),
                |target| format!("{} has been fully restored.\r\n", target.name),
            )
            .await
        }
        StaffAction::Announce { message, reason } => {
            ensure_capability(&access, StaffCapability::StaffAnnounce)?;
            let message = normalise_text(&message, MAX_STAFF_MESSAGE_BYTES)?;
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            store
                .append_staff_audit(audit_record(
                    &access,
                    None,
                    StaffCapability::StaffAnnounce,
                    "staff_announce",
                    reason,
                    json!({"actor_name": actor.name, "message": &message}),
                ))
                .await?;
            Ok(AppliedStaff {
                output: "Announcement accepted.\r\n".to_owned(),
                domain_events: Vec::new(),
                refresh_account: None,
                announcement: Some(message),
            })
        }
        StaffAction::Audit { reason } => {
            ensure_capability(&access, StaffCapability::StaffAudit)?;
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            let entries = store
                .staff_audit_entries(audit_record(
                    &access,
                    None,
                    StaffCapability::StaffAudit,
                    "staff_audit_view",
                    reason,
                    json!({"actor_name": actor.name}),
                ))
                .await?;
            Ok(read_only(audit_output(&entries)))
        }
        StaffAction::SetProfile {
            target,
            profile,
            lore_title,
            reason,
        } => {
            ensure_capability(&access, StaffCapability::StaffAssign)?;
            let target_name = normalise_text(&target, MAX_STAFF_TARGET_BYTES)?;
            let target = store.social_target(&target_name).await?;
            let target_access = store.staff_access(target.character_id).await?;
            ensure_higher_authority(&access, &target_access)?;
            if profile.is_some_and(|profile| profile.authority_rank() >= authority_rank(&access)) {
                return Err(StoreError::StaffHierarchy.into());
            }
            let lore_title = lore_title
                .map(|title| normalise_text(&title, MAX_STAFF_TITLE_BYTES))
                .transpose()?;
            if profile.is_none() && lore_title.is_some() {
                return Err(StoreError::InvalidStaffInput.into());
            }
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            let audit = audit_record(
                &access,
                Some(target.account_id),
                StaffCapability::StaffAssign,
                "staff_profile_set",
                reason,
                json!({
                    "actor_name": actor.name,
                    "target_name": &target.character_name,
                    "profile": profile.map(StaffProfileKey::as_str),
                    "lore_title": lore_title.clone(),
                }),
            );
            store
                .set_staff_profile(target.account_id, profile, lore_title.as_deref(), audit)
                .await?;
            Ok(AppliedStaff {
                output: format!(
                    "{} now has staff profile {}.\r\n",
                    target.character_name,
                    profile.map(StaffProfileKey::as_str).unwrap_or("none")
                ),
                domain_events: Vec::new(),
                refresh_account: Some(target.account_id),
                announcement: None,
            })
        }
        StaffAction::SetCapability {
            target,
            capability,
            enabled,
            reason,
        } => {
            ensure_capability(&access, StaffCapability::StaffAssign)?;
            if enabled && !access.capabilities.contains(&capability) {
                return Err(StoreError::StaffForbidden.into());
            }
            let target_name = normalise_text(&target, MAX_STAFF_TARGET_BYTES)?;
            let target = store.social_target(&target_name).await?;
            let target_access = store.staff_access(target.character_id).await?;
            ensure_higher_authority(&access, &target_access)?;
            let reason = normalise_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            store
                .set_staff_capability(
                    target.account_id,
                    capability,
                    enabled,
                    audit_record(
                        &access,
                        Some(target.account_id),
                        StaffCapability::StaffAssign,
                        "staff_capability_set",
                        reason,
                        json!({
                            "actor_name": actor.name,
                            "target_name": &target.character_name,
                            "capability": capability.as_str(),
                            "enabled": enabled,
                        }),
                    ),
                )
                .await?;
            Ok(AppliedStaff {
                output: format!(
                    "{} capability {} for {}.\r\n",
                    if enabled { "Granted" } else { "Revoked" },
                    capability.as_str(),
                    target.character_name
                ),
                domain_events: Vec::new(),
                refresh_account: Some(target.account_id),
                announcement: None,
            })
        }
        StaffAction::RetireArea { .. } | StaffAction::RehydrateArea { .. } => {
            Err(GameError::from(StoreError::InvalidStaffState(
                "area lifecycle is handled by the operations surface".to_owned(),
            )))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn mutate_target<F, O>(
    world: &mut World,
    store: &dyn CharacterStore,
    actor_session: SessionId,
    actor_access: &StaffAccess,
    actor_name: &str,
    target_name: String,
    reason: String,
    capability: StaffCapability,
    action: &'static str,
    mutation: F,
    output: O,
) -> Result<AppliedStaff, GameError>
where
    F: FnOnce(&mut World, SessionId) -> Result<tiamud_core::ActionOutcome, tiamud_core::WorldError>,
    O: FnOnce(&ConnectedCharacter) -> String,
{
    let target = connected_target(world, &target_name)?;
    let reason = authorize_target(store, actor_access, &target, capability, reason).await?;
    let checkpoint = world.checkpoint();
    let outcome = match mutation(world, target.session_id) {
        Ok(outcome) => outcome,
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error.into());
        }
    };
    let mut events = outcome.domain_events;
    match reconcile_world_trades(world, store, &checkpoint).await {
        Ok(trade_events) => events.extend(trade_events),
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error);
        }
    }
    let characters = match durable_connected_characters(world) {
        Ok(characters) => characters,
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error.into());
        }
    };
    let audit = audit_record(
        actor_access,
        Some(target.account_id),
        capability,
        action,
        reason,
        staff_target_details(actor_name, &target.name),
    );
    if let Err(error) = store
        .commit_staff_action(
            characters.clone(),
            world.runtime_snapshot(),
            events.clone(),
            audit,
        )
        .await
    {
        // A failed or ambiguous commit leaves the store state unknown.
        world.invalidate_durable_baseline();
        world.restore_checkpoint(checkpoint);
        return Err(error.into());
    }
    // Every connected character was written with its in-memory aggregate.
    world.mark_durable_commit(true, characters, true);
    debug_assert!(world.connected_sessions().contains(&actor_session));
    Ok(AppliedStaff {
        output: bounded_output(output(&target)),
        domain_events: events,
        refresh_account: None,
        announcement: None,
    })
}

async fn authorize_target(
    store: &dyn CharacterStore,
    actor_access: &StaffAccess,
    target: &ConnectedCharacter,
    capability: StaffCapability,
    reason: String,
) -> Result<String, GameError> {
    ensure_capability(actor_access, capability)?;
    let target_access = store.staff_access(target.character_id).await?;
    ensure_higher_authority(actor_access, &target_access)?;
    normalise_text(&reason, MAX_SOCIAL_REASON_BYTES).map_err(GameError::from)
}

pub(crate) fn ensure_capability(
    access: &StaffAccess,
    capability: StaffCapability,
) -> Result<(), StoreError> {
    access
        .capabilities
        .contains(&capability)
        .then_some(())
        .ok_or(StoreError::StaffForbidden)
}

fn ensure_higher_authority(actor: &StaffAccess, target: &StaffAccess) -> Result<(), StoreError> {
    if actor.account_id == target.account_id || authority_rank(actor) <= authority_rank(target) {
        Err(StoreError::StaffHierarchy)
    } else {
        Ok(())
    }
}

fn authority_rank(access: &StaffAccess) -> u8 {
    access
        .profile
        .map(StaffProfileKey::authority_rank)
        .unwrap_or(0)
}

fn connected_target(world: &World, target: &str) -> Result<ConnectedCharacter, GameError> {
    let target = normalise_text(target, MAX_STAFF_TARGET_BYTES)?;
    let matches = world
        .connected_characters()?
        .into_iter()
        .filter(|candidate| candidate.name.eq_ignore_ascii_case(&target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(target.clone()),
        [] => Err(StoreError::SocialTargetNotFound.into()),
        [_, ..] => Err(StoreError::AmbiguousSocialTarget.into()),
    }
}

fn durable_connected_characters(
    world: &World,
) -> Result<Vec<tiamud_core::DurableCharacterSnapshot>, tiamud_core::WorldError> {
    world
        .connected_sessions()
        .into_iter()
        .map(|session| world.durable_character_snapshot(session))
        .collect()
}

fn audit_record(
    access: &StaffAccess,
    target_account_id: Option<AccountId>,
    capability: StaffCapability,
    action: &str,
    reason: String,
    details: Value,
) -> StaffAuditRecord {
    StaffAuditRecord {
        actor_account_id: access.account_id,
        target_account_id,
        capability,
        action: action.to_owned(),
        reason,
        details,
    }
}

fn staff_target_details(actor_name: &str, target_name: &str) -> Value {
    json!({"actor_name": actor_name, "target_name": target_name})
}

fn status_output(access: &StaffAccess) -> String {
    let mut output = String::from("Staff status:\r\n");
    push_bounded_line(
        &mut output,
        &format!(
            "Profile: {}",
            access
                .profile
                .map(StaffProfileKey::as_str)
                .unwrap_or("none")
        ),
    );
    if let Some(title) = &access.lore_title {
        push_bounded_line(&mut output, &format!("Lore title: {}", single_line(title)));
    }
    if access.capabilities.is_empty() {
        push_bounded_line(&mut output, "Capabilities: none");
    } else {
        push_bounded_line(&mut output, "Capabilities:");
        for capability in &access.capabilities {
            push_bounded_line(&mut output, &format!("- {}", capability.as_str()));
        }
    }
    bounded_output(output)
}

fn reports_output(reports: &[StaffReport]) -> String {
    let mut output = format!("Open reports ({}):\r\n", reports.len());
    if reports.is_empty() {
        push_bounded_line(&mut output, "- none");
    }
    for report in reports {
        push_bounded_line(
            &mut output,
            &format!(
                "- #{}: {} reported {} at {} — {}",
                report.id,
                single_line(&report.reporter_name),
                single_line(&report.target_name),
                report.created_at,
                single_line(&report.reason),
            ),
        );
        if let Some(evidence) = &report.mail_evidence {
            push_bounded_line(
                &mut output,
                &format!(
                    "  mail from {} · subject: {} · excerpt: {}",
                    single_line(&evidence.sender_name),
                    single_line(&evidence.subject),
                    mail_excerpt(&evidence.body),
                ),
            );
        }
        if let Some(evidence) = &report.board_evidence {
            push_bounded_line(
                &mut output,
                &format!(
                    "  board {} · author: {} · subject: {} · excerpt: {}",
                    evidence.board.as_str(),
                    single_line(&evidence.author_name),
                    single_line(&evidence.subject),
                    mail_excerpt(&evidence.body),
                ),
            );
        }
    }
    bounded_output(output)
}

fn mail_excerpt(value: &str) -> String {
    const MAX_BYTES: usize = 256;
    let mut value = single_line(value);
    if value.len() <= MAX_BYTES {
        return value;
    }
    let mut boundary = MAX_BYTES.saturating_sub(3);
    while !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    value.truncate(boundary);
    value.push_str("...");
    value
}

fn audit_output(entries: &[StaffAuditEntry]) -> String {
    let mut output = format!("Staff audit ({}):\r\n", entries.len());
    if entries.is_empty() {
        push_bounded_line(&mut output, "- none");
    }
    for entry in entries {
        let details = serde_json::from_str::<Value>(&entry.details_json).ok();
        let actor = details
            .as_ref()
            .and_then(|value| value.get("actor_name"))
            .and_then(Value::as_str)
            .map(single_line)
            .unwrap_or_else(|| "staff member".to_owned());
        let target = details
            .as_ref()
            .and_then(|value| value.get("target_name"))
            .and_then(Value::as_str)
            .map(single_line);
        let target = target
            .map(|target| format!(" -> {target}"))
            .unwrap_or_default();
        push_bounded_line(
            &mut output,
            &format!(
                "- {}: {}{} used {} ({}) — {}",
                entry.created_at,
                actor,
                target,
                single_line(&entry.action),
                entry.capability.as_str(),
                single_line(&entry.reason),
            ),
        );
    }
    bounded_output(output)
}

fn read_only(output: String) -> AppliedStaff {
    AppliedStaff {
        output: bounded_output(output),
        domain_events: Vec::new(),
        refresh_account: None,
        announcement: None,
    }
}

fn push_bounded_line(output: &mut String, line: &str) {
    const TRUNCATED: &str = "- [output truncated]\r\n";
    if output.len() + line.len() + 2 + TRUNCATED.len() <= MAX_STAFF_OUTPUT_BYTES {
        output.push_str(line);
        output.push_str("\r\n");
    } else if !output.ends_with(TRUNCATED)
        && output.len() + TRUNCATED.len() <= MAX_STAFF_OUTPUT_BYTES
    {
        output.push_str(TRUNCATED);
    }
}

fn bounded_output(mut output: String) -> String {
    if output.len() <= MAX_STAFF_OUTPUT_BYTES {
        return output;
    }
    let mut boundary = MAX_STAFF_OUTPUT_BYTES.saturating_sub(24).min(output.len());
    while !output.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    output.truncate(boundary);
    output.push_str("\r\n[output truncated]\r\n");
    output
}

pub(crate) fn normalise_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidStaffInput)
    } else {
        Ok(value.to_owned())
    }
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn access(profile: Option<StaffProfileKey>, capabilities: &[StaffCapability]) -> StaffAccess {
        StaffAccess {
            account_id: AccountId(1),
            role: AccountRole::Player,
            profile,
            lore_title: None,
            capabilities: capabilities.iter().copied().collect::<BTreeSet<_>>(),
        }
    }

    #[test]
    fn presentation_role_is_derived_from_profile_not_legacy_role() {
        let mut administrator = access(Some(StaffProfileKey::Administrator), &[]);
        administrator.role = AccountRole::Player;
        assert_eq!(derived_role(&administrator), AccountRole::Admin);

        let mut player = access(None, &[StaffCapability::StaffInspect]);
        player.role = AccountRole::Admin;
        assert_eq!(derived_role(&player), AccountRole::Player);

        for profile in [StaffProfileKey::Helper, StaffProfileKey::Builder] {
            let mut access = access(Some(profile), &[]);
            access.role = AccountRole::Admin;
            assert_eq!(derived_role(&access), AccountRole::Player);
        }
        assert_eq!(
            derived_role(&access(Some(StaffProfileKey::Moderator), &[])),
            AccountRole::Moderator
        );
    }

    #[test]
    fn hierarchy_is_explicit_and_rejects_self_equal_or_higher_targets() {
        let moderator = access(Some(StaffProfileKey::Moderator), &[]);
        let mut player = access(None, &[]);
        player.account_id = AccountId(2);
        assert!(ensure_higher_authority(&moderator, &player).is_ok());

        let mut peer = access(Some(StaffProfileKey::Moderator), &[]);
        peer.account_id = AccountId(2);
        assert!(ensure_higher_authority(&moderator, &peer).is_err());

        let mut implementor = access(Some(StaffProfileKey::Implementor), &[]);
        implementor.account_id = AccountId(2);
        assert!(ensure_higher_authority(&moderator, &implementor).is_err());
        assert!(ensure_higher_authority(&moderator, &moderator).is_err());
    }

    #[test]
    fn status_never_uses_legacy_role_or_durable_account_id() {
        let mut access = access(
            Some(StaffProfileKey::Helper),
            &[StaffCapability::ReportsView],
        );
        access.account_id = AccountId(9_991);
        access.role = AccountRole::Admin;
        let output = status_output(&access);
        assert!(output.contains("Profile: helper"));
        assert!(output.contains("reports.view"));
        assert!(!output.contains("9991"));
        assert!(!output.to_ascii_lowercase().contains("admin"));
    }

    #[test]
    fn audit_output_is_bounded_and_never_projects_durable_ids_or_raw_json() {
        let entries = (0..100)
            .map(|index| StaffAuditEntry {
                sequence: index,
                actor_account_id: AccountId(77),
                target_account_id: Some(AccountId(88)),
                capability: StaffCapability::StaffAudit,
                action: "staff_audit_view".to_owned(),
                reason: "r".repeat(1_024),
                details_json: json!({"actor_name": "Aria", "target_name": "Borin"}).to_string(),
                created_at: 1,
            })
            .collect::<Vec<_>>();
        let output = audit_output(&entries);
        assert!(output.len() <= MAX_STAFF_OUTPUT_BYTES);
        assert!(!output.contains("AccountId"));
        assert!(!output.contains("\"actor_name\""));
        assert!(!output.contains("77"));
        assert!(!output.contains("88"));
    }

    #[test]
    fn persisted_control_characters_cannot_create_extra_output_lines() {
        assert_eq!(single_line("reason\nwith\ttabs"), "reason with tabs");
    }
}
