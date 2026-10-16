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
        ServerMessageType::StoryEntered,
        ServerMessageType::StoryView,
        ServerMessageType::StoryLeft,
        ServerMessageType::StoryError,
        ServerMessageType::StorySlots,
        ServerMessageType::StorySaved,
        ServerMessageType::StoryRestored,
        ServerMessageType::StoryLobby,
        ServerMessageType::StoryTally,
        ServerMessageType::StoryInvitation,
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

#[test]
fn story_payloads_have_their_documented_shape() {
    assert_wire(
        StoryEntered {
            story_id: "crypt-of-the-necromancer".into(),
            title: "La cripta del nigromante".into(),
            locale: "es".into(),
            locales: vec!["en".into(), "es".into()],
        },
        json!({"storyId": "crypt-of-the-necromancer", "title": "La cripta del nigromante",
               "locale": "es", "locales": ["en", "es"]}),
    );
    assert_wire(
        StoryView {
            view_rev: 2,
            part: 0,
            parts: 1,
            turn: 1,
            can_undo: true,
            items: vec![
                StoryItem::Scene {
                    title: "La cripta".into(),
                    text: "Un pasillo.".into(),
                    score: -3,
                    ending: None,
                    prompt: None,
                    background: Some(StoryImage {
                        manifest: format!("/assets/{}/manifest.json", "a".repeat(64)),
                        alt: "Una oficina bajo la lluvia.".into(),
                    }),
                },
                StoryItem::Cue {
                    action: StoryCueAction::Stop,
                    channel: MediaChannel::Music,
                    url: None,
                    looped: false,
                },
                StoryItem::Cue {
                    action: StoryCueAction::Play,
                    channel: MediaChannel::Effects,
                    url: Some("/assets/x/audio-a.ogg".into()),
                    looped: false,
                },
                StoryItem::Portrait {
                    name: "Lola".into(),
                    image: None,
                },
                StoryItem::Portrait {
                    name: "Dámaso".into(),
                    image: Some(StoryImage {
                        manifest: format!("/assets/{}/manifest.json", "b".repeat(64)),
                        alt: "Retrato de Dámaso.".into(),
                    }),
                },
                StoryItem::Block {
                    kind: StoryBlockKind::Event,
                    speaker: None,
                    portrait: None,
                    text: "Tirada 2d6".into(),
                },
                StoryItem::Block {
                    kind: StoryBlockKind::Line,
                    speaker: Some("Lola".into()),
                    portrait: None,
                    text: "Hola.".into(),
                },
                StoryItem::Stat {
                    label: "Vida".into(),
                    value: "12".into(),
                },
                StoryItem::Item {
                    name: "Antorcha".into(),
                },
                StoryItem::Choice {
                    id: "c0123456789abcdef".into(),
                    label: "Avanzar".into(),
                },
                StoryItem::Verb {
                    id: "c1111111111111111".into(),
                    label: "Mirar".into(),
                    needs_target: true,
                },
                StoryItem::Target {
                    verb: "c1111111111111111".into(),
                    id: "c2222222222222222".into(),
                    label: "Mesa".into(),
                },
                StoryItem::DialogueOption {
                    id: "c3333333333333333".into(),
                    label: "Adiós".into(),
                },
            ],
        },
        json!({
            "viewRev": 2, "part": 0, "parts": 1, "turn": 1, "canUndo": true,
            "items": [
                {"type": "scene", "title": "La cripta", "text": "Un pasillo.", "score": -3, "ending": null,
                 "background": {"manifest": format!("/assets/{}/manifest.json", "a".repeat(64)),
                                "alt": "Una oficina bajo la lluvia."}},
                {"type": "cue", "action": "stop", "channel": "music", "loop": false},
                {"type": "cue", "action": "play", "channel": "effects", "url": "/assets/x/audio-a.ogg", "loop": false},
                {"type": "portrait", "name": "Lola"},
                {"type": "portrait", "name": "Dámaso",
                 "image": {"manifest": format!("/assets/{}/manifest.json", "b".repeat(64)),
                           "alt": "Retrato de Dámaso."}},
                {"type": "block", "kind": "event", "text": "Tirada 2d6"},
                {"type": "block", "kind": "line", "speaker": "Lola", "text": "Hola."},
                {"type": "stat", "label": "Vida", "value": "12"},
                {"type": "item", "name": "Antorcha"},
                {"type": "choice", "id": "c0123456789abcdef", "label": "Avanzar"},
                {"type": "verb", "id": "c1111111111111111", "label": "Mirar", "needsTarget": true},
                {"type": "target", "verb": "c1111111111111111", "id": "c2222222222222222", "label": "Mesa"},
                {"type": "dialogueOption", "id": "c3333333333333333", "label": "Adiós"}
            ]
        }),
    );
    assert_wire(
        StoryEnding {
            kind: StoryEndingKind::Victory,
        },
        json!({"kind": "victory"}),
    );
    assert_wire(
        StoryLeft {
            reason: StoryLeftReason::Evicted,
        },
        json!({"reason": "evicted"}),
    );
    assert_wire(
        StoryError {
            code: StoryErrorCode::StaleView,
            message: "x".into(),
        },
        json!({"code": "stale_view", "message": "x"}),
    );

    let choose: StoryChooseData =
        serde_json::from_value(json!({"id": "c0123456789abcdef", "viewRev": 3})).unwrap();
    assert_eq!(choose.view_rev, 3);
    for rejected in [
        json!({"id": "c0", "viewRev": 3, "x": 1}),
        json!({"id": "c0"}),
        json!({"id": 1, "viewRev": 3}),
        json!({"id": "c0", "viewRev": -1}),
    ] {
        assert!(serde_json::from_value::<StoryChooseData>(rejected).is_err());
    }
    let menu: StoryMenuData = serde_json::from_value(json!({"verb": "c1", "viewRev": 3})).unwrap();
    assert_eq!(menu.target, None);
    assert!(
        serde_json::from_value::<StoryMenuData>(
            json!({"verb": "c1", "target": "c2", "viewRev": 3, "x": 1})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<StoryEmptyData>(json!({})).is_ok());
    assert!(serde_json::from_value::<StoryEmptyData>(json!({"x": 1})).is_err());
    for name in [
        "story.choose",
        "story.menu",
        "story.dialogue",
        "story.undo",
        "story.leave",
        "story.save",
        "story.restore",
        "story.slots",
        "story.deleteSlot",
        "story.continue",
        "story.share",
        "story.join",
        "story.invite",
        "story.kick",
        "story.mode",
        "story.vote",
    ] {
        assert!(serde_json::from_value::<ClientMessageType>(json!(name)).is_ok());
    }
}

#[test]
fn story_slot_payloads_have_their_documented_shape() {
    assert_wire(
        StorySlots {
            slots: vec![
                StorySlotSummary {
                    slot: 0,
                    updated_at: 1_700_000_000,
                    compatible: true,
                },
                StorySlotSummary {
                    slot: 3,
                    updated_at: 1_700_000_100,
                    compatible: false,
                },
            ],
            offer_continue: true,
        },
        json!({
            "slots": [
                {"slot": 0, "updatedAt": 1_700_000_000, "compatible": true},
                {"slot": 3, "updatedAt": 1_700_000_100, "compatible": false}
            ],
            "offerContinue": true
        }),
    );
    assert_wire(
        StorySaved {
            slot: 2,
            updated_at: 5,
        },
        json!({"slot": 2, "updatedAt": 5}),
    );
    assert_wire(StoryRestored { slot: 0 }, json!({"slot": 0}));
    for (code, wire) in [
        (StoryErrorCode::SlotsUnavailable, "slots_unavailable"),
        (StoryErrorCode::InvalidSlot, "invalid_slot"),
        (StoryErrorCode::SlotEmpty, "slot_empty"),
        (StoryErrorCode::SlotCorrupt, "slot_corrupt"),
        (StoryErrorCode::SlotIncompatible, "slot_incompatible"),
        (StoryErrorCode::SlotCapacity, "slot_capacity"),
        (StoryErrorCode::RateLimited, "rate_limited"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
    }
    let slot: StorySlotData = serde_json::from_value(json!({"slot": 4})).unwrap();
    assert_eq!(slot.slot, 4);
    for rejected in [
        json!({"slot": 4, "x": 1}),
        json!({}),
        json!({"slot": -1}),
        json!({"slot": 256}),
        json!({"slot": "1"}),
    ] {
        assert!(serde_json::from_value::<StorySlotData>(rejected).is_err());
    }
}

#[test]
fn shared_story_payloads_have_their_documented_shape() {
    assert_wire(
        StoryLobby {
            mode: StoryDecisionMode::Vote,
            vote_seconds: 60,
            you: StorySeatRole::Host,
            code: Some("ABCD2345".into()),
            seats: vec![
                StorySeat {
                    name: "Aria".into(),
                    role: StorySeatRole::Host,
                    connected: true,
                    turn: false,
                },
                StorySeat {
                    name: "Bruno".into(),
                    role: StorySeatRole::Player,
                    connected: false,
                    turn: false,
                },
            ],
            spectators: 1,
            max_seats: 8,
            max_spectators: 32,
        },
        json!({
            "mode": "vote",
            "voteSeconds": 60,
            "you": "host",
            "code": "ABCD2345",
            "seats": [
                {"name": "Aria", "role": "host", "connected": true, "turn": false},
                {"name": "Bruno", "role": "player", "connected": false, "turn": false}
            ],
            "spectators": 1,
            "maxSeats": 8,
            "maxSpectators": 32
        }),
    );
    assert_wire(
        StoryLobby {
            mode: StoryDecisionMode::Turns,
            vote_seconds: 30,
            you: StorySeatRole::Spectator,
            code: None,
            seats: Vec::new(),
            spectators: 0,
            max_seats: 2,
            max_spectators: 0,
        },
        json!({
            "mode": "turns", "voteSeconds": 30, "you": "spectator", "seats": [],
            "spectators": 0, "maxSeats": 2, "maxSpectators": 0
        }),
    );
    assert_wire(
        StoryTally {
            view_rev: 4,
            mode: StoryDecisionMode::Vote,
            choices: vec![StoryTallyEntry {
                choice_id: "c0123456789abcdef".into(),
                votes: 2,
            }],
            voted: 2,
            eligible: 3,
            remaining_seconds: Some(41),
            your_vote: Some("c0123456789abcdef".into()),
        },
        json!({
            "viewRev": 4, "mode": "vote",
            "choices": [{"choiceId": "c0123456789abcdef", "votes": 2}],
            "voted": 2, "eligible": 3, "remainingSeconds": 41,
            "yourVote": "c0123456789abcdef"
        }),
    );
    assert_wire(
        StoryTally {
            view_rev: 1,
            mode: StoryDecisionMode::Host,
            choices: Vec::new(),
            voted: 0,
            eligible: 1,
            remaining_seconds: None,
            your_vote: None,
        },
        json!({"viewRev": 1, "mode": "host", "choices": [], "voted": 0, "eligible": 1}),
    );
    assert_wire(
        StoryInvitation {
            host: "Aria".into(),
            story_id: "cripta".into(),
            title: "Cripta".into(),
            code: "ABCD2345".into(),
        },
        json!({"host": "Aria", "storyId": "cripta", "title": "Cripta", "code": "ABCD2345"}),
    );
    for (reason, wire) in [
        (StoryLeftReason::Kicked, "kicked"),
        (StoryLeftReason::Ended, "ended"),
    ] {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(wire));
    }
    for (code, wire) in [
        (StoryErrorCode::HostOnly, "host_only"),
        (StoryErrorCode::Spectator, "spectator"),
        (StoryErrorCode::NotYourTurn, "not_your_turn"),
        (StoryErrorCode::UnknownCode, "unknown_code"),
        (StoryErrorCode::SeatsFull, "seats_full"),
        (StoryErrorCode::NotShared, "not_shared"),
        (StoryErrorCode::UnknownMember, "unknown_member"),
        (StoryErrorCode::InvalidSetting, "invalid_setting"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
    }

    let vote: StoryVoteData =
        serde_json::from_value(json!({"choiceId": "c0123456789abcdef", "viewRev": 2})).unwrap();
    assert_eq!(vote.choice_id.as_deref(), Some("c0123456789abcdef"));
    let abstain: StoryVoteData = serde_json::from_value(json!({"viewRev": 2})).unwrap();
    assert_eq!(abstain.choice_id, None);
    let join: StoryJoinData =
        serde_json::from_value(json!({"code": "ABCD2345", "spectate": false})).unwrap();
    assert!(!join.spectate);
    let mode: StoryModeData =
        serde_json::from_value(json!({"mode": "turns", "voteSeconds": 45})).unwrap();
    assert_eq!(mode.mode, StoryDecisionMode::Turns);
    let name: StoryNameData = serde_json::from_value(json!({"name": "Bruno"})).unwrap();
    assert_eq!(name.name, "Bruno");
    for rejected in [
        json!({"viewRev": 2, "x": 1}),
        json!({"choiceId": "c0123456789abcdef"}),
    ] {
        assert!(serde_json::from_value::<StoryVoteData>(rejected).is_err());
    }
    assert!(serde_json::from_value::<StoryJoinData>(json!({"code": "ABCD2345"})).is_err());
    assert!(serde_json::from_value::<StoryModeData>(json!({"mode": "chaos"})).is_err());
    assert!(serde_json::from_value::<StoryNameData>(json!({"name": "B", "x": 1})).is_err());
}

#[test]
fn story_catalog_and_line_portraits_have_their_documented_shape() {
    assert_wire(
        StoryCatalog {
            part: 0,
            parts: 1,
            stories: vec![
                StoryCatalogEntry {
                    story_id: "crypt-of-the-necromancer".into(),
                    title: "Crypt of the Necromancer".into(),
                    locales: vec!["en".into(), "es".into()],
                },
                StoryCatalogEntry {
                    story_id: "neon-rain".into(),
                    title: "Neon Rain".into(),
                    locales: vec!["en".into(), "es".into()],
                },
            ],
        },
        json!({"part": 0, "parts": 1, "stories": [
            {"storyId": "crypt-of-the-necromancer", "title": "Crypt of the Necromancer",
             "locales": ["en", "es"]},
            {"storyId": "neon-rain", "title": "Neon Rain", "locales": ["en", "es"]}
        ]}),
    );
    assert_wire(
        StoryItem::Block {
            kind: StoryBlockKind::Line,
            speaker: Some("Lola".into()),
            portrait: Some(StoryImage {
                manifest: format!("/assets/{}/manifest.json", "c".repeat(64)),
                alt: "Portrait silhouette of Lola.".into(),
            }),
            text: "Hi.".into(),
        },
        json!({"type": "block", "kind": "line", "speaker": "Lola",
               "portrait": {"manifest": format!("/assets/{}/manifest.json", "c".repeat(64)),
                            "alt": "Portrait silhouette of Lola."},
               "text": "Hi."}),
    );
    assert_eq!(
        serde_json::to_value(ServerMessageType::StoryCatalog).unwrap(),
        json!("story.catalog")
    );
    assert_eq!(ServerMessageType::StoryCatalog.as_str(), "story.catalog");
    assert!(serde_json::from_value::<ClientMessageType>(json!("story.catalog")).is_ok());
    assert_eq!(MAX_STORY_LOCALES, 16);
}

#[test]
fn parser_story_payloads_have_their_documented_shape() {
    assert_wire(
        StoryView {
            view_rev: 5,
            part: 0,
            parts: 1,
            turn: 3,
            can_undo: true,
            items: vec![
                StoryItem::Scene {
                    title: "Linterna".into(),
                    text: "Linterna".into(),
                    score: 10,
                    ending: None,
                    prompt: Some(StoryPrompt {
                        text: ">".into(),
                        question: true,
                    }),
                    background: None,
                },
                StoryItem::Block {
                    kind: StoryBlockKind::Command,
                    speaker: None,
                    portrait: None,
                    text: "enciende la lampara".into(),
                },
                StoryItem::Block {
                    kind: StoryBlockKind::Reply,
                    speaker: None,
                    portrait: None,
                    text: "¿Cuál quieres decir?".into(),
                },
            ],
        },
        json!({
            "viewRev": 5, "part": 0, "parts": 1, "turn": 3, "canUndo": true,
            "items": [
                {"type": "scene", "title": "Linterna", "text": "Linterna", "score": 10, "ending": null,
                 "prompt": {"text": ">", "question": true}},
                {"type": "block", "kind": "command", "text": "enciende la lampara"},
                {"type": "block", "kind": "reply", "text": "¿Cuál quieres decir?"}
            ]
        }),
    );
    for (code, wire) in [
        (StoryErrorCode::NoParser, "no_parser"),
        (StoryErrorCode::InvalidCommand, "invalid_command"),
        (StoryErrorCode::UnknownLocale, "unknown_locale"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
    }
    let command: StoryCommandData =
        serde_json::from_value(json!({"text": "coge la llave", "viewRev": 4})).unwrap();
    assert_eq!(command.text, "coge la llave");
    assert_eq!(command.view_rev, 4);
    for rejected in [
        json!({"text": "n"}),
        json!({"viewRev": 4}),
        json!({"text": "n", "viewRev": 4, "x": 1}),
        json!({"text": 1, "viewRev": 4}),
        json!({"text": "n", "viewRev": -1}),
    ] {
        assert!(serde_json::from_value::<StoryCommandData>(rejected).is_err());
    }
    assert!(serde_json::from_value::<ClientMessageType>(json!("story.command")).is_ok());
    assert_eq!(MAX_STORY_COMMAND_BYTES, 256);
}
