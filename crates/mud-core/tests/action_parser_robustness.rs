use tiamud_core::{ParseActionError, parse_action};

#[test]
fn surrounding_whitespace_is_semantically_inert_for_seed_commands() {
    let commands = [
        "look",
        "n",
        "go south",
        "take blue lantern",
        "party invite Aria",
        "quest complete first-light",
        "tell Aria Meet me at the gate.",
        "mute Griefer 15m repeated spam",
        "role Rowan moderator",
        "unknown command",
        "",
    ];
    let padding = ["", " ", "\t", "\r\n", "\u{2003}"];

    for command in commands {
        for prefix in padding {
            for suffix in padding {
                let padded = format!("{prefix}{command}{suffix}");
                assert_eq!(parse_action(&padded), parse_action(command));
            }
        }
    }
}

#[test]
fn deterministic_utf8_mutations_do_not_break_trim_invariance() {
    const ALPHABET: &[char] = &[
        'a', 'Z', '0', ' ', '\t', '\r', '\n', '-', '_', '|', 'á', '♥', '\u{2003}', '\0',
    ];
    let mut state = 0x5eed_f00d_cafe_babe_u64;

    for case in 0..4_096 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let length = (state as usize) % 129;
        let mut input = String::with_capacity(length);
        for offset in 0..length {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            input.push(ALPHABET[(state as usize + case + offset) % ALPHABET.len()]);
        }

        assert_eq!(parse_action(&input), parse_action(input.trim()));
    }
}

#[test]
fn overflowing_moderation_duration_is_rejected_without_wrapping() {
    let command = "ban target 18446744073709551615d repeated abuse";
    assert_eq!(
        parse_action(command),
        Err(ParseActionError::Unknown(command.to_owned()))
    );
}
