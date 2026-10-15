//! Golden tests: every typed message must encode to exactly the bytes the
//! server produced with the former ad hoc `json!` construction.

use super::*;
use serde_json::{Value, json};

/// Asserts value equality and byte equality of the compact encoding.
fn assert_wire(typed: impl Serialize, expected: Value) {
    let typed = serde_json::to_value(typed).expect("typed message serializes");
    assert_eq!(typed, expected);
    assert_eq!(typed.to_string(), expected.to_string());
}

#[test]
fn envelopes_match_former_construction() {
    let data = json!({"level": "error", "message": "x"});
    assert_wire(
        ServerEnvelope::new(ServerMessageType::ServerNotice, 7, &data),
        json!({"v": 1, "type": "server.notice", "seq": 7, "data": data}),
    );
    assert_wire(
        UnsequencedEnvelope::new(
            ServerMessageType::ServerHeartbeat,
            ServerHeartbeat {
                nonce: 3,
                timeout_ms: 45_000,
            },
        ),
        json!({
            "v": 1,
            "type": "server.heartbeat",
            "data": {"nonce": 3_u64, "timeoutMs": std::time::Duration::from_secs(45).as_millis()}
        }),
    );
}

#[test]
fn message_type_names_match_serde_names() {
    for message_type in [
        ServerMessageType::ServerWelcome,
        ServerMessageType::ServerNotice,
        ServerMessageType::ServerHeartbeat,
        ServerMessageType::ActionResult,
        ServerMessageType::SceneSnapshot,
        ServerMessageType::PlayerSnapshot,
        ServerMessageType::ScenePatch,
        ServerMessageType::EntityUpsert,
        ServerMessageType::EntityRemove,
        ServerMessageType::VitalsUpdate,
        ServerMessageType::InventoryPatch,
        ServerMessageType::ContainerContents,
        ServerMessageType::TradeContents,
        ServerMessageType::RewardOdds,
        ServerMessageType::MailList,
        ServerMessageType::MailMessage,
        ServerMessageType::BoardCatalog,
        ServerMessageType::BoardList,
        ServerMessageType::BoardEntry,
        ServerMessageType::MapSnapshot,
        ServerMessageType::MessageAppend,
        ServerMessageType::ChatMessage,
        ServerMessageType::SyncScene,
        ServerMessageType::SyncPlayer,
        ServerMessageType::MediaPlay,
        ServerMessageType::MediaStop,
    ] {
        assert_eq!(
            serde_json::to_value(message_type).unwrap(),
            Value::String(message_type.as_str().to_owned())
        );
    }
}

#[test]
fn control_payloads_match_former_construction() {
    assert_wire(
        ServerWelcome {
            server_name: "Tiamud".into(),
            motd: "Welcome to Tiamud, a text-complete world of adventures.".into(),
            session_id: "42".into(),
        },
        json!({
            "serverName": "Tiamud",
            "motd": "Welcome to Tiamud, a text-complete world of adventures.",
            "sessionId": "42"
        }),
    );
    assert_wire(
        ServerNotice::with_code(NoticeLevel::Error, "access.session_limit", "limit"),
        json!({"level": "error", "code": "access.session_limit", "message": "limit"}),
    );
    assert_wire(
        ServerNotice::new(NoticeLevel::Info, "draining"),
        json!({"level": "info", "message": "draining"}),
    );
    assert_wire(
        ServerNotice::new(NoticeLevel::Warning, "w"),
        json!({"level": "warning", "message": "w"}),
    );
    let message: Option<String> = None;
    assert_wire(
        ActionResult {
            request_id: "cmd-1".into(),
            ok: true,
            message: message.clone(),
        },
        json!({"requestId": "cmd-1", "ok": true, "message": message}),
    );
    assert_wire(
        ActionResult {
            request_id: "cmd-2".into(),
            ok: false,
            message: Some("No.".into()),
        },
        json!({"requestId": "cmd-2", "ok": false, "message": Some("No.")}),
    );
}

#[test]
fn state_payloads_match_former_construction() {
    let scene = json!({"viewId": "v1", "revision": 4, "title": "Square", "prose": []});
    let presentation = json!({"entities": [{"entityId": "o1"}]});
    // scene.snapshot used `data = json!(scene); data["presentation"] = ...`.
    let mut former = scene.clone();
    former["presentation"] = presentation.clone();
    assert_wire(
        SceneFrame {
            scene: &scene,
            presentation: Some(&presentation),
        },
        former,
    );
    assert_wire(
        SceneFrame::<_, Value> {
            scene: &scene,
            presentation: None,
        },
        scene.clone(),
    );
    assert_wire(
        SyncScene {
            sync_id: "sync-9".into(),
            after_seq: 4,
            scene: &scene,
        },
        json!({"syncId": "sync-9", "afterSeq": 4_u64, "scene": scene}),
    );
    let player = json!({"revision": 2});
    assert_wire(
        SyncPlayer::new("sync-9", 4, &player),
        json!({"syncId": "sync-9", "afterSeq": 4_u64, "player": player, "complete": true}),
    );

    let mut patch = ScenePatch::<Value, Value, Value>::new("v1", 5);
    assert!(!patch.has_changes());
    patch.title = Some("Gate".into());
    patch.image = Some(None);
    patch.exits = Some(vec![json!({"direction": "north"})]);
    assert!(patch.has_changes());
    assert_wire(
        patch,
        json!({
            "viewId": "v1",
            "revision": 5,
            "title": "Gate",
            "image": null,
            "exits": [{"direction": "north"}],
        }),
    );

    assert_wire(
        EntityRemove {
            view_id: "v1".into(),
            revision: 5,
            kind: EntityKind::Npc,
            id: "o2".into(),
        },
        json!({"viewId": "v1", "revision": 5, "kind": "npc", "id": "o2"}),
    );
    let entity = json!({"id": "o3", "label": "Lamp"});
    assert_wire(
        EntityUpsert {
            view_id: "v1".into(),
            revision: 5,
            kind: EntityKind::Item,
            id: "o3".into(),
            entity: &entity,
        },
        json!({"viewId": "v1", "revision": 5, "kind": "item", "id": "o3", "entity": entity}),
    );
    let character = json!({"name": "Aster"});
    assert_wire(
        VitalsUpdate {
            revision: 2,
            character: &character,
        },
        json!({"revision": 2, "character": character}),
    );
    let capacity = json!({"usedSlots": 1});
    assert_wire(
        InventoryPatch {
            revision: 2,
            inventory: vec![&entity],
            equipment: Vec::<Value>::new(),
            inventory_capacity: &capacity,
        },
        json!({
            "revision": 2,
            "inventory": [entity],
            "equipment": [],
            "inventoryCapacity": capacity,
        }),
    );
    let map = json!({"currentRoomId": "r1", "rooms": [], "paths": []});
    assert_wire(
        MapSnapshot {
            revision: 2,
            map: &map,
        },
        json!({"revision": 2, "map": map}),
    );
    assert_wire(
        ContainerContents {
            container: "o4".into(),
            items: vec![&entity],
            page: 1,
            total_pages: 2,
            capacity: &capacity,
        },
        json!({
            "container": "o4",
            "items": [entity],
            "page": 1_u16,
            "totalPages": 2_u16,
            "capacity": capacity,
        }),
    );
    assert_wire(
        TradeContents {
            trade_id: "o5".into(),
            container: "o4".into(),
            items: vec![&entity],
            page: 1,
            total_pages: 1,
            capacity: &capacity,
        },
        json!({
            "tradeId": "o5",
            "container": "o4",
            "items": [entity],
            "page": 1_u16,
            "totalPages": 1_u16,
            "capacity": capacity,
        }),
    );
}

#[test]
fn narrative_and_media_payloads_match_former_construction() {
    let event = json!({"revision": 3, "kind": "gameplay", "text": "Hit.", "actorId": "o1"});
    for (cue, former_cue) in [
        (
            CombatCue::attack("o1"),
            json!({"kind": "attack", "entityId": "o1", "pose": "attack", "intensity": 2, "durationMs": 420}),
        ),
        (
            CombatCue::hit("o1"),
            json!({"kind": "hit", "entityId": "o1", "pose": "hit", "intensity": 2, "durationMs": 320}),
        ),
        (
            CombatCue::defeat("o1"),
            json!({"kind": "defeat", "entityId": "o1", "pose": "defeated", "intensity": 3, "durationMs": 700}),
        ),
    ] {
        let mut former = event.clone();
        former["cue"] = former_cue;
        assert_wire(
            MessageAppend {
                event: &event,
                cue: Some(cue),
            },
            former,
        );
    }
    assert_wire(
        MessageAppend {
            event: &event,
            cue: None,
        },
        event.clone(),
    );
    assert_wire(
        ChatMessage {
            channel: ChatChannel::Telepathy,
            sender: "Aster".into(),
            text: "hi".into(),
        },
        json!({"channel": "telepathy", "sender": "Aster", "text": "hi"}),
    );
    assert_wire(
        MediaPlay {
            channel: MediaChannel::Ambience,
            url: "/assets/x/audio-a.ogg".into(),
            looped: true,
        },
        json!({"channel": "ambience", "url": "/assets/x/audio-a.ogg", "loop": true}),
    );
    assert_wire(
        MediaStop {
            channel: MediaChannel::Music,
        },
        json!({"channel": "music"}),
    );
    for name in ["ambience", "effects", "music"] {
        let channel = MediaChannel::from_name(name).expect("closed channel");
        assert_eq!(serde_json::to_value(channel).unwrap(), json!(name));
    }
    assert_eq!(MediaChannel::from_name("voice"), None);
}

#[test]
fn mail_and_board_payloads_match_former_construction() {
    let summary = MailMessageSummary {
        id: "m0123".into(),
        sender: "Orin".into(),
        subject: "Hello".into(),
        created_at: 10,
        expires_at: 20,
        read: false,
    };
    let former_summary = json!({
        "id": "m0123",
        "sender": "Orin",
        "subject": "Hello",
        "createdAt": 10_i64,
        "expiresAt": 20_i64,
        "read": false,
    });
    assert_wire(&summary, former_summary.clone());
    assert_wire(
        MailList {
            page: 1,
            total_pages: 1,
            total_messages: 1,
            unread_count: 1,
            capacity: 64,
            messages: vec![summary],
        },
        json!({
            "page": 1_u32,
            "totalPages": 1_u32,
            "totalMessages": 1_u32,
            "unreadCount": 1_u32,
            "capacity": 64_u32,
            "messages": [former_summary],
        }),
    );
    assert_wire(
        MailMessage {
            id: "m0123".into(),
            sender: "Orin".into(),
            subject: "Hello".into(),
            body: "Body | pipe".into(),
            created_at: 10,
            expires_at: 20,
            read: true,
        },
        json!({
            "id": "m0123",
            "sender": "Orin",
            "subject": "Hello",
            "body": "Body | pipe",
            "createdAt": 10_i64,
            "expiresAt": 20_i64,
            "read": true,
        }),
    );
    assert_wire(
        BoardCatalog {
            boards: vec![BoardDescriptor {
                key: BoardKey::Newbie,
                label: "Personas nuevas".into(),
                description: "Preguntas y orientación para empezar.".into(),
                can_post: true,
            }],
            retention_days: 90,
        },
        json!({
            "boards": [{
                "key": "newbie",
                "label": "Personas nuevas",
                "description": "Preguntas y orientación para empezar.",
                "canPost": true,
            }],
            "retentionDays": 90,
        }),
    );
    let post = BoardPostSummary {
        post_id: "b0123".into(),
        board: BoardKey::General,
        author: "Aster".into(),
        subject: "Topic".into(),
        created_at: 1,
        expires_at: 2,
        can_delete: true,
        can_report: false,
    };
    let former_post = json!({
        "postId": "b0123",
        "board": "general",
        "author": "Aster",
        "subject": "Topic",
        "createdAt": 1_i64,
        "expiresAt": 2_i64,
        "canDelete": true,
        "canReport": false,
    });
    assert_wire(&post, former_post.clone());
    assert_wire(
        BoardList {
            board: BoardKey::General,
            page: 1,
            total_pages: 1,
            total_posts: 1,
            capacity: 1_000,
            posts: vec![post],
        },
        json!({
            "board": "general",
            "page": 1_u32,
            "totalPages": 1_u32,
            "totalPosts": 1_u32,
            "capacity": 1_000_u32,
            "posts": [former_post],
        }),
    );
    // board.entry inserted `body` into the summary object.
    let mut former_entry = former_post;
    former_entry
        .as_object_mut()
        .unwrap()
        .insert("body".into(), json!("Text"));
    assert_wire(
        BoardEntry {
            post_id: "b0123".into(),
            board: BoardKey::General,
            author: "Aster".into(),
            subject: "Topic".into(),
            body: "Text".into(),
            created_at: 1,
            expires_at: 2,
            can_delete: true,
            can_report: false,
        },
        former_entry,
    );
}

#[test]
fn client_payloads_keep_their_strict_shapes() {
    let hello: ClientHello = serde_json::from_value(json!({
        "v": 1,
        "type": "client.hello",
        "data": {
            "client": "web",
            "capabilities": {"images": ["avif", "webp", "jpeg"], "reducedMotion": true, "future": 1},
            "auth": {"token": "ab", "characterId": 7}
        }
    }))
    .expect("additive capability keys are tolerated");
    assert_eq!(hello.data.capabilities.images, SUPPORTED_IMAGE_FORMATS);
    assert!(!hello.data.capabilities.audio);
    assert_eq!(hello.data.auth.character_id, 7);

    for rejected in [
        json!({"v": 1, "type": "client.hello", "data": {"client": "telnet", "capabilities": {"images": [], "reducedMotion": false}, "auth": {"token": "ab", "characterId": 7}}}),
        json!({"v": 1, "type": "client.hello", "data": {"client": "web", "capabilities": {"images": ["png"], "reducedMotion": false}, "auth": {"token": "ab", "characterId": 7}}}),
        json!({"v": 1, "type": "client.hello", "data": {"client": "web", "capabilities": {"images": [], "reducedMotion": false}, "auth": {"token": "ab", "characterId": 7, "extra": 1}}}),
        json!({"v": 1, "type": "client.hello", "data": {"client": "web", "capabilities": {"images": [], "reducedMotion": false}, "auth": {"token": "ab", "characterId": 7}, "extra": 1}}),
        json!({"v": 1, "type": "client.hello", "data": {"client": "web", "capabilities": {"images": []}, "auth": {"token": "ab", "characterId": 7}}}),
    ] {
        assert!(serde_json::from_value::<ClientHello>(rejected).is_err());
    }
    assert!(
        serde_json::from_value::<CommandSubmitData>(
            json!({"requestId": "a", "command": "look", "x": 1})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<SyncRequestData>(json!({"afterSeq": 1, "x": 1})).is_err());
    assert!(serde_json::from_value::<ClientHeartbeatData>(json!({"nonce": 1, "x": 1})).is_err());
    assert!(
        serde_json::from_value::<ClientEnvelope>(json!({"v": 1, "type": "x", "data": {}, "x": 1}))
            .is_err()
    );
}
