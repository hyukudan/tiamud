use crate::{
    persistence::{BoardEntry, BoardPage, CharacterStore, StoreError},
    social::{SocialRuntime, ensure_chat_allowed},
    world_actor::GameError,
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use std::fmt::Write as _;
use tiamud_core::{
    BOARD_PUBLIC_ID_BYTES, BoardAction, BoardKey, MAX_BOARD_BODY_BYTES, MAX_BOARD_PAGE,
    MAX_BOARD_REPORT_REASON_BYTES, MAX_BOARD_SUBJECT_BYTES, SessionId, World,
    valid_board_public_id, valid_board_subject, valid_board_visible_text,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDescriptor {
    pub board: BoardKey,
    pub can_post: bool,
}

#[derive(Debug)]
pub struct AppliedBoard {
    pub output: String,
    pub catalog: Option<Vec<BoardDescriptor>>,
    pub page: Option<BoardPage>,
    pub entry: Option<BoardEntry>,
}

pub async fn apply_board(
    runtime: &mut SocialRuntime,
    world: &World,
    store: &dyn CharacterStore,
    session: SessionId,
    action: BoardAction,
) -> Result<AppliedBoard, GameError> {
    let actor = world.character_snapshot(session)?;
    match action {
        BoardAction::Catalog => {
            let can_post = !store.social_profile(actor.id).await?.muted;
            let catalog = BoardKey::ALL
                .into_iter()
                .map(|board| BoardDescriptor { board, can_post })
                .collect::<Vec<_>>();
            let output = render_catalog(&catalog);
            Ok(AppliedBoard {
                output,
                catalog: Some(catalog),
                page: None,
                entry: None,
            })
        }
        BoardAction::List { board, page } => {
            ensure_page(page)?;
            let page = store
                .list_board_posts(actor.id, board, u32::from(page))
                .await?;
            let output = render_page(&page);
            Ok(AppliedBoard {
                output,
                catalog: None,
                page: Some(page),
                entry: None,
            })
        }
        BoardAction::Read { post_id } => {
            ensure_public_id(&post_id)?;
            let entry = store.read_board_post(actor.id, &post_id).await?;
            let output = render_entry(&entry);
            Ok(AppliedBoard {
                output,
                catalog: None,
                page: None,
                entry: Some(entry),
            })
        }
        BoardAction::Post {
            board,
            subject,
            body,
        } => {
            let subject = normalise_text(&subject, MAX_BOARD_SUBJECT_BYTES)?;
            let body = normalise_text(&body, MAX_BOARD_BODY_BYTES)?;
            if !valid_board_subject(&subject) {
                return Err(StoreError::InvalidBoardInput.into());
            }
            ensure_chat_allowed(runtime, store, actor.id).await?;
            let public_id = random_public_id();
            let entry = store
                .publish_board_post(actor.id, board, &public_id, &subject, &body)
                .await?;
            let output = format!(
                "Board post {} published to {}.\r\n",
                entry.summary.public_id,
                entry.summary.board.as_str(),
            );
            Ok(AppliedBoard {
                output,
                catalog: None,
                page: None,
                entry: Some(entry),
            })
        }
        BoardAction::Delete { post_id } => {
            ensure_public_id(&post_id)?;
            store.delete_board_post(actor.id, &post_id).await?;
            Ok(AppliedBoard {
                output: "Board post deleted.\r\n".to_owned(),
                catalog: None,
                page: None,
                entry: None,
            })
        }
        BoardAction::Report { post_id, reason } => {
            ensure_public_id(&post_id)?;
            let reason = normalise_text(&reason, MAX_BOARD_REPORT_REASON_BYTES)?;
            let report_id = store.report_board_post(actor.id, &post_id, &reason).await?;
            Ok(AppliedBoard {
                output: format!(
                    "Board post reported for moderator review as report {report_id}.\r\n"
                ),
                catalog: None,
                page: None,
                entry: None,
            })
        }
    }
}

fn ensure_page(page: u16) -> Result<(), StoreError> {
    (1..=MAX_BOARD_PAGE)
        .contains(&page)
        .then_some(())
        .ok_or(StoreError::InvalidBoardInput)
}

fn ensure_public_id(value: &str) -> Result<(), StoreError> {
    valid_board_public_id(value)
        .then_some(())
        .ok_or(StoreError::InvalidBoardInput)
}

fn normalise_text(value: &str, maximum_bytes: usize) -> Result<String, StoreError> {
    let value = value.trim();
    valid_board_visible_text(value, maximum_bytes)
        .then(|| value.to_owned())
        .ok_or(StoreError::InvalidBoardInput)
}

fn random_public_id() -> String {
    let mut random = [0_u8; 16];
    OsRng.fill_bytes(&mut random);
    let mut id = String::with_capacity(BOARD_PUBLIC_ID_BYTES);
    id.push('b');
    for byte in random {
        write!(&mut id, "{byte:02x}").expect("writing to a String cannot fail");
    }
    debug_assert!(valid_board_public_id(&id));
    id
}

fn render_catalog(catalog: &[BoardDescriptor]) -> String {
    let mut output = "Community boards:\r\n".to_owned();
    for descriptor in catalog {
        writeln!(
            &mut output,
            "- {} ({})\r",
            descriptor.board.as_str(),
            if descriptor.can_post {
                "posting available"
            } else {
                "read only"
            },
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn render_page(page: &BoardPage) -> String {
    let mut output = format!(
        "Board {} page {}/{} ({} post(s); capacity {}).\r\n",
        page.board.as_str(),
        page.page,
        page.total_pages,
        page.total_posts,
        page.capacity,
    );
    if page.posts.is_empty() {
        output.push_str("- No posts on this page.\r\n");
        return output;
    }
    for post in &page.posts {
        writeln!(
            &mut output,
            "- {} by {}: {} (posted {}, expires {})\r",
            post.public_id, post.author_name, post.subject, post.created_at, post.expires_at,
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn render_entry(entry: &BoardEntry) -> String {
    format!(
        "Board post {} on {} by {}\r\nSubject: {}\r\nPosted: {} · Expires: {}\r\n\r\n{}\r\n",
        entry.summary.public_id,
        entry.summary.board.as_str(),
        entry.summary.author_name,
        entry.summary.subject,
        entry.summary.created_at,
        entry.summary.expires_at,
        entry.body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::BoardPostSummary;

    #[test]
    fn generated_public_ids_are_closed_and_unpredictable_per_call() {
        let first = random_public_id();
        let second = random_public_id();
        assert!(valid_board_public_id(&first));
        assert!(valid_board_public_id(&second));
        assert_ne!(first, second);
    }

    #[test]
    fn text_renderers_are_complete_without_internal_identities() {
        let summary = BoardPostSummary {
            public_id: "b0123456789abcdef0123456789abcdef".into(),
            board: BoardKey::General,
            author_name: "Aria".into(),
            subject: "Gate <&>".into(),
            created_at: 10,
            expires_at: 20,
            can_delete: true,
            can_report: false,
        };
        let page = BoardPage {
            board: BoardKey::General,
            page: 1,
            total_pages: 1,
            total_posts: 1,
            capacity: 1_000,
            posts: vec![summary.clone()],
        };
        let page_text = render_page(&page);
        assert!(page_text.contains(&summary.public_id));
        assert!(page_text.contains("Gate <&>"));
        let entry_text = render_entry(&BoardEntry {
            summary,
            body: "Body | remains plain text".into(),
        });
        assert!(entry_text.contains("Body | remains plain text"));
        assert!(!entry_text.contains("CharacterId"));
        assert!(!entry_text.contains("AccountId"));
    }
}
