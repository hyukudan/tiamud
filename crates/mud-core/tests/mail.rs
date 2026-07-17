use tiamud_core::{
    MAIL_PUBLIC_ID_BYTES, MAX_MAIL_BODY_BYTES, MAX_MAIL_PAGE, MAX_MAIL_REPORT_REASON_BYTES,
    MAX_MAIL_SUBJECT_BYTES, MAX_MAIL_TARGET_BYTES, MailAction, ParseActionError, PlayerAction,
    parse_action, valid_mail_public_id, valid_mail_visible_text,
};

const MAIL_ID: &str = "m0123456789abcdef0123456789abcdef";

#[test]
fn parses_the_complete_mail_command_surface_without_lowercasing_authored_text() {
    assert_eq!(
        parse_action("mail"),
        Ok(PlayerAction::Mail(MailAction::List { page: 1 }))
    );
    assert_eq!(
        parse_action("MAIL LIST 7"),
        Ok(PlayerAction::Mail(MailAction::List { page: 7 }))
    );
    assert_eq!(
        parse_action(&format!("mail read {MAIL_ID}")),
        Ok(PlayerAction::Mail(MailAction::Read {
            mail_id: MAIL_ID.into(),
        }))
    );
    assert_eq!(
        parse_action(&format!("mail delete {MAIL_ID}")),
        Ok(PlayerAction::Mail(MailAction::Delete {
            mail_id: MAIL_ID.into(),
        }))
    );
    assert_eq!(
        parse_action("mail send Mara | Puerta <Norte> & Sur | Conserva MAYÚSCULAS | y barras"),
        Ok(PlayerAction::Mail(MailAction::Send {
            target: "Mara".into(),
            subject: "Puerta <Norte> & Sur".into(),
            body: "Conserva MAYÚSCULAS | y barras".into(),
        }))
    );
    assert_eq!(
        parse_action(&format!("mail report {MAIL_ID} | Acoso reiterado")),
        Ok(PlayerAction::Mail(MailAction::Report {
            mail_id: MAIL_ID.into(),
            reason: "Acoso reiterado".into(),
        }))
    );
}

#[test]
fn rejects_bad_pages_ids_delimiters_controls_and_bidi_formatting() {
    for command in [
        "mail list 0",
        "mail list 8",
        "mail list two",
        "mail read m0123",
        "mail delete m0123456789abcdef0123456789abcdeg",
        "mail send Mara | only-subject",
        "mail send Mara || body",
        "mail send Mara | subject | \u{202e}body",
        &format!("mail report {MAIL_ID} | \u{2066}hidden"),
    ] {
        assert!(parse_action(command).is_err(), "accepted {command:?}");
    }
    for command in ["mail read", "mail delete", "mail send", "mail report"] {
        assert_eq!(
            parse_action(command),
            Err(ParseActionError::MissingTarget { command: "mail" })
        );
    }
}

#[test]
fn mail_limits_measure_utf8_bytes_and_public_ids_are_exact() {
    assert_eq!(MAIL_PUBLIC_ID_BYTES, MAIL_ID.len());
    assert!(valid_mail_public_id(MAIL_ID));
    assert!(!valid_mail_public_id(&MAIL_ID.to_ascii_uppercase()));
    assert!(valid_mail_visible_text(
        &"é".repeat(MAX_MAIL_SUBJECT_BYTES / 2),
        MAX_MAIL_SUBJECT_BYTES,
    ));
    assert!(!valid_mail_visible_text(
        &"é".repeat(MAX_MAIL_SUBJECT_BYTES / 2 + 1),
        MAX_MAIL_SUBJECT_BYTES,
    ));
    for maximum in [
        MAX_MAIL_TARGET_BYTES,
        MAX_MAIL_SUBJECT_BYTES,
        MAX_MAIL_BODY_BYTES,
        MAX_MAIL_REPORT_REASON_BYTES,
    ] {
        assert!(valid_mail_visible_text(&"x".repeat(maximum), maximum));
        assert!(!valid_mail_visible_text(&"x".repeat(maximum + 1), maximum));
    }
    assert_eq!(MAX_MAIL_PAGE, 7);
}
