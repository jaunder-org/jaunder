use std::{fmt, str::FromStr};

use thiserror::Error;

/// A validated post slug matching `[a-z0-9][a-z0-9-]*`.
///
/// Constructed via [`FromStr`]; invalid strings are rejected at the boundary
/// so interior code works only with already-valid slugs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slug(String);

/// Error returned when a string cannot be parsed as a [`Slug`].
#[derive(Debug, Error)]
#[error("slug must be non-empty and match [a-z0-9][a-z0-9-]*")]
pub struct InvalidSlug;

impl FromStr for Slug {
    type Err = InvalidSlug;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidSlug);
        }
        let mut chars = s.chars();
        // First character must be alphanumeric (lowercase)
        let first = chars.next().ok_or(InvalidSlug)?;
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err(InvalidSlug);
        }
        // Remaining characters: lowercase alphanumeric or hyphen
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(InvalidSlug);
        }
        Ok(Slug(s.to_owned()))
    }
}

impl Slug {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Converts a title to a slug candidate by lowercasing ASCII alphanumeric
/// characters and collapsing runs of non-alphanumeric characters into hyphens.
///
/// Returns `None` if the title contains no ASCII alphanumeric characters.
pub fn slugify_title(title: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_dash = false;
        } else if !slug.is_empty() && !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_parses_valid_values() {
        assert!("hello-world".parse::<Slug>().is_ok());
        assert!("abc123".parse::<Slug>().is_ok());
        assert!("a".parse::<Slug>().is_ok());
        assert!("0".parse::<Slug>().is_ok());
        assert!("my-post-2024".parse::<Slug>().is_ok());
    }

    #[test]
    fn slug_rejects_invalid_values() {
        // empty
        assert!("".parse::<Slug>().is_err());
        // uppercase
        assert!("Hello".parse::<Slug>().is_err());
        // starts with hyphen
        assert!("-hello".parse::<Slug>().is_err());
        // spaces
        assert!("hello world".parse::<Slug>().is_err());
        // underscore
        assert!("hello_world".parse::<Slug>().is_err());
        // special chars
        assert!("hello@world".parse::<Slug>().is_err());
    }

    #[test]
    fn slug_display_returns_inner_string() {
        let s: Slug = "my-post".parse().unwrap();
        assert_eq!(s.to_string(), "my-post");
        assert_eq!(s.as_str(), "my-post");
    }

    #[test]
    fn slugify_title_lowercases_and_separates_words() {
        assert_eq!(
            slugify_title("Hello, World from Rust"),
            Some("hello-world-from-rust".to_string())
        );
    }

    #[test]
    fn slugify_title_trims_non_alphanumeric_boundaries() {
        assert_eq!(slugify_title("  ---Hello!!!  "), Some("hello".to_string()));
    }

    #[test]
    fn slugify_title_rejects_titles_without_ascii_alphanumerics() {
        assert_eq!(slugify_title("!!!"), None);
        assert_eq!(slugify_title("—"), None);
    }

    #[test]
    fn slugify_title_single_word() {
        assert_eq!(slugify_title("Rust"), Some("rust".to_string()));
    }
}
