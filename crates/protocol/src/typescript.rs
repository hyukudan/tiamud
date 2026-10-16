//! Generates `apps/web/src/lib/generated/protocol.ts` and fails when the
//! committed file is stale.
//!
//! Check (part of `cargo test --workspace`):
//!
//! ```sh
//! cargo test --locked -p tiamud-protocol
//! ```
//!
//! Regenerate after changing a wire type, then review the diff:
//!
//! ```sh
//! TIAMUD_UPDATE_GENERATED_PROTOCOL=1 cargo test --locked -p tiamud-protocol
//! ```

use super::*;
use std::path::PathBuf;
use ts_rs::Config;

const UPDATE_VARIABLE: &str = "TIAMUD_UPDATE_GENERATED_PROTOCOL";

fn generated_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/src/lib/generated/protocol.ts")
}

/// Ordered declarations. Order is part of the generated output, so new types
/// are appended within their group rather than sorted implicitly.
fn declarations(config: &Config) -> Vec<String> {
    vec![
        // Envelope and vocabulary.
        ServerMessageType::decl(config),
        ClientMessageType::decl(config),
        ServerEnvelope::<()>::decl(config),
        UnsequencedEnvelope::<()>::decl(config),
        // Client → server.
        ClientKind::decl(config),
        ImageFormat::decl(config),
        ClientHelloData::decl(config),
        ClientCapabilities::decl(config),
        ClientAuth::decl(config),
        CommandSubmitData::decl(config),
        SyncRequestData::decl(config),
        ClientHeartbeatData::decl(config),
        // Server → client: control.
        ServerWelcome::decl(config),
        NoticeLevel::decl(config),
        ServerNotice::decl(config),
        ServerHeartbeat::decl(config),
        ActionResult::decl(config),
        // Server → client: scene and player state.
        SyncScene::<()>::decl(config),
        SyncPlayer::<()>::decl(config),
        ScenePatch::<(), (), ()>::decl(config),
        EntityKind::decl(config),
        EntityRemove::decl(config),
        EntityUpsert::<()>::decl(config),
        VitalsUpdate::<()>::decl(config),
        InventoryPatch::<(), (), ()>::decl(config),
        MapSnapshot::<()>::decl(config),
        ContainerContents::<(), ()>::decl(config),
        TradeContents::<(), ()>::decl(config),
        // Server → client: narrative, conversation and media.
        CombatCueKind::decl(config),
        CombatCuePose::decl(config),
        CombatCue::decl(config),
        ChatChannel::decl(config),
        ChatMessage::decl(config),
        MediaChannel::decl(config),
        MediaPlay::decl(config),
        MediaStop::decl(config),
        // Server → client: offline mail and community boards.
        MailMessageSummary::decl(config),
        MailList::decl(config),
        MailMessage::decl(config),
        BoardKey::decl(config),
        BoardDescriptor::decl(config),
        BoardCatalog::decl(config),
        BoardPostSummary::decl(config),
        BoardList::decl(config),
        BoardEntry::decl(config),
        // Story sessions (ADR 0029).
        StoryChooseData::decl(config),
        StoryMenuData::decl(config),
        StoryEmptyData::decl(config),
        StoryEntered::decl(config),
        StoryBlockKind::decl(config),
        StoryEndingKind::decl(config),
        StoryEnding::decl(config),
        StoryCueAction::decl(config),
        StoryPrompt::decl(config),
        StoryImage::decl(config),
        StoryItem::decl(config),
        StoryView::decl(config),
        StoryLeftReason::decl(config),
        StoryLeft::decl(config),
        StoryErrorCode::decl(config),
        StoryError::decl(config),
        // Story save slots (ADR 0030).
        StorySlotData::decl(config),
        StorySlotSummary::decl(config),
        StorySlots::decl(config),
        StorySaved::decl(config),
        StoryRestored::decl(config),
        // Shared story instances (ADR 0033).
        StoryDecisionMode::decl(config),
        StorySeatRole::decl(config),
        StoryJoinData::decl(config),
        StoryNameData::decl(config),
        StoryModeData::decl(config),
        StoryVoteData::decl(config),
        StorySeat::decl(config),
        StoryLobby::decl(config),
        StoryTallyEntry::decl(config),
        StoryTally::decl(config),
        StoryInvitation::decl(config),
        // Parser stories (ADR 0031 server wiring).
        StoryCommandData::decl(config),
    ]
}

/// Prefixes the `type` line of a declaration (after any JSDoc) with `export`.
fn exported(declaration: &str) -> String {
    let mut output = String::with_capacity(declaration.len() + 8);
    let mut exported = false;
    for line in declaration.lines() {
        if !exported && line.starts_with("type ") {
            output.push_str("export ");
            exported = true;
        }
        output.push_str(line);
        output.push('\n');
    }
    assert!(exported, "declaration without a type line: {declaration}");
    output
}

fn render() -> String {
    // JSON numbers decode to JavaScript numbers; every protocol integer is
    // bounded well below 2^53 by the server before it is projected.
    let config = Config::new().with_large_int("number");
    let mut output = String::from(
        "// @generated by `cargo test -p tiamud-protocol` from crates/protocol/src/lib.rs.\n\
         // Do not edit by hand: run `TIAMUD_UPDATE_GENERATED_PROTOCOL=1 cargo test -p tiamud-protocol`.\n\
         // These declarations describe the `tiamud.v1` wire shape; the browser still\n\
         // validates every inbound frame at runtime in `../protocol.ts`.\n\n",
    );
    output.push_str(&format!(
        "export type ProtocolVersion = {PROTOCOL_VERSION};\n\nexport const WEBSOCKET_SUBPROTOCOL = '{WEBSOCKET_SUBPROTOCOL}';\n"
    ));
    for declaration in declarations(&config) {
        output.push('\n');
        output.push_str(&exported(&declaration));
    }
    output
}

#[test]
fn generated_typescript_is_current() {
    let path = generated_path();
    let rendered = render();
    if std::env::var_os(UPDATE_VARIABLE).is_some_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().expect("generated path has a parent"))
            .expect("create generated directory");
        std::fs::write(&path, &rendered).expect("write generated TypeScript");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == rendered,
        "{} is stale; run `{UPDATE_VARIABLE}=1 cargo test --locked -p tiamud-protocol` and review the diff",
        path.display()
    );
}
