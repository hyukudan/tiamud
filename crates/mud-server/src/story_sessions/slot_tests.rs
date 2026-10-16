//! SQLite-backed coverage of story save slots wired to story sessions
//! (ADR 0029 + ADR 0030): manual save, autosave on leave, idle eviction and
//! shutdown, continue, corruption, pack revision changes, caps and rate
//! limits.

use super::*;
use crate::persistence::SqliteCharacterStore;
use std::path::PathBuf;
use tempfile::TempDir;
use tiamud_persistence_sqlite::story_saves::{
    MAX_STORIES_WITH_SAVES_PER_ACCOUNT, story_state_sha256,
};

const CRIPTA: &str = "cripta-del-nigromante";

fn cripta() -> StoryPack {
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../story-core/fixtures/cripta-del-nigromante.story.json"),
    )
    .unwrap();
    StoryPack::from_json_slice(&bytes).unwrap()
}

/// Same pack id, different content: another revision of the story.
fn cripta_revised() -> StoryPack {
    let mut pack = cripta();
    pack.title = serde_json::from_value(serde_json::json!({"es": "La cripta (revisada)"})).unwrap();
    pack
}

struct Fixture {
    _directory: TempDir,
    path: PathBuf,
    store: Arc<dyn CharacterStore>,
    account: AccountId,
}

async fn fixture() -> Fixture {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = SqliteCharacterStore::open(&path).await.unwrap();
    let account = store.load_or_create_account("Lectora").await.unwrap().id;
    Fixture {
        _directory: directory,
        path,
        store: Arc::new(store),
        account,
    }
}

fn stories_with(pack: StoryPack, store: &Arc<dyn CharacterStore>, idle: Duration) -> StoryManager {
    let stories = StoryManager::new(
        StoryCatalog::from_packs(vec![pack], BTreeMap::new()).unwrap(),
        StoryLimits {
            max_instances: 8,
            idle_timeout: idle,
            ..StoryLimits::default()
        },
    );
    stories.attach_store(store.clone());
    stories
}

async fn advance(link: &StoryLink, view: StoryProjection, steps: usize) -> StoryProjection {
    let mut view = view;
    for _ in 0..steps {
        let action = view.actions().remove(0);
        view = link.input(action.request(), view.view_rev).await.unwrap();
    }
    view
}

/// Scene title, turn, score and labels: everything but the per-view ids.
fn shape(view: &StoryProjection) -> (String, u32, i64, Vec<String>) {
    (
        view.title.clone(),
        view.turn,
        view.score,
        view.actions()
            .iter()
            .map(|action| action.label().to_owned())
            .collect(),
    )
}

async fn wait_for_autosave(fixture: &Fixture) {
    for _ in 0..100 {
        let slots = fixture
            .store
            .list_story_slots(fixture.account, CRIPTA)
            .await
            .unwrap();
        if slots.iter().any(|slot| slot.slot.is_autosave()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no autosave was written");
}

#[tokio::test]
async fn save_evict_and_continue_round_trip_through_sqlite() {
    let fixture = fixture().await;
    let stories = stories_with(cripta(), &fixture.store, Duration::from_millis(150));
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    // A fresh instance without saves offers nothing to continue.
    let offer = link.entry_slots().await.unwrap();
    assert!(offer.slots.is_empty());
    assert!(!offer.offer_continue);

    let saved_view = advance(&link, link.view().await.unwrap(), 2).await;
    let saved = link.save(3).await.unwrap();
    assert_eq!(saved.slot, 3);
    let later = advance(&link, saved_view.clone(), 1).await;
    assert!(later.turn > saved_view.turn);

    // Idle eviction writes the autosave from the exiting instance task.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(stories.live_instances(), 0);
    wait_for_autosave(&fixture).await;
    assert_eq!(link.view().await, Err(StoryFailure::Gone));

    // Entering again starts fresh and offers to continue the autosave.
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    assert_eq!(link.view().await.unwrap().turn, 0);
    let offer = link.entry_slots().await.unwrap();
    assert!(offer.offer_continue);
    assert_eq!(
        offer
            .slots
            .iter()
            .map(|slot| (slot.slot, slot.compatible))
            .collect::<Vec<_>>(),
        [(0, true), (3, true)]
    );
    // Re-attaching to the live instance never repeats the offer.
    assert!(
        stories
            .attach(fixture.account)
            .unwrap()
            .entry_slots()
            .await
            .is_none()
    );

    let continued = link.restore(0).await.unwrap();
    assert_eq!(shape(&continued), shape(&later));
    // Undo history is not persisted.
    assert!(!continued.can_undo);
    let manual = link.restore(3).await.unwrap();
    assert_eq!(shape(&manual), shape(&saved_view));
    assert!(manual.view_rev > continued.view_rev);
    // The restored game plays on.
    let next = advance(&link, manual, 1).await;
    assert_eq!(next.turn, saved_view.turn + 1);
    assert!(next.can_undo);
    let metrics = stories.prometheus();
    assert!(metrics.contains("tiamud_story_saves_total{kind=\"manual\",result=\"ok\"} 1"));
    assert!(metrics.contains("tiamud_story_saves_total{kind=\"autosave\",result=\"ok\"} 1"));
    assert!(metrics.contains("tiamud_story_restores_total{result=\"ok\"} 2"));
}

#[tokio::test]
async fn leaving_autosaves_progress_and_shutdown_autosaves_live_instances() {
    let fixture = fixture().await;
    let stories = stories_with(cripta(), &fixture.store, Duration::from_secs(60));

    // No progress, no autosave.
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    assert_eq!(
        link.leave().await.unwrap(),
        LeaveOutcome { autosaved: false }
    );
    assert!(
        fixture
            .store
            .list_story_slots(fixture.account, CRIPTA)
            .await
            .unwrap()
            .is_empty()
    );

    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    let played = advance(&link, link.view().await.unwrap(), 1).await;
    assert_eq!(
        link.leave().await.unwrap(),
        LeaveOutcome { autosaved: true }
    );
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    assert!(link.entry_slots().await.unwrap().offer_continue);
    assert_eq!(shape(&link.restore(0).await.unwrap()), shape(&played));

    // Graceful shutdown closes the instance and autosaves its progress.
    let deeper = advance(&link, link.view().await.unwrap(), 1).await;
    assert_eq!(stories.shutdown().await, 1);
    assert_eq!(stories.live_instances(), 0);
    assert_eq!(link.view().await, Err(StoryFailure::Gone));
    assert_eq!(
        stories.enter(fixture.account, CRIPTA).err(),
        Some(StoryFailure::Busy)
    );
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_instances_closed_total{reason=\"shutdown\"} 1")
    );
    // A restarted server continues from the shutdown autosave.
    let restarted = stories_with(cripta(), &fixture.store, Duration::from_secs(60));
    let link = restarted.enter(fixture.account, CRIPTA).unwrap();
    assert!(link.entry_slots().await.unwrap().offer_continue);
    assert_eq!(shape(&link.restore(0).await.unwrap()), shape(&deeper));
}

#[tokio::test]
async fn corrupted_slots_fail_with_a_friendly_typed_error() {
    let fixture = fixture().await;
    let stories = stories_with(cripta(), &fixture.store, Duration::from_secs(60));
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    let view = advance(&link, link.view().await.unwrap(), 1).await;
    link.save(2).await.unwrap();
    link.save(4).await.unwrap();
    let tamper = rusqlite::Connection::open(&fixture.path).unwrap();

    // Bytes that no longer match their digest.
    tamper
        .execute(
            "UPDATE story_saves SET state_json = replace(state_json, '\"turn\":1', '\"turn\":2') WHERE slot = 2",
            [],
        )
        .unwrap();
    assert_eq!(link.restore(2).await, Err(StoryFailure::SlotCorrupt));

    // A consistent digest over a state that names an unknown node: only the
    // story-core revalidation against the pack notices.
    let forged = serde_json::to_string(&{
        let mut state: serde_json::Value = serde_json::from_str(
            &tamper
                .query_row(
                    "SELECT state_json FROM story_saves WHERE slot = 4",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        state["node"] = serde_json::json!("nodo-que-no-existe");
        state
    })
    .unwrap();
    tamper
        .execute(
            "UPDATE story_saves SET state_json = ?1, state_sha256 = ?2 WHERE slot = 4",
            rusqlite::params![forged, story_state_sha256(&forged)],
        )
        .unwrap();
    assert_eq!(link.restore(4).await, Err(StoryFailure::SlotCorrupt));

    // Nothing changed in the live game, and the message leaks no internals.
    assert_eq!(shape(&link.view().await.unwrap()), shape(&view));
    let message = StoryFailure::SlotCorrupt.message();
    assert!(!message.contains("sha") && !message.contains("node") && !message.contains(CRIPTA));
    // Saving over a corrupt slot repairs it.
    link.save(4).await.unwrap();
    assert_eq!(shape(&link.restore(4).await.unwrap()), shape(&view));
}

#[tokio::test]
async fn a_changed_pack_revision_refuses_older_slots() {
    let fixture = fixture().await;
    let original = stories_with(cripta(), &fixture.store, Duration::from_secs(60));
    let link = original.enter(fixture.account, CRIPTA).unwrap();
    advance(&link, link.view().await.unwrap(), 1).await;
    link.save(1).await.unwrap();
    assert!(link.leave().await.unwrap().autosaved);

    let revised = stories_with(cripta_revised(), &fixture.store, Duration::from_secs(60));
    let link = revised.enter(fixture.account, CRIPTA).unwrap();
    let offer = link.entry_slots().await.unwrap();
    // Listed but marked incompatible; no continue offer.
    assert!(!offer.offer_continue);
    assert!(offer.slots.iter().all(|slot| !slot.compatible));
    assert_eq!(link.restore(1).await, Err(StoryFailure::SlotIncompatible));
    assert_eq!(link.restore(0).await, Err(StoryFailure::SlotIncompatible));
    assert!(
        revised
            .prometheus()
            .contains("tiamud_story_restores_total{result=\"rejected\"} 2")
    );
    // A new save under the revised pack is compatible again.
    link.save(1).await.unwrap();
    assert!(
        link.slots()
            .await
            .unwrap()
            .iter()
            .any(|slot| slot.slot == 1 && slot.compatible)
    );
}

/// The digest the in-memory catalog records for `pack` (its canonical JSON).
fn revision_of(pack: &StoryPack) -> String {
    sha256_hex(&serde_json::to_vec(pack).unwrap())
}

#[tokio::test]
async fn declared_compatible_revisions_load_after_revalidation() {
    let fixture = fixture().await;
    // Revision A: the fixture as a StoryPack v3 document.
    let mut first = cripta();
    first.upgrade();
    let original = stories_with(first.clone(), &fixture.store, Duration::from_secs(60));
    let link = original.enter(fixture.account, CRIPTA).unwrap();
    let saved_view = advance(&link, link.view().await.unwrap(), 2).await;
    link.save(1).await.unwrap();
    assert!(link.leave().await.unwrap().autosaved);

    // Revision B declares A compatible: both slots are listed as
    // compatible, continue is offered and the state loads unchanged.
    let mut second = first.clone();
    second.title = serde_json::from_value(serde_json::json!({"es": "La cripta (2.ª)"})).unwrap();
    second.compatible_revisions = vec![revision_of(&first)];
    let revised = stories_with(second.clone(), &fixture.store, Duration::from_secs(60));
    let link = revised.enter(fixture.account, CRIPTA).unwrap();
    let offer = link.entry_slots().await.unwrap();
    assert!(offer.offer_continue);
    assert!(offer.slots.iter().all(|slot| slot.compatible));
    let restored = link.restore(1).await.unwrap();
    assert_eq!(shape(&restored).1, saved_view.turn);
    assert_eq!(
        shape(&restored).3,
        shape(&saved_view).3,
        "same choices after the compatible load"
    );
    assert!(link.restore(0).await.is_ok());
    assert!(link.leave().await.is_ok());

    // Revision C also declares A compatible but adds a variable, so A's
    // state no longer re-validates: typed `slot_incompatible`, no change.
    let mut third = first.clone();
    let mut variables = serde_json::to_value(&third.variables).unwrap();
    variables.as_array_mut().unwrap().push(serde_json::json!({
        "id": "nueva", "kind": {"bool": {"initial": false}}
    }));
    third.variables = serde_json::from_value(variables).unwrap();
    third.compatible_revisions = vec![revision_of(&first)];
    third.validate().unwrap();
    let strict = stories_with(third, &fixture.store, Duration::from_secs(60));
    let link = strict.enter(fixture.account, CRIPTA).unwrap();
    let before = link.view().await.unwrap();
    assert_eq!(link.restore(1).await, Err(StoryFailure::SlotIncompatible));
    let after = link.view().await.unwrap();
    assert_eq!(shape(&before), shape(&after));
    assert_eq!(after.view_rev, before.view_rev);
    assert!(link.leave().await.is_ok());

    // Revision D lists nothing: A's slots are refused as before.
    let mut fourth = second;
    fourth.compatible_revisions.clear();
    let refused = stories_with(fourth, &fixture.store, Duration::from_secs(60));
    let link = refused.enter(fixture.account, CRIPTA).unwrap();
    assert_eq!(link.restore(1).await, Err(StoryFailure::SlotIncompatible));
}

#[tokio::test]
async fn slot_numbers_capacity_and_unavailable_store_are_typed() {
    let fixture = fixture().await;
    let stories = stories_with(cripta(), &fixture.store, Duration::from_secs(60));
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    assert_eq!(link.save(0).await, Err(StoryFailure::InvalidSlot));
    assert_eq!(link.save(10).await, Err(StoryFailure::InvalidSlot));
    assert_eq!(link.restore(10).await, Err(StoryFailure::InvalidSlot));
    assert_eq!(link.delete_slot(10).await, Err(StoryFailure::InvalidSlot));
    assert_eq!(link.restore(5).await, Err(StoryFailure::SlotEmpty));
    assert_eq!(link.delete_slot(5).await, Err(StoryFailure::SlotEmpty));
    link.save(5).await.unwrap();
    assert!(link.delete_slot(5).await.unwrap().is_empty());

    // Fill the account's story capacity with other stories.
    for index in 0..MAX_STORIES_WITH_SAVES_PER_ACCOUNT {
        let story = format!("otra-{index}");
        fixture
            .store
            .save_story_slot(StorySaveWrite {
                account_id: fixture.account,
                story_id: story.clone(),
                slot: StorySlot::new(1).unwrap(),
                pack_id: story.clone(),
                pack_sha256: "a".repeat(64),
                state_format_version: 2,
                state_json: format!("{{\"format_version\":2,\"pack_id\":\"{story}\"}}"),
            })
            .await
            .unwrap();
    }
    assert_eq!(link.save(1).await, Err(StoryFailure::SlotCapacity));
    // A failed autosave never blocks leaving.
    advance(&link, link.view().await.unwrap(), 1).await;
    assert_eq!(
        link.leave().await.unwrap(),
        LeaveOutcome { autosaved: false }
    );
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_saves_total{kind=\"autosave\",result=\"failed\"} 1")
    );

    // Without a store every slot operation is unavailable.
    let detached = StoryManager::new(
        StoryCatalog::from_packs(vec![cripta()], BTreeMap::new()).unwrap(),
        StoryLimits::default(),
    );
    let link = detached.enter(AccountId(1), CRIPTA).unwrap();
    advance(&link, link.view().await.unwrap(), 1).await;
    assert_eq!(link.save(1).await, Err(StoryFailure::SlotsUnavailable));
    assert_eq!(link.restore(1).await, Err(StoryFailure::SlotsUnavailable));
    assert_eq!(link.slots().await, Err(StoryFailure::SlotsUnavailable));
    assert!(link.entry_slots().await.is_none());
    assert_eq!(
        link.leave().await.unwrap(),
        LeaveOutcome { autosaved: false }
    );
}

#[tokio::test]
async fn slot_operations_are_rate_limited_per_account() {
    let fixture = fixture().await;
    let stories = stories_with(cripta(), &fixture.store, Duration::from_secs(60));
    let link = stories.enter(fixture.account, CRIPTA).unwrap();
    for slot in 1..=u8::try_from(SLOT_WRITES_PER_MINUTE).unwrap() {
        link.save(slot).await.unwrap();
    }
    assert_eq!(link.save(9).await, Err(StoryFailure::RateLimited));
    assert_eq!(link.delete_slot(1).await, Err(StoryFailure::RateLimited));
    // Reads have their own budget.
    for _ in 0..SLOT_READ_BURST {
        link.slots().await.unwrap();
    }
    assert_eq!(link.slots().await, Err(StoryFailure::RateLimited));
    assert_eq!(link.restore(1).await, Err(StoryFailure::RateLimited));
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_slot_rate_limited_total 4")
    );
    // Another account is unaffected.
    let other = fixture
        .store
        .load_or_create_account("Otra")
        .await
        .unwrap()
        .id;
    let link = stories.enter(other, CRIPTA).unwrap();
    link.save(1).await.unwrap();
}

/// Fixed `updatedAt` in the recorded fixture; real values are wall-clock.
const FIXTURE_UPDATED_AT: i64 = 1_760_536_800;

fn normalized(slots: Vec<protocol::StorySlotSummary>) -> Vec<protocol::StorySlotSummary> {
    slots
        .into_iter()
        .map(|slot| protocol::StorySlotSummary {
            updated_at: FIXTURE_UPDATED_AT,
            ..slot
        })
        .collect()
}

/// The Playwright save/leave/continue scenario replays these real server
/// payloads. Regenerate with `TIAMUD_UPDATE_STORY_E2E_FIXTURE=1`.
#[tokio::test]
async fn browser_slot_fixture_is_current() {
    const UPDATE: &str = "TIAMUD_UPDATE_STORY_E2E_FIXTURE";
    let fixture = fixture().await;
    let stories = stories_with(cripta(), &fixture.store, Duration::from_secs(60));

    let link = stories
        .enter_seeded(fixture.account, CRIPTA, 7, Some(1))
        .unwrap();
    let first = link.view().await.unwrap();
    let entry = link.entry_slots().await.unwrap();
    let action = first.actions().remove(0);
    let input = super::tests::wire_input(&action, first.view_rev);
    let played = link.input(action.request(), first.view_rev).await.unwrap();
    let saved = link.save(1).await.unwrap();
    let listed = normalized(link.slots().await.unwrap());
    assert!(link.leave().await.unwrap().autosaved);

    let link = stories
        .enter_seeded(fixture.account, CRIPTA, 7, Some(1))
        .unwrap();
    let again = link.view().await.unwrap();
    let offer = link.entry_slots().await.unwrap();
    assert!(offer.offer_continue);
    let continued = link.restore(0).await.unwrap();
    assert_eq!(shape(&continued), shape(&played));

    let document = serde_json::json!({
        "storyId": CRIPTA,
        "title": link.title(),
        "enter": {"view": first.web_parts(false), "slots": entry},
        "play": {"input": input, "label": action.label(), "view": played.web_parts(false)},
        "save": {
            "input": {"type": "story.save", "data": {"slot": 1}},
            "saved": protocol::StorySaved { updated_at: FIXTURE_UPDATED_AT, ..saved },
        },
        "slots": {
            "input": {"type": "story.slots", "data": {}},
            "slots": protocol::StorySlots { slots: listed, offer_continue: false },
        },
        "reenter": {
            "view": again.web_parts(false),
            "slots": protocol::StorySlots {
                slots: normalized(offer.slots),
                offer_continue: offer.offer_continue,
            },
        },
        "continue": {
            "input": {"type": "story.continue", "data": {}},
            "restored": protocol::StoryRestored { slot: 0 },
            "view": continued.web_parts(false),
        },
    });
    let rendered = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/web/e2e/fixtures/story-slots.json");
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

#[test]
fn slot_commands_parse_in_both_languages() {
    for (line, expected) in [
        ("guardar 3", StoryInputCommand::Save(Some(3))),
        ("SAVE 9", StoryInputCommand::Save(Some(9))),
        ("guardar", StoryInputCommand::Save(None)),
        ("guardar tres", StoryInputCommand::Save(None)),
        ("cargar 0", StoryInputCommand::Restore(Some(0))),
        ("load 2", StoryInputCommand::Restore(Some(2))),
        ("restore 4", StoryInputCommand::Restore(Some(4))),
        ("partidas", StoryInputCommand::Slots),
        ("saves", StoryInputCommand::Slots),
        ("borrar 1", StoryInputCommand::Delete(Some(1))),
        ("delete 1", StoryInputCommand::Delete(Some(1))),
        ("continuar", StoryInputCommand::Continue),
        ("continue", StoryInputCommand::Continue),
        ("guardar 1 2", StoryInputCommand::Other),
    ] {
        assert_eq!(parse_story_input(line), expected, "{line}");
    }
}

#[test]
fn slot_listing_renders_dates_and_incompatible_slots() {
    assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(format_utc(1_760_536_800), "2025-10-15 14:00 UTC");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
    let slots = [
        protocol::StorySlotSummary {
            slot: 0,
            updated_at: 0,
            compatible: true,
        },
        protocol::StorySlotSummary {
            slot: 2,
            updated_at: 60,
            compatible: false,
        },
    ];
    let text = render_slots(&slots, "es");
    assert!(text.contains("0 (autoguardado) — 1970-01-01 00:00 UTC"));
    assert!(text.contains("2 — 1970-01-01 00:01 UTC (otra versión"));
    let offer = render_continue_offer(
        &protocol::StorySlots {
            slots: slots.to_vec(),
            offer_continue: true,
        },
        "es",
    )
    .unwrap();
    assert!(offer.contains("continuar"));
    assert!(
        render_continue_offer(
            &protocol::StorySlots {
                slots: slots.to_vec(),
                offer_continue: false,
            },
            "es",
        )
        .is_none()
    );
    assert!(render_slots(&[], "es").contains("guardar N"));
    // An English session reads the same listing in English.
    let english = render_slots(&slots, "en");
    assert!(english.starts_with("Saved games:"));
    assert!(english.contains("0 (autosave) — 1970-01-01 00:00 UTC"));
    assert!(english.contains("(another version of the story; cannot be loaded)"));
    assert!(render_slots(&[], "en-GB").contains("save N"));
    let offer = render_continue_offer(
        &protocol::StorySlots {
            slots: slots.to_vec(),
            offer_continue: true,
        },
        "en",
    )
    .unwrap();
    assert!(offer.starts_with("You have an autosaved game (1970-01-01 00:00 UTC)."));
}
