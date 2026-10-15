//! Golden tests for the typed `tiamud.v1` construction in `web.rs`.
//!
//! Each `former_*` helper is a verbatim copy of the ad hoc `json!`
//! construction that the typed `tiamud-protocol` messages replaced. Every test
//! drives the production queueing function with authoritative core projections
//! and requires the queued frame to be value- and byte-identical.

use super::*;
use crate::persistence::{BoardPostSummary, MailMessageSummary};
use serde_json::json;

fn metrics() -> Arc<RuntimeMetrics> {
    Arc::new(RuntimeMetrics::default())
}

fn drain(outbound: &OutboundHandle) -> Vec<Value> {
    std::iter::from_fn(|| outbound.pop().map(|frame| frame.value)).collect()
}

fn assert_same_wire(actual: &[Value], former: &[Value]) {
    assert_eq!(actual, former);
    for (actual, former) in actual.iter().zip(former) {
        assert_eq!(actual.to_string(), former.to_string());
    }
}

fn former_frame(message_type: &str, seq: u64, data: Value) -> Value {
    json!({"v": 1, "type": message_type, "seq": seq, "data": data})
}

fn demo_world(session: SessionId) -> tiamud_core::World {
    let mut world = tiamud_core::World::demo();
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(i64::try_from(session.0).unwrap() + 1),
                account_id: tiamud_core::AccountId(i64::try_from(session.0).unwrap() + 2),
                name: "Golden".into(),
                room_id: world.start_room_id().to_owned(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    world
}

fn former_scene_with_presentation(
    scene: &SceneView,
    presentation: &crate::runtime_content::ScenePresentation,
) -> Value {
    let mut value = json!(scene);
    if !presentation.is_empty() {
        value["presentation"] = json!(presentation);
    }
    value
}

#[test]
fn control_frames_match_former_construction() {
    let welcome = sequenced_frame(
        ServerMessageType::ServerWelcome,
        1,
        protocol::ServerWelcome {
            server_name: "Tiamud".to_owned(),
            motd: "Welcome to Tiamud, a text-complete world of adventures.".to_owned(),
            session_id: SessionId(42).0.to_string(),
        },
    )
    .unwrap();
    assert_same_wire(
        &[welcome],
        &[former_frame(
            "server.welcome",
            1,
            json!({
                "serverName": "Tiamud",
                "motd": "Welcome to Tiamud, a text-complete world of adventures.",
                "sessionId": SessionId(42).0.to_string()
            }),
        )],
    );

    let code = "access.character_connected";
    let message = "That character is already connected.";
    assert_same_wire(
        &[
            pre_welcome_rejection(
                "access.protocol_rejected",
                "Expected an authenticated client.hello",
            )
            .unwrap(),
            pre_welcome_rejection(code, message).unwrap(),
        ],
        &[
            json!({"v": 1, "type": "server.notice", "seq": 1, "data": {"level": "error", "code": "access.protocol_rejected", "message": "Expected an authenticated client.hello"}}),
            json!({"v": 1, "type": "server.notice", "seq": 1, "data": {"level": "error", "code": code, "message": message}}),
        ],
    );

    let heartbeat_nonce = 9_u64;
    assert_same_wire(
        &[heartbeat_frame(heartbeat_nonce).unwrap()],
        &[json!({
            "v": 1,
            "type": "server.heartbeat",
            "data": {
                "nonce": heartbeat_nonce,
                "timeoutMs": HEARTBEAT_TIMEOUT.as_millis()
            }
        })],
    );

    let outbound = OutboundHandle::new(8, metrics());
    let mut sequence = 3;
    assert!(queue_sequenced(
        &outbound,
        &mut sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::ServerNotice,
        protocol::ServerNotice::new(
            protocol::NoticeLevel::Info,
            "Server shutdown is starting; new traffic is stopping and accepted actions will be drained.",
        ),
    ));
    assert!(queue_notice(
        &outbound,
        &mut sequence,
        "Command rate limit exceeded."
    ));
    assert!(queue_action_result(
        &outbound,
        &mut sequence,
        "cmd-1",
        true,
        None
    ));
    assert!(queue_action_result(
        &outbound,
        &mut sequence,
        "cmd-2",
        false,
        Some("No.".to_owned())
    ));
    let none: Option<String> = None;
    assert_same_wire(
        &drain(&outbound),
        &[
            former_frame(
                "server.notice",
                3,
                json!({
                    "level": "info",
                    "message": "Server shutdown is starting; new traffic is stopping and accepted actions will be drained."
                }),
            ),
            former_frame(
                "server.notice",
                4,
                json!({"level": "error", "message": "Command rate limit exceeded."}),
            ),
            former_frame(
                "action.result",
                5,
                json!({"requestId": "cmd-1", "ok": true, "message": none}),
            ),
            former_frame(
                "action.result",
                6,
                json!({"requestId": "cmd-2", "ok": false, "message": Some("No.".to_owned())}),
            ),
        ],
    );
}

#[test]
fn chat_messages_match_former_construction_for_every_channel() {
    use crate::social::{SocialChannel, SocialMessage};
    for channel in [
        SocialChannel::Announcement,
        SocialChannel::Say,
        SocialChannel::Yell,
        SocialChannel::Shout,
        SocialChannel::Tell,
        SocialChannel::Telepathy,
        SocialChannel::Emote,
        SocialChannel::Party,
        SocialChannel::General,
        SocialChannel::Newbie,
        SocialChannel::Staff,
    ] {
        let delivery = SocialMessage {
            channel,
            sender: "Aster".into(),
            text: "Hello | there".into(),
        };
        let former = json!(delivery);
        let typed = protocol_value(chat_message(delivery)).unwrap();
        assert_same_wire(&[typed], &[former]);
    }
}

#[test]
fn scene_player_and_sync_frames_match_former_construction() {
    let session = SessionId(9_100);
    let world = demo_world(session);
    let scene = world.current_scene(session).unwrap();
    let player = world.player_state_view(session).unwrap();
    for presentation in [
        crate::runtime_content::ScenePresentation::default(),
        super::tests::test_entity_presentation(),
    ] {
        let outbound = OutboundHandle::new(8, metrics());
        let mut sequence = 20;
        assert!(queue_scene(
            &outbound,
            &mut sequence,
            ServerMessageType::SceneSnapshot,
            &scene,
            &presentation,
        ));
        // Critical frames evict queued state frames, so drain per call.
        let mut actual = drain(&outbound);
        assert!(queue_player(&outbound, &mut sequence, &player));
        actual.extend(drain(&outbound));
        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            19,
            &scene,
            &presentation,
            &player,
        ));
        actual.extend(drain(&outbound));
        assert_same_wire(
            &actual,
            &[
                former_frame(
                    "scene.snapshot",
                    20,
                    former_scene_with_presentation(&scene, &presentation),
                ),
                former_frame("player.snapshot", 21, json!(player)),
                former_frame(
                    "sync.scene",
                    22,
                    json!({
                        "syncId": "sync-22",
                        "afterSeq": 19_u64,
                        "scene": former_scene_with_presentation(&scene, &presentation),
                    }),
                ),
                former_frame(
                    "sync.player",
                    23,
                    json!({
                        "syncId": "sync-22",
                        "afterSeq": 19_u64,
                        "player": json!(player),
                        "complete": true,
                    }),
                ),
            ],
        );
    }
}

/// Verbatim former `queue_semantic_updates` body, collecting frames.
fn former_semantic_frames(
    sequence: &mut u64,
    previous_scene: &SceneView,
    scene: &SceneView,
    previous_player: &PlayerStateView,
    player: &PlayerStateView,
) -> Vec<Value> {
    let mut frames = Vec::new();
    let mut push = |message_type: &str, data: Value| {
        frames.push(former_frame(message_type, *sequence, data));
        *sequence = sequence.wrapping_add(1).max(1);
    };
    let previous_scene_json = json!(previous_scene);
    let scene_json = json!(scene);
    let previous_scene_object = previous_scene_json.as_object().unwrap();
    let scene_object = scene_json.as_object().unwrap();
    let former_entities = |scene: &Map<String, Value>| {
        let mut entities = BTreeMap::new();
        for (kind, field) in [("item", "items"), ("npc", "npcs")] {
            let Some(values) = scene.get(field).and_then(Value::as_array) else {
                continue;
            };
            for value in values {
                if let Some(id) = value.get("id").and_then(Value::as_str) {
                    entities.insert((kind.to_owned(), id.to_owned()), value.clone());
                }
            }
        }
        entities
    };
    let previous_entities = former_entities(previous_scene_object);
    let entities = former_entities(scene_object);
    let entity_delta_count = previous_entities
        .keys()
        .filter(|key| !entities.contains_key(*key))
        .count()
        + entities
            .iter()
            .filter(|(key, entity)| {
                previous_entities
                    .get(*key)
                    .is_none_or(|previous| previous != *entity)
            })
            .count();
    let emit_scene_deltas = entity_delta_count <= MAX_WS_SEMANTIC_DELTA_FRAMES;
    let mut patch = Map::from_iter([
        ("viewId".to_owned(), json!(scene.view_id)),
        ("revision".to_owned(), json!(scene.revision)),
    ]);
    for field in ["title", "prose", "image", "exits", "weather"] {
        if previous_scene_object.get(field) != scene_object.get(field) {
            patch.insert(
                field.to_owned(),
                scene_object.get(field).cloned().unwrap_or(Value::Null),
            );
        }
    }
    if emit_scene_deltas && patch.len() > 2 {
        push("scene.patch", Value::Object(patch));
    }
    for ((kind, id), _) in previous_entities
        .iter()
        .filter(|(key, _)| emit_scene_deltas && !entities.contains_key(*key))
    {
        push(
            "entity.remove",
            json!({
                "viewId": scene.view_id,
                "revision": scene.revision,
                "kind": kind,
                "id": id,
            }),
        );
    }
    for ((kind, id), entity) in entities.iter().filter(|(key, entity)| {
        emit_scene_deltas
            && previous_entities
                .get(*key)
                .is_none_or(|previous| previous != *entity)
    }) {
        push(
            "entity.upsert",
            json!({
                "viewId": scene.view_id,
                "revision": scene.revision,
                "kind": kind,
                "id": id,
                "entity": entity,
            }),
        );
    }
    if previous_player.character != player.character {
        push(
            "vitals.update",
            json!({"revision": player.revision, "character": player.character}),
        );
    }
    if previous_player.inventory != player.inventory
        || previous_player.equipment != player.equipment
        || previous_player.inventory_capacity != player.inventory_capacity
    {
        push(
            "inventory.patch",
            json!({
                "revision": player.revision,
                "inventory": player.inventory,
                "equipment": player.equipment,
                "inventoryCapacity": player.inventory_capacity,
            }),
        );
    }
    if previous_player.map != player.map {
        push(
            "map.snapshot",
            json!({"revision": player.revision, "map": player.map}),
        );
    }
    frames
}

#[test]
fn semantic_delta_frames_match_former_construction() {
    let session = SessionId(9_200);
    let mut world = demo_world(session);
    let mut previous_scene = world.current_scene(session).unwrap();
    let mut previous_player = world.player_state_view(session).unwrap();
    let mut observed_types = HashSet::new();
    for command in ["take lantern", "north", "south", "drop lantern"] {
        world
            .apply(session, parse_action(command).unwrap())
            .unwrap_or_else(|error| panic!("{command}: {error}"));
        let scene = world.current_scene(session).unwrap();
        let player = world.player_state_view(session).unwrap();
        let outbound = OutboundHandle::new(32, metrics());
        let mut sequence = 100;
        let mut former_sequence = sequence;
        assert!(queue_semantic_updates(
            &outbound,
            &mut sequence,
            &previous_scene,
            &scene,
            &previous_player,
            &player,
        ));
        let actual = drain(&outbound);
        let mut former = former_semantic_frames(
            &mut former_sequence,
            &previous_scene,
            &scene,
            &previous_player,
            &player,
        );
        // The former tail called queue_player for other player-state changes;
        // queue_player is covered by its own golden test.
        if actual.len() == former.len() + 1 {
            former.push(actual.last().unwrap().clone());
            assert_eq!(actual.last().unwrap()["type"], "player.snapshot");
        }
        assert_same_wire(&actual, &former);
        observed_types.extend(
            actual
                .iter()
                .map(|frame| frame["type"].as_str().unwrap().to_owned()),
        );
        previous_scene = scene;
        previous_player = player;
    }
    for expected in [
        "scene.patch",
        "entity.remove",
        "entity.upsert",
        "inventory.patch",
        "map.snapshot",
    ] {
        assert!(
            observed_types.contains(expected),
            "golden sequence did not exercise {expected}: {observed_types:?}"
        );
    }
}

#[test]
fn paged_item_frames_match_former_construction() {
    let session = SessionId(9_300);
    let mut world = demo_world(session);
    world
        .apply(session, parse_action("take lantern").unwrap())
        .unwrap();
    let player = world.player_state_view(session).unwrap();
    assert!(!player.inventory.is_empty());
    let container = ContainerContentsView {
        selector: "o00000000000000aa".into(),
        items: player.inventory.clone(),
        page: 1,
        total_pages: 2,
        capacity: player.inventory_capacity.clone(),
    };
    let trade = TradeContentsView {
        trade_id: "o00000000000000bb".into(),
        container: "o00000000000000aa".into(),
        items: player.inventory.clone(),
        page: 2,
        total_pages: 2,
        capacity: player.inventory_capacity.clone(),
    };
    let former_items = |items: &[tiamud_core::PlayerItemView]| {
        items
            .iter()
            .map(|item| {
                let mut summary = json!(item);
                if let Some(object) = summary.as_object_mut() {
                    object.remove("description");
                }
                summary
            })
            .collect::<Vec<_>>()
    };
    let outbound = OutboundHandle::new(8, metrics());
    let mut sequence = 1;
    assert!(queue_container_contents(
        &outbound,
        &mut sequence,
        &container
    ));
    assert!(queue_trade_contents(&outbound, &mut sequence, &trade));
    assert_same_wire(
        &drain(&outbound),
        &[
            former_frame(
                "container.contents",
                1,
                json!({
                    "container": container.selector,
                    "items": former_items(&container.items),
                    "page": container.page,
                    "totalPages": container.total_pages,
                    "capacity": container.capacity,
                }),
            ),
            former_frame(
                "trade.contents",
                2,
                json!({
                    "tradeId": trade.trade_id,
                    "container": trade.container,
                    "items": former_items(&trade.items),
                    "page": trade.page,
                    "totalPages": trade.total_pages,
                    "capacity": trade.capacity,
                }),
            ),
        ],
    );
}

#[test]
fn mail_and_board_frames_match_former_construction() {
    let mail_summary = MailMessageSummary {
        public_id: format!("m{}", "a".repeat(32)),
        sender_name: "Orin".into(),
        subject: "Lanterns".into(),
        created_at: 1_786_000_000,
        expires_at: 1_788_592_000,
        read: false,
    };
    let page = MailPage {
        page: 1,
        total_pages: 3,
        total_messages: 21,
        unread_count: 4,
        capacity: 64,
        messages: vec![mail_summary.clone()],
    };
    let message = MailMessage {
        summary: MailMessageSummary {
            read: true,
            ..mail_summary.clone()
        },
        body: "Meet at the gate | bring oil".into(),
    };
    let post = BoardPostSummary {
        public_id: format!("b{}", "c".repeat(32)),
        board: BoardKey::Newbie,
        author_name: "Aster".into(),
        subject: "First steps".into(),
        created_at: 1_786_000_100,
        expires_at: 1_793_776_100,
        can_delete: true,
        can_report: false,
    };
    let board_page = BoardPage {
        board: BoardKey::Newbie,
        page: 2,
        total_pages: 2,
        total_posts: 11,
        capacity: 1_000,
        posts: vec![post.clone()],
    };
    let entry = BoardEntry {
        summary: post.clone(),
        body: "Read the plaque.".into(),
    };
    let catalog = [
        BoardDescriptor {
            board: BoardKey::General,
            can_post: true,
        },
        BoardDescriptor {
            board: BoardKey::Newbie,
            can_post: false,
        },
    ];

    let former_mail_summary = |summary: &MailMessageSummary| {
        json!({
            "id": summary.public_id,
            "sender": summary.sender_name,
            "subject": summary.subject,
            "createdAt": summary.created_at,
            "expiresAt": summary.expires_at,
            "read": summary.read,
        })
    };
    let former_board_summary = |summary: &BoardPostSummary| {
        json!({
            "postId": summary.public_id,
            "board": summary.board,
            "author": summary.author_name,
            "subject": summary.subject,
            "createdAt": summary.created_at,
            "expiresAt": summary.expires_at,
            "canDelete": summary.can_delete,
            "canReport": summary.can_report,
        })
    };
    let mut former_entry = former_board_summary(&entry.summary);
    former_entry
        .as_object_mut()
        .unwrap()
        .insert("body".into(), json!(entry.body));
    let former_boards = catalog
        .iter()
        .map(|descriptor| {
            let (label, description) = match descriptor.board {
                BoardKey::General => ("General", "Conversación comunitaria deliberada."),
                BoardKey::Newbie => ("Personas nuevas", "Preguntas y orientación para empezar."),
            };
            json!({
                "key": descriptor.board,
                "label": label,
                "description": description,
                "canPost": descriptor.can_post,
            })
        })
        .collect::<Vec<_>>();

    let outbound = OutboundHandle::new(8, metrics());
    let mut sequence = 40;
    assert!(queue_mail_page(&outbound, &mut sequence, &page));
    assert!(queue_mail_message(&outbound, &mut sequence, &message));
    assert!(queue_board_catalog(&outbound, &mut sequence, &catalog));
    assert!(queue_board_page(&outbound, &mut sequence, &board_page));
    assert!(queue_board_entry(&outbound, &mut sequence, &entry));
    let summary = &message.summary;
    assert_same_wire(
        &drain(&outbound),
        &[
            former_frame(
                "mail.list",
                40,
                json!({
                    "page": page.page,
                    "totalPages": page.total_pages,
                    "totalMessages": page.total_messages,
                    "unreadCount": page.unread_count,
                    "capacity": page.capacity,
                    "messages": page.messages.iter().map(former_mail_summary).collect::<Vec<_>>(),
                }),
            ),
            former_frame(
                "mail.message",
                41,
                json!({
                    "id": summary.public_id,
                    "sender": summary.sender_name,
                    "subject": summary.subject,
                    "body": message.body,
                    "createdAt": summary.created_at,
                    "expiresAt": summary.expires_at,
                    "read": summary.read,
                }),
            ),
            former_frame(
                "board.catalog",
                42,
                json!({"boards": former_boards, "retentionDays": 90}),
            ),
            former_frame(
                "board.list",
                43,
                json!({
                    "board": board_page.board,
                    "page": board_page.page,
                    "totalPages": board_page.total_pages,
                    "totalPosts": board_page.total_posts,
                    "capacity": board_page.capacity,
                    "posts": board_page.posts.iter().map(former_board_summary).collect::<Vec<_>>(),
                }),
            ),
            former_frame("board.entry", 44, former_entry),
        ],
    );
}

#[test]
fn narrative_and_media_frames_match_former_construction() {
    let presentation = super::tests::test_entity_presentation();
    let presented_id = serde_json::to_value(&presentation).unwrap()["entities"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let empty = crate::runtime_content::ScenePresentation::default();
    let event = |target_id: Option<String>| tiamud_core::ViewEvent {
        revision: 8,
        kind: tiamud_core::ViewEventKind::Gameplay,
        text: "Authoritative combat narration.".into(),
        actor_id: presented_id.clone(),
        target_id,
        item_id: None,
    };
    let outbound = OutboundHandle::new(16, metrics());
    let mut sequence = 1;
    let mut former = Vec::new();
    let mut actual = Vec::new();
    for (kind, former_cue) in [
        (
            CombatCueKind::Attack,
            json!({"kind": "attack", "entityId": presented_id, "pose": "attack", "intensity": 2, "durationMs": 420}),
        ),
        (
            CombatCueKind::Hit,
            json!({"kind": "hit", "entityId": presented_id, "pose": "hit", "intensity": 2, "durationMs": 320}),
        ),
        (
            CombatCueKind::Defeat,
            json!({"kind": "defeat", "entityId": presented_id, "pose": "defeated", "intensity": 3, "durationMs": 700}),
        ),
    ] {
        let seq = sequence;
        assert!(queue_view_event(
            &outbound,
            &mut sequence,
            ProjectedViewEvent {
                event: event(Some("o0000000000000001".into())),
                combat_cue: Some(crate::world_actor::AuthoritativeCombatCue {
                    kind,
                    entity_id: presented_id.clone(),
                }),
            },
            &presentation,
            &empty,
        ));
        actual.extend(drain(&outbound));
        let mut data = json!(event(Some("o0000000000000001".into())));
        data["cue"] = former_cue;
        former.push(former_frame("message.append", seq, data));
    }
    // A cue for an entity without presentation is suppressed.
    assert!(queue_view_event(
        &outbound,
        &mut sequence,
        ProjectedViewEvent {
            event: event(None),
            combat_cue: Some(crate::world_actor::AuthoritativeCombatCue {
                kind: CombatCueKind::Hit,
                entity_id: "o0000000000000009".into(),
            }),
        },
        &empty,
        &empty,
    ));
    actual.extend(drain(&outbound));
    former.push(former_frame("message.append", 4, json!(event(None))));
    let coalesced = (0..=MAX_PROJECTED_EVENTS_PER_BATCH)
        .map(|_| ProjectedViewEvent {
            event: event(None),
            combat_cue: None,
        })
        .collect::<Vec<_>>();
    let count = coalesced.len();
    assert!(queue_view_events(
        &outbound,
        &mut sequence,
        coalesced,
        &empty,
        &empty,
    ));
    actual.extend(drain(&outbound));
    former.push(former_frame(
        "server.notice",
        5,
        json!({
            "level": "info",
            "code": "world.events_coalesced",
            "message": format!(
                "{} world changes were coalesced; the authoritative scene follows.",
                count
            ),
        }),
    ));

    let digest = "b".repeat(64);
    let ambience = tiamud_core::SceneAudio {
        channel: "ambience".into(),
        url: format!("/assets/{digest}/audio-rain.opus"),
        looped: true,
    };
    let music = tiamud_core::SceneAudio {
        channel: "music".into(),
        url: format!("/assets/{digest}/audio-theme.ogg"),
        looped: false,
    };
    assert!(queue_media_transition(
        &outbound,
        &mut sequence,
        true,
        Some(&ambience),
        Some(&music),
    ));
    former.push(former_frame(
        "media.stop",
        6,
        json!({"channel": ambience.channel}),
    ));
    former.push(former_frame(
        "media.play",
        7,
        json!({"channel": music.channel, "url": music.url, "loop": music.looped}),
    ));
    actual.extend(drain(&outbound));
    assert_same_wire(&actual, &former);
}
