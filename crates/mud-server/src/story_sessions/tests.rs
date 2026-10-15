use super::*;
use serde_json::json;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../story-core/fixtures")
}

fn fixture(name: &str) -> StoryPack {
    let bytes = std::fs::read(fixture_dir().join(format!("{name}.story.json"))).unwrap();
    StoryPack::from_json_slice(&bytes).unwrap()
}

fn fixture_pack() -> StoryPack {
    fixture(CRIPTA)
}

fn tiny_pack(id: &str) -> StoryPack {
    serde_json::from_value(json!({
        "format_version": 1,
        "id": id,
        "title": {"es": "Breve"},
        "start": "a",
        "nodes": [
            {"id": "a", "text": {"es": "Inicio."}, "choices": [
                {"id": "ir", "label": {"es": "Ir"}, "target": "b"}
            ]},
            {"id": "b", "text": {"es": "Fin."}, "ending": {"id": "fin", "kind": "victory"}}
        ]
    }))
    .unwrap()
}

fn manager(packs: Vec<StoryPack>, limits: StoryLimits) -> StoryManager {
    StoryManager::new(
        StoryCatalog::from_packs(packs, BTreeMap::new()).unwrap(),
        limits,
    )
}

const CRIPTA: &str = "cripta-del-nigromante";
const LLUVIA: &str = "lluvia-de-neon";

fn choice_ids(view: &StoryProjection) -> Vec<String> {
    view.items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Choice { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn texts(view: &StoryProjection, wanted: StoryBlockKind) -> Vec<String> {
    view.items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Block { kind, text, .. } if *kind == wanted => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn cues(view: &StoryProjection) -> Vec<StoryItem> {
    view.items
        .iter()
        .filter(|item| matches!(item, StoryItem::Cue { .. }))
        .cloned()
        .collect()
}

#[test]
fn configuration_is_disabled_by_default_and_fails_closed() {
    let disabled = story_config(None, None, None).unwrap();
    assert_eq!(disabled.directory, None);
    assert_eq!(disabled.limits, StoryLimits::default());

    let configured =
        story_config(Some("stories".into()), Some("8".into()), Some("120".into())).unwrap();
    assert_eq!(configured.directory, Some(PathBuf::from("stories")));
    assert_eq!(configured.limits.max_instances, 8);
    assert_eq!(configured.limits.idle_timeout, Duration::from_secs(120));

    assert_eq!(
        story_config(Some(" x".into()), None, None),
        Err(StoryConfigError::InvalidDirectory)
    );
    assert_eq!(
        story_config(Some(String::new()), None, None),
        Err(StoryConfigError::InvalidDirectory)
    );
    for invalid in ["0", "4097", "-1", "many"] {
        assert_eq!(
            story_config(Some("d".into()), Some(invalid.into()), None),
            Err(StoryConfigError::InvalidMaxInstances)
        );
    }
    for invalid in ["59", "86401", "x"] {
        assert_eq!(
            story_config(Some("d".into()), None, Some(invalid.into())),
            Err(StoryConfigError::InvalidIdleSeconds)
        );
    }
    assert_eq!(
        story_config(None, Some("4".into()), None),
        Err(StoryConfigError::LimitsWithoutDirectory)
    );
}

#[test]
fn catalog_loads_the_fixture_directory_and_rejects_bad_content() {
    let catalog = StoryCatalog::load_dir(&fixture_dir()).unwrap();
    assert_eq!(
        catalog.listing(),
        vec![
            (CRIPTA.to_owned(), "La cripta del nigromante".to_owned()),
            (LLUVIA.to_owned(), "Lluvia de neón".to_owned()),
        ]
    );

    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        StoryCatalog::load_dir(empty.path()),
        Err(StoryCatalogError::Empty)
    ));

    let invalid = tempfile::tempdir().unwrap();
    std::fs::write(
        invalid.path().join("x.story.json"),
        b"{\"format_version\":1}",
    )
    .unwrap();
    assert!(matches!(
        StoryCatalog::load_dir(invalid.path()),
        Err(StoryCatalogError::Invalid { .. })
    ));

    let mut broken = tiny_pack("roto");
    broken.start = "missing".into();
    assert!(matches!(
        StoryCatalog::from_packs(vec![broken], BTreeMap::new()),
        Err(StoryCatalogError::Invalid { .. })
    ));
    assert!(matches!(
        StoryCatalog::from_packs(vec![tiny_pack("a"), tiny_pack("a")], BTreeMap::new()),
        Err(StoryCatalogError::Duplicate(_))
    ));
}

#[test]
fn media_map_accepts_only_reviewed_content_addressed_audio() {
    let digest = "a".repeat(64);
    let valid = format!(
        r#"{{"formatVersion":1,"assets":{{"musica-combate":"assets/{digest}/audio-combate.ogg"}}}}"#
    );
    let media = parse_media_manifest(valid.as_bytes()).unwrap();
    assert_eq!(
        media.get("musica-combate").map(String::as_str),
        Some(format!("/assets/{digest}/audio-combate.ogg").as_str())
    );
    for invalid in [
        r#"{"formatVersion":1,"assets":{"x":"https://example.test/a.ogg"}}"#.to_owned(),
        format!(r#"{{"formatVersion":1,"assets":{{"x":"assets/{digest}/image.png"}}}}"#),
        format!(r#"{{"formatVersion":1,"assets":{{"Bad Id":"assets/{digest}/audio-a.ogg"}}}}"#),
        format!(r#"{{"formatVersion":2,"assets":{{"x":"assets/{digest}/audio-a.ogg"}}}}"#),
        r#"{"formatVersion":1,"assets":{},"extra":true}"#.to_owned(),
    ] {
        assert!(
            parse_media_manifest(invalid.as_bytes()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn story_commands_parse_in_both_languages() {
    assert_eq!(parse_story_command("historias"), Some(StoryCommand::List));
    assert_eq!(parse_story_command("Stories"), Some(StoryCommand::List));
    assert_eq!(
        parse_story_command("historia Cripta-del-Nigromante"),
        Some(StoryCommand::Enter(CRIPTA.into()))
    );
    assert_eq!(
        parse_story_command("story cripta-del-nigromante"),
        Some(StoryCommand::Enter(CRIPTA.into()))
    );
    assert_eq!(
        parse_story_command("salir historia"),
        Some(StoryCommand::Leave)
    );
    assert_eq!(
        parse_story_command("story leave"),
        Some(StoryCommand::Leave)
    );
    assert_eq!(parse_story_command("look"), None);
    assert_eq!(parse_story_command("say historia"), None);

    assert_eq!(parse_story_input(" 2 "), StoryInputCommand::Number(2));
    assert_eq!(parse_story_input("0"), StoryInputCommand::Other);
    assert_eq!(parse_story_input("9999"), StoryInputCommand::Other);
    assert_eq!(parse_story_input("deshacer"), StoryInputCommand::Undo);
    assert_eq!(parse_story_input("UNDO"), StoryInputCommand::Undo);
    assert_eq!(parse_story_input("salir"), StoryInputCommand::Leave);
    assert_eq!(parse_story_input("leave"), StoryInputCommand::Leave);
    assert_eq!(parse_story_input("ver"), StoryInputCommand::Look);
    assert_eq!(parse_story_input("mirar"), StoryInputCommand::Other);
    assert_eq!(parse_story_input("north"), StoryInputCommand::Other);
}

#[tokio::test]
async fn entering_projects_opaque_choices_without_authored_ids() {
    let stories = manager(vec![fixture_pack()], StoryLimits::default());
    let link = stories.enter(AccountId(1), CRIPTA).unwrap();
    let view = link.view().await.unwrap();
    assert_eq!(view.view_rev, 1);
    assert_eq!(view.title, "La cripta del nigromante");
    let ids = choice_ids(&view);
    assert_eq!(ids.len(), 1);
    assert!(valid_choice_id(&ids[0]));
    let encoded = serde_json::to_string(&view.web_parts(true)).unwrap();
    for secret in [
        "tirar-destreza",
        "vestibulo",
        "vida_esqueleto",
        "t0c0",
        "rng",
    ] {
        assert!(!encoded.contains(secret), "{secret} leaked: {encoded}");
    }
    // Hidden variables are not stats.
    let stats = view
        .items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Stat { label, .. } => Some(label.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(stats, ["Destreza", "Resistencia"]);
    assert!(!view.can_undo);
}

#[tokio::test]
async fn forged_and_stale_ids_are_rejected_without_changing_state() {
    let stories = manager(vec![fixture_pack()], StoryLimits::default());
    let link = stories.enter(AccountId(1), CRIPTA).unwrap();
    let view = link.view().await.unwrap();
    let real = choice_ids(&view)[0].clone();

    for forged in [
        "",
        "c0",
        "t0c0",
        "C0123456789ABCDEF",
        "c0123456789abcdeg",
        "c0123456789abcdef",
    ] {
        assert_eq!(
            link.choose(forged, view.view_rev).await,
            Err(StoryFailure::UnknownChoice),
            "{forged}"
        );
    }
    assert_eq!(
        link.choose(&real, view.view_rev + 1).await,
        Err(StoryFailure::StaleView)
    );
    // A choice id is not a menu verb and the gamebook has no dialogue.
    assert_eq!(
        link.input(
            StoryRequest::Menu {
                verb: real.clone(),
                target: None
            },
            view.view_rev
        )
        .await,
        Err(StoryFailure::UnknownChoice)
    );
    assert_eq!(
        link.input(StoryRequest::Dialogue { id: real.clone() }, view.view_rev)
            .await,
        Err(StoryFailure::UnknownChoice)
    );
    assert_eq!(link.view().await.unwrap(), view);

    let next = link.choose(&real, view.view_rev).await.unwrap();
    assert_eq!(next.view_rev, view.view_rev + 1);
    assert!(next.can_undo);
    assert!(
        texts(&next, StoryBlockKind::Event)
            .iter()
            .any(|text| text.starts_with("Tirada 1d6+6"))
    );
    // The id of the previous view no longer resolves, even with the new rev.
    assert_eq!(
        link.choose(&real, next.view_rev).await,
        Err(StoryFailure::UnknownChoice)
    );
    assert_eq!(
        link.choose(&choice_ids(&next)[0], view.view_rev).await,
        Err(StoryFailure::StaleView)
    );
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_inputs_total{result=\"accepted\"} 1")
    );
}

#[tokio::test]
async fn undo_restores_the_previous_state_under_a_new_revision() {
    let stories = manager(vec![fixture_pack()], StoryLimits::default());
    let link = stories.enter(AccountId(1), CRIPTA).unwrap();
    let start = link.view().await.unwrap();
    assert_eq!(link.undo().await, Err(StoryFailure::NothingToUndo));
    let next = link
        .choose(&choice_ids(&start)[0], start.view_rev)
        .await
        .unwrap();
    let undone = link.undo().await.unwrap();
    assert_eq!(undone.view_rev, next.view_rev + 1);
    assert_eq!(undone.turn, 0);
    assert_eq!(undone.items.len(), start.items.len());
    assert_ne!(choice_ids(&undone), choice_ids(&start));
    assert!(!undone.can_undo);
}

/// Plays by always taking the first numbered action until an ending.
async fn play_first_actions(link: &StoryLink) -> Vec<StoryProjection> {
    let mut views = vec![link.view().await.unwrap()];
    for _ in 0..200 {
        let view = views.last().unwrap();
        if view.ending.is_some() {
            return views;
        }
        let action = view.actions().remove(0);
        let next = link.input(action.request(), view.view_rev).await.unwrap();
        views.push(next);
    }
    panic!("no ending reached");
}

#[tokio::test]
async fn the_sample_gamebook_reaches_an_ending_and_then_refuses_choices() {
    let stories = manager(vec![fixture_pack()], StoryLimits::default());
    let link = stories
        .enter_seeded(AccountId(1), CRIPTA, 7, Some(1))
        .unwrap();
    let views = play_first_actions(&link).await;
    let last = views.last().unwrap();
    assert!(last.ending.is_some());
    assert!(last.actions().is_empty());
    assert_eq!(
        link.choose("c0123456789abcdef", last.view_rev).await,
        Err(StoryFailure::Ended)
    );
    let text = render_text(last);
    assert!(text.contains("FIN — "));
    assert!(text.contains("«salir» (leave)"));

    // Same seeds, same inputs: the same playthrough.
    let replay = manager(vec![fixture_pack()], StoryLimits::default());
    let again = replay
        .enter_seeded(AccountId(1), CRIPTA, 7, Some(1))
        .unwrap();
    assert_eq!(play_first_actions(&again).await, views);
}

/// Takes the numbered action whose label is exactly `label`.
async fn act(link: &StoryLink, view: &StoryProjection, label: &str) -> StoryProjection {
    let actions = view.actions();
    let action = actions
        .iter()
        .find(|action| action.label() == label)
        .unwrap_or_else(|| panic!("{label:?} not in {actions:?}"));
    link.input(action.request(), view.view_rev).await.unwrap()
}

#[tokio::test]
async fn the_menu_adventure_plays_verbs_targets_and_dialogue_to_its_ending() {
    let stories = manager(vec![fixture(LLUVIA)], StoryLimits::default());
    let link = stories
        .enter_seeded(AccountId(1), LLUVIA, 1, Some(2))
        .unwrap();
    let mut view = link.view().await.unwrap();
    assert!(
        view.items
            .iter()
            .any(|item| matches!(item, StoryItem::Verb { label, needs_target: true, .. } if label == "INVESTIGAR"))
    );
    // A verb needing a target refuses to run without one or with a target of
    // another verb.
    let verbs = view
        .items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Verb { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let foreign_target = view
        .items
        .iter()
        .find_map(|item| match item {
            StoryItem::Target { verb, id, .. } if verb != &verbs[0] => Some(id.clone()),
            _ => None,
        })
        .unwrap();
    for target in [None, Some(foreign_target)] {
        assert_eq!(
            link.input(
                StoryRequest::Menu {
                    verb: verbs[0].clone(),
                    target
                },
                view.view_rev
            )
            .await,
            Err(StoryFailure::UnknownChoice)
        );
    }
    for label in [
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Escritorio",
        "USAR → Tarjeta de memoria",
        "MOVERSE → Puerta",
        "HABLAR → Lola",
    ] {
        view = act(&link, &view, label).await;
    }
    // In a dialogue only its options are offered.
    assert!(
        view.actions()
            .iter()
            .all(|action| matches!(action, StoryAction::Dialogue { .. }))
    );
    assert!(view.items.iter().any(
        |item| matches!(item, StoryItem::Block { kind: StoryBlockKind::Line, speaker: Some(name), .. } if name == "Lola")
    ));
    let text = render_text(&view);
    assert!(text.contains("Lola: «"));
    for label in [
        "Pedir un café solo",
        "Seguir hablando",
        "Despedirte",
        "MOSTRAR → Tarjeta de memoria",
        "INVESTIGAR → Reservado",
        "MOVERSE → Callejón trasero",
        "HABLAR → Dámaso",
        "Acusarle con el vídeo y el mechero",
    ] {
        view = act(&link, &view, label).await;
    }
    assert_eq!(view.ending, Some(StoryEndingKind::Victory));
    assert_eq!(view.score, 100);
    let undone = link.undo().await.unwrap();
    assert!(undone.ending.is_none());
    assert!(
        undone
            .actions()
            .iter()
            .any(|action| matches!(action, StoryAction::Dialogue { .. }))
    );
}

#[tokio::test]
async fn one_instance_per_account_and_a_global_bound() {
    let stories = manager(
        vec![fixture_pack(), tiny_pack("breve")],
        StoryLimits {
            max_instances: 2,
            idle_timeout: Duration::from_secs(60),
            ..StoryLimits::default()
        },
    );
    let first = stories.enter(AccountId(1), CRIPTA).unwrap();
    let rev = first.view().await.unwrap().view_rev;
    // Re-entering the same story resumes it; another story is refused.
    let resumed = stories.enter(AccountId(1), CRIPTA).unwrap();
    assert_eq!(resumed.view().await.unwrap().view_rev, rev);
    assert_eq!(
        stories.enter(AccountId(1), "breve").err(),
        Some(StoryFailure::AlreadyInStory)
    );
    assert_eq!(
        stories.enter(AccountId(1), "nope").err(),
        Some(StoryFailure::UnknownStory)
    );
    stories.enter(AccountId(2), "breve").unwrap();
    assert_eq!(
        stories.enter(AccountId(3), "breve").err(),
        Some(StoryFailure::Capacity)
    );
    assert_eq!(stories.live_instances(), 2);

    first.leave().await.unwrap();
    assert_eq!(stories.live_instances(), 1);
    assert_eq!(first.view().await, Err(StoryFailure::Gone));
    assert!(stories.attach(AccountId(1)).is_none());
    stories.enter(AccountId(3), "breve").unwrap();
    let metrics = stories.prometheus();
    assert!(metrics.contains("tiamud_story_instances 2\n"));
    assert!(metrics.contains("tiamud_story_instances_closed_total{reason=\"left\"} 1"));
    assert!(metrics.contains("tiamud_story_enter_rejected_total{reason=\"capacity\"} 1"));
    assert!(metrics.contains("tiamud_story_enter_rejected_total{reason=\"account\"} 1"));
    assert!(metrics.contains("tiamud_story_enter_rejected_total{reason=\"unknown\"} 1"));
    assert!(!metrics.contains(CRIPTA));
}

#[tokio::test]
async fn idle_instances_are_evicted_and_reported_as_gone() {
    let stories = manager(
        vec![fixture_pack()],
        StoryLimits {
            max_instances: 4,
            idle_timeout: Duration::from_millis(50),
            ..StoryLimits::default()
        },
    );
    let link = stories.enter(AccountId(1), CRIPTA).unwrap();
    link.view().await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(stories.live_instances(), 0);
    assert!(stories.attach(AccountId(1)).is_none());
    assert_eq!(link.view().await, Err(StoryFailure::Gone));
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_instances_closed_total{reason=\"evicted\"} 1")
    );
    // The account may start again after eviction.
    stories.enter(AccountId(1), CRIPTA).unwrap();
}

#[tokio::test]
async fn reconnecting_sessions_reattach_and_receive_the_same_view() {
    let stories = manager(vec![fixture_pack()], StoryLimits::default());
    let link = stories.enter(AccountId(5), CRIPTA).unwrap();
    let view = link.view().await.unwrap();
    let next = link
        .choose(&choice_ids(&view)[0], view.view_rev)
        .await
        .unwrap();
    drop(link);
    let reattached = stories.attach(AccountId(5)).unwrap();
    let resent = reattached.view().await.unwrap();
    assert_eq!(resent, next);
}

fn huge_pack() -> StoryPack {
    let label = "é".repeat(120);
    let variables = (0..200)
        .map(|index| {
            json!({"id": format!("v{index}"), "label": {"es": format!("{label}{index}")}, "visible": true,
                "kind": {"int": {"min": 0, "max": 10, "initial": 1}}})
        })
        .collect::<Vec<_>>();
    let items = (0..200)
        .map(|index| json!({"id": format!("i{index}"), "name": {"es": format!("{label}{index}")}}))
        .collect::<Vec<_>>();
    let inventory = (0..200)
        .map(|index| format!("i{index}"))
        .collect::<Vec<_>>();
    let choices = (0..32)
        .map(|index| json!({"id": format!("c{index}"), "label": {"es": format!("\"{label}\"{index}")}, "target": "a"}))
        .collect::<Vec<_>>();
    let text = format!("{}\n{}", "\\".repeat(2000), "ñ".repeat(1000));
    serde_json::from_value(json!({
        "format_version": 1, "id": "enorme", "title": {"es": "\"".repeat(200)}, "start": "a",
        "variables": variables, "items": items, "initial_inventory": inventory,
        "nodes": [{"id": "a", "text": {"es": text}, "choices": choices}]
    }))
    .unwrap()
}

#[tokio::test]
async fn large_views_paginate_within_sixteen_kib_without_losing_content() {
    let pack = huge_pack();
    pack.validate().unwrap();
    let stories = manager(vec![pack], StoryLimits::default());
    let link = stories.enter(AccountId(1), "enorme").unwrap();
    let view = link.view().await.unwrap();
    let parts = view.web_parts(true);
    assert!(parts.len() > 1, "the huge view must paginate");
    let mut items = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        assert_eq!(usize::from(part.part), index);
        assert_eq!(usize::from(part.parts), parts.len());
        assert_eq!(part.view_rev, view.view_rev);
        let frame = serde_json::to_vec(&protocol::ServerEnvelope::new(
            ServerMessageType::StoryView,
            u64::MAX,
            part,
        ))
        .unwrap();
        assert!(frame.len() <= MAX_STORY_FRAME_BYTES, "{}", frame.len());
        items.extend(part.items.clone());
    }
    assert_eq!(items, view.items);
}

#[tokio::test]
async fn telnet_text_numbers_choices_and_shows_every_playable_fact() {
    let stories = manager(vec![fixture_pack()], StoryLimits::default());
    let link = stories
        .enter_seeded(AccountId(1), CRIPTA, 7, Some(1))
        .unwrap();
    let start = link.view().await.unwrap();
    let text = render_text(&start);
    assert!(text.contains("== La cripta del nigromante =="));
    assert!(text.contains("1) Tira un dado y suma 6 para fijar tu destreza\r\n"));
    assert!(text.contains("Destreza: 0"));
    assert!(text.contains("Inventario: (vacío)"));
    assert!(!text.contains(&choice_ids(&start)[0]));
    let next = link
        .choose(&choice_ids(&start)[0], start.view_rev)
        .await
        .unwrap();
    let text = render_text(&next);
    assert!(text.contains("* Tirada 1d6+6: ["));
    assert!(text.contains("* Destreza: 0 → "));
    assert!(text.contains("1) Coger la antorcha\r\n2) Avanzar por el pasillo del norte\r\n"));
}

#[tokio::test]
async fn unreviewed_cues_never_play_and_reviewed_cues_resolve() {
    let digest = "b".repeat(64);
    let mut media = BTreeMap::new();
    media.insert(
        "musica-combate".to_owned(),
        format!("/assets/{digest}/audio-combate.ogg"),
    );
    let walk = |stories: StoryManager| async move {
        let link = stories
            .enter_seeded(AccountId(1), CRIPTA, 7, Some(1))
            .unwrap();
        let mut view = link.view().await.unwrap();
        for label in [
            "Tira un dado y suma 6 para fijar tu destreza",
            "Avanzar por el pasillo del norte",
            "Un esqueleto armado se alza: luchar",
        ] {
            view = act(&link, &view, label).await;
        }
        (link, view)
    };
    let (link, view) = walk(StoryManager::new(
        StoryCatalog::from_packs(vec![fixture_pack()], media).unwrap(),
        StoryLimits::default(),
    ))
    .await;
    let combat = vec![StoryItem::Cue {
        action: StoryCueAction::Play,
        channel: MediaChannel::Music,
        url: Some(format!("/assets/{digest}/audio-combate.ogg")),
        looped: true,
    }];
    assert_eq!(cues(&view), combat);
    // Recovery restores the music once; audio-off clients receive no cues.
    let resent = link.view().await.unwrap();
    assert_eq!(cues(&resent), combat);
    assert!(view.web_parts(false).iter().all(|part| {
        part.items
            .iter()
            .all(|item| !matches!(item, StoryItem::Cue { .. }))
    }));

    // Without a media map the same path emits nothing for the music cue.
    let (_, plain) = walk(manager(vec![fixture_pack()], StoryLimits::default())).await;
    assert!(cues(&plain).is_empty());
}

const LLUVIA_SCRIPT: [&str; 15] = [
    "INVESTIGAR → Ventana",
    "INVESTIGAR → Ventana",
    "INVESTIGAR → Ventana",
    "INVESTIGAR → Escritorio",
    "USAR → Tarjeta de memoria",
    "MOVERSE → Puerta",
    "HABLAR → Lola",
    "Pedir un café solo",
    "Seguir hablando",
    "Despedirte",
    "MOSTRAR → Tarjeta de memoria",
    "INVESTIGAR → Reservado",
    "MOVERSE → Callejón trasero",
    "HABLAR → Dámaso",
    "Acusarle con el vídeo y el mechero",
];

/// The `tiamud.v1` client message a browser sends for `action`.
pub(super) fn wire_input(action: &StoryAction, view_rev: u32) -> serde_json::Value {
    match action.request() {
        StoryRequest::Choose { id } => {
            json!({"type": "story.choose", "data": {"id": id, "viewRev": view_rev}})
        }
        StoryRequest::Dialogue { id } => {
            json!({"type": "story.dialogue", "data": {"id": id, "viewRev": view_rev}})
        }
        StoryRequest::Menu { verb, target } => {
            let mut data = json!({"verb": verb, "viewRev": view_rev});
            if let Some(target) = target {
                data["target"] = json!(target);
            }
            json!({"type": "story.menu", "data": data})
        }
    }
}

/// Plays `script` (labels of numbered actions; `None` = first action) and
/// records each view's frames with the exact client input that follows it.
async fn record(
    stories: &StoryManager,
    story_id: &str,
    script: &[Option<&str>],
) -> serde_json::Value {
    let link = stories
        .enter_seeded(AccountId(1), story_id, 7, Some(1))
        .unwrap();
    let mut view = link.view().await.unwrap();
    let mut steps = Vec::new();
    for wanted in script {
        let actions = view.actions();
        let action = match wanted {
            Some(label) => actions
                .iter()
                .find(|action| action.label() == *label)
                .unwrap()
                .clone(),
            None => actions[0].clone(),
        };
        steps.push(json!({
            "view": view.web_parts(false),
            "label": action.label(),
            "input": wire_input(&action, view.view_rev),
        }));
        view = link.input(action.request(), view.view_rev).await.unwrap();
    }
    assert!(
        view.ending.is_some(),
        "{story_id} script must reach an ending"
    );
    steps.push(json!({"view": view.web_parts(false), "label": null, "input": null}));
    link.leave().await.unwrap();
    json!({"storyId": story_id, "title": link.title(), "steps": steps})
}

/// The Playwright story scenarios replay real server frames. This test
/// regenerates or checks `apps/web/e2e/fixtures/story-*.json`.
#[tokio::test]
async fn browser_e2e_fixtures_are_current() {
    const UPDATE: &str = "TIAMUD_UPDATE_STORY_E2E_FIXTURE";
    let stories = manager(
        vec![fixture_pack(), fixture(LLUVIA)],
        StoryLimits::default(),
    );
    let cripta_length = play_first_actions(
        &stories
            .enter_seeded(AccountId(9), CRIPTA, 7, Some(1))
            .unwrap(),
    )
    .await
    .len()
        - 1;
    let fixtures = [
        (
            "story-cripta.json",
            record(&stories, CRIPTA, &vec![None; cripta_length]).await,
        ),
        (
            "story-lluvia.json",
            record(&stories, LLUVIA, &LLUVIA_SCRIPT.map(Some)).await,
        ),
    ];
    for (name, document) in fixtures {
        let rendered = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/web/e2e/fixtures")
            .join(name);
        if std::env::var_os(UPDATE).is_some_and(|value| value == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &rendered).unwrap();
            continue;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == rendered,
            "{} is stale; run `{UPDATE}=1 cargo test --locked -p tiamud-server browser_e2e_fixtures` and review the diff",
            path.display()
        );
    }
}
