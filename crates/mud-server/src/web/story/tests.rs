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
