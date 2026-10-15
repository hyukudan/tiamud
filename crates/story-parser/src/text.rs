//! Normalization and tokenization.
//!
//! Matching is done on *folded* words: lowercase, with the acute, grave,
//! circumflex, tilde-on-vowel and diaeresis accents of Latin vowels removed
//! (`á é í ó ú ü` → `a e i o u`). The letter `ñ` is **kept distinct** from `n`:
//! in Spanish they are different letters (`año`/`ano`, `caña`/`cana`), and the
//! Spanish Inform library and PAWS tradition keep them apart too. Decomposed
//! input (`n` + U+0303) is recomposed to `ñ`; other combining marks are
//! dropped. `ç` is kept as is.

use crate::{MAX_INPUT_BYTES, MAX_WORDS};

/// One word after normalization. `surface` is the lowercased original (used
/// in messages); `folded` is the matching key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tok {
    pub surface: String,
    pub folded: String,
    pub kind: TokKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokKind {
    /// An ordinary word.
    Word,
    /// A list conjunction (`y`, `and` or a comma) kept inside a command.
    Conj,
    /// A pronoun implied by an enclitic (`cógelo`).
    Pronoun,
}

impl Tok {
    pub fn word(surface: String, folded: String) -> Self {
        Self {
            surface,
            folded,
            kind: TokKind::Word,
        }
    }

    pub fn is_word(&self) -> bool {
        self.kind == TokKind::Word
    }
}

/// Raw lexical units before contraction, phrase and command splitting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawTok {
    Word(Tok),
    /// `,` — splits commands only when a verb follows.
    Comma,
    /// `.`, `;`, `!` or `?` — always splits commands.
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenizeError {
    TooLong,
    TooManyWords,
}

fn fold_char(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

fn is_combining(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

/// Appends the folded form of one (already lowercased) char.
fn push_folded(folded: &mut String, c: char) {
    if is_combining(c) {
        if c == '\u{303}' && folded.ends_with('n') {
            folded.pop();
            folded.push('ñ');
        }
        return;
    }
    folded.push(fold_char(c));
}

/// Folds a word or phrase for matching: lowercase, accents removed from
/// vowels, `ñ` kept. Characters that are neither alphanumeric nor combining
/// marks are kept as they are, so callers can detect them.
pub fn fold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        for lower in c.to_lowercase() {
            push_folded(&mut out, lower);
        }
    }
    out
}

/// Splits a line into words and separators. Alphanumeric characters (and
/// combining marks inside a word) form words; `.`, `;`, `!`, `?` stop a
/// command; `,` is a soft separator; everything else is whitespace.
pub(crate) fn tokenize(input: &str) -> Result<Vec<RawTok>, TokenizeError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(TokenizeError::TooLong);
    }
    let mut out = Vec::new();
    let mut surface = String::new();
    let mut folded = String::new();
    let mut words = 0usize;

    let mut flush = |surface: &mut String,
                     folded: &mut String,
                     out: &mut Vec<RawTok>|
     -> Result<(), TokenizeError> {
        if folded.is_empty() {
            surface.clear();
            return Ok(());
        }
        words = words.saturating_add(1);
        if words > MAX_WORDS {
            return Err(TokenizeError::TooManyWords);
        }
        out.push(RawTok::Word(Tok::word(
            std::mem::take(surface),
            std::mem::take(folded),
        )));
        Ok(())
    };

    for c in input.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                surface.push(lower);
                push_folded(&mut folded, lower);
            }
        } else if is_combining(c) && !surface.is_empty() {
            surface.push(c);
            push_folded(&mut folded, c);
        } else {
            flush(&mut surface, &mut folded, &mut out)?;
            match c {
                '.' | ';' | '!' | '?' => out.push(RawTok::Stop),
                ',' => out.push(RawTok::Comma),
                _ => {}
            }
        }
    }
    flush(&mut surface, &mut folded, &mut out)?;
    Ok(out)
}

/// Capitalizes the first alphabetic character of a rendered message, after
/// any leading `¿`/`¡` or punctuation.
pub(crate) fn capitalize_first(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut done = false;
    for c in text.chars() {
        if !done && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            done = true;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(input: &str) -> Vec<String> {
        tokenize(input)
            .unwrap()
            .into_iter()
            .filter_map(|t| match t {
                RawTok::Word(t) => Some(t.folded),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn folds_accents_but_keeps_enye() {
        assert_eq!(fold("CÓGELO"), "cogelo");
        assert_eq!(fold("Pingüino"), "pinguino");
        assert_eq!(fold("Año"), "año");
        assert_eq!(fold("ENSEÑA"), "enseña");
        assert_eq!(fold("n\u{303}u"), "ñu");
        assert_eq!(fold("a\u{301}rbol"), "arbol");
    }

    #[test]
    fn tokenizes_punctuation_and_whitespace() {
        assert_eq!(
            words("  ¿Coge   la\tLLAVE?! \"ya\" "),
            ["coge", "la", "llave", "ya"]
        );
        let raw = tokenize("n, s. e;o").unwrap();
        assert_eq!(raw.len(), 7);
        assert_eq!(raw[1], RawTok::Comma);
        assert_eq!(raw[3], RawTok::Stop);
        assert_eq!(raw[5], RawTok::Stop);
        assert_eq!(words("la\u{301}mpara"), ["lampara"]);
    }

    #[test]
    fn rejects_overlong_lines() {
        assert_eq!(
            tokenize(&"a".repeat(MAX_INPUT_BYTES + 1)),
            Err(TokenizeError::TooLong)
        );
        assert_eq!(
            tokenize(&"a ".repeat(MAX_WORDS + 1)),
            Err(TokenizeError::TooManyWords)
        );
        assert!(tokenize(&"a ".repeat(MAX_WORDS)).is_ok());
    }

    #[test]
    fn capitalizes_after_inverted_marks() {
        assert_eq!(capitalize_first("¿qué quieres?"), "¿Qué quieres?");
        assert_eq!(capitalize_first("what?"), "What?");
        assert_eq!(capitalize_first(""), "");
    }
}
