//! Deterministic ADR content transformations.

use anyhow::{Result, bail};

use crate::adr_readme;
use crate::doc_links;

/// Four-digit zero-padded number, e.g. `34 -> "0034"`.
pub(super) fn pad(n: u32) -> String {
    format!("{n:04}")
}

/// Replace every occurrence of `old_stem` with `new_stem`.
pub(super) fn rewrite_stem(content: &str, old_stem: &str, new_stem: &str) -> String {
    content.replace(old_stem, new_stem)
}

/// Rewrite every inline link target in `body`, removing one leading `../`.
///
/// A draft moves up exactly one directory at promotion (`docs/adr/drafts/x.md` ->
/// `docs/adr/NNNN-x.md`), so each of its relative targets is off by exactly one
/// level. Stating that invariant directly covers more than a sibling-ADR-specific
/// rewrite would — `../template.md` is the shape `drafts/README.md` models for
/// authors, and it breaks the same way.
///
/// Only targets inside `](...)` are touched, and only outside code spans and fenced
/// blocks: a draft may legitimately discuss `../` in prose or show it in a shell
/// snippet, and a blanket string replace would corrupt those. Targets that cannot
/// lose a level (`..`, `../`, a bare name, a non-initial `../`) and non-relative
/// targets (URLs, anchors) are left alone.
pub(super) fn strip_one_level(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut cursor = 0;
    for link in doc_links::links_in(body) {
        if !doc_links::is_relative_target(&link.target) {
            continue;
        }
        // `..` has no prefix to strip; `../` would strip to nothing.
        let Some(rest) = link.target.strip_prefix("../") else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        out.push_str(&body[cursor..link.span.start]);
        out.push_str(rest);
        cursor = link.span.end;
    }
    out.push_str(&body[cursor..]);
    out
}

/// Rewrite a `proposed` status token to `accepted` — the acceptance event that
/// promotion *is*, finally written down. `None` when there is no status line, or
/// when its token is anything else.
///
/// Only `proposed` moves. `superseded`, `rejected` and `deprecated` on a draft are
/// deliberate authorial statements — an ADR written to record a reversal, or to
/// document a decision already dead — and promotion must not overwrite an author's
/// explicit claim with a default. The guard is whole-remainder equality, so a
/// multi-token status (`proposed (pending #742)`) is left alone too: it is
/// malformed, and `adr-format` should say so on a stable tree rather than have
/// promotion half-fix it.
///
/// The edit is confined to the status line's byte span, taken from the one shared
/// [`adr_readme::status_line`] parse, so prose elsewhere in the draft that happens
/// to contain the word "proposed" is untouched.
pub(super) fn accept_proposed_status(body: &str) -> Option<String> {
    let status = adr_readme::status_line(body)?;
    if status.rest != adr_readme::PROPOSED {
        return None;
    }
    let line = &body[status.span.clone()];
    Some(format!(
        "{}{}{}",
        &body[..status.span.start],
        line.replacen(adr_readme::PROPOSED, adr_readme::ACCEPTED, 1),
        &body[status.span.end..]
    ))
}

/// Promote the required first-line draft heading token and leave the body intact.
///
/// This deliberately does not use a whole-body replacement: ADRs about the draft
/// workflow may discuss the literal `ADR-DRAFT` token in prose or code spans.
pub(super) fn promote_heading(body: &str, number: u32, draft_rel: &str) -> Result<String> {
    let required = "# ADR-DRAFT: ";
    let Some(rest) = body.strip_prefix(required) else {
        bail!("{draft_rel} must start with `{required}` and a non-empty title");
    };
    let title = rest.split_once('\n').map_or(rest, |(line, _)| line);
    if title.trim().is_empty() {
        bail!("{draft_rel} must start with `{required}` and a non-empty title");
    }
    Ok(format!("# ADR-{}: {rest}", pad(number)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_is_four_digits() {
        assert_eq!(pad(34), "0034");
        assert_eq!(pad(5), "0005");
    }

    #[test]
    fn rewrite_stem_replaces_path_form_refs() {
        let content = "See [the ADR](docs/adr/0034-bar.md) and 0034-bar.md again.";
        let out = rewrite_stem(content, "0034-bar", "0035-bar");
        assert_eq!(
            out,
            "See [the ADR](docs/adr/0035-bar.md) and 0035-bar.md again."
        );
    }

    #[test]
    fn strip_one_level_drops_a_single_leading_parent() {
        assert_eq!(strip_one_level("[x](../0001-foo.md)"), "[x](0001-foo.md)");
    }

    #[test]
    fn strip_one_level_drops_only_one_of_two() {
        assert_eq!(
            strip_one_level("[x](../../CONTRIBUTING.md)"),
            "[x](../CONTRIBUTING.md)"
        );
    }

    #[test]
    fn strip_one_level_leaves_bare_targets_alone() {
        assert_eq!(strip_one_level("[x](template.md)"), "[x](template.md)");
    }

    #[test]
    fn strip_one_level_leaves_dot_dot_edge_cases_alone() {
        assert_eq!(strip_one_level("[x](..)"), "[x](..)");
        assert_eq!(strip_one_level("[x](../)"), "[x](../)");
        assert_eq!(strip_one_level("[x](a/../b.md)"), "[x](a/../b.md)");
    }

    #[test]
    fn strip_one_level_ignores_urls_and_anchors() {
        let body = "[x](https://e.com/../a) [y](#s)";
        assert_eq!(strip_one_level(body), body);
    }

    #[test]
    fn strip_one_level_spares_links_inside_code() {
        // Real `](...)` links, so this fails against any implementation that
        // rewrites targets without honouring the code carve-out.
        let body = "prose ../foo\n\n```\n[a](../x.md)\n```\n\n`[b](../y.md)`\n";
        assert_eq!(strip_one_level(body), body);
    }

    #[test]
    fn strip_one_level_rewrites_every_link_in_one_pass() {
        assert_eq!(
            strip_one_level("[a](../x.md) and [b](../y.md)"),
            "[a](x.md) and [b](y.md)"
        );
    }

    #[test]
    fn promote_heading_rewrites_only_the_required_token() {
        let body = "# ADR-DRAFT: Title\n\nThe ADR-DRAFT token remains in prose.\n";
        assert_eq!(
            promote_heading(body, 7, "docs/adr/drafts/title.md").unwrap(),
            "# ADR-0007: Title\n\nThe ADR-DRAFT token remains in prose.\n"
        );
    }

    #[test]
    fn promote_heading_rejects_a_missing_or_empty_title() {
        for body in ["# Different: Title\n", "# ADR-DRAFT:   \n"] {
            assert!(promote_heading(body, 7, "docs/adr/drafts/title.md").is_err());
        }
    }

    #[test]
    fn accept_proposed_status_rewrites_only_the_status_token() {
        let body = "# ADR-DRAFT: Title\n\n- Status: proposed\n\nThe proposal was proposed here.\n";
        assert_eq!(
            accept_proposed_status(body).unwrap(),
            "# ADR-DRAFT: Title\n\n- Status: accepted\n\nThe proposal was proposed here.\n"
        );
    }

    #[test]
    fn accept_proposed_status_preserves_a_deliberate_status() {
        let body = "# ADR-DRAFT: Title\n\n- Status: rejected\n";
        assert_eq!(accept_proposed_status(body), None);
    }
}
