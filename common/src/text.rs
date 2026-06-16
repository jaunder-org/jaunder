//! Small shared text-normalization helpers.

/// Trims `s` and returns the trimmed slice unless it is empty.
///
/// This is the single definition of the codebase's "blank input means absent"
/// rule for optional text fields: leading/trailing whitespace is stripped, and
/// an empty or whitespace-only value becomes `None`. Use it wherever optional
/// text should treat blank input as cleared (display names, summaries, slug
/// overrides, optional config values, …).
#[must_use]
pub fn non_empty(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Owned-`String` counterpart of [`non_empty`]: trims and returns the value,
/// or `None` when it is empty or whitespace-only. Convenient for
/// `Option<String>` pipelines via `opt.and_then(non_empty_owned)`.
#[must_use]
pub fn non_empty_owned(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.len() == s.len() {
        // No surrounding whitespace — reuse the existing allocation.
        Some(s)
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{non_empty, non_empty_owned};

    #[test]
    fn non_empty_returns_none_for_empty() {
        assert_eq!(non_empty(""), None);
    }

    #[test]
    fn non_empty_returns_none_for_whitespace_only() {
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty("\t \n"), None);
    }

    #[test]
    fn non_empty_trims_surrounding_whitespace() {
        assert_eq!(non_empty("  Alice  "), Some("Alice"));
    }

    #[test]
    fn non_empty_returns_value_when_unpadded() {
        assert_eq!(non_empty("Alice"), Some("Alice"));
    }

    #[test]
    fn non_empty_owned_mirrors_non_empty() {
        assert_eq!(non_empty_owned(String::new()), None);
        assert_eq!(non_empty_owned("   ".to_owned()), None);
        assert_eq!(non_empty_owned("  hi  ".to_owned()), Some("hi".to_owned()));
        assert_eq!(non_empty_owned("hi".to_owned()), Some("hi".to_owned()));
    }
}
