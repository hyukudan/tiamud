use super::super::{DO, OPT_GMCP, OutputFrame, TelnetEvent, TelnetParser};
use super::*;
use crate::story_sessions::{StoryCatalog, StoryLimits};
use std::collections::BTreeMap;
use tiamud_story_core::StoryPack;

const CRIPTA: &str = "cripta-del-nigromante";

fn stories(media: BTreeMap<String, String>) -> StoryManager {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../story-core/fixtures/cripta-del-nigromante.story.json"),
    )
    .unwrap();
    let pack = StoryPack::from_json_slice(&bytes).unwrap();
    StoryManager::new(
        StoryCatalog::from_packs(vec![pack], media).unwrap(),
        StoryLimits::default(),
    )
}

fn gmcp_input(packages: &str, media: bool) -> TelnetInput {
    let mut input = TelnetInput::default();
    input.negotiation.begin_initial_negotiation();
    input.negotiation.handle(vec![TelnetEvent::Negotiation {
        command: DO,
        option: OPT_GMCP,
    }]);
    input.negotiation.handle(vec![TelnetEvent::Subnegotiation {
        option: OPT_GMCP,
        data: format!("Core.Supports.Set {packages}").into_bytes(),
    }]);
    if media {
        input.negotiation.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"media":true}"#.to_vec(),
        }]);
    }
    input
}

/// Drains queued output as (text, GMCP payloads).
fn drain(output: &TelnetOutputQueue) -> (String, Vec<String>) {
    let mut state = output.shared.state.lock().unwrap();
    let mut bytes = Vec::new();
    for frame in state.frames.drain(..) {
        if let OutputFrame::Bytes { bytes: frame, .. } = frame {
            bytes.extend(frame);
        }
    }
    state.byte_len = 0;
    let mut parser = TelnetParser::default();
    let text = parser.feed(&bytes);
    let gmcp = parser
        .take_events()
        .into_iter()
        .filter_map(|event| match event {
            TelnetEvent::Subnegotiation {
                option: OPT_GMCP,
                data,
            } => Some(String::from_utf8(data).unwrap()),
            _ => None,
        })
        .collect();
    (String::from_utf8(text).unwrap(), gmcp)
}

#[tokio::test]
async fn telnet_plays_by_number_undoes_and_leaves() {
    let output = TelnetOutputQueue::new(256, 1024 * 1024);
    let mut input = TelnetInput::default();
    let mut story = TelnetStory::new(Some(stories(BTreeMap::new())), AccountId(9));

    assert_eq!(
        story.line(&output, &mut input, "north").await.unwrap(),
        StoryLineOutcome::World
    );
    assert_eq!(
        story.line(&output, &mut input, "historias").await.unwrap(),
        StoryLineOutcome::Handled
    );
    assert!(
        drain(&output)
            .0
            .contains("cripta-del-nigromante — La cripta del nigromante")
    );

    story
        .line(&output, &mut input, &format!("historia {CRIPTA}"))
        .await
        .unwrap();
    assert!(story.active());
    let (text, gmcp) = drain(&output);
    assert!(text.contains("Entras en «La cripta del nigromante»."));
    assert!(text.contains("1) Tira un dado y suma 6 para fijar tu destreza\r\n"));
    assert!(gmcp.is_empty(), "no GMCP without negotiation");

    // World commands are not forwarded while in a story.
    assert_eq!(
        story.line(&output, &mut input, "north").await.unwrap(),
        StoryLineOutcome::Handled
    );
    assert!(drain(&output).0.contains("absorto en una historia"));
    story.line(&output, &mut input, "7").await.unwrap();
    assert!(drain(&output).0.contains("Esa opción no está disponible."));

    story.line(&output, &mut input, "1").await.unwrap();
    let (text, _) = drain(&output);
    assert!(text.contains("* Tirada 1d6+6: ["));
    assert!(text.contains("2) Avanzar por el pasillo del norte"));

    story.line(&output, &mut input, "deshacer").await.unwrap();
    assert!(drain(&output).0.contains("1) Tira un dado"));

    assert_eq!(
        story.line(&output, &mut input, "salir").await.unwrap(),
        StoryLineOutcome::Left
    );
    assert!(!story.active());
    assert!(drain(&output).0.contains("Sales de la historia"));
    assert_eq!(
        story
            .line(&output, &mut input, "salir historia")
            .await
            .unwrap(),
        StoryLineOutcome::Handled
    );
    assert!(
        drain(&output)
            .0
            .contains("No estás dentro de ninguna historia.")
    );
}

#[tokio::test]
async fn telnet_resumes_after_reconnect_and_reports_disabled_stories() {
    let manager = stories(BTreeMap::new());
    let output = TelnetOutputQueue::new(256, 1024 * 1024);
    let mut input = TelnetInput::default();
    let mut first = TelnetStory::new(Some(manager.clone()), AccountId(3));
    first
        .line(&output, &mut input, &format!("story {CRIPTA}"))
        .await
        .unwrap();
    first.line(&output, &mut input, "1").await.unwrap();
    let (before, _) = drain(&output);

    let mut second = TelnetStory::new(Some(manager), AccountId(3));
    second.resume(&output, &mut input).await.unwrap();
    assert!(second.active());
    let (resumed, _) = drain(&output);
    assert!(resumed.contains("Continúas la historia «La cripta del nigromante»."));
    let tail = before.rsplit("== La cripta").next().unwrap();
    assert!(resumed.ends_with(tail));

    let mut disabled = TelnetStory::new(None, AccountId(4));
    disabled.resume(&output, &mut input).await.unwrap();
    assert_eq!(
        disabled
            .line(&output, &mut input, "historias")
            .await
            .unwrap(),
        StoryLineOutcome::Handled
    );
    assert!(drain(&output).0.contains("no están disponibles"));
}

#[tokio::test]
async fn gmcp_story_summary_and_media_follow_negotiation() {
    let digest = "c".repeat(64);
    let mut media = BTreeMap::new();
    media.insert(
        "musica-combate".to_owned(),
        format!("/assets/{digest}/audio-combate.ogg"),
    );
    let output = TelnetOutputQueue::new(256, 1024 * 1024);
    let mut input = gmcp_input(r#"["Tiamud.Story 1","Client.Media 1"]"#, true);
    let mut story = TelnetStory::new(Some(stories(media)), AccountId(1));
    story
        .line(&output, &mut input, &format!("historia {CRIPTA}"))
        .await
        .unwrap();
    let (_, gmcp) = drain(&output);
    let view = gmcp
        .iter()
        .find_map(|payload| payload.strip_prefix("Tiamud.Story.View "))
        .unwrap();
    let view: serde_json::Value = serde_json::from_str(view).unwrap();
    assert_eq!(view["choices"][0]["number"], 1);
    assert_eq!(view["title"], "La cripta del nigromante");
    assert!(view["ending"].is_null());
    assert!(
        view["choices"][0].get("id").is_none(),
        "opaque ids stay out of GMCP"
    );

    // Roll, go north, then fight: the reviewed combat music cue plays.
    for (choice, label) in [
        ("1", "Tira un dado"),
        ("2", "Avanzar por el pasillo"),
        ("1", "Un esqueleto armado"),
    ] {
        assert!(
            story
                .last_view
                .as_ref()
                .unwrap()
                .actions()
                .get(choice.parse::<usize>().unwrap() - 1)
                .unwrap()
                .label()
                .starts_with(label)
        );
        story.line(&output, &mut input, choice).await.unwrap();
    }
    let (_, gmcp) = drain(&output);
    assert!(gmcp.iter().any(|payload| {
        payload.starts_with("Client.Media.Play ")
            && payload.contains("audio-combate.ogg")
            && payload.contains("\"loop\":true")
    }));

    story.line(&output, &mut input, "salir").await.unwrap();
    let (_, gmcp) = drain(&output);
    assert!(gmcp.contains(&r#"Tiamud.Story.Left {"reason":"left"}"#.to_owned()));
    assert!(gmcp.contains(&r#"Client.Media.Stop {"channel":"music"}"#.to_owned()));
}

#[tokio::test]
async fn telnet_saves_loads_lists_deletes_and_continues_with_gmcp_slots() {
    use crate::persistence::CharacterStore as _;
    let directory = tempfile::TempDir::new().unwrap();
    let store =
        crate::persistence::SqliteCharacterStore::open(&directory.path().join("state.sqlite3"))
            .await
            .unwrap();
    let account = store.load_or_create_account("Lectora").await.unwrap().id;
    let manager = stories(BTreeMap::new());
    manager.attach_store(std::sync::Arc::new(store));
    let output = TelnetOutputQueue::new(256, 1024 * 1024);
    let mut input = gmcp_input(r#"["Tiamud.Story 1"]"#, false);
    let mut story = TelnetStory::new(Some(manager), account);

    story
        .line(&output, &mut input, &format!("historia {CRIPTA}"))
        .await
        .unwrap();
    let (text, gmcp) = drain(&output);
    assert!(!text.contains("autoguardada"));
    assert!(gmcp.contains(&r#"Tiamud.Story.Slots {"offerContinue":false,"slots":[]}"#.to_owned()));

    story.line(&output, &mut input, "guardar 0").await.unwrap();
    assert!(drain(&output).0.contains("la 0 es el autoguardado"));
    story.line(&output, &mut input, "1").await.unwrap();
    drain(&output);
    story.line(&output, &mut input, "save 4").await.unwrap();
    assert!(
        drain(&output)
            .0
            .contains("Partida guardada en la ranura 4.")
    );
    story.line(&output, &mut input, "partidas").await.unwrap();
    let (text, gmcp) = drain(&output);
    assert!(text.contains("Partidas guardadas:\r\n  4 — "));
    assert!(gmcp.iter().any(|payload| {
        payload.starts_with("Tiamud.Story.Slots ") && payload.contains("\"slot\":4")
    }));

    // Leaving autosaves; the next entry offers to continue.
    assert_eq!(
        story.line(&output, &mut input, "salir").await.unwrap(),
        StoryLineOutcome::Left
    );
    assert!(drain(&output).0.contains("autoguardado"));
    story
        .line(&output, &mut input, &format!("historia {CRIPTA}"))
        .await
        .unwrap();
    let (text, gmcp) = drain(&output);
    assert!(text.contains("Escribe «continuar» (continue)"));
    assert!(
        gmcp.iter()
            .any(|payload| payload.contains("\"offerContinue\":true"))
    );
    assert!(text.contains("1) Tira un dado"));

    story.line(&output, &mut input, "continuar").await.unwrap();
    let (text, _) = drain(&output);
    assert!(text.contains("Continúas la partida autoguardada."));
    assert!(text.contains("2) Avanzar por el pasillo del norte"));

    story.line(&output, &mut input, "borrar 4").await.unwrap();
    assert!(drain(&output).0.contains("Partida de la ranura 4 borrada."));
    story.line(&output, &mut input, "cargar 4").await.unwrap();
    assert!(drain(&output).0.contains("Esa ranura está vacía."));
    story.line(&output, &mut input, "load 12").await.unwrap();
    assert!(drain(&output).0.contains("del 1 al 9"));
}

#[test]
fn oversized_gmcp_story_summary_degrades_within_the_limit() {
    let mut input = gmcp_input(r#"["Tiamud.Story 1"]"#, false);
    let view = StoryProjection {
        story_id: "x".into(),
        title: "t".repeat(200),
        view_rev: 4,
        turn: 1,
        score: 0,
        ending: None,
        items: (0..32)
            .map(|index| tiamud_protocol::StoryItem::Choice {
                id: format!("c{index:016x}"),
                label: format!("{}{index}", "\"".repeat(250)),
            })
            .chain((0..64).map(|index| tiamud_protocol::StoryItem::Stat {
                label: format!("{}{index}", "s".repeat(250)),
                value: "1".into(),
            }))
            .collect(),
        can_undo: false,
    };
    let mut music = false;
    let bytes = input.negotiation.story_gmcp(&view, &mut music);
    let mut parser = TelnetParser::default();
    parser.feed(&bytes);
    let payloads = parser
        .take_events()
        .into_iter()
        .filter_map(|event| match event {
            TelnetEvent::Subnegotiation { data, .. } => Some(data),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].len() <= MAX_GMCP_BYTES);
    let summary: serde_json::Value =
        serde_json::from_slice(payloads[0].strip_prefix(b"Tiamud.Story.View ").unwrap()).unwrap();
    assert_eq!(summary["truncated"], true);
    assert_eq!(summary["viewRev"], 4);
}
