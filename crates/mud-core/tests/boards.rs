use tiamud_core::{
    BOARD_PUBLIC_ID_BYTES, BoardAction, BoardKey, MAX_BOARD_BODY_BYTES, MAX_BOARD_PAGE,
    MAX_BOARD_REPORT_REASON_BYTES, MAX_BOARD_SUBJECT_BYTES, ParseActionError, PlayerAction,
    parse_action, valid_board_public_id, valid_board_subject, valid_board_visible_text,
};

const POST_ID: &str = "b0123456789abcdef0123456789abcdef";

#[test]
fn parses_the_exact_board_text_contract_without_lowercasing_content() {
    assert_eq!(
        parse_action("board"),
        Ok(PlayerAction::Board(BoardAction::Catalog))
    );
    assert_eq!(
        parse_action("board list GENERAL"),
        Ok(PlayerAction::Board(BoardAction::List {
            board: BoardKey::General,
            page: 1,
        }))
    );
    assert_eq!(
        parse_action("board list newbie 100"),
        Ok(PlayerAction::Board(BoardAction::List {
            board: BoardKey::Newbie,
            page: 100,
        }))
    );
    assert_eq!(
        parse_action(&format!("board read {POST_ID}")),
        Ok(PlayerAction::Board(BoardAction::Read {
            post_id: POST_ID.into(),
        }))
    );
    assert_eq!(
        parse_action("board post General | Plan <&> | Meet | beyond the gate."),
        Ok(PlayerAction::Board(BoardAction::Post {
            board: BoardKey::General,
            subject: "Plan <&>".into(),
            body: "Meet | beyond the gate.".into(),
        }))
    );
    assert_eq!(
        parse_action(&format!("board delete {POST_ID}")),
        Ok(PlayerAction::Board(BoardAction::Delete {
            post_id: POST_ID.into(),
        }))
    );
    assert_eq!(
        parse_action(&format!("board report {POST_ID} | abuse | repeated")),
        Ok(PlayerAction::Board(BoardAction::Report {
            post_id: POST_ID.into(),
            reason: "abuse | repeated".into(),
        }))
    );
}

#[test]
fn rejects_unknown_boards_bad_pages_and_shape_smuggling() {
    for command in [
        "board list",
        "board list staff",
        "board list general 0",
        "board list general 101",
        "board list general 1 extra",
        "board read",
        "board read b0123456789ABCDEF0123456789abcdef",
        "board delete b0123",
        "board post",
        "board post general | subject",
        "board post unknown | subject | body",
        "board post general | subject |",
        "board report",
        "board report b0123456789abcdef0123456789abcdef",
        "board report b0123456789abcdef0123456789abcdef |",
    ] {
        assert!(parse_action(command).is_err(), "accepted {command:?}");
    }
    assert_eq!(
        parse_action("board list"),
        Err(ParseActionError::MissingTarget { command: "board" })
    );
}

#[test]
fn board_validators_are_byte_bounded_and_reject_controls_bidi_and_pipe_subjects() {
    assert_eq!(POST_ID.len(), BOARD_PUBLIC_ID_BYTES);
    assert!(valid_board_public_id(POST_ID));
    assert!(!valid_board_public_id("m0123456789abcdef0123456789abcdef"));
    assert!(!valid_board_public_id("b0123456789ABCDEF0123456789abcdef"));

    assert!(valid_board_subject("A <safe> & subject"));
    assert!(!valid_board_subject("bad | subject"));
    assert!(!valid_board_subject(" safe"));
    assert!(!valid_board_subject("safe\nunsafe"));
    assert!(!valid_board_subject("safe\u{202e}unsafe"));
    assert!(valid_board_visible_text(
        "body | keeps the separator",
        MAX_BOARD_BODY_BYTES
    ));
    assert!(valid_board_visible_text(
        "reason | keeps it too",
        MAX_BOARD_REPORT_REASON_BYTES
    ));

    let exact_subject = "é".repeat(MAX_BOARD_SUBJECT_BYTES / 2);
    assert_eq!(exact_subject.len(), MAX_BOARD_SUBJECT_BYTES);
    assert!(valid_board_subject(&exact_subject));
    assert!(!valid_board_subject(&format!("{exact_subject}é")));

    let exact_body = "é".repeat(MAX_BOARD_BODY_BYTES / 2);
    assert!(valid_board_visible_text(&exact_body, MAX_BOARD_BODY_BYTES));
    assert!(!valid_board_visible_text(
        &format!("{exact_body}é"),
        MAX_BOARD_BODY_BYTES
    ));
    assert_eq!(MAX_BOARD_PAGE, 100);
}

#[test]
fn board_keys_have_a_closed_stable_wire_vocabulary() {
    assert_eq!(BoardKey::ALL, [BoardKey::General, BoardKey::Newbie]);
    assert_eq!(BoardKey::General.as_str(), "general");
    assert_eq!(BoardKey::Newbie.as_str(), "newbie");
    assert_eq!(BoardKey::parse(" General "), Some(BoardKey::General));
    assert_eq!(BoardKey::parse("newbie"), Some(BoardKey::Newbie));
    assert_eq!(BoardKey::parse("staff"), None);
}
