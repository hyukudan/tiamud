//! Browser adapter coverage of shared story instances (ADR 0033) and the
//! recorder of the two-player Playwright fixture.

use super::super::{
    ClientMessage, ClientMessageError, MAX_WS_MESSAGE_BYTES, OutboundHandle, parse_client_message,
};
use super::*;
use crate::operations::RuntimeMetrics;
use crate::story_sessions::{StoryCatalog, StoryLimits};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use tiamud_story_core::StoryPack;

const CRIPTA: &str = "cripta-del-nigromante";
const ARIA: AccountId = AccountId(1);
const BRUNO: AccountId = AccountId(2);
/// Placeholder that replaces the random join code in the recorded fixture.
const FIXTURE_CODE: &str = "ABCD2345";

fn stories() -> StoryManager {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../story-core/fixtures/cripta-del-nigromante.story.json"),
    )
    .unwrap();
    StoryManager::new(
        StoryCatalog::from_packs(
            vec![StoryPack::from_json_slice(&bytes).unwrap()],
            BTreeMap::new(),
        )
        .unwrap(),
        StoryLimits::default(),
    )
}

fn outbound() -> OutboundHandle {
    OutboundHandle::new(256, Arc::new(RuntimeMetrics::default()))
}

/// Pops every queued frame as `{type, data}`, asserting the frame budget.
fn frames(outbound: &OutboundHandle) -> Vec<Value> {
    let mut frames = Vec::new();
    while let Some(frame) = outbound.pop() {
        assert!(serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);
        frames.push(json!({"type": frame.value["type"], "data": frame.value["data"]}));
    }
    frames
}

fn types(frames: &[Value]) -> Vec<&str> {
    frames
        .iter()
        .map(|frame| frame["type"].as_str().unwrap())
        .collect()
}

fn find<'a>(frames: &'a [Value], kind: &str) -> &'a Value {
    &frames
        .iter()
        .rev()
        .find(|frame| frame["type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} in {frames:?}"))["data"]
}

struct Player {
    session: WebStorySession,
    out: OutboundHandle,
    seq: u64,
}

impl Player {
    fn new(manager: &StoryManager, account: AccountId, name: &str) -> Self {
        Self {
            session: WebStorySession::new(Some(manager.clone()), account, false).with_name(name),
            out: outbound(),
            seq: 1,
        }
    }

    /// Handles every story event that is ready now.
    async fn pump(&mut self) {
        while let Ok(event) =
            tokio::time::timeout(Duration::from_millis(1), self.session.next_event()).await
        {
            assert!(self.session.handle_event(&self.out, &mut self.seq, event));
        }
    }

    async fn command(&mut self, text: &str) -> bool {
        self.session
            .text_command(&self.out, &mut self.seq, "REQUEST", text)
            .await
            .unwrap()
    }

    fn take(&mut self) -> Vec<Value> {
        frames(&self.out)
    }
}

fn view_rev(view: &Value) -> u32 {
    u32::try_from(view["viewRev"].as_u64().unwrap()).unwrap()
}

/// First numbered action's leaf id (choice order matches the server's).
fn first_choice(view: &Value) -> (String, String) {
    let item = view["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "choice" || item["type"] == "dialogueOption")
        .unwrap();
    (
        item["id"].as_str().unwrap().to_owned(),
        item["label"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test(start_paused = true)]
async fn two_browsers_share_vote_and_recover() {
    let manager = stories();
    let mut aria = Player::new(&manager, ARIA, "Aria");
    let mut bruno = Player::new(&manager, BRUNO, "Bruno");

    assert!(aria.command(&format!("historia compartir {CRIPTA}")).await);
    let hosted = aria.take();
    assert_eq!(
        types(&hosted),
        [
            "action.result",
            "story.entered",
            "story.lobby",
            "story.view",
            "story.tally"
        ]
    );
    let lobby = find(&hosted, "story.lobby");
    assert_eq!(lobby["you"], "host");
    assert_eq!(lobby["mode"], "vote");
    let code = lobby["code"].as_str().unwrap().to_owned();

    // The host invites Bruno; his session receives the code.
    assert!(aria.command("invitar Bruno").await);
    assert_eq!(find(&aria.take(), "action.result")["ok"], true);
    bruno.pump().await;
    let invited = bruno.take();
    assert_eq!(types(&invited), ["story.invitation"]);
    assert_eq!(invited[0]["data"]["code"], code.as_str());
    assert_eq!(invited[0]["data"]["host"], "Aria");

    // Typed join.
    assert!(
        bruno
            .session
            .join(&bruno.out, &mut bruno.seq, &code, false)
            .await
    );
    let joined = bruno.take();
    assert_eq!(
        types(&joined),
        ["story.entered", "story.lobby", "story.view", "story.tally"]
    );
    let lobby = find(&joined, "story.lobby");
    assert_eq!(lobby["you"], "player");
    assert!(lobby.get("code").is_none(), "only the host sees the code");
    assert_eq!(find(&joined, "story.view")["canUndo"], false);
    aria.pump().await;
    let seen = aria.take();
    assert_eq!(
        find(&seen, "story.lobby")["seats"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Aria votes by typed number; Bruno by `story.vote`.
    let view = find(&joined, "story.view").clone();
    let (leaf, _) = first_choice(&view);
    assert!(aria.command("1").await);
    let voted = aria.take();
    assert_eq!(
        find(&voted, "action.result")["message"],
        format!("Votas por: {}", first_choice(&view).1)
    );
    aria.pump().await;
    let tally = aria.take();
    assert_eq!(find(&tally, "story.tally")["voted"], 1);
    assert_eq!(find(&tally, "story.tally")["yourVote"], leaf.as_str());
    bruno.pump().await;
    let tally = find(&bruno.take(), "story.tally").clone();
    assert_eq!(tally["voted"], 1);
    assert!(tally.get("yourVote").is_none());
    assert_eq!(tally["choices"][0]["choiceId"], leaf.as_str());

    // Forged and stale votes.
    assert!(
        bruno
            .session
            .vote(
                &bruno.out,
                &mut bruno.seq,
                Some("c0123456789abcdef".into()),
                view_rev(&view)
            )
            .await
    );
    assert_eq!(find(&bruno.take(), "story.error")["code"], "unknown_choice");
    assert!(
        bruno
            .session
            .vote(
                &bruno.out,
                &mut bruno.seq,
                Some(leaf.clone()),
                view_rev(&view) + 3
            )
            .await
    );
    let stale = bruno.take();
    assert_eq!(find(&stale, "story.error")["code"], "stale_view");
    assert_eq!(
        types(&stale)[1..],
        ["story.lobby", "story.view", "story.tally"]
    );

    assert!(
        bruno
            .session
            .vote(
                &bruno.out,
                &mut bruno.seq,
                Some(leaf.clone()),
                view_rev(&view)
            )
            .await
    );
    let decided = bruno.take();
    assert_eq!(view_rev(find(&decided, "story.view")), view_rev(&view) + 1);
    bruno.pump().await;
    assert_eq!(types(&bruno.take()), ["story.tally"], "no duplicate view");
    aria.pump().await;
    let aria_decided = aria.take();
    assert_eq!(types(&aria_decided), ["story.view", "story.tally"]);
    assert_eq!(find(&aria_decided, "story.view")["canUndo"], true);

    // Host-only actions answer typed errors.
    assert!(bruno.session.undo(&bruno.out, &mut bruno.seq).await);
    assert_eq!(find(&bruno.take(), "story.error")["code"], "host_only");
    assert!(
        bruno
            .session
            .mode(&bruno.out, &mut bruno.seq, StoryDecisionMode::Host, None)
            .await
    );
    assert_eq!(find(&bruno.take(), "story.error")["code"], "host_only");

    // Recovery re-sends lobby, view and tally; a reconnect resumes the seat.
    assert!(bruno.session.resend(&bruno.out, &mut bruno.seq).await);
    assert_eq!(
        types(&bruno.take()),
        ["story.lobby", "story.view", "story.tally"]
    );
    drop(bruno);
    let mut again = Player::new(&manager, BRUNO, "Bruno");
    assert!(again.session.resume(&again.out, &mut again.seq).await);
    assert_eq!(
        types(&again.take()),
        ["story.entered", "story.lobby", "story.view", "story.tally"]
    );

    // The host kicks Bruno: his session leaves with `kicked`.
    assert!(aria.session.kick(&aria.out, &mut aria.seq, "Bruno").await);
    again.pump().await;
    let kicked = again.take();
    assert_eq!(find(&kicked, "story.left")["reason"], "kicked");
    assert!(!again.session.active());
    assert!(
        again
            .session
            .join(&again.out, &mut again.seq, &code, false)
            .await
    );
    assert_eq!(find(&again.take(), "story.error")["code"], "unknown_code");

    // The last seat leaving ends the instance.
    assert!(aria.session.leave(&aria.out, &mut aria.seq).await);
    assert_eq!(find(&aria.take(), "story.left")["reason"], "left");
    assert_eq!(manager.live_instances(), 0);
}

#[tokio::test(start_paused = true)]
async fn spectators_watch_without_acting_and_see_the_end() {
    let manager = stories();
    let mut aria = Player::new(&manager, ARIA, "Aria");
    let mut carla = Player::new(&manager, AccountId(3), "Carla");
    assert!(aria.command(&format!("story host {CRIPTA}")).await);
    let code = find(&aria.take(), "story.lobby")["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        carla
            .command(&format!("story watch {}", code.to_lowercase()))
            .await
    );
    let watching = carla.take();
    assert_eq!(find(&watching, "story.lobby")["you"], "spectator");
    assert!(carla.command("1").await);
    assert_eq!(find(&carla.take(), "story.error")["code"], "spectator");
    assert!(carla.command("asientos").await);
    let seats = carla.take();
    assert!(
        find(&seats, "action.result")["message"]
            .as_str()
            .unwrap()
            .contains("Espectadores: 1/32")
    );
    // The only seat leaves: the spectator's story ends.
    assert!(aria.session.leave(&aria.out, &mut aria.seq).await);
    carla.pump().await;
    assert_eq!(find(&carla.take(), "story.left")["reason"], "ended");
    assert!(!carla.session.active());
}

#[test]
fn shared_story_client_messages_parse_strictly() {
    assert!(matches!(
        parse_client_message(
            r#"{"v":1,"type":"story.vote","data":{"choiceId":"c0123456789abcdef","viewRev":2}}"#
        ),
        Ok(ClientMessage::StoryVote { leaf: Some(id), view_rev: 2 }) if id == "c0123456789abcdef"
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.vote","data":{"viewRev":2}}"#),
        Ok(ClientMessage::StoryVote {
            leaf: None,
            view_rev: 2
        })
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.share","data":{}}"#),
        Ok(ClientMessage::StoryShare)
    ));
    assert!(matches!(
        parse_client_message(
            r#"{"v":1,"type":"story.join","data":{"code":"abcd2345","spectate":true}}"#
        ),
        Ok(ClientMessage::StoryJoin { spectate: true, .. })
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.invite","data":{"name":" Bruno "}}"#),
        Ok(ClientMessage::StoryInvite { name }) if name == "Bruno"
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.kick","data":{"name":"Bruno"}}"#),
        Ok(ClientMessage::StoryKick { .. })
    ));
    assert!(matches!(
        parse_client_message(
            r#"{"v":1,"type":"story.mode","data":{"mode":"turns","voteSeconds":45}}"#
        ),
        Ok(ClientMessage::StoryMode {
            mode: StoryDecisionMode::Turns,
            vote_seconds: Some(45)
        })
    ));
    let long_name = format!(
        r#"{{"v":1,"type":"story.kick","data":{{"name":"{}"}}}}"#,
        "n".repeat(65)
    );
    for invalid in [
        r#"{"v":1,"type":"story.vote","data":{"choiceId":"c0","viewRev":0}}"#,
        r#"{"v":1,"type":"story.vote","data":{"choiceId":"","viewRev":1}}"#,
        r#"{"v":1,"type":"story.vote","data":{"choiceId":"c0/","viewRev":1}}"#,
        r#"{"v":1,"type":"story.vote","data":{"viewRev":1,"x":1}}"#,
        r#"{"v":1,"type":"story.share","data":{"x":1}}"#,
        r#"{"v":1,"type":"story.join","data":{"code":"ABCD2345"}}"#,
        r#"{"v":1,"type":"story.join","data":{"code":"ABCD23456","spectate":false}}"#,
        r#"{"v":1,"type":"story.join","data":{"code":"AB-D2345","spectate":false}}"#,
        r#"{"v":1,"type":"story.invite","data":{"name":""}}"#,
        r#"{"v":1,"type":"story.invite","data":{"name":"a\u0007"}}"#,
        &long_name,
        r#"{"v":1,"type":"story.mode","data":{"mode":"chaos"}}"#,
        r#"{"v":1,"type":"story.mode","data":{"mode":"vote","voteSeconds":-1}}"#,
    ] {
        assert_eq!(
            parse_client_message(invalid).err(),
            Some(ClientMessageError::InvalidEnvelope),
            "{invalid}"
        );
    }
}

/// The two-player Playwright scenario replays these real server frames:
/// Aria shares the story, Bruno joins with the code and both vote every
/// choice through `story.vote` until an ending. The random join code is
/// replaced by a fixed placeholder; ids and dice come from fixed seeds.
#[tokio::test(start_paused = true)]
async fn browser_shared_fixture_is_current() {
    const UPDATE: &str = "TIAMUD_UPDATE_STORY_E2E_FIXTURE";
    let manager = stories();
    // A seeded solo instance that `historia compartir` then shares.
    drop(manager.enter_seeded(ARIA, CRIPTA, 7, Some(1)).unwrap());
    let mut players = [
        Player::new(&manager, ARIA, "Aria"),
        Player::new(&manager, BRUNO, "Bruno"),
    ];
    let mut steps = Vec::new();
    let step = |actor: &str, input: Value, label: Option<&str>, a: Vec<Value>, b: Vec<Value>| json!({"actor": actor, "input": input, "label": label, "frames": {"A": a, "B": b}});

    let share = format!("historia compartir {CRIPTA}");
    assert!(players[0].command(&share).await);
    players[1].pump().await;
    let (a, b) = (players[0].take(), players[1].take());
    let code = find(&a, "story.lobby")["code"].as_str().unwrap().to_owned();
    let title = find(&a, "story.entered")["title"]
        .as_str()
        .unwrap()
        .to_owned();
    steps.push(step(
        "A",
        json!({"type": "command.submit", "data": {"command": share}}),
        None,
        a,
        b,
    ));
    let join = format!("historia unirse {code}");
    assert!(players[1].command(&join).await);
    players[0].pump().await;
    let (a, b) = (players[0].take(), players[1].take());
    let mut view = find(&b, "story.view").clone();
    steps.push(step(
        "B",
        json!({"type": "command.submit", "data": {"command": join}}),
        None,
        a,
        b,
    ));
    let mut rounds = 0;
    while view["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "choice")
    {
        rounds += 1;
        assert!(rounds < 64, "the vote walk must reach an ending");
        let (leaf, label) = first_choice(&view);
        let rev = view_rev(&view);
        for (index, actor) in ["A", "B"].into_iter().enumerate() {
            let player = &mut players[index];
            assert!(
                player
                    .session
                    .vote(&player.out, &mut player.seq, Some(leaf.clone()), rev)
                    .await
            );
            for player in &mut players {
                player.pump().await;
            }
            let (a, b) = (players[0].take(), players[1].take());
            if index == 1 {
                view = find(&b, "story.view").clone();
            }
            steps.push(step(
                actor,
                json!({"type": "story.vote", "data": {"choiceId": leaf, "viewRev": rev}}),
                Some(&label),
                a,
                b,
            ));
        }
    }
    assert!(
        view["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "scene" && !item["ending"].is_null()),
        "the shared walk ends in an ending"
    );
    let document = json!({
        "storyId": CRIPTA,
        "title": title,
        "code": FIXTURE_CODE,
        "steps": steps,
    });
    let rendered = format!("{}\n", serde_json::to_string_pretty(&document).unwrap())
        .replace(&code, FIXTURE_CODE);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/web/e2e/fixtures/story-shared.json");
    if std::env::var_os(UPDATE).is_some_and(|value| value == "1") {
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == rendered,
        "{} is stale; run `{UPDATE}=1 cargo test --locked -p tiamud-server browser_` and review the diff",
        path.display()
    );
}
