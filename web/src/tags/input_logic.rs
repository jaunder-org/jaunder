//! Pure, host-tested logic for the `TagInput` widget (ADR-0070 §6): tag-list
//! dedup, autocomplete keyboard navigation, and typed-tag parsing — extracted
//! out of the wasm-only `component` so it stays host-compiled and
//! coverage-measured. (There is no `#[component]` coverage exemption, #520 — a
//! wasm-only component simply never host-compiles, so extraction is the only
//! route to coverage.)

use common::seed::TagSummary;
use common::tag::TagLabel;

/// Append `tag` unless one with the same canonical slug is already present.
/// Dedup is on the slug, so the first-committed casing of a tag's `display` wins.
pub fn push_unique(tags: &mut Vec<TagSummary>, tag: TagSummary) {
    if !tags.iter().any(|existing| existing.slug == tag.slug) {
        tags.push(tag);
    }
}

/// Autocomplete selection after `ArrowDown`: the first row when nothing is
/// selected, otherwise the next row, clamped at the last. Left unchanged when the
/// suggestion list is empty.
#[must_use]
pub fn next_suggestion(selected: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return selected;
    }
    Some(selected.map_or(0, |n| (n + 1).min(len - 1)))
}

/// Autocomplete selection after `ArrowUp`: steps one row up, clearing the
/// selection when stepping past the first row. Left unchanged (`None`) when
/// nothing is selected.
#[must_use]
pub fn prev_suggestion(selected: Option<usize>) -> Option<usize> {
    selected.and_then(|n| n.checked_sub(1))
}

/// Parse a typed token into a committable [`TagSummary`], or an error message for
/// the inline `.j-tag-error`. Validation goes through `TagLabel::from_str` — the
/// single validity source shared with the server's arg-decode — which trims and
/// validates *without* lowercasing, so the author's casing is preserved in
/// `display` while `slug` is canonicalised (#416 Decision 4).
///
/// # Errors
/// Returns the `TagLabel` validation error as a display string when `raw` is not
/// a valid tag (empty, whitespace-only, or contains disallowed characters).
pub fn parse_committed_tag(raw: &str) -> Result<TagSummary, String> {
    match raw.parse::<TagLabel>() {
        Ok(label) => Ok(TagSummary {
            slug: label.slug(),
            display: label,
        }),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{next_suggestion, parse_committed_tag, prev_suggestion, push_unique};
    use common::seed::TagSummary;

    fn summary(display: &str) -> TagSummary {
        let label: common::tag::TagLabel = display.parse().unwrap();
        TagSummary {
            slug: label.slug(),
            display: label,
        }
    }

    #[test]
    fn push_unique_adds_new_and_skips_duplicate_slug() {
        let mut tags = vec![summary("Rust")];
        // Same canonical slug (different casing) is a duplicate — not added.
        push_unique(&mut tags, summary("rust"));
        assert_eq!(tags.len(), 1);
        // A distinct slug is appended.
        push_unique(&mut tags, summary("Leptos"));
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].display, "Rust");
        assert_eq!(tags[1].display, "Leptos");
    }

    #[test]
    fn next_suggestion_selects_first_then_advances_and_clamps() {
        assert_eq!(next_suggestion(None, 3), Some(0));
        assert_eq!(next_suggestion(Some(0), 3), Some(1));
        assert_eq!(next_suggestion(Some(2), 3), Some(2), "clamps at last row");
    }

    #[test]
    fn next_suggestion_leaves_selection_unchanged_when_empty() {
        assert_eq!(next_suggestion(None, 0), None);
        assert_eq!(next_suggestion(Some(1), 0), Some(1));
    }

    #[test]
    fn prev_suggestion_steps_up_and_clears_past_first() {
        assert_eq!(prev_suggestion(Some(2)), Some(1));
        assert_eq!(prev_suggestion(Some(0)), None, "clears past the first row");
        assert_eq!(prev_suggestion(None), None);
    }

    #[test]
    fn parse_committed_tag_preserves_casing_canonicalises_slug_and_trims() {
        let tag = parse_committed_tag("Rust").unwrap();
        assert_eq!(tag.slug, "rust");
        assert_eq!(tag.display, "Rust");

        let trimmed = parse_committed_tag(" ab ").unwrap();
        assert_eq!(trimmed.display, "ab");
    }

    #[test]
    fn parse_committed_tag_rejects_invalid_token() {
        assert!(parse_committed_tag("bad tag").is_err());
    }
}
