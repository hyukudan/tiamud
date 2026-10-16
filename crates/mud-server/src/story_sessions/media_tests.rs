//! Story presentation: reviewed background and portrait images, per-session
//! locales, the pack-authored parser prompt and the reviewed sample media map
//! shipped with the fixtures.

use super::*;
use serde_json::json;
use std::path::PathBuf;

const LLUVIA: &str = "lluvia-de-neon";
const CRIPTA: &str = "cripta-del-nigromante";

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../story-core/fixtures")
}

fn public_assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/public")
}

fn fixture(name: &str) -> StoryPack {
    StoryPack::from_json_slice(
        &std::fs::read(fixture_dir().join(format!("{name}.story.json"))).unwrap(),
    )
    .unwrap()
}

/// The reviewed sample media map shipped next to the fixtures.
pub(super) fn fixture_media() -> StoryMedia {
    parse_media_manifest(&std::fs::read(fixture_dir().join(STORY_MEDIA_FILE)).unwrap()).unwrap()
}

fn catalog_stories(packs: Vec<StoryPack>, media: StoryMedia) -> StoryManager {
    StoryManager::new(
        StoryCatalog::from_packs(packs, media).unwrap(),
        StoryLimits::default(),
    )
}

fn background(view: &StoryProjection) -> Option<protocol::StoryImage> {
    view.items.iter().find_map(|item| match item {
        StoryItem::Scene { background, .. } => background.clone(),
        _ => None,
    })
}

fn portraits(view: &StoryProjection) -> Vec<(String, Option<protocol::StoryImage>)> {
    view.items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Portrait { name, image } => Some((name.clone(), image.clone())),
            _ => None,
        })
        .collect()
}

async fn act(link: &StoryLink, view: &StoryProjection, label: &str) -> StoryProjection {
    let action = view
        .actions()
        .into_iter()
        .find(|action| action.label() == label)
        .unwrap_or_else(|| panic!("no action {label:?}"));
    link.input(action.request(), view.view_rev).await.unwrap()
}

#[test]
fn the_shipped_media_map_points_at_reviewed_pipeline_bundles() {
    let media = fixture_media();
    assert!(media.audio.is_empty());
    assert_eq!(
        media.images.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "fondo-club",
            "fondo-oficina",
            "retrato-damaso-neutral",
            "retrato-lola-neutral"
        ]
    );
    for (id, image) in &media.images {
        // Every image has Spanish and English alt text.
        for locale in ["es", "en"] {
            assert!(image.alt.0.contains_key(locale), "{id} lacks {locale} alt");
        }
        let manifest_path = public_assets().join(image.manifest.trim_start_matches('/'));
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        let digest = image
            .manifest
            .trim_start_matches("/assets/")
            .trim_end_matches("/manifest.json");
        assert_eq!(manifest["schema"], 2, "{id}");
        assert_eq!(manifest["kind"], "atmosphere", "{id}");
        assert_eq!(manifest["assetId"], format!("sha256:{digest}"), "{id}");
        assert_eq!(manifest["license"], "CC-BY-4.0", "{id}");
        assert_eq!(manifest["author"], "Tiamud project contributors", "{id}");
        for field in ["provenance", "generator", "humanReviewedAt", "alt"] {
            assert!(
                manifest[field]
                    .as_str()
                    .is_some_and(|text| !text.is_empty()),
                "{id} lacks {field}"
            );
        }
        let mut formats = BTreeSet::new();
        for variant in manifest["variants"].as_array().unwrap() {
            let path = variant["path"].as_str().unwrap();
            assert!(path.starts_with(&format!("{digest}/scene-")), "{path}");
            let bytes = std::fs::read(public_assets().join("assets").join(path)).unwrap();
            assert_eq!(variant["bytes"], bytes.len(), "{path}");
            assert_eq!(variant["sha256"], sha256_hex(&bytes), "{path}");
            formats.insert(variant["format"].as_str().unwrap().to_owned());
        }
        assert_eq!(
            formats.into_iter().collect::<Vec<_>>(),
            ["avif", "jpeg", "webp"],
            "{id}"
        );
    }
}

#[test]
fn image_media_maps_are_bounded_and_fail_closed() {
    let digest = "c".repeat(64);
    let valid = json!({
        "formatVersion": 2,
        "images": {"fondo": {"manifest": format!("assets/{digest}/manifest.json"),
                             "alt": {"es": "Un fondo.", "en": "A background."}}}
    });
    let media = parse_media_manifest(&serde_json::to_vec(&valid).unwrap()).unwrap();
    assert_eq!(
        media.image("fondo", "en"),
        Some(protocol::StoryImage {
            manifest: format!("/assets/{digest}/manifest.json"),
            alt: "A background.".into(),
        })
    );
    // Locale fallback follows the text rules; unknown ids give nothing.
    assert_eq!(media.image("fondo", "fr").unwrap().alt, "Un fondo.");
    assert_eq!(media.image("otro", "es"), None);

    let image = |manifest: serde_json::Value, alt: serde_json::Value| json!({"formatVersion": 2, "images": {"fondo": {"manifest": manifest, "alt": alt}}});
    let good_manifest = json!(format!("assets/{digest}/manifest.json"));
    let good_alt = json!({"es": "Un fondo."});
    for invalid in [
        // Images need format 2.
        json!({"formatVersion": 1, "images": {"fondo": {"manifest": good_manifest, "alt": good_alt}}}),
        // Only pipeline manifests, never arbitrary or remote URLs.
        image(json!("https://example.test/fondo.jpg"), good_alt.clone()),
        image(
            json!(format!("assets/{digest}/scene-640.jpg")),
            good_alt.clone(),
        ),
        image(
            json!(format!("/assets/{digest}/manifest.json")),
            good_alt.clone(),
        ),
        image(json!("assets/ABC/manifest.json"), good_alt.clone()),
        // Alt text is required, bounded and plain.
        image(good_manifest.clone(), json!({})),
        image(good_manifest.clone(), json!({"es": "  "})),
        image(good_manifest.clone(), json!({"es": "uno\ndos"})),
        image(
            good_manifest.clone(),
            json!({"es": "x".repeat(protocol::MAX_STORY_IMAGE_ALT_BYTES + 1)}),
        ),
        image(good_manifest.clone(), json!({"!!": "Un fondo."})),
        image(good_manifest.clone(), json!({"es": "a", "ES": "b"})),
        // Strict shape and bad ids.
        json!({"formatVersion": 2, "images": {"fondo": {"manifest": good_manifest, "alt": good_alt, "x": 1}}}),
        json!({"formatVersion": 2, "images": {"Fondo Malo": {"manifest": good_manifest, "alt": good_alt}}}),
    ] {
        assert!(
            parse_media_manifest(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{invalid}"
        );
    }
    let too_many = (0..=MAX_MEDIA_IMAGES)
        .map(|index| {
            (
                format!("img-{index}"),
                json!({"manifest": good_manifest, "alt": good_alt}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    assert!(
        parse_media_manifest(
            &serde_json::to_vec(&json!({"formatVersion": 2, "images": too_many})).unwrap()
        )
        .is_err()
    );
}

#[tokio::test]
async fn reviewed_images_resolve_in_the_view_and_unknown_ids_degrade_to_text() {
    let stories = catalog_stories(vec![fixture(LLUVIA)], fixture_media());
    let link = stories
        .enter_seeded(AccountId(1), LLUVIA, 7, Some(1))
        .unwrap();
    let office = link.view().await.unwrap();
    let image = background(&office).expect("the office background is reviewed");
    assert!(image.manifest.starts_with("/assets/fb9c51c7"));
    assert!(image.alt.starts_with("Un despacho de detective"));
    // Telnet shows the alt text in place of the image.
    assert!(render_text(&office).contains(&format!("[{}]\r\n", image.alt)));

    let mut view = office;
    for label in [
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Escritorio",
        "USAR → Tarjeta de memoria",
        "MOVERSE → Puerta",
    ] {
        view = act(&link, &view, label).await;
    }
    // The club: background and Lola's default (neutral) portrait.
    assert!(background(&view).unwrap().alt.contains("club"));
    let present = portraits(&view);
    assert_eq!(present.len(), 1);
    assert_eq!(present[0].0, "Lola");
    assert!(
        present[0]
            .1
            .as_ref()
            .unwrap()
            .alt
            .starts_with("Silueta de Lola")
    );
    // A web part carries the image item even with audio off.
    assert!(
        view.web_parts(false)
            .iter()
            .any(|part| part.items.iter().any(|item| matches!(
                item,
                StoryItem::Scene {
                    background: Some(_),
                    ..
                }
            )))
    );

    // Without a media map every image disappears and the text is unchanged.
    let plain = stories_without_media();
    let link = plain
        .enter_seeded(AccountId(1), LLUVIA, 7, Some(1))
        .unwrap();
    let bare = link.view().await.unwrap();
    assert_eq!(background(&bare), None);
    assert!(
        !render_text(&bare).contains("[Un despacho"),
        "no alt line without a reviewed image"
    );
    let strip = |view: &StoryProjection| {
        view.items
            .iter()
            .filter(|item| !matches!(item, StoryItem::Scene { .. } | StoryItem::Portrait { .. }))
            .cloned()
            .collect::<Vec<_>>()
    };
    let reference = catalog_stories(vec![fixture(LLUVIA)], fixture_media())
        .enter_seeded(AccountId(2), LLUVIA, 7, Some(1))
        .unwrap()
        .view()
        .await
        .unwrap();
    assert_eq!(strip(&bare), strip(&reference));
}

fn stories_without_media() -> StoryManager {
    catalog_stories(vec![fixture(LLUVIA)], StoryMedia::default())
}

#[tokio::test]
async fn an_unreviewed_background_or_expression_only_loses_the_image() {
    // The alley background has no reviewed image; Dámaso's portrait does.
    let pack = fixture(LLUVIA);
    let media = fixture_media();
    let stories = catalog_stories(vec![pack], media);
    let link = stories
        .enter_seeded(AccountId(1), LLUVIA, 7, Some(1))
        .unwrap();
    let mut view = link.view().await.unwrap();
    for label in [
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Ventana",
        "INVESTIGAR → Escritorio",
        "USAR → Tarjeta de memoria",
        "MOVERSE → Puerta",
        "HABLAR → Lola",
        "Pedir un café solo",
        "Seguir hablando",
        "Despedirte",
        "MOSTRAR → Tarjeta de memoria",
        "INVESTIGAR → Reservado",
        "MOVERSE → Callejón trasero",
    ] {
        view = act(&link, &view, label).await;
    }
    assert_eq!(background(&view), None, "fondo-callejon is not reviewed");
    let present = portraits(&view);
    assert_eq!(present[0].0, "Dámaso");
    assert!(present[0].1.as_ref().unwrap().alt.contains("Dámaso"));
    // The scene text is all a text client needs.
    let text = render_text(&view);
    assert!(text.contains("Presentes: Dámaso"));
}

#[tokio::test]
async fn sessions_choose_their_locale_among_the_pack_locales() {
    let stories = catalog_stories(vec![fixture(LLUVIA), fixture(CRIPTA)], fixture_media());
    let catalog = stories.inner.catalog.get(LLUVIA).unwrap().clone();
    assert_eq!(catalog.locales(), ["es", "en"]);
    assert_eq!(stories.inner.catalog.get(CRIPTA).unwrap().locales(), ["es"]);
    // Unknown, empty or oversized requests are refused before any instance.
    for bad in ["fr", "", "x".repeat(64).as_str()] {
        assert_eq!(
            stories.enter_in(AccountId(1), LLUVIA, Some(bad)).err(),
            Some(StoryFailure::UnknownLocale)
        );
    }
    assert_eq!(
        stories.enter_in(AccountId(1), CRIPTA, Some("en")).err(),
        Some(StoryFailure::UnknownLocale)
    );
    assert_eq!(stories.live_instances(), 0);
    assert_eq!(
        StoryFailure::UnknownLocale.code(),
        StoryErrorCode::UnknownLocale
    );

    // `en-GB` resolves to the pack's `en`; title, text and alt follow it.
    let english = stories
        .enter_seeded_in(AccountId(1), LLUVIA, Some("en-GB"), 7, Some(1))
        .unwrap();
    assert_eq!(english.locale(), "en");
    assert_eq!(english.title(), "Neon Rain");
    let view = english.view().await.unwrap();
    assert_eq!(view.title, "Neon Rain");
    assert!(
        background(&view)
            .unwrap()
            .alt
            .starts_with("A detective's office")
    );
    let labels = view
        .actions()
        .iter()
        .map(|action| action.label().to_owned())
        .collect::<Vec<_>>();
    let spanish = stories
        .enter_seeded(AccountId(2), LLUVIA, 7, Some(1))
        .unwrap();
    assert_eq!(spanish.locale(), "es");
    assert_eq!(spanish.title(), "Lluvia de neón");
    let spanish_labels = spanish
        .view()
        .await
        .unwrap()
        .actions()
        .iter()
        .map(|action| action.label().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(labels.len(), spanish_labels.len());
    assert_ne!(labels, spanish_labels, "menu labels are translated");

    // Re-entering a live instance keeps its language.
    let again = stories.enter_in(AccountId(1), LLUVIA, Some("es")).unwrap();
    assert_eq!(again.locale(), "en");
}

fn bilingual_parser_pack(prompt: Option<serde_json::Value>) -> StoryPack {
    let mut world = json!({
        "lexicons": [{"id": "es"}, {"id": "en"}],
        "objects": [{
            "id": "llave", "name": {"es": "llave", "en": "key"}, "gender": "feminine",
            "nouns": {"es": ["llave"], "en": ["key"]},
            "description": {"es": "Una llave pequeña.", "en": "A small key."},
            "location": {"room": "cocina"}, "portable": true
        }]
    });
    if let Some(prompt) = prompt {
        world["prompt"] = prompt;
    }
    let pack = StoryPack::from_json_slice(
        &serde_json::to_vec(&json!({
            "format_version": 3,
            "id": "cocina",
            "title": {"es": "La cocina", "en": "The kitchen"},
            "start": "cocina",
            "nodes": [{
                "id": "cocina",
                "text": {"es": "Una cocina fría.", "en": "A cold kitchen."},
                "room": {"name": {"es": "Cocina", "en": "Kitchen"}}
            }],
            "world": world
        }))
        .unwrap(),
    )
    .unwrap();
    pack.validate().unwrap();
    pack
}

#[tokio::test]
async fn the_parser_lexicon_and_prompt_follow_the_session_locale() {
    let stories = catalog_stories(
        vec![bilingual_parser_pack(Some(
            json!({"es": "¿Qué haces?", "en": "What now?"}),
        ))],
        StoryMedia::default(),
    );
    assert_eq!(
        stories.inner.catalog.get("cocina").unwrap().locales(),
        ["es", "en"]
    );
    let english = stories
        .enter_seeded_in(AccountId(1), "cocina", Some("en"), 7, Some(1))
        .unwrap();
    let view = english.view().await.unwrap();
    assert_eq!(view.prompt().unwrap().text, "What now?");
    assert_eq!(view.title, "The kitchen");
    let reply = english
        .command("take the key", view.view_rev)
        .await
        .unwrap();
    let replies = reply
        .view
        .items
        .iter()
        .filter_map(|item| match item {
            StoryItem::Block {
                kind: StoryBlockKind::Reply,
                text,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(replies, ["You take the key."]);
    // Spanish words are not English commands.
    let reply = english
        .command("coge la llave", reply.view.view_rev)
        .await
        .unwrap();
    assert!(reply.view.turn == 1, "an unknown verb takes no turn");

    let spanish = stories
        .enter_seeded(AccountId(2), "cocina", 7, Some(1))
        .unwrap();
    let view = spanish.view().await.unwrap();
    assert_eq!(view.prompt().unwrap().text, "¿Qué haces?");
    let reply = spanish
        .command("coge la llave", view.view_rev)
        .await
        .unwrap();
    assert!(
        reply
            .view
            .items
            .iter()
            .any(|item| matches!(item, StoryItem::Block { kind: StoryBlockKind::Reply, text, .. } if text == "Coges la llave."))
    );

    // Without an authored prompt the parser shows `>`.
    let plain = stories_with_parser_default();
    let link = plain
        .enter_seeded(AccountId(3), "cocina", 7, Some(1))
        .unwrap();
    assert_eq!(link.view().await.unwrap().prompt().unwrap().text, ">");
}

fn stories_with_parser_default() -> StoryManager {
    catalog_stories(vec![bilingual_parser_pack(None)], StoryMedia::default())
}

#[test]
fn entered_payload_names_the_locale_and_its_alternatives() {
    let stories = catalog_stories(vec![fixture(LLUVIA)], StoryMedia::default());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let link = stories.enter_in(AccountId(1), LLUVIA, Some("en")).unwrap();
        let entered = protocol::StoryEntered {
            story_id: link.story_id().to_owned(),
            title: link.title().to_owned(),
            locale: link.locale().to_owned(),
            locales: link.locales().to_vec(),
        };
        assert_eq!(
            serde_json::to_value(entered).unwrap(),
            json!({"storyId": LLUVIA, "title": "Neon Rain", "locale": "en", "locales": ["es", "en"]})
        );
    });
}
