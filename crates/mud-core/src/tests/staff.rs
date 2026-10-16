use super::*;

#[test]
fn staff_profiles_use_explicit_ranks_and_closed_default_bundles() {
    assert!(StaffProfileKey::Helper.authority_rank() < StaffProfileKey::Moderator.authority_rank());
    assert!(
        StaffProfileKey::Administrator.authority_rank()
            < StaffProfileKey::Implementor.authority_rank()
    );
    assert_eq!(StaffProfileKey::Builder.default_capabilities(), &[]);
    assert!(
        StaffProfileKey::Moderator
            .default_capabilities()
            .contains(&StaffCapability::ModerationBan)
    );
    assert_eq!(
        StaffProfileKey::Administrator.default_capabilities(),
        &StaffCapability::ALL
    );
}

#[test]
fn staff_relocation_and_restore_are_checkpoint_safe_and_project_no_stable_ids() {
    let mut world = World::demo();
    let actor = SessionId(501);
    let target = SessionId(502);
    world
        .connect(
            actor,
            CharacterSnapshot {
                id: CharacterId(501),
                account_id: AccountId(501),
                name: "Keeper".into(),
                room_id: "demo:lantern-square".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    world
        .connect(
            target,
            CharacterSnapshot {
                id: CharacterId(502),
                account_id: AccountId(502),
                name: "Aria".into(),
                room_id: "demo:north-gate".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    let inspection = world.staff_inspection_as_plain_text(target).unwrap();
    assert!(inspection.contains("Character: Aria"));
    assert!(!inspection.contains("demo:north-gate"));
    assert!(!inspection.contains("account #"));

    let checkpoint = world.checkpoint();
    let relocated = world.staff_goto(actor, target).unwrap();
    assert!(matches!(
        relocated.domain_events.as_slice(),
        [DomainEvent::CharacterRelocated { actor_id, .. }] if *actor_id == CharacterId(501)
    ));
    assert_eq!(
        world.character_snapshot(actor).unwrap().room_id,
        "demo:north-gate"
    );
    world.restore_checkpoint(checkpoint);
    assert_eq!(
        world.character_snapshot(actor).unwrap().room_id,
        "demo:lantern-square"
    );

    let mut gameplay = world.character_gameplay_snapshot(target).unwrap();
    gameplay.vitals.health = 0;
    gameplay.recovery_at_tick = Some(world.logical_tick().saturating_add(3));
    world
        .restore_character_gameplay(CharacterId(502), gameplay)
        .unwrap();
    let restored = world.staff_restore(actor, target).unwrap();
    assert!(matches!(
        restored.domain_events.as_slice(),
        [DomainEvent::Gameplay {
            event: GameplayEvent::CharacterRestored { character_id, .. }
        }] if *character_id == CharacterId(502)
    ));
    let restored_gameplay = world.character_gameplay_snapshot(target).unwrap();
    assert_eq!(
        restored_gameplay.vitals.health,
        restored_gameplay.vitals.max_health
    );
    assert_eq!(restored_gameplay.recovery_at_tick, None);
}
