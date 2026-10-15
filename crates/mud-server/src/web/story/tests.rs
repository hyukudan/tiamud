use super::super::{
    ClientMessage, ClientMessageError, MAX_WS_MESSAGE_BYTES, OutboundHandle, parse_client_message,
};
use super::*;
use crate::operations::RuntimeMetrics;
use crate::story_sessions::{StoryCatalog, StoryLimits};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use tiamud_story_core::StoryPack;

const CRIPTA: &str = "cripta-del-nigromante";

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
    OutboundHandle::new(64, Arc::new(RuntimeMetrics::default()))
}

/// Pops every queued frame, asserting the 16 KiB budget and sequence order.
fn frames(outbound: &OutboundHandle) -> Vec<Value> {
    let mut frames = Vec::new();
    while let Some(frame) = outbound.pop() {
        assert!(serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);
        frames.push(frame.value);
    }
    frames
}

fn types(frames: &[Value]) -> Vec<&str> {
    frames
        .iter()
        .map(|frame| frame["type"].as_str().unwrap())
        .collect()
}

fn last_view(frames: &[Value]) -> &Value {
    frames
        .iter()
        .rev()
        .find(|frame| frame["type"] == "story.view")
        .map(|frame| &frame["data"])
        .unwrap()
}

#[tokio::test]
async fn browser_enters_chooses_recovers_and_leaves() {
    let manager = stories();
    let out = outbound();
    let mut seq = 1;
    let mut session = WebStorySession::new(Some(manager.clone()), AccountId(1), false);

    assert_eq!(
        session.text_command(&out, &mut seq, "r1", "look").await,
        None
    );
    assert_eq!(
        session
            .text_command(&out, &mut seq, "r2", &format!("historia {CRIPTA}"))
            .await,
        Some(true)
    );
    let entered = frames(&out);
    assert_eq!(
        types(&entered),
        ["action.result", "story.entered", "story.view"]
    );
    assert_eq!(entered[1]["data"]["storyId"], CRIPTA);
    let view = last_view(&entered).clone();
    assert_eq!(view["parts"], 1);
    let id = view["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "choice")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let rev = u32::try_from(view["viewRev"].as_u64().unwrap()).unwrap();

    // Forged and stale ids are rejected; stale answers with the current view.
    assert!(
        session
            .input(
                &out,
                &mut seq,
                StoryRequest::Choose {
                    id: "c0123456789abcdef".to_owned()
                },
                rev
            )
            .await
    );
    let forged = frames(&out);
    assert_eq!(types(&forged), ["story.error"]);
    assert_eq!(forged[0]["data"]["code"], "unknown_choice");
    assert!(
        session
            .input(
                &out,
                &mut seq,
                StoryRequest::Choose { id: id.clone() },
                rev + 5
            )
            .await
    );
    let stale = frames(&out);
    assert_eq!(types(&stale), ["story.error", "story.view"]);
    assert_eq!(stale[0]["data"]["code"], "stale_view");
    assert_eq!(last_view(&stale)["viewRev"], rev);

    assert!(
        session
            .input(&out, &mut seq, StoryRequest::Choose { id: id.clone() }, rev)
            .await
    );
    let chosen = frames(&out);
    assert_eq!(last_view(&chosen)["viewRev"], rev + 1);
    assert_eq!(last_view(&chosen)["canUndo"], true);

    // Recovery re-sends the same authoritative view.
    assert!(session.resend(&out, &mut seq).await);
    assert_eq!(last_view(&frames(&out)), last_view(&chosen));

    // A reconnecting session resumes the same instance.
    let mut reconnected = WebStorySession::new(Some(manager.clone()), AccountId(1), false);
    assert!(reconnected.resume(&out, &mut seq).await);
    let resumed = frames(&out);
    assert_eq!(types(&resumed), ["story.entered", "story.view"]);
    assert_eq!(last_view(&resumed), last_view(&chosen));

    // Typed numbers map to the current view; other world commands wait.
    assert_eq!(
        session.text_command(&out, &mut seq, "r3", "north").await,
        Some(true)
    );
    let refused = frames(&out);
    assert_eq!(refused[0]["data"]["ok"], false);
    assert_eq!(
        session.text_command(&out, &mut seq, "r4", "1").await,
        Some(true)
    );
    assert_eq!(types(&frames(&out)), ["action.result", "story.view"]);

    assert!(session.undo(&out, &mut seq).await);
    assert_eq!(types(&frames(&out)), ["story.view"]);

    assert!(session.leave(&out, &mut seq).await);
    let left = frames(&out);
    assert_eq!(types(&left), ["story.left"]);
    assert_eq!(left[0]["data"]["reason"], "left");
    assert!(!session.active());
    assert_eq!(manager.live_instances(), 0);

    // The other session learns the instance is gone on its next input.
    assert!(reconnected.undo(&out, &mut seq).await);
    let gone = frames(&out);
    assert_eq!(types(&gone), ["story.error", "story.left"]);
    assert_eq!(gone[1]["data"]["reason"], "evicted");
    assert!(!reconnected.active());

    assert!(
        session
            .input(&out, &mut seq, StoryRequest::Choose { id: id.clone() }, rev)
            .await
    );
    assert_eq!(frames(&out)[0]["data"]["code"], "not_in_story");
}

#[tokio::test]
async fn audio_clients_stop_story_music_on_leave_and_disabled_servers_answer() {
    let out = outbound();
    let mut seq = 1;
    let mut session = WebStorySession::new(Some(stories()), AccountId(2), true);
    session
        .text_command(&out, &mut seq, "r1", &format!("story {CRIPTA}"))
        .await;
    frames(&out);
    session.leave(&out, &mut seq).await;
    assert_eq!(types(&frames(&out)), ["media.stop", "story.left"]);

    let mut disabled = WebStorySession::new(None, AccountId(2), false);
    assert_eq!(
        disabled
            .text_command(&out, &mut seq, "r2", "historias")
            .await,
        Some(true)
    );
    let answer = frames(&out);
    assert_eq!(answer[0]["data"]["ok"], false);
    assert_eq!(
        disabled.text_command(&out, &mut seq, "r3", "look").await,
        None
    );
}

#[test]
fn story_client_messages_parse_strictly() {
    assert!(matches!(
        parse_client_message(
            r#"{"v":1,"type":"story.choose","data":{"id":"c0123456789abcdef","viewRev":2}}"#
        ),
        Ok(ClientMessage::StoryInput { request: StoryRequest::Choose { id }, view_rev: 2 }) if id == "c0123456789abcdef"
    ));
    assert!(matches!(
        parse_client_message(
            r#"{"v":1,"type":"story.dialogue","data":{"id":"c0123456789abcdef","viewRev":2}}"#
        ),
        Ok(ClientMessage::StoryInput {
            request: StoryRequest::Dialogue { .. },
            view_rev: 2
        })
    ));
    assert!(matches!(
        parse_client_message(
            r#"{"v":1,"type":"story.menu","data":{"verb":"c0123456789abcdef","target":"c1123456789abcdef","viewRev":3}}"#
        ),
        Ok(ClientMessage::StoryInput {
            request: StoryRequest::Menu {
                target: Some(_),
                ..
            },
            view_rev: 3
        })
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.undo","data":{}}"#),
        Ok(ClientMessage::StoryUndo)
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.leave","data":{}}"#),
        Ok(ClientMessage::StoryLeave)
    ));
    for invalid in [
        r#"{"v":1,"type":"story.choose","data":{"id":"c0","viewRev":0}}"#,
        r#"{"v":1,"type":"story.choose","data":{"id":"","viewRev":1}}"#,
        r#"{"v":1,"type":"story.choose","data":{"id":"c0123456789abcdef0","viewRev":1}}"#,
        r#"{"v":1,"type":"story.choose","data":{"id":"c0/","viewRev":1}}"#,
        r#"{"v":1,"type":"story.choose","data":{"id":"c0","viewRev":1,"x":1}}"#,
        r#"{"v":1,"type":"story.choose","data":{"id":"c0","viewRev":4294967296}}"#,
        r#"{"v":1,"type":"story.undo","data":{"x":1}}"#,
        r#"{"v":1,"type":"story.menu","data":{"viewRev":1}}"#,
        r#"{"v":1,"type":"story.menu","data":{"verb":"c0","target":"","viewRev":1}}"#,
        r#"{"v":1,"type":"story.menu","data":{"verb":"c0","viewRev":0}}"#,
        r#"{"v":1,"type":"story.leave","data":[]}"#,
    ] {
        assert_eq!(
            parse_client_message(invalid).err(),
            Some(ClientMessageError::InvalidEnvelope),
            "{invalid}"
        );
    }
}

#[test]
fn story_slot_messages_parse_strictly() {
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.save","data":{"slot":3}}"#),
        Ok(ClientMessage::StorySave { slot: 3 })
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.restore","data":{"slot":0}}"#),
        Ok(ClientMessage::StoryRestore { slot: 0 })
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.deleteSlot","data":{"slot":9}}"#),
        Ok(ClientMessage::StoryDeleteSlot { slot: 9 })
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.slots","data":{}}"#),
        Ok(ClientMessage::StorySlots)
    ));
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.continue","data":{}}"#),
        Ok(ClientMessage::StoryRestore { slot: 0 })
    ));
    for invalid in [
        r#"{"v":1,"type":"story.save","data":{"slot":10}}"#,
        r#"{"v":1,"type":"story.save","data":{"slot":-1}}"#,
        r#"{"v":1,"type":"story.save","data":{"slot":"1"}}"#,
        r#"{"v":1,"type":"story.save","data":{}}"#,
        r#"{"v":1,"type":"story.restore","data":{"slot":1,"x":1}}"#,
        r#"{"v":1,"type":"story.deleteSlot","data":{"slot":300}}"#,
        r#"{"v":1,"type":"story.slots","data":{"x":1}}"#,
        r#"{"v":1,"type":"story.continue","data":[]}"#,
    ] {
        assert_eq!(
            parse_client_message(invalid).err(),
            Some(ClientMessageError::InvalidEnvelope),
            "{invalid}"
        );
    }
}

#[tokio::test]
async fn browser_saves_leaves_reenters_and_continues_with_sqlite_slots() {
    use crate::persistence::CharacterStore as _;
    let directory = tempfile::TempDir::new().unwrap();
    let store =
        crate::persistence::SqliteCharacterStore::open(&directory.path().join("state.sqlite3"))
            .await
            .unwrap();
    let account = store.load_or_create_account("Lectora").await.unwrap().id;
    let manager = stories();
    manager.attach_store(Arc::new(store));
    let out = outbound();
    let mut seq = 1;
    let mut session = WebStorySession::new(Some(manager.clone()), account, false);

    session
        .text_command(&out, &mut seq, "r1", &format!("historia {CRIPTA}"))
        .await;
    let entered = frames(&out);
    // A fresh instance lists its (empty) slots without a continue offer.
    assert_eq!(
        types(&entered),
        [
            "action.result",
            "story.entered",
            "story.view",
            "story.slots"
        ]
    );
    assert_eq!(
        entered[3]["data"],
        serde_json::json!({"slots": [], "offerContinue": false})
    );

    // Slot 0 is the autosave: saving there is a typed error.
    assert!(session.save(&out, &mut seq, 0).await);
    let refused = frames(&out);
    assert_eq!(types(&refused), ["story.error"]);
    assert_eq!(refused[0]["data"]["code"], "invalid_slot");

    session.text_command(&out, &mut seq, "r2", "1").await;
    let played = last_view(&frames(&out)).clone();
    assert!(session.save(&out, &mut seq, 2).await);
    let saved = frames(&out);
    assert_eq!(types(&saved), ["story.saved"]);
    assert_eq!(saved[0]["data"]["slot"], 2);

    assert!(session.slots(&out, &mut seq).await);
    let listed = frames(&out);
    assert_eq!(types(&listed), ["story.slots"]);
    assert_eq!(listed[0]["data"]["slots"][0]["slot"], 2);
    assert_eq!(listed[0]["data"]["slots"][0]["compatible"], true);

    // Leaving autosaves; entering again offers to continue.
    assert!(session.leave(&out, &mut seq).await);
    assert_eq!(types(&frames(&out)), ["story.left"]);
    session
        .text_command(&out, &mut seq, "r3", &format!("historia {CRIPTA}"))
        .await;
    let again = frames(&out);
    assert_eq!(
        types(&again),
        [
            "action.result",
            "story.entered",
            "story.view",
            "story.slots"
        ]
    );
    assert_eq!(again[3]["data"]["offerContinue"], true);
    assert_eq!(last_view(&again)["turn"], 0);

    assert!(session.restore(&out, &mut seq, 0).await);
    let continued = frames(&out);
    assert_eq!(types(&continued), ["story.restored", "story.view"]);
    assert_eq!(continued[0]["data"]["slot"], 0);
    let view = last_view(&continued);
    assert_eq!(view["turn"], played["turn"]);
    assert_eq!(view["canUndo"], false);

    // Text commands mirror the typed messages.
    session.text_command(&out, &mut seq, "r4", "partidas").await;
    let text = frames(&out);
    assert_eq!(types(&text), ["action.result", "story.slots"]);
    assert!(
        text[0]["data"]["message"]
            .as_str()
            .unwrap()
            .contains("0 (autoguardado)")
    );
    session.text_command(&out, &mut seq, "r5", "borrar 2").await;
    let deleted = frames(&out);
    assert_eq!(types(&deleted), ["action.result", "story.slots"]);
    assert_eq!(deleted[1]["data"]["slots"].as_array().unwrap().len(), 1);
    session.text_command(&out, &mut seq, "r6", "cargar 2").await;
    let empty = frames(&out);
    assert_eq!(types(&empty), ["action.result", "story.error"]);
    assert_eq!(empty[1]["data"]["code"], "slot_empty");
    session
        .text_command(&out, &mut seq, "r7", "continuar")
        .await;
    assert_eq!(
        types(&frames(&out)),
        ["action.result", "story.restored", "story.view"]
    );
}

fn parser_stories() -> StoryManager {
    let read = |path: &str| {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../story-core/fixtures")
                .join(path),
        )
        .unwrap();
        StoryPack::from_json_slice(&bytes).unwrap()
    };
    StoryManager::new(
        StoryCatalog::from_packs(
            vec![
                read("parser/faro-de-punta-gris.story.json"),
                read("cripta-del-nigromante.story.json"),
            ],
            BTreeMap::new(),
        )
        .unwrap(),
        StoryLimits::default(),
    )
}

fn view_rev(view: &Value) -> u32 {
    u32::try_from(view["viewRev"].as_u64().unwrap()).unwrap()
}

fn kinds<'a>(view: &'a Value, kind: &str) -> Vec<&'a str> {
    view["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "block" && item["kind"] == kind)
        .map(|item| item["text"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn browser_plays_a_parser_story_with_story_command_and_typed_text() {
    let manager = parser_stories();
    let out = outbound();
    let mut seq = 1;
    let mut session = WebStorySession::new(Some(manager), AccountId(1), false);
    assert_eq!(
        session
            .text_command(&out, &mut seq, "r1", "historia faro-de-punta-gris")
            .await,
        Some(true)
    );
    let entered = frames(&out);
    let view = last_view(&entered).clone();
    assert_eq!(view["items"][0]["prompt"]["text"], ">");
    assert_eq!(view["items"][0]["prompt"]["question"], false);

    // `story.command` echoes the line and answers with reply blocks.
    assert!(
        session
            .command(&out, &mut seq, "examina la barca", view_rev(&view))
            .await
    );
    let answered = frames(&out);
    assert_eq!(types(&answered), ["story.view"]);
    let view = last_view(&answered).clone();
    assert_eq!(kinds(&view, "command"), ["examina la barca"]);
    assert!(
        kinds(&view, "reply")
            .join("\n")
            .contains("una medalla de plata")
    );

    // A stale view is answered with the current one; nothing changes.
    assert!(session.command(&out, &mut seq, "n", 1).await);
    let stale = frames(&out);
    assert_eq!(types(&stale), ["story.error", "story.view"]);
    assert_eq!(stale[0]["data"]["code"], "stale_view");
    assert_eq!(view_rev(last_view(&stale)), view_rev(&view));

    // Typed text through `command.submit` reaches the parser too, while
    // reserved words keep their meaning.
    assert_eq!(
        session
            .text_command(&out, &mut seq, "r2", "coge la medalla")
            .await,
        Some(true)
    );
    let typed = frames(&out);
    assert_eq!(types(&typed), ["action.result", "story.view"]);
    assert_eq!(typed[0]["data"]["ok"], true);
    let view = last_view(&typed).clone();
    assert_eq!(kinds(&view, "reply"), ["Coges la medalla de plata."]);

    // «guardar» without a slot lists the slots (none here: no store).
    assert!(
        session
            .command(&out, &mut seq, "guardar", view_rev(&view))
            .await
    );
    let save = frames(&out);
    assert_eq!(types(&save), ["story.view", "story.error"]);
    assert_eq!(save[1]["data"]["code"], "slots_unavailable");
    let view = last_view(&save).clone();

    // The parser's quit leaves the story like «salir».
    assert!(
        session
            .command(&out, &mut seq, "fin", view_rev(&view))
            .await
    );
    let quit = frames(&out);
    assert_eq!(types(&quit), ["story.view", "story.left"]);
    assert!(!session.active());

    // A gamebook refuses free text with a typed error.
    assert_eq!(
        session
            .text_command(&out, &mut seq, "r3", "historia cripta-del-nigromante")
            .await,
        Some(true)
    );
    let view = last_view(&frames(&out)).clone();
    assert!(view["items"][0].get("prompt").is_none());
    assert!(
        session
            .command(&out, &mut seq, "mira", view_rev(&view))
            .await
    );
    let refused = frames(&out);
    assert_eq!(types(&refused), ["story.error"]);
    assert_eq!(refused[0]["data"]["code"], "no_parser");
}

#[test]
fn story_command_envelopes_are_shape_checked() {
    assert!(matches!(
        parse_client_message(r#"{"v":1,"type":"story.command","data":{"text":"coge la llave","viewRev":2}}"#),
        Ok(ClientMessage::StoryCommand { text, view_rev: 2 }) if text == "coge la llave"
    ));
    let long = format!(
        r#"{{"v":1,"type":"story.command","data":{{"text":"{}","viewRev":2}}}}"#,
        "a".repeat(257)
    );
    for invalid in [
        r#"{"v":1,"type":"story.command","data":{"text":"n","viewRev":0}}"#,
        r#"{"v":1,"type":"story.command","data":{"text":"n"}}"#,
        r#"{"v":1,"type":"story.command","data":{"text":"n","viewRev":1,"x":1}}"#,
        r#"{"v":1,"type":"story.command","data":{"text":7,"viewRev":1}}"#,
        long.as_str(),
    ] {
        assert_eq!(
            parse_client_message(invalid).err(),
            Some(ClientMessageError::InvalidEnvelope),
            "{invalid}"
        );
    }
}
