//! Parser stories (StoryPack v3, ADR 0031) played through the session
//! manager: free-text commands, echo and replies, questions and answers,
//! meta commands mapped to session operations, bounds, the per-account
//! rate limit, the shared-instance policy and the recorded browser fixture.

use super::*;
use serde_json::json;
use std::path::PathBuf;

const FARO: &str = "faro-de-punta-gris";
const CRIPTA: &str = "cripta-del-nigromante";
const HOST: AccountId = AccountId(1);
const BRUNO: AccountId = AccountId(2);
const CARLA: AccountId = AccountId(3);

/// The reviewed walkthrough of the fixture, ending with the lantern's
/// question and its answer instead of «enciende el fanal».
const SCRIPT: &[&str] = &[
    "examina la barca",
    "coge la medalla",
    "n",
    "habla con remedios",
    "dale la medalla a remedios",
    "e",
    "abre el cofre",
    "coge la lámpara y enciéndela",
    "o",
    "abre la puerta con la llave",
    "abre la puerta. n",
    "lee el cuaderno",
    "sube",
    "coge la aceitera",
    "sube",
    "llena el depósito con la aceitera",
    "enciende la lampara",
    "el fanal",
];

fn fixture(path: &str) -> StoryPack {
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../story-core/fixtures")
            .join(path),
    )
    .unwrap();
    StoryPack::from_json_slice(&bytes).unwrap()
}

fn faro() -> StoryPack {
    fixture("parser/faro-de-punta-gris.story.json")
}

fn stories() -> StoryManager {
    StoryManager::new(
        StoryCatalog::from_packs(
            vec![faro(), fixture("cripta-del-nigromante.story.json")],
            BTreeMap::new(),
        )
        .unwrap(),
        StoryLimits::default(),
    )
}

fn blocks(view: &StoryProjection, wanted: StoryBlockKind) -> Vec<String> {
    view.items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Block { kind, text, .. } if *kind == wanted => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The scene text: a room's name in parser stories.
fn scene_text(view: &StoryProjection) -> String {
    view.items
        .iter()
        .find_map(|item| match item {
            StoryItem::Scene { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn replies(view: &StoryProjection) -> String {
    blocks(view, StoryBlockKind::Reply).join("\n")
}

async fn say(link: &StoryLink, view: &StoryProjection, text: &str) -> CommandReply {
    link.command(text, view.view_rev)
        .await
        .unwrap_or_else(|failure| panic!("{text:?}: {failure:?}"))
}

#[tokio::test]
async fn the_parser_adventure_plays_to_its_ending_through_commands() {
    let stories = stories();
    let link = stories.enter_seeded(HOST, FARO, 7, Some(1)).unwrap();
    assert!(link.is_parser());
    let mut view = link.view().await.unwrap();
    // The opening: room name as scene text, intro and description as
    // replies, a free-text prompt and no numbered actions.
    assert_eq!(
        view.prompt(),
        Some(&protocol::StoryPrompt {
            text: ">".into(),
            question: false
        })
    );
    assert!(replies(&view).starts_with("La tormenta te sorprendió"));
    assert!(view.actions().is_empty());
    assert!(blocks(&view, StoryBlockKind::Command).is_empty());
    let text = render_text(&view);
    assert!(text.contains("Playa de guijarros\r\n"));
    assert!(text.contains("Escribe una orden"));

    for line in &SCRIPT[..SCRIPT.len() - 2] {
        let reply = say(&link, &view, line).await;
        assert_eq!(reply.follow_up, None, "{line}");
        view = reply.view;
        assert_eq!(blocks(&view, StoryBlockKind::Command), vec![*line]);
    }
    // The ambiguous lamp asks; the prompt says a question is pending and
    // the question does not take a turn.
    let turn = view.turn;
    view = say(&link, &view, "enciende la lampara").await.view;
    assert_eq!(
        replies(&view),
        "¿Cuál quieres decir: la lámpara de aceite o el fanal?"
    );
    assert!(view.prompt().unwrap().question);
    assert_eq!(view.turn, turn);
    assert!(render_text(&view).contains("Responde a la pregunta"));
    // Answering finishes the story with a victory and full score.
    view = say(&link, &view, "el fanal").await.view;
    assert_eq!(view.ending, Some(StoryEndingKind::Victory));
    assert_eq!(view.score, 30);
    assert_eq!(view.prompt(), None);
    assert_eq!(
        link.command("mira", view.view_rev).await,
        Err(StoryFailure::Ended)
    );
}

#[tokio::test]
async fn meta_commands_map_to_session_operations() {
    let stories = stories();
    let link = stories.enter_seeded(HOST, FARO, 7, Some(1)).unwrap();
    let start = link.view().await.unwrap();

    // Score and help answer in the view without taking a turn.
    let score = say(&link, &start, "puntos").await;
    assert_eq!(score.follow_up, None);
    assert_eq!(replies(&score.view), "Llevas 0 de 30 puntos en 0 turnos.");
    assert!(!score.view.can_undo);

    // A command is an undo step; «deshacer» undoes it like `story.undo`.
    let taken = say(&link, &score.view, "examina la barca").await.view;
    assert_eq!(taken.turn, 1);
    assert!(taken.can_undo);
    let undone = say(&link, &taken, "deshacer").await.view;
    assert_eq!(undone.turn, 0);
    assert!(!undone.can_undo);
    assert_eq!(
        link.command("deshacer", undone.view_rev).await,
        Err(StoryFailure::NothingToUndo)
    );
    // A line that runs commands and then undoes discards exactly that line.
    let line = say(&link, &undone, "examina la barca. deshacer").await.view;
    assert_eq!(line.turn, 0);

    // Save and restore without a slot point to the slot listing.
    let save = say(&link, &line, "guardar").await;
    assert_eq!(save.follow_up, Some(CommandFollowUp::Slots));
    assert!(replies(&save.view).contains("guardar N"));
    let restore = say(&link, &save.view, "cargar").await;
    assert_eq!(restore.follow_up, Some(CommandFollowUp::Slots));
    assert!(replies(&restore.view).contains("cargar N"));

    // Restart needs a confirmation; any other input cancels it.
    let moved = say(&link, &restore.view, "n").await.view;
    assert_eq!(moved.turn, 1);
    let asked = say(&link, &moved, "reiniciar").await.view;
    assert!(replies(&asked).contains("Escribe «reiniciar» otra vez"));
    assert_eq!(asked.turn, 1);
    let cancelled = say(&link, &asked, "mira").await.view;
    let asked = say(&link, &cancelled, "reiniciar").await.view;
    let restarted = say(&link, &asked, "reiniciar").await.view;
    assert_eq!(restarted.turn, 0);
    assert_eq!(scene_text(&restarted), "Playa de guijarros");
    assert!(replies(&restarted).contains("La historia vuelve a empezar."));
    assert!(restarted.can_undo);
    // The restart itself can be undone.
    let back = say(&link, &restarted, "deshacer").await.view;
    assert_eq!(back.turn, 2);
    assert_eq!(scene_text(&back), "Explanada del faro");

    // The parser's quit asks the transport to leave.
    let quit = say(&link, &back, "fin").await;
    assert_eq!(quit.follow_up, Some(CommandFollowUp::Leave));
}

#[tokio::test]
async fn commands_are_bounded_checked_and_rate_limited() {
    let stories = stories();
    let link = stories.enter_seeded(HOST, FARO, 7, Some(1)).unwrap();
    let view = link.view().await.unwrap();
    let long = "a".repeat(MAX_COMMAND_BYTES + 1);
    assert_eq!(
        link.command(&long, view.view_rev).await,
        Err(StoryFailure::InvalidCommand)
    );
    assert_eq!(
        StoryFailure::InvalidCommand.code(),
        StoryErrorCode::InvalidCommand
    );
    // Exactly 256 bytes reach the parser, which answers in the view.
    let limit = say(&link, &view, &"a".repeat(MAX_COMMAND_BYTES)).await.view;
    assert_eq!(limit.turn, 0);
    assert_eq!(
        link.command("mira", view.view_rev).await,
        Err(StoryFailure::StaleView)
    );
    // Control characters never reach the echo.
    let echoed = say(&link, &limit, "\u{7}mira\r\n").await.view;
    assert_eq!(blocks(&echoed, StoryBlockKind::Command), vec!["mira"]);

    // A gamebook has no parser.
    let other = stories.enter_seeded(BRUNO, CRIPTA, 7, Some(1)).unwrap();
    let gamebook = other.view().await.unwrap();
    assert!(!other.is_parser());
    assert_eq!(gamebook.prompt(), None);
    assert_eq!(
        other.command("mira", gamebook.view_rev).await,
        Err(StoryFailure::NoParser)
    );

    // The per-account burst is bounded.
    let mut current = echoed;
    let mut limited = false;
    for _ in 0..(COMMAND_BURST as usize * 3) {
        match link.command("mira", current.view_rev).await {
            Ok(reply) => current = reply.view,
            Err(failure) => {
                assert_eq!(failure, StoryFailure::CommandsRateLimited);
                assert_eq!(failure.code(), StoryErrorCode::RateLimited);
                limited = true;
                break;
            }
        }
    }
    assert!(limited, "the command burst must be bounded");
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_command_rate_limited_total 1")
    );
}

#[tokio::test]
async fn shared_parser_stories_never_vote_on_free_text() {
    let stories = stories();
    let host = stories
        .enter_seeded(HOST, FARO, 7, Some(1))
        .unwrap()
        .share("Aria")
        .await
        .unwrap();
    let code = host.frames().unwrap().borrow().lobby(HOST).code.unwrap();
    let bruno = stories.join(BRUNO, "Bruno", &code, false).await.unwrap();
    let carla = stories.join(CARLA, "Carla", &code, true).await.unwrap();
    let view = host.view().await.unwrap();

    // Vote mode (the default): only the host types; others are told so.
    assert_eq!(
        bruno.command("n", view.view_rev).await,
        Err(StoryFailure::HostOnly)
    );
    assert_eq!(
        carla.command("n", view.view_rev).await,
        Err(StoryFailure::Spectator)
    );
    let moved = say(&host, &view, "n").await.view;
    assert_eq!(scene_text(&moved), "Explanada del faro");
    // Every participant follows the same view through the shared frame.
    let seen = bruno.frames().unwrap().borrow().view.clone();
    assert_eq!(seen.view_rev, moved.view_rev);
    assert_eq!(blocks(&seen, StoryBlockKind::Command), vec!["n"]);

    // Turns mode: the turn holder types; a parse error or a question keeps
    // the turn, an executed command passes it on.
    host.set_mode(StoryDecisionMode::Turns, None).await.unwrap();
    assert_eq!(
        bruno.command("s", moved.view_rev).await,
        Err(StoryFailure::NotYourTurn)
    );
    let unknown = say(&host, &moved, "xyzzy").await.view;
    assert_eq!(unknown.turn, moved.turn);
    let mut current = say(&host, &unknown, "s").await.view;
    assert_eq!(
        host.command("n", current.view_rev).await,
        Err(StoryFailure::NotYourTurn)
    );
    // Undo and restart replace the game for everyone: host only.
    assert_eq!(
        bruno.command("deshacer", current.view_rev).await,
        Err(StoryFailure::HostOnly)
    );
    assert_eq!(
        bruno.command("reiniciar", current.view_rev).await,
        Err(StoryFailure::HostOnly)
    );
    current = say(&bruno, &current, "n").await.view;
    assert_eq!(scene_text(&current), "Explanada del faro");
    // The turn came back to the host.
    say(&host, &current, "mira").await;
}

#[test]
fn reserved_words_keep_their_meaning_in_parser_stories() {
    for (line, expected) in [
        ("mira", StoryInputCommand::Other),
        ("coge la llave", StoryInputCommand::Other),
        ("look", StoryInputCommand::Other),
        ("l", StoryInputCommand::Other),
        ("guardar", StoryInputCommand::Other),
        ("cargar", StoryInputCommand::Other),
        ("salir", StoryInputCommand::Leave),
        ("deshacer", StoryInputCommand::Undo),
        ("ver", StoryInputCommand::Look),
        ("guardar 3", StoryInputCommand::Save(Some(3))),
        ("cargar 0", StoryInputCommand::Restore(Some(0))),
        ("partidas", StoryInputCommand::Slots),
        ("2", StoryInputCommand::Number(2)),
        ("votar 1", StoryInputCommand::Vote(Some(1))),
    ] {
        assert_eq!(parse_parser_story_input(line), expected, "{line}");
    }
    // Gamebooks keep `look` and a bare `guardar` reserved.
    assert_eq!(parse_story_input("look"), StoryInputCommand::Look);
    assert_eq!(parse_story_input("guardar"), StoryInputCommand::Save(None));
}

/// The Playwright parser scenario replays real server frames: each view
/// with the exact `story.command` the browser sends next. Regenerate with
/// `TIAMUD_UPDATE_STORY_E2E_FIXTURE=1`.
#[tokio::test]
async fn browser_parser_fixture_is_current() {
    const UPDATE: &str = "TIAMUD_UPDATE_STORY_E2E_FIXTURE";
    let stories = stories();
    let link = stories
        .enter_seeded(AccountId(1), FARO, 7, Some(1))
        .unwrap();
    let mut view = link.view().await.unwrap();
    let mut steps = Vec::new();
    for line in SCRIPT {
        steps.push(json!({
            "view": view.web_parts(false),
            "label": line,
            "input": {"type": "story.command", "data": {"text": line, "viewRev": view.view_rev}},
        }));
        view = say(&link, &view, line).await.view;
    }
    assert!(view.ending.is_some(), "the script must reach the ending");
    steps.push(json!({"view": view.web_parts(false), "label": null, "input": null}));
    for part in steps
        .iter()
        .flat_map(|step| step["view"].as_array().unwrap())
    {
        let frame = serde_json::to_vec(
            &json!({"v": 1, "type": "story.view", "seq": u64::MAX, "data": part}),
        )
        .unwrap();
        assert!(frame.len() <= MAX_STORY_FRAME_BYTES);
    }
    let document = json!({"storyId": FARO, "title": link.title(), "steps": steps});
    let rendered = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/web/e2e/fixtures/story-faro.json");
    if std::env::var_os(UPDATE).is_some_and(|value| value == "1") {
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == rendered,
        "{} is stale; run `{UPDATE}=1 cargo test --locked -p tiamud-server browser_parser_fixture` and review the diff",
        path.display()
    );
}
