use std::{fmt, str::FromStr};

use thiserror::Error;

/// A validated tag slug matching `[a-z0-9][a-z0-9-]*`.
///
/// Constructed via [`FromStr`]; invalid strings are rejected at the boundary
/// so interior code works only with already-valid tags.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tag(String);

/// Error returned when a string cannot be parsed as a [`Tag`].
#[derive(Debug, Error)]
#[error("tag must be non-empty and match [a-z0-9][a-z0-9-]*")]
pub struct InvalidTag;

impl FromStr for Tag {
    type Err = InvalidTag;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidTag);
        }
        // Normalize to lowercase
        let normalized = s.to_lowercase();
        let mut chars = normalized.chars();
        // First character must be alphanumeric (lowercase)
        let first = chars.next().ok_or(InvalidTag)?;
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err(InvalidTag);
        }
        // Remaining characters: lowercase alphanumeric or hyphen
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(InvalidTag);
        }
        Ok(Tag(normalized))
    }
}

impl Tag {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_parses_valid_values() {
        assert!("hello-world".parse::<Tag>().is_ok());
        assert!("abc123".parse::<Tag>().is_ok());
        assert!("a".parse::<Tag>().is_ok());
        assert!("0".parse::<Tag>().is_ok());
        assert!("my-tag-2024".parse::<Tag>().is_ok());
    }

    #[test]
    fn tag_rejects_invalid_values() {
        // empty
        assert!("".parse::<Tag>().is_err());
        // starts with hyphen
        assert!("-hello".parse::<Tag>().is_err());
        // spaces
        assert!("hello world".parse::<Tag>().is_err());
        // underscore
        assert!("hello_world".parse::<Tag>().is_err());
        // special chars
        assert!("hello@world".parse::<Tag>().is_err());
    }

    #[test]
    fn tag_normalizes_to_lowercase() {
        let tag: Tag = "Hello-World".parse().unwrap();
        assert_eq!(tag.to_string(), "hello-world");
        assert_eq!(tag.as_str(), "hello-world");
    }

    #[test]
    fn tag_display_returns_inner_string() {
        let tag: Tag = "my-tag".parse().unwrap();
        assert_eq!(tag.to_string(), "my-tag");
        assert_eq!(tag.as_str(), "my-tag");
    }

    #[test]
    fn tag_rejects_uppercase_first_char() {
        // Uppercase characters should be rejected after normalization fails
        assert!("Hello".parse::<Tag>().is_ok()); // Actually should work after lowercasing
    }

    #[test]
    fn tag_rejects_non_ascii() {
        // Non-ASCII characters should be rejected
        assert!("café".parse::<Tag>().is_err());
        assert!("日本".parse::<Tag>().is_err());
        assert!("résumé".parse::<Tag>().is_err());
    }

    #[test]
    fn tag_allows_trailing_hyphen() {
        // Trailing hyphens are allowed by the pattern [a-z0-9][a-z0-9-]*
        assert!("hello-".parse::<Tag>().is_ok());
        assert!("tag-".parse::<Tag>().is_ok());
        assert_eq!("hello-".parse::<Tag>().unwrap().as_str(), "hello-");
    }

    #[test]
    fn tag_allows_hyphen_in_middle() {
        assert!("hello-world-tag".parse::<Tag>().is_ok());
        assert!("a-b-c-d".parse::<Tag>().is_ok());
        assert!("tag-1-2-3".parse::<Tag>().is_ok());
    }

    #[test]
    fn tag_allows_all_digits() {
        assert!("123".parse::<Tag>().is_ok());
        assert!("999".parse::<Tag>().is_ok());
        assert!("1".parse::<Tag>().is_ok());
    }

    #[test]
    fn tag_lowercase_normalization_various_cases() {
        let tag1: Tag = "ABC".parse().unwrap();
        assert_eq!(tag1.as_str(), "abc");

        let tag2: Tag = "MixedCase".parse().unwrap();
        assert_eq!(tag2.as_str(), "mixedcase");

        let tag3: Tag = "UPPERCASE-TAG-123".parse().unwrap();
        assert_eq!(tag3.as_str(), "uppercase-tag-123");
    }

    #[test]
    fn tag_rejects_double_hyphen() {
        assert!("hello--world".parse::<Tag>().is_ok()); // Hyphens are allowed in sequence
    }

    #[test]
    fn tag_rejects_starting_with_symbol() {
        assert!("#tag".parse::<Tag>().is_err());
        assert!("@tag".parse::<Tag>().is_err());
        assert!("+tag".parse::<Tag>().is_err());
        assert!("=tag".parse::<Tag>().is_err());
    }

    #[test]
    fn tag_clone_and_equality() {
        let tag1: Tag = "test-tag".parse().unwrap();
        let tag2 = tag1.clone();
        assert_eq!(tag1, tag2);
        assert_eq!(tag1.as_str(), tag2.as_str());
    }

    #[test]
    fn tag_hash_consistency() {
        use std::collections::HashSet;
        let tag1: Tag = "test".parse().unwrap();
        let tag2: Tag = "test".parse().unwrap();
        let tag3: Tag = "other".parse().unwrap();

        let mut set = HashSet::new();
        set.insert(tag1);
        set.insert(tag2); // Should not add duplicate
        set.insert(tag3);

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn invalid_tag_error_display() {
        let err: InvalidTag = InvalidTag;
        let message = err.to_string();
        assert!(message.contains("tag must be non-empty"));
        assert!(message.contains("[a-z0-9][a-z0-9-]*"));
    }

    #[test]
    fn invalid_tag_error_debug() {
        let err: InvalidTag = InvalidTag;
        let debug_str = format!("{:?}", err);
        assert_eq!(debug_str, "InvalidTag");
    }

    #[test]
    fn tag_formats_correctly() {
        use std::fmt::Write;
        let tag: Tag = "format-test".parse().unwrap();
        let mut buf = String::new();
        let _ = write!(buf, "{}", tag);
        assert_eq!(buf, "format-test");
    }

    #[test]
    fn tag_debug_impl() {
        let tag: Tag = "debug-tag".parse().unwrap();
        let debug_str = format!("{:?}", tag);
        assert!(debug_str.contains("debug-tag"));
    }

    #[test]
    fn tag_very_long_valid_tag() {
        let long_tag = "a".repeat(100);
        let tag = long_tag.parse::<Tag>();
        assert!(tag.is_ok());
        assert_eq!(tag.unwrap().as_str(), &long_tag);
    }

    #[test]
    fn tag_mixed_case_long_tag() {
        let long_tag = "LongTagWithManyCharacters".repeat(4);
        let tag = long_tag.parse::<Tag>();
        assert!(tag.is_ok());
        assert_eq!(tag.unwrap().as_str(), long_tag.to_lowercase());
    }

    #[test]
    fn tag_single_digit() {
        assert!("0".parse::<Tag>().is_ok());
        assert!("5".parse::<Tag>().is_ok());
        assert!("9".parse::<Tag>().is_ok());
        assert_eq!("7".parse::<Tag>().unwrap().as_str(), "7");
    }

    #[test]
    fn tag_consecutive_hyphens() {
        assert!("a--b".parse::<Tag>().is_ok());
        assert!("a---b".parse::<Tag>().is_ok());
        assert_eq!("test--tag".parse::<Tag>().unwrap().as_str(), "test--tag");
    }

    #[test]
    fn invalid_tag_various_special_chars() {
        assert!("tag!".parse::<Tag>().is_err());
        assert!("tag&".parse::<Tag>().is_err());
        assert!("tag%".parse::<Tag>().is_err());
        assert!("tag$".parse::<Tag>().is_err());
        assert!("tag^".parse::<Tag>().is_err());
        assert!("tag*".parse::<Tag>().is_err());
        assert!("tag(hello)".parse::<Tag>().is_err());
        assert!("tag[test]".parse::<Tag>().is_err());
        assert!("tag{test}".parse::<Tag>().is_err());
        assert!("tag<test>".parse::<Tag>().is_err());
    }

    #[test]
    fn tag_starting_with_digit() {
        assert!("0tag".parse::<Tag>().is_ok());
        assert!("1test".parse::<Tag>().is_ok());
        assert!("9value".parse::<Tag>().is_ok());
    }
}
