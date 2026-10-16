use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::MAX_LOCALES;

/// Bounded locale keys mapped to plain display text. Resolution is
/// deterministic: exact locale, language prefix, Spanish, English, then the
/// first lexicographic entry. Text is presentation only; no rule reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalizedText(pub BTreeMap<String, String>);

impl LocalizedText {
    /// Builds a single-locale text. Convenience for tests and tools.
    pub fn single(locale: &str, text: &str) -> Self {
        let mut map = BTreeMap::new();
        map.insert(locale.to_owned(), text.to_owned());
        Self(map)
    }

    /// Resolves the text for `locale` using the documented fallback order.
    pub fn resolve(&self, locale: &str) -> Option<&str> {
        let normalized = locale.trim().to_ascii_lowercase();
        let find = |wanted: &str| {
            self.0
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
        };
        find(&normalized)
            .or_else(|| {
                normalized
                    .split_once('-')
                    .and_then(|(language, _)| find(language))
            })
            .or_else(|| find("es"))
            .or_else(|| find("en"))
            .or_else(|| self.0.first_key_value())
            .map(|(_, value)| value.as_str())
    }

    /// Checks locale count, locale syntax, uniqueness and per-locale bytes.
    /// Newlines are allowed only when `allow_newlines` is set.
    pub(crate) fn check(&self, max_bytes: usize, allow_newlines: bool) -> Result<(), String> {
        if self.0.is_empty() {
            return Err("at least one locale is required".to_owned());
        }
        if self.0.len() > MAX_LOCALES {
            return Err(format!("at most {MAX_LOCALES} locales are allowed"));
        }
        let mut seen = BTreeSet::new();
        for (locale, text) in &self.0 {
            if !valid_locale(locale) {
                return Err("a locale key is invalid".to_owned());
            }
            if !seen.insert(locale.to_ascii_lowercase()) {
                return Err("a locale is duplicated case-insensitively".to_owned());
            }
            let trimmed = text.trim();
            if trimmed.is_empty() || text.len() > max_bytes {
                return Err(format!("text must contain 1..={max_bytes} bytes"));
            }
            if text
                .chars()
                .any(|c| c.is_control() && !(allow_newlines && c == '\n'))
            {
                return Err("text contains control characters".to_owned());
            }
        }
        Ok(())
    }
}

fn valid_locale(locale: &str) -> bool {
    let bytes = locale.as_bytes();
    (2..=35).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && !locale.ends_with('-')
        && !locale.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_with_spanish_fallback() {
        let mut map = BTreeMap::new();
        map.insert("en".to_owned(), "Door".to_owned());
        map.insert("es".to_owned(), "Puerta".to_owned());
        let text = LocalizedText(map);
        assert_eq!(text.resolve("es-ES"), Some("Puerta"));
        assert_eq!(text.resolve("EN"), Some("Door"));
        assert_eq!(text.resolve("fr"), Some("Puerta"));
    }

    #[test]
    fn rejects_bad_text() {
        assert!(LocalizedText::default().check(10, false).is_err());
        assert!(LocalizedText::single("es", "  ").check(10, false).is_err());
        assert!(
            LocalizedText::single("es", "abcdefghijk")
                .check(10, false)
                .is_err()
        );
        assert!(
            LocalizedText::single("es", "a\nb")
                .check(10, false)
                .is_err()
        );
        assert!(LocalizedText::single("es", "a\nb").check(10, true).is_ok());
        assert!(
            LocalizedText::single("es", "a\u{7}")
                .check(10, true)
                .is_err()
        );
        assert!(LocalizedText::single("e", "a").check(10, true).is_err());
        assert!(LocalizedText::single("es-", "a").check(10, true).is_err());
    }
}
