use envy_import::{
    LicenseReviewStatus, ProgramOwner, TextEncoding, parse_import, parse_world_pack,
};
use std::process::Command;
use world_pack::LegacyBitvector;

const FULL_AREA: &[u8] = include_bytes!("fixtures/full_merc_family.are");

#[test]
fn imports_all_supported_historical_sections_without_executing_content() {
    let result = parse_import(
        FULL_AREA,
        "synthetic-full",
        Some("full_merc_family.are".to_owned()),
        TextEncoding::Ascii,
    )
    .unwrap();

    assert_eq!(result.legacy.area.braced_values, [100, 2999]);
    assert_eq!(
        result.legacy.area.display_name,
        "Synthetic Merc-family compatibility area"
    );
    assert_eq!(result.legacy.area.recall_vnum, Some(100));
    assert_eq!(result.legacy.mobiles.len(), 1);
    assert_eq!(result.legacy.objects.len(), 2);
    assert_eq!(result.legacy.resets.len(), 7);
    assert_eq!(result.legacy.shops.len(), 1);
    assert_eq!(result.legacy.specials.len(), 2);
    assert_eq!(result.legacy.programs.len(), 4);

    let mobile = &result.legacy.mobiles[0];
    assert_eq!(mobile.vnum, 1000);
    assert_eq!(mobile.attribute_lines[1].tokens[3], "3d8+20");
    assert_eq!(mobile.programs[0].trigger, "greet_prog");
    assert!(mobile.programs.iter().all(|program| !program.executable));

    let object = &result.legacy.objects[0];
    assert_eq!(object.vnum, 2000);
    assert_eq!(object.item_type, 5);
    assert_eq!(object.values, ["0", "0", "0", "0"]);
    assert!(
        result
            .legacy
            .specials
            .iter()
            .all(|special| !special.executable)
    );
    assert!(
        result
            .legacy
            .programs
            .iter()
            .all(|program| !program.executable)
    );
    assert!(result.legacy.programs.iter().any(|program| {
        matches!(
            &program.owner,
            ProgramOwner::ExternalSection { section, owner_vnum }
                if section == "MOBPROGS" && *owner_vnum == Some(1000)
        )
    }));
    assert!(result.legacy.programs.iter().any(|program| {
        matches!(program.owner, ProgramOwner::Room { vnum: 100 })
            && program.trigger == "speech_prog"
    }));
}

#[test]
fn only_unconditional_room_object_resets_become_world_pack_items() {
    let result = parse_import(FULL_AREA, "synthetic-full", None, TextEncoding::Ascii).unwrap();

    assert_eq!(
        result.world_pack.schema_version,
        world_pack::CURRENT_SCHEMA_VERSION
    );
    assert!(result.world_pack.gameplay.loot_tables.is_empty());
    assert!(result.world_pack.gameplay.loot_sources.is_empty());
    assert!(result.world_pack.gameplay.abilities.is_empty());
    assert!(
        result
            .world_pack
            .gameplay
            .abilities
            .iter()
            .all(|ability| ability.communication.is_none())
    );
    assert_eq!(result.world_pack.item_prototypes.len(), 1);
    assert_eq!(result.world_pack.initial_item_instances.len(), 1);
    let prototype = &result.world_pack.item_prototypes[0];
    assert_eq!(prototype.id, "synthetic-full:object:2000");
    assert_eq!(prototype.name, "a brass lantern");
    assert!(prototype.aliases.iter().any(|alias| alias == "lantern"));
    assert_eq!(prototype.stack_policy, world_pack::ItemStackPolicy::Unique);
    assert_eq!(prototype.max_stack_units, 1);
    assert!(prototype.container.is_none());
    assert_eq!(
        prototype.transfer_policy,
        world_pack::ItemTransferPolicy::Transferable
    );
    assert_eq!(
        result.world_pack.initial_item_instances[0].id,
        "synthetic-full:object:2000:1"
    );
    assert!(matches!(
        &result.world_pack.initial_item_instances[0].location,
        world_pack::InitialItemLocation::Room { room_id }
            if room_id == &result.world_pack.rooms[0].id
    ));
    assert_eq!(
        result.legacy.resets[2].world_pack_item_id.as_deref(),
        Some("synthetic-full:object:2000:1")
    );
    assert!(result.legacy.resets[3].world_pack_item_id.is_none());
    result.world_pack.validate().unwrap();
}

#[test]
fn diagnostics_are_actionable_for_every_lossy_construct() {
    let result = parse_import(FULL_AREA, "synthetic-full", None, TextEncoding::Ascii).unwrap();
    let codes = result
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in [
        "mobile_preserved_not_converted",
        "object_prototype_not_placed",
        "container_reset_not_converted",
        "reset_preserved_not_converted",
        "shop_preserved_not_converted",
        "special_never_executed",
        "program_never_executed",
    ] {
        assert!(codes.contains(&expected), "missing diagnostic {expected}");
    }
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.line.is_some() || diagnostic.section == "AREA" })
    );
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.recommendation.trim().is_empty())
    );
}

#[test]
fn historical_helps_are_bounded_preserved_and_never_published_implicitly() {
    let source = String::from_utf8(FULL_AREA.to_vec()).unwrap().replace(
        "#HELPS\n0 $~",
        "#HELPS\n1 LANTERN LIGHT~\nA synthetic help entry retained for license review.~\n0 $~",
    );
    let result = parse_import(source.as_bytes(), "helps", None, TextEncoding::Ascii).unwrap();

    assert_eq!(result.legacy.helps.len(), 1);
    assert_eq!(result.legacy.helps[0].level, 1);
    assert_eq!(result.legacy.helps[0].keywords, "LANTERN LIGHT");
    assert!(result.legacy.helps[0].text.contains("license review"));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "help_preserved_not_converted")
    );
    let world_json = serde_json::to_string(&result.world_pack).unwrap();
    assert!(!world_json.contains("license review"));
}

#[test]
fn compatibility_wrapper_still_returns_the_bare_world_pack() {
    let detailed = parse_import(FULL_AREA, "synthetic-full", None, TextEncoding::Ascii).unwrap();
    let compatible =
        parse_world_pack(FULL_AREA, "synthetic-full", None, TextEncoding::Ascii).unwrap();

    assert_eq!(compatible, detailed.world_pack);
}

#[test]
fn report_records_reproducible_provenance_without_guessing_a_license() {
    let first = parse_import(
        FULL_AREA,
        "provenance",
        Some("source.are".to_owned()),
        TextEncoding::Ascii,
    )
    .unwrap();
    let second = parse_import(FULL_AREA, "provenance", None, TextEncoding::Ascii).unwrap();

    assert_eq!(first.provenance.source_file.as_deref(), Some("source.are"));
    assert_eq!(first.provenance.input_bytes, FULL_AREA.len());
    assert_eq!(first.provenance.input_sha256.len(), 64);
    assert_eq!(
        first.provenance.input_sha256,
        second.provenance.input_sha256
    );
    assert_eq!(
        first.provenance.license_status,
        LicenseReviewStatus::UndeterminedReviewRequired
    );
    assert!(first.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "source_license_review_required"
            && diagnostic.recommendation.contains("human")
    }));
}

#[test]
fn unknown_descriptors_and_broken_cross_references_are_actionable() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace("5 0 1\n0 0 0 0", "99 4 8\n0 0 0 0")
        .replace("M 0 1000 1 101", "M 0 1999 1 101")
        .replace("O 2000 spec_legacy_lantern", "O 2999 spec_legacy_lantern")
        .replace("#MOBPROGS\n#1000", "#MOBPROGS\n#9999");
    let result = parse_import(source.as_bytes(), "references", None, TextEncoding::Ascii).unwrap();

    for (code, vnum) in [
        ("unknown_object_type_descriptor", 2000),
        ("object_extra_flag_mask_preserved", 2000),
        ("object_wear_flag_mask_preserved", 2000),
        ("external_or_missing_reset_target", 1999),
        ("external_or_missing_special_target", 2999),
        ("external_or_missing_program_owner", 9999),
    ] {
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code && diagnostic.vnum == Some(vnum)),
            "missing {code} for {vnum}"
        );
    }
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unknown_reset_command" && diagnostic.vnum == Some(9000)
    }));
}

#[test]
fn rom_alphabetic_bitvectors_are_preserved_without_inventing_numeric_masks() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replacen("0 1\nD0", "AB 1\nD0", 1)
        .replacen("5 0 1\n0 0 0 0", "5 CD E\n0 0 0 0", 1);
    let result = parse_import(source.as_bytes(), "rom-flags", None, TextEncoding::Ascii).unwrap();

    assert_eq!(
        result.world_pack.rooms[0].legacy.as_ref().unwrap().flags,
        LegacyBitvector::Alphabetic("AB".into())
    );
    assert_eq!(
        result.legacy.objects[0].extra_flags,
        LegacyBitvector::Alphabetic("CD".into())
    );
    assert_eq!(
        result.legacy.objects[0].wear_flags,
        LegacyBitvector::Alphabetic("E".into())
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "rom_alphabetic_room_bitvector_preserved" })
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "rom_alphabetic_object_bitvector_preserved"
            && diagnostic.message.contains("was not converted")
    }));
    result.world_pack.validate().unwrap();
}

#[test]
fn directed_unreachable_rooms_are_imported_with_a_conservative_diagnostic() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace("D0\nA test road leads north.~\nroad~\n0 -1 101\n", "");
    let result = parse_import(
        source.as_bytes(),
        "one-way-topology",
        None,
        TextEncoding::Ascii,
    )
    .unwrap();

    assert_eq!(result.world_pack.rooms.len(), 2);
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "directed_unreachable_room_preserved"
            && diagnostic.vnum == Some(101)
            && diagnostic.recommendation.contains("teleportation")
    }));
    result.world_pack.validate().unwrap();
}

#[test]
fn duplicate_shop_and_special_assignments_are_rejected() {
    let duplicate_shop = String::from_utf8(FULL_AREA.to_vec()).unwrap().replace(
        "1000 5 18 0 0 0 120 80 8 20\n0",
        "1000 5 18 0 0 0 120 80 8 20\n1000 5 18 0 0 0 120 80 8 20\n0",
    );
    let error = parse_import(
        duplicate_shop.as_bytes(),
        "duplicate-shop",
        None,
        TextEncoding::Ascii,
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate shop definition"));

    let duplicate_special = String::from_utf8(FULL_AREA.to_vec()).unwrap().replace(
        "O 2000 spec_legacy_lantern\nS",
        "O 2000 spec_legacy_lantern\nO 2000 spec_other\nS",
    );
    let error = parse_import(
        duplicate_special.as_bytes(),
        "duplicate-special",
        None,
        TextEncoding::Ascii,
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate special assignment"));
}

#[test]
fn report_json_marks_all_historical_behaviour_inert() {
    let result = parse_import(FULL_AREA, "synthetic-full", None, TextEncoding::Ascii).unwrap();
    let json = serde_json::to_string(&result).unwrap();

    assert!(json.contains("\"executable\":false"));
    assert!(!json.contains("\"executable\":true"));
    assert!(json.contains("Never execute this historical program."));
    let world_json = serde_json::to_string(&result.world_pack).unwrap();
    assert!(!world_json.contains("greet_prog"));
    assert!(!world_json.contains("Never execute"));
}

#[test]
fn conditional_object_reset_is_not_misrepresented_as_an_initial_item() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace("O 0 2000 1 100", "O 1 2000 1 100");
    let result = parse_import(source.as_bytes(), "conditional", None, TextEncoding::Ascii).unwrap();

    assert!(result.world_pack.item_prototypes.is_empty());
    assert!(result.world_pack.initial_item_instances.is_empty());
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "conditional_object_reset_not_converted" && diagnostic.vnum == Some(2000)
    }));
}

#[test]
fn missing_reset_targets_produce_line_specific_diagnostics() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace("O 0 2000 1 100", "O 0 2999 1 100");
    let result = parse_import(source.as_bytes(), "missing", None, TextEncoding::Ascii).unwrap();
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "missing_object_prototype")
        .unwrap();

    assert_eq!(diagnostic.vnum, Some(2999));
    assert!(diagnostic.line.is_some());
    assert!(result.world_pack.item_prototypes.is_empty());
    assert!(result.world_pack.initial_item_instances.is_empty());
}

#[test]
fn malformed_shop_record_is_rejected_instead_of_guessed() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace("1000 5 18 0 0 0 120 80 8 20", "1000 5 18");
    let error = parse_import(source.as_bytes(), "bad-shop", None, TextEncoding::Ascii).unwrap_err();

    assert!(error.to_string().contains("shop record has 3 values"));
}

#[test]
fn bounded_mutations_of_a_structurally_valid_area_never_panic() {
    for case in 0..256 {
        let mut mutated = FULL_AREA.to_vec();
        let index = (case * 7919) % mutated.len();
        mutated[index] ^= (case as u8).wrapping_mul(31).max(1);
        let _ = parse_import(
            &mutated,
            &format!("mutation-{case}"),
            None,
            if case % 2 == 0 {
                TextEncoding::Ascii
            } else {
                TextEncoding::Utf8
            },
        );
    }
}

#[test]
fn convert_keeps_stdout_machine_readable_and_emits_warnings_on_stderr() {
    let fixture = format!(
        "{}/tests/fixtures/full_merc_family.are",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_envy-import"))
        .args(["convert", &fixture, "--id", "cli-fixture"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["id"], "cli-fixture");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Recommendation:"));
    assert!(stderr.contains("historical procedure"));
    assert!(stderr.contains("was not executed"));
}

#[test]
fn crlf_area_files_parse_with_the_same_section_semantics() {
    let crlf = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace('\n', "\r\n");
    let result = parse_import(crlf.as_bytes(), "crlf", None, TextEncoding::Ascii).unwrap();

    assert_eq!(result.legacy.mobiles[0].vnum, 1000);
    assert_eq!(result.world_pack.item_prototypes.len(), 1);
    assert_eq!(result.world_pack.initial_item_instances.len(), 1);
    assert_eq!(result.legacy.shops[0].keeper_vnum, 1000);
}

#[test]
fn repeated_object_resets_create_stable_distinct_item_instances() {
    let source = String::from_utf8(FULL_AREA.to_vec())
        .unwrap()
        .replace("O 0 2000 1 100", "O 0 2000 1 100\nO 0 2000 2 101");
    let result = parse_import(source.as_bytes(), "instances", None, TextEncoding::Ascii).unwrap();

    assert_eq!(result.world_pack.item_prototypes.len(), 1);
    assert_eq!(
        result.world_pack.item_prototypes[0].id,
        "instances:object:2000"
    );
    assert_eq!(result.world_pack.initial_item_instances.len(), 2);
    assert_eq!(
        result.world_pack.initial_item_instances[0].id,
        "instances:object:2000:1"
    );
    assert_eq!(
        result.world_pack.initial_item_instances[1].id,
        "instances:object:2000:2"
    );
    assert_eq!(
        result.world_pack.initial_item_instances[0].prototype_id,
        "instances:object:2000"
    );
    assert_eq!(
        result.world_pack.initial_item_instances[1].prototype_id,
        "instances:object:2000"
    );
    result.world_pack.validate().unwrap();
}
