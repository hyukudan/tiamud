//! SQLite coverage of the schema v16 story save slots (ADR 0030): round
//! trip, overwrite, caps, corruption detection, pack compatibility, account
//! cascade, migration of a v15 database and a story-core round trip.

use super::*;
use std::collections::BTreeSet;
use tempfile::TempDir;
use tiamud_persistence_sqlite::story_saves::{
    MAX_STORIES_WITH_SAVES_PER_ACCOUNT, MAX_STORY_SAVE_BYTES_PER_ACCOUNT,
    MAX_STORY_SAVE_STATE_BYTES, story_state_sha256,
};
use tiamud_story_core::{StoryInput, StoryPack, StoryState, step, view};

const CRYPT: &[u8] =
    include_bytes!("../../../story-core/fixtures/cripta-del-nigromante.story.json");
const NEON: &[u8] = include_bytes!("../../../story-core/fixtures/lluvia-de-neon.story.json");

fn slot(value: u8) -> StorySlot {
    StorySlot::new(value).unwrap()
}

fn state_json(pack_id: &str, filler: usize) -> String {
    format!(
        "{{\"format_version\":2,\"pack_id\":\"{pack_id}\",\"node\":\"{}\"}}",
        "n".repeat(filler)
    )
}

fn write(
    account_id: AccountId,
    story_id: &str,
    slot_number: u8,
    pack_sha: char,
    json: String,
) -> StorySaveWrite {
    StorySaveWrite {
        account_id,
        story_id: story_id.to_owned(),
        slot: slot(slot_number),
        pack_id: story_id.to_owned(),
        pack_sha256: pack_sha.to_string().repeat(64),
        state_format_version: 2,
        state_json: json,
    }
}

fn pack_for(story_id: &str, pack_sha: char) -> StoryPackCompatibility {
    StoryPackCompatibility {
        pack_id: story_id.to_owned(),
        pack_sha256: pack_sha.to_string().repeat(64),
        compatible_pack_sha256: BTreeSet::new(),
    }
}

async fn open_file_store() -> (TempDir, std::path::PathBuf, SqliteCharacterStore) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = SqliteCharacterStore::open(&path).await.unwrap();
    (directory, path, store)
}

#[test]
fn story_slot_bounds_are_the_schema_bounds() {
    assert_eq!(StorySlot::AUTOSAVE.get(), 0);
    assert!(StorySlot::AUTOSAVE.is_autosave());
    assert!(StorySlot::new(9).is_some());
    assert!(StorySlot::new(10).is_none());
    assert!(!slot(1).is_autosave());
}

#[tokio::test]
async fn sqlite_story_slots_round_trip_overwrite_list_and_delete() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let reader = store.load_or_create_account("Reader").await.unwrap();
    let other = store.load_or_create_account("Other").await.unwrap();

    assert!(
        store
            .list_story_slots(reader.id, "cripta")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .load_story_slot(reader.id, "cripta", slot(3), pack_for("cripta", 'a'))
            .await
            .unwrap(),
        None
    );

    let first = state_json("cripta", 4);
    let saved = store
        .save_story_slot(write(reader.id, "cripta", 3, 'a', first.clone()))
        .await
        .unwrap();
    assert_eq!(saved.slot, slot(3));
    assert_eq!(saved.state_sha256, story_state_sha256(&first));
    assert_eq!(saved.state_bytes as usize, first.len());
    assert_eq!(saved.state_format_version, 2);
    assert_eq!(saved.created_at, saved.updated_at);

    let loaded = store
        .load_story_slot(reader.id, "cripta", slot(3), pack_for("cripta", 'a'))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.state_json, first);
    assert_eq!(loaded.summary, saved);

    // Overwrite replaces the document and pack revision, keeps created_at.
    let second = state_json("cripta", 9);
    let overwritten = store
        .save_story_slot(write(reader.id, "cripta", 3, 'b', second.clone()))
        .await
        .unwrap();
    assert_eq!(overwritten.created_at, saved.created_at);
    assert!(overwritten.updated_at >= saved.updated_at);
    assert_eq!(overwritten.pack_sha256, "b".repeat(64));
    let reloaded = store
        .load_story_slot(reader.id, "cripta", slot(3), pack_for("cripta", 'b'))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.state_json, second);

    store
        .save_story_slot(write(reader.id, "cripta", 0, 'b', state_json("cripta", 1)))
        .await
        .unwrap();
    store
        .save_story_slot(write(other.id, "cripta", 5, 'b', state_json("cripta", 1)))
        .await
        .unwrap();
    let listed = store.list_story_slots(reader.id, "cripta").await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|save| save.slot.get())
            .collect::<Vec<_>>(),
        vec![0, 3],
        "listing is per account and per story, in slot order"
    );
    assert_eq!(listed[1], overwritten);

    assert!(
        store
            .delete_story_slot(reader.id, "cripta", slot(3))
            .await
            .unwrap()
    );
    assert!(
        !store
            .delete_story_slot(reader.id, "cripta", slot(3))
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .load_story_slot(reader.id, "cripta", slot(3), pack_for("cripta", 'b'))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.list_story_slots(other.id, "cripta").await.unwrap()[0]
            .slot
            .get(),
        5
    );
}

#[tokio::test]
async fn sqlite_story_slot_inputs_are_validated_before_any_write() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let reader = store.load_or_create_account("Reader").await.unwrap();

    let mut bad_story = write(reader.id, "cripta", 1, 'a', state_json("cripta", 1));
    bad_story.story_id = "Cripta".into();
    let mut bad_digest = write(reader.id, "cripta", 1, 'a', state_json("cripta", 1));
    bad_digest.pack_sha256 = "A".repeat(64);
    let mut wrong_pack_in_document = write(reader.id, "cripta", 1, 'a', state_json("otra", 1));
    wrong_pack_in_document.pack_id = "cripta".into();
    let mut wrong_version = write(reader.id, "cripta", 1, 'a', state_json("cripta", 1));
    wrong_version.state_format_version = 1;
    let not_json = write(reader.id, "cripta", 1, 'a', "{\"format_version\":2".into());
    for invalid in [
        bad_story,
        bad_digest,
        wrong_pack_in_document,
        wrong_version,
        not_json,
    ] {
        assert!(matches!(
            store.save_story_slot(invalid).await,
            Err(StoreError::InvalidStorySave)
        ));
    }
    let oversized = state_json("cripta", MAX_STORY_SAVE_STATE_BYTES);
    assert!(matches!(
        store
            .save_story_slot(write(reader.id, "cripta", 1, 'a', oversized))
            .await,
        Err(StoreError::StorySaveTooLarge {
            maximum: MAX_STORY_SAVE_STATE_BYTES
        })
    ));
    assert!(matches!(
        store
            .save_story_slot(write(
                AccountId(404),
                "cripta",
                1,
                'a',
                state_json("cripta", 1)
            ))
            .await,
        Err(StoreError::AccountNotFound(AccountId(404)))
    ));
    assert!(matches!(
        store.list_story_slots(reader.id, "no valido").await,
        Err(StoreError::InvalidStorySave)
    ));
    let mut too_many_compatible = pack_for("cripta", 'a');
    too_many_compatible.compatible_pack_sha256 =
        (0..17).map(|index| format!("{index:064x}")).collect();
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(1), too_many_compatible)
            .await,
        Err(StoreError::InvalidStorySave)
    ));
    assert!(
        store
            .list_story_slots(reader.id, "cripta")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_story_slot_caps_bound_stories_and_bytes_per_account() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let reader = store.load_or_create_account("Reader").await.unwrap();

    for index in 0..MAX_STORIES_WITH_SAVES_PER_ACCOUNT {
        let story = format!("historia-{index}");
        store
            .save_story_slot(write(reader.id, &story, 1, 'a', state_json(&story, 1)))
            .await
            .unwrap();
    }
    let extra = "historia-extra";
    assert!(matches!(
        store
            .save_story_slot(write(reader.id, extra, 1, 'a', state_json(extra, 1)))
            .await,
        Err(StoreError::StorySaveCapacity)
    ));
    // A story that already has saves may still use its other slots.
    store
        .save_story_slot(write(
            reader.id,
            "historia-0",
            2,
            'a',
            state_json("historia-0", 1),
        ))
        .await
        .unwrap();
    // Deleting the only slot of a story frees one story place.
    assert!(
        store
            .delete_story_slot(reader.id, "historia-1", slot(1))
            .await
            .unwrap()
    );
    store
        .save_story_slot(write(reader.id, extra, 1, 'a', state_json(extra, 1)))
        .await
        .unwrap();

    // Byte cap: fill with near-maximal slots across stories, then overflow.
    let other = store.load_or_create_account("Hoarder").await.unwrap();
    let slots_per_story = tiamud_persistence_sqlite::story_saves::STORY_SLOTS_PER_STORY;
    let large = |story: &str| state_json(story, MAX_STORY_SAVE_STATE_BYTES - 64);
    let fits = MAX_STORY_SAVE_BYTES_PER_ACCOUNT / large("grande-0").len();
    assert!(fits > slots_per_story && fits < 2 * slots_per_story);
    let place = |index: usize| {
        (
            format!("grande-{}", index / slots_per_story),
            u8::try_from(index % slots_per_story).unwrap(),
        )
    };
    for index in 0..fits {
        let (story, number) = place(index);
        store
            .save_story_slot(write(other.id, &story, number, 'a', large(&story)))
            .await
            .unwrap();
    }
    let (story, number) = place(fits);
    assert!(matches!(
        store
            .save_story_slot(write(other.id, &story, number, 'a', large(&story)))
            .await,
        Err(StoreError::StorySaveCapacity)
    ));
    // Overwriting an existing slot only counts the size difference.
    store
        .save_story_slot(write(other.id, "grande-0", 0, 'b', large("grande-0")))
        .await
        .unwrap();
    assert_eq!(
        store
            .list_story_slots(other.id, "grande-1")
            .await
            .unwrap()
            .len(),
        fits - slots_per_story
    );
}

#[tokio::test]
async fn sqlite_story_slot_corruption_is_a_typed_error_never_a_panic() {
    let (_directory, path, store) = open_file_store().await;
    let reader = store.load_or_create_account("Reader").await.unwrap();
    let json = state_json("cripta", 6);
    store
        .save_story_slot(write(reader.id, "cripta", 2, 'a', json.clone()))
        .await
        .unwrap();
    let tamper = rusqlite::Connection::open(&path).unwrap();

    // Same byte length, different content: only the digest notices.
    tamper
        .execute(
            "UPDATE story_saves SET state_json = replace(state_json, 'nnnnnn', 'mmmmmm')",
            [],
        )
        .unwrap();
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(2), pack_for("cripta", 'a'))
            .await,
        Err(StoreError::StorySaveCorrupt)
    ));

    // A consistent digest over a document that names another pack is still
    // corruption: metadata and content must agree.
    let foreign = state_json("otra", 6);
    tamper
        .execute(
            "UPDATE story_saves SET state_json = ?1, state_sha256 = ?2",
            rusqlite::params![foreign, story_state_sha256(&foreign)],
        )
        .unwrap();
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(2), pack_for("cripta", 'a'))
            .await,
        Err(StoreError::StorySaveCorrupt)
    ));

    // Flexible typing lets an out-of-band writer store a value of the wrong
    // type; reading it is corruption too.
    tamper
        .execute("UPDATE story_saves SET state_format_version = 'dos'", [])
        .unwrap_err();
    tamper
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE story_saves SET created_at = 'ayer';",
        )
        .unwrap();
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(2), pack_for("cripta", 'a'))
            .await,
        Err(StoreError::StorySaveCorrupt)
    ));
    assert!(matches!(
        store.list_story_slots(reader.id, "cripta").await,
        Err(StoreError::StorySaveCorrupt)
    ));

    // Overwriting a corrupt slot with a valid save repairs it.
    tamper
        .execute_batch("UPDATE story_saves SET created_at = 1, updated_at = 1;")
        .unwrap();
    store
        .save_story_slot(write(reader.id, "cripta", 2, 'a', json.clone()))
        .await
        .unwrap();
    assert_eq!(
        store
            .load_story_slot(reader.id, "cripta", slot(2), pack_for("cripta", 'a'))
            .await
            .unwrap()
            .unwrap()
            .state_json,
        json
    );
}

#[tokio::test]
async fn sqlite_story_slot_load_fails_closed_on_pack_revision_mismatch() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let reader = store.load_or_create_account("Reader").await.unwrap();
    store
        .save_story_slot(write(reader.id, "cripta", 1, 'a', state_json("cripta", 1)))
        .await
        .unwrap();

    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(1), pack_for("cripta", 'b'))
            .await,
        Err(StoreError::StorySavePackMismatch)
    ));
    let mut renamed = pack_for("cripta", 'a');
    renamed.pack_id = "cripta-2".into();
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(1), renamed)
            .await,
        Err(StoreError::StorySavePackMismatch)
    ));
    let mut declared = pack_for("cripta", 'b');
    declared.compatible_pack_sha256.insert("a".repeat(64));
    let loaded = store
        .load_story_slot(reader.id, "cripta", slot(1), declared)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.summary.pack_sha256,
        "a".repeat(64),
        "a compatible load reports the revision the state was saved with"
    );
}

#[tokio::test]
async fn sqlite_account_deletion_cascades_to_story_slots() {
    let (_directory, path, store) = open_file_store().await;
    let reader = store.load_or_create_account("Reader").await.unwrap();
    let other = store.load_or_create_account("Other").await.unwrap();
    for account in [reader.id, other.id] {
        store
            .save_story_slot(write(account, "cripta", 0, 'a', state_json("cripta", 1)))
            .await
            .unwrap();
    }
    store.shutdown().await.unwrap();

    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
    connection
        .execute("DELETE FROM accounts WHERE id = ?1", [reader.id.0])
        .unwrap();
    let remaining: Vec<i64> = connection
        .prepare("SELECT account_id FROM story_saves")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(remaining, vec![other.id.0]);
}

#[tokio::test]
async fn sqlite_store_migrates_a_v15_database_to_story_slots() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("v15.sqlite3");
    {
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        tiamud_persistence_sqlite::migrate(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts (id, name, auth_state) VALUES (7, 'Veteran', 'registered');
                 DROP TABLE story_saves;
                 PRAGMA user_version = 15;",
            )
            .unwrap();
    }

    let store = SqliteCharacterStore::open(&path).await.unwrap();
    let veteran = AccountId(7);
    assert!(
        store
            .list_story_slots(veteran, "cripta")
            .await
            .unwrap()
            .is_empty()
    );
    store
        .save_story_slot(write(veteran, "cripta", 0, 'a', state_json("cripta", 1)))
        .await
        .unwrap();
    store.shutdown().await.unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        16
    );
}

/// A real StoryState from each sample fixture survives the store byte for
/// byte and re-validates against its pack; it is rejected against the other.
#[tokio::test]
async fn sqlite_story_slot_round_trips_a_story_core_state_that_revalidates() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let reader = store.load_or_create_account("Reader").await.unwrap();
    let crypt = StoryPack::from_json_slice(CRYPT).unwrap();
    crypt.validate().unwrap();
    let neon = StoryPack::from_json_slice(NEON).unwrap();
    neon.validate().unwrap();

    for (bytes, pack, other) in [(CRYPT, &crypt, &neon), (NEON, &neon, &crypt)] {
        let mut state = StoryState::new(pack, 0x5eed).unwrap();
        // Advance a few turns through the first available choice so the
        // saved state is not merely the initial one.
        for _ in 0..3 {
            let current = view(pack, &state).unwrap();
            let Some(choice) = current.choices.first() else {
                break;
            };
            state = step(
                pack,
                &state,
                StoryInput::Choose {
                    token: choice.token.clone(),
                },
            )
            .unwrap()
            .0;
        }
        let json = String::from_utf8(state.to_json().unwrap()).unwrap();
        let pack_sha256 = hex::encode(Sha256::digest(bytes));
        let saved = store
            .save_story_slot(StorySaveWrite {
                account_id: reader.id,
                story_id: pack.id.clone(),
                slot: StorySlot::AUTOSAVE,
                pack_id: pack.id.clone(),
                pack_sha256: pack_sha256.clone(),
                state_format_version: state.format_version,
                state_json: json.clone(),
            })
            .await
            .unwrap();
        assert_eq!(saved.state_sha256, story_state_sha256(&json));

        let loaded = store
            .load_story_slot(
                reader.id,
                &pack.id,
                StorySlot::AUTOSAVE,
                StoryPackCompatibility {
                    pack_id: pack.id.clone(),
                    pack_sha256,
                    compatible_pack_sha256: BTreeSet::new(),
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state_json, json, "stored bytes are exact");
        let restored = StoryState::from_json_slice(loaded.state_json.as_bytes(), pack).unwrap();
        assert_eq!(restored, state);
        assert!(StoryState::from_json_slice(loaded.state_json.as_bytes(), other).is_err());
    }
}
