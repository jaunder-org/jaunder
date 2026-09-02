//! Small shared text-normalization helpers.

use unicode_segmentation::UnicodeSegmentation;

/// Truncates `s` to the longest leading run of **whole grapheme clusters** whose total `cost`
/// stays within `budget`.
///
/// Cutting on cluster boundaries is the point: a byte- or `char`-wise cut can separate a base
/// character from its combining marks, corrupting Devanagari vowel signs, Arabic harakat, and
/// emoji ZWJ sequences into mojibake.
///
/// `cost` is the caller's because the budget's *unit* differs per use — scalar count for a
/// slug's character cap, percent-encoded byte length for a media filename's filesystem cap.
/// Shared so the two do not drift.
///
/// Returns an **empty string** when not even the first cluster fits. A caller that cannot use
/// an empty result must substitute something itself; this function does not guess.
#[must_use]
pub fn truncate_by_graphemes(s: &str, budget: usize, cost: impl Fn(&str) -> usize) -> String {
    let mut out = String::with_capacity(s.len().min(budget));
    let mut used = 0usize;
    for grapheme in s.graphemes(true) {
        let this = cost(grapheme);
        if used + this > budget {
            break;
        }
        out.push_str(grapheme);
        used += this;
    }
    out
}

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

/// Trims `s` and returns it when non-empty and no longer than `max_chars`
/// Unicode scalar values.
#[must_use]
pub(crate) fn bounded_non_empty(s: &str, max_chars: usize) -> Option<&str> {
    let trimmed = non_empty(s)?;
    trimmed.chars().nth(max_chars).is_none().then_some(trimmed)
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
    use super::{bounded_non_empty, non_empty, non_empty_owned};

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
    fn bounded_non_empty_counts_unicode_scalars_after_trimming() {
        assert_eq!(bounded_non_empty("  é界  ", 2), Some("é界"));
        assert_eq!(bounded_non_empty("é界x", 2), None);
    }

    #[test]
    fn non_empty_owned_mirrors_non_empty() {
        assert_eq!(non_empty_owned(String::new()), None);
        assert_eq!(non_empty_owned("   ".to_owned()), None);
        assert_eq!(non_empty_owned("  hi  ".to_owned()), Some("hi".to_owned()));
        assert_eq!(non_empty_owned("hi".to_owned()), Some("hi".to_owned()));
    }
}
