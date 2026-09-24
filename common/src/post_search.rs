//! Shared normalization for the persisted Manage Posts search projection.

use unicode_casefold::UnicodeCaseFold;

use crate::post_title::PostTitle;
use crate::slug::Slug;

/// Derives the byte-identical search projection persisted by both storage backends.
#[must_use]
pub fn post_search_projection(title: Option<&PostTitle>, slug: &Slug) -> String {
    let authored = title.map_or_else(
        || slug.as_ref().to_owned(),
        |title| format!("{} {}", title.as_ref(), slug.as_ref()),
    );
    normalize_post_search_query(&authored)
}

/// Normalizes search input with Unicode default case folding and collapsed whitespace.
#[must_use]
pub fn normalize_post_search_query(input: &str) -> String {
    input
        .chars()
        .case_fold()
        .fold(
            (String::new(), true),
            |(mut output, in_space), character| {
                if character.is_whitespace() {
                    (output, true)
                } else {
                    if in_space && !output.is_empty() {
                        output.push(' ');
                    }
                    output.push(character);
                    (output, false)
                }
            },
        )
        .0
}

#[cfg(test)]
mod tests {
    use super::{normalize_post_search_query, post_search_projection};
    use crate::post_title::PostTitle;
    use crate::slug::Slug;

    #[test]
    fn projection_uses_unicode_case_folding_and_collapsed_whitespace() {
        // The persisted value and inbound query must agree on Unicode case folding and
        // whitespace so both database backends can use byte-identical substring matching.
        let title: PostTitle = "  Straße\u{a0}\nPost  ".parse().unwrap();
        let slug: Slug = "fallback-slug".parse().unwrap();

        assert_eq!(
            post_search_projection(Some(&title), &slug),
            "strasse post fallback-slug"
        );
        assert_eq!(
            normalize_post_search_query(" STRASSE\tPOST "),
            "strasse post"
        );
    }

    #[test]
    fn projection_always_contains_the_slug_for_untitled_posts() {
        // Textless Posts remain searchable and identifiable through their durable slug.
        let slug: Slug = "only-slug".parse().unwrap();

        assert_eq!(post_search_projection(None, &slug), "only-slug");
    }
}
