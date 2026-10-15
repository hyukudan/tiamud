//! Lua StoryPack scripts in server sessions (ADR 0034, phase S6 wiring):
//! the startup switch, the scripted fixture played to an ending through
//! the instance task, script failures and the per-input budget leaving the
//! state untouched, closing after repeated failures with the autosave of
//! the last good state, the bounded pool answering `busy`, and a shared
//! instance whose vote runs a script.

use super::*;
use crate::persistence::SqliteCharacterStore;
use serde_json::json;
use std::path::PathBuf;
use tempfile::TempDir;

const DUELO: &str = "duelo-en-el-paso";
const ROTO: &str = "guion-roto";
const ARIA: AccountId = AccountId(1);
const BRUNO: AccountId = AccountId(2);

fn duelo() -> StoryPack {
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../story-script/fixtures/duelo-en-el-paso.story.json"),
    )
    .unwrap();
    StoryPack::from_json_slice(&bytes).unwrap()
}

/// A safe first step (no script), then a choice whose effect script never
/// returns: the sandbox instruction quota stops it every time.
fn roto() -> StoryPack {
    serde_json::from_value(json!({
        "format_version": 3,
        "id": ROTO,
        "title": {"es": "Guion roto", "en": "Broken script"},
        "start": "a",
        "variables": [
            {"id": "n", "kind": {"int": {"min": 0, "max": 9, "initial": 0}}}
        ],
        "scripts": [{
            "id": "bucle",
            "kind": "effect",
            "source": "function run(story, args)\n  while true do end\nend\n",
            "writes": ["n"]
        }],
        "nodes": [
            {"id": "a", "text": {"es": "Inicio.", "en": "Start."}, "choices": [
                {"id": "ir", "label": {"es": "Ir", "en": "Go"}, "target": "b"}
            ]},
            {"id": "b", "text": {"es": "La sala del bucle.", "en": "The loop room."}, "choices": [
                {"id": "romper", "label": {"es": "Romper", "en": "Break"},
                 "effects": [{"script": {"id": "bucle"}}], "target": "c"}
            ]},
            {"id": "c", "text": {"es": "Fin.", "en": "End."}, "ending": {"id": "fin", "kind": "victory"}}
        ]
    }))
    .unwrap()
}

fn scripted(packs: Vec<StoryPack>, scripts: ScriptSettings) -> StoryManager {
    StoryManager::new(
        StoryCatalog::from_packs_with(packs, BTreeMap::new(), true).unwrap(),
        StoryLimits {
            scripts: ScriptSettings {
                enabled: true,
                ..scripts
            },
            ..StoryLimits::default()
        },
    )
}

fn manager() -> StoryManager {
    scripted(vec![duelo(), roto()], ScriptSettings::default())
}

fn labels(view: &StoryProjection) -> Vec<String> {
    view.actions()
        .iter()
        .map(|action| action.label().to_owned())
        .collect()
}

fn action(view: &StoryProjection, label: &str) -> StoryAction {
    view.actions()
        .into_iter()
        .find(|action| action.label() == label)
        .unwrap_or_else(|| panic!("no action {label:?} in {:?}", labels(view)))
}

async fn take(link: &StoryLink, view: &StoryProjection, label: &str) -> StoryProjection {
    link.input(action(view, label).request(), view.view_rev)
        .await
        .unwrap()
}

fn metric(stories: &StoryManager, name: &str) -> u64 {
    let text = stories.prometheus();
    text.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
        .unwrap_or_else(|| panic!("metric {name} missing"))
}

#[test]
fn the_catalog_refuses_scripted_packs_unless_scripts_are_enabled() {
    let refused = StoryCatalog::from_packs(vec![duelo()], BTreeMap::new()).unwrap_err();
    assert!(
        matches!(&refused, StoryCatalogError::Invalid { reason, .. } if reason.contains("TIAMUD_STORY_SCRIPTS")),
        "{refused}"
    );
    let catalog = StoryCatalog::from_packs_with(vec![duelo()], BTreeMap::new(), true).unwrap();
    assert!(catalog.get(DUELO).unwrap().scripted);

    // From a directory, as the server loads it at startup.
    let directory = TempDir::new().unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../story-script/fixtures/duelo-en-el-paso.story.json"),
        directory.path().join("duelo-en-el-paso.story.json"),
    )
    .unwrap();
    assert!(matches!(
        StoryCatalog::load_dir_with(directory.path(), false),
        Err(StoryCatalogError::Invalid { .. })
    ));
    assert_eq!(
        StoryCatalog::load_dir_with(directory.path(), true)
            .unwrap()
            .len(),
        1
    );

    // Enabled, every source is compiled: a syntax error stops the load.
    let mut broken = duelo();
    broken.scripts[0].source = "function run(story, args)\n  if then\nend\n".to_owned();
    let error = StoryCatalog::from_packs_with(vec![broken], BTreeMap::new(), true).unwrap_err();
    assert!(error.to_string().contains("does not compile"), "{error}");
}

#[test]
fn script_configuration_is_off_by_default_and_fails_closed() {
    let mut config = story_config(Some("stories".into()), None, None).unwrap();
    assert!(!config.limits.scripts.enabled);
    apply_script_story_env(&mut config, &ScriptStoryEnv::default()).unwrap();
    assert!(!config.limits.scripts.enabled);
    apply_script_story_env(
        &mut config,
        &ScriptStoryEnv {
            enabled: Some("1".into()),
            budget_ms: Some("50".into()),
            workers: Some("2".into()),
            queue: None,
        },
    )
    .unwrap();
    let scripts = &config.limits.scripts;
    assert!(scripts.enabled);
    assert_eq!(scripts.budget, Duration::from_millis(50));
    assert_eq!((scripts.workers, scripts.queue), (2, 8));

    let on = |env: ScriptStoryEnv| ScriptStoryEnv {
        enabled: Some("1".into()),
        ..env
    };
    for (env, error) in [
        (
            ScriptStoryEnv {
                enabled: Some("yes".into()),
                ..ScriptStoryEnv::default()
            },
            StoryConfigError::InvalidScriptsFlag,
        ),
        (
            ScriptStoryEnv {
                budget_ms: Some("50".into()),
                ..ScriptStoryEnv::default()
            },
            StoryConfigError::ScriptLimitsWithoutScripts,
        ),
        (
            on(ScriptStoryEnv {
                budget_ms: Some("9".into()),
                ..ScriptStoryEnv::default()
            }),
            StoryConfigError::InvalidScriptBudget,
        ),
        (
            on(ScriptStoryEnv {
                budget_ms: Some("401".into()),
                ..ScriptStoryEnv::default()
            }),
            StoryConfigError::InvalidScriptBudget,
        ),
        (
            on(ScriptStoryEnv {
                workers: Some("0".into()),
                ..ScriptStoryEnv::default()
            }),
            StoryConfigError::InvalidScriptWorkers,
        ),
        (
            on(ScriptStoryEnv {
                queue: Some("1025".into()),
                ..ScriptStoryEnv::default()
            }),
            StoryConfigError::InvalidScriptQueue,
        ),
    ] {
        let mut config = story_config(Some("stories".into()), None, None).unwrap();
        assert_eq!(apply_script_story_env(&mut config, &env), Err(error));
    }
    let mut disabled = story_config(None, None, None).unwrap();
    assert_eq!(
        apply_script_story_env(
            &mut disabled,
            &ScriptStoryEnv {
                enabled: Some("1".into()),
                ..ScriptStoryEnv::default()
            }
        ),
        Err(StoryConfigError::LimitsWithoutDirectory)
    );
}

#[tokio::test]
async fn the_scripted_duel_plays_to_an_ending_through_the_instance_task() {
    let stories = manager();
    let mut endings = BTreeSet::new();
    for seed in 1_u64..=6 {
        let account = AccountId(100 + i64::try_from(seed).unwrap());
        let link = stories
            .enter_seeded(account, DUELO, seed, Some(seed))
            .unwrap();
        let start = link.view().await.unwrap();
        assert_eq!(labels(&start), ["Desenvainar (tira un dado y suma 6)"]);
        let mut view = take(&link, &start, "Desenvainar (tira un dado y suma 6)").await;
        let mut rounds = 0;
        while view.ending.is_none() {
            rounds += 1;
            assert!(rounds < 64, "the duel never ended");
            // The flee condition is a script evaluated with each view.
            let flee = labels(&view).contains(&"Huir por la ladera".to_owned());
            view = if seed % 3 == 0 && flee {
                take(&link, &view, "Huir por la ladera").await
            } else if seed % 2 == 0
                && labels(&view).contains(&"Beber la poción de vigor".to_owned())
            {
                take(&link, &view, "Beber la poción de vigor").await
            } else {
                take(&link, &view, "Atacar con la espada").await
            };
        }
        assert!(view.actions().is_empty());
        endings.insert(format!("{:?}", view.ending));
        assert!(link.leave().await.is_ok());
    }
    assert!(!endings.is_empty());
    assert!(metric(&stories, "tiamud_story_script_invocations_total") > 0);
    assert_eq!(metric(&stories, "tiamud_story_script_failures_total"), 0);
    assert_eq!(metric(&stories, "tiamud_story_script_busy_total"), 0);
}

#[tokio::test]
async fn a_runaway_script_fails_the_step_and_leaves_the_state_untouched() {
    let stories = manager();
    let link = stories.enter_in(ARIA, ROTO, Some("en")).unwrap();
    let start = link.view().await.unwrap();
    let room = take(&link, &start, "Go").await;
    assert_eq!(room.turn, 1);
    let failed = link
        .input(action(&room, "Break").request(), room.view_rev)
        .await;
    assert_eq!(failed, Err(StoryFailure::Script));
    // Nothing changed: same view, same ids, same turn.
    assert_eq!(link.view().await.unwrap(), room);
    assert_eq!(metric(&stories, "tiamud_story_script_failures_total"), 1);
    // Players only ever see the generic message, in the story's language.
    assert_eq!(StoryFailure::Script.code(), StoryErrorCode::Internal);
    let english = StoryFailure::Script.message_in(link.locale());
    assert!(english.starts_with("The story could not apply"));
    assert!(!english.contains("bucle"));
    assert!(
        StoryFailure::Script
            .message_in("es")
            .starts_with("La historia no pudo")
    );
    // A working action afterwards resets the failure count.
    assert!(link.undo().await.is_ok());
}

#[tokio::test]
async fn an_exhausted_input_budget_fails_without_running_more_scripts() {
    let stories = scripted(
        vec![duelo()],
        ScriptSettings {
            budget: Duration::ZERO,
            ..ScriptSettings::default()
        },
    );
    let link = stories.enter_seeded(ARIA, DUELO, 3, Some(3)).unwrap();
    // The start scene has no script; the duel's view has a condition script.
    let start = link.view().await.unwrap();
    let failed = link
        .input(
            action(&start, "Desenvainar (tira un dado y suma 6)").request(),
            start.view_rev,
        )
        .await;
    assert_eq!(failed, Err(StoryFailure::Script));
    assert_eq!(link.view().await.unwrap(), start);
    assert_eq!(
        metric(&stories, "tiamud_story_script_budget_exceeded_total"),
        1
    );
    assert_eq!(metric(&stories, "tiamud_story_script_invocations_total"), 0);
}

#[tokio::test]
async fn repeated_script_failures_close_the_instance_with_the_last_good_autosave() {
    let directory = TempDir::new().unwrap();
    let store = SqliteCharacterStore::open(directory.path().join("state.sqlite3"))
        .await
        .unwrap();
    let account = store.load_or_create_account("Aria").await.unwrap().id;
    let store: Arc<dyn CharacterStore> = Arc::new(store);
    let stories = manager();
    stories.attach_store(store.clone());

    let link = stories.enter(account, ROTO).unwrap();
    let start = link.view().await.unwrap();
    let room = take(&link, &start, "Ir").await;
    let request = action(&room, "Romper").request();
    for _ in 0..scripts::SCRIPT_FAILURES_BEFORE_CLOSE {
        assert_eq!(
            link.input(request.clone(), room.view_rev).await,
            Err(StoryFailure::Script)
        );
    }
    // The instance closed on its own; its autosave holds the last good state.
    for _ in 0..100 {
        if stories.live_instances() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(stories.live_instances(), 0);
    assert_eq!(link.view().await, Err(StoryFailure::Gone));
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_instances_closed_total{reason=\"script_failures\"} 1")
    );
    let slots = store.list_story_slots(account, ROTO).await.unwrap();
    assert!(slots.iter().any(|slot| slot.slot.is_autosave()));
    let again = stories.enter(account, ROTO).unwrap();
    let restored = again.restore(0).await.unwrap();
    assert_eq!(restored.turn, room.turn);
    assert_eq!(labels(&restored), labels(&room));
}

#[tokio::test]
async fn a_saturated_script_pool_answers_busy_and_changes_nothing() {
    let stories = scripted(
        vec![duelo()],
        ScriptSettings {
            workers: 1,
            queue: 0,
            ..ScriptSettings::default()
        },
    );
    let link = stories.enter_seeded(ARIA, DUELO, 5, Some(5)).unwrap();
    let start = link.view().await.unwrap();
    // Another instance's job holds the only admission.
    let held = stories.inner.pool.admit().unwrap();
    let request = action(&start, "Desenvainar (tira un dado y suma 6)").request();
    assert_eq!(
        link.input(request.clone(), start.view_rev).await,
        Err(StoryFailure::Busy)
    );
    assert_eq!(link.view().await.unwrap(), start);
    assert_eq!(metric(&stories, "tiamud_story_script_busy_total"), 1);
    assert_eq!(
        StoryFailure::Busy.message_in("en"),
        "The story is busy. Please try again."
    );
    // Once the pool frees up the same input applies.
    drop(held);
    let duel = link.input(request, start.view_rev).await.unwrap();
    assert!(labels(&duel).contains(&"Atacar con la espada".to_owned()));
}

#[tokio::test]
async fn a_shared_vote_resolves_before_its_scripts_run() {
    let stories = manager();
    let host = stories
        .enter_seeded(ARIA, DUELO, 9, Some(9))
        .unwrap()
        .share("Aria")
        .await
        .unwrap();
    let code = host.frames().unwrap().borrow().lobby(ARIA).code.unwrap();
    let bruno = stories.join(BRUNO, "Bruno", &code, false).await.unwrap();
    let mut view = host.view().await.unwrap();
    let mut decisions = 0;
    while view.ending.is_none() {
        decisions += 1;
        assert!(decisions < 64, "the shared duel never ended");
        let label = if labels(&view)
            .iter()
            .any(|label| label.starts_with("Desenvainar"))
        {
            "Desenvainar (tira un dado y suma 6)"
        } else {
            "Atacar con la espada"
        };
        let chosen = action(&view, label);
        // One vote of two: counted, nothing runs yet.
        let pending = host.input(chosen.request(), view.view_rev).await.unwrap();
        assert_eq!(pending.view_rev, view.view_rev);
        let invocations = metric(&stories, "tiamud_story_script_invocations_total");
        // The second vote decides; the script runs once for the winner.
        view = bruno
            .vote(Some(chosen.leaf_id().to_owned()), view.view_rev)
            .await
            .unwrap();
        assert!(metric(&stories, "tiamud_story_script_invocations_total") > invocations);
        let seen = bruno.frames().unwrap().borrow().view.clone();
        assert_eq!(seen.view_rev, view.view_rev);
    }
    assert_eq!(metric(&stories, "tiamud_story_script_failures_total"), 0);
    assert!(metric(&stories, "tiamud_story_shared_decisions_total") >= 2);
}
