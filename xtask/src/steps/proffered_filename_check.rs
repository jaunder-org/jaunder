//! The `proffered-filename-position` static check (#720): confines
//! `common::media::ProfferedFilename` to axum **extractor** positions.
//!
//! `ProfferedFilename` is the inbound twin of `Filename` for the three routes carrying a
//! filename path segment. Its `FromStr` *encodes* — axum has already percent-decoded the
//! segment — so it holds the canonical bytes and converts into `Filename` by a rewrap.
//! Because encoding is not idempotent, feeding it a value that is *already* canonical
//! double-encodes; the type therefore has exactly one legitimate source (a route segment)
//! and exactly one legitimate sink (the rewrap).
//!
//! Most of that is enforced structurally: the type carries no `Display`, `Serialize`,
//! `Deref` or `sqlx` bridge, so it cannot be rendered or stored. What privacy cannot do is
//! keep it out of ordinary signatures — it must be `pub` for the `server` crate's route
//! declarations to name it — which is what this guard covers.
//!
//! **The discriminator is bare-versus-wrapped, not field-versus-not.** The serve route's
//! legitimate position *is* a struct field:
//!
//! Illustration, not a test: `SoftPath`, `ProfferedFilename` and `Deserialize` are
//! `server`/`common` types, none of them in this crate's dependency graph.
//!
//! ```text
//! #[derive(Deserialize)]
//! pub struct ServeParams {
//!     pub filename: SoftPath<ProfferedFilename>,
//! }
//! ```
//!
//! so a "no struct fields" rule would be undecidable. A mention is allowed when it is
//! wrapped in `SoftPath<…>` or sits inside a `Path<(…)>` tuple; a *bare* mention in a
//! struct field, a `#[server]` parameter, a return type, or a plain fn parameter is a
//! violation.
//!
//! Accepted limitation (as in [`super::proffered_secret_check`]): matching is per-line, so
//! a deliberate `use …::ProfferedFilename as Alias;` rename evades the guard, and a
//! `Path<(…)>` split across lines would read as bare. Both are as visible in review as
//! adding a file to an allowlist. The real call sites are single-line and well under the
//! 100-column default.

use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// The guarded type's identifier, matched whole-word.
const POLICED_TYPE: &str = "ProfferedFilename";

/// The file that defines the type and its conversion — the one place it is *supposed* to
/// appear outside an extractor position, so it is exempt wholesale.
const OWNER_FILE: &str = "common/src/media.rs";

/// Source roots scanned recursively for `.rs` files. Mirrors
/// [`super::proffered_secret_check`]'s list: every crate that can name a `common` type, so
/// a leak cannot hide in an unscanned member.
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "test-support/src",
];

/// Byte index of the first **whole-word** `POLICED_TYPE` occurrence in `line`, or `None`.
///
/// Deliberately duplicated from [`super::proffered_secret_check::type_index`] rather than
/// extracted: consolidating would refactor an existing gate that #720 has no mandate to
/// touch, for nine lines. If a third guard ever needs it, extract then — as a decision, not
/// a discovery.
fn type_index(line: &str) -> Option<usize> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(POLICED_TYPE).find_map(|(i, _)| {
        let before_ok = line[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        let after_ok = line[i + POLICED_TYPE.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident(c));
        (before_ok && after_ok).then_some(i)
    })
}

/// Whether the mention at `at` is wrapped in an extractor — the only allowed shape.
///
/// Two forms, matching the two real call sites: `SoftPath<ProfferedFilename>` (the serve
/// route's `ServeParams` field) and a `Path<(…)>` tuple element (the `AtomPub` member
/// handlers).
fn is_wrapped(line: &str, at: usize) -> bool {
    let before = &line[..at];
    if before.ends_with("SoftPath<") {
        return true;
    }
    // Inside a `Path<(…)>` tuple: the *nearest* preceding `Path<(` must still be open at
    // the mention, i.e. no `)>` closes it in between. A bare `contains` would accept a bare
    // mention that merely shares a line with some unrelated extractor — e.g.
    // `fn f(Path((a,)): Path<(Username,)>, leaked: ProfferedFilename)`.
    let Some(open) = before.rfind("Path<(") else {
        return false;
    };
    !before[open..].contains(")>") && line[at..].contains(")>")
}

/// 1-based line numbers of every whole-word mention that is neither an allowed occurrence
/// (a `use` import or a comment) nor wrapped in an extractor. Pure given the source, so it
/// is unit-tested directly.
///
/// Imports are tracked as a small state machine rather than by line prefix, because a
/// braced import routinely wraps:
///
/// Input data to the scanner, not a test: it is a `&str` this fn is fed, and `common`
/// is not in this crate's dependency graph. The real assertions are in this file's
/// `tests` module, which passes exactly this shape through `violations`.
///
/// ```text
/// use common::media::{
///     detect_content_type, media_path, ContentHash, Filename, MediaSource,
///     ProfferedFilename,
/// };
/// ```
///
/// The mention sits on a *continuation* line, which starts with neither `use ` nor `//`. A
/// prefix-only rule reports it — as this guard did on its first run against the real tree,
/// which is how the gap was found.
fn violations(source: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut in_use = false;
    for (i, raw) in source.lines().enumerate() {
        let t = raw.trim();
        let opens_use = t.starts_with("use ") || t.starts_with("pub use ");
        if opens_use {
            // A single-line import ends on its own line; a braced one stays open until `;`.
            in_use = !t.ends_with(';');
        }
        let inside_import = in_use || opens_use;

        if let Some(at) = type_index(raw)
            && !inside_import
            && !t.starts_with("//")
            && !is_wrapped(raw, at)
        {
            out.push(i + 1);
        }

        if in_use && t.ends_with(';') {
            in_use = false;
        }
    }
    out
}

/// The failure detail for all offending mentions across the scanned files, or `None` when
/// every mention is allowed. The owner file is skipped. Pure given the `(path, source)`
/// pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        if path.ends_with(OWNER_FILE) {
            continue;
        }
        for ln in violations(source) {
            lines.push(format!(
                "{path}:{ln}: bare `{POLICED_TYPE}` outside an extractor position — the \
                 inbound twin must be rewrapped into `Filename` at the handler, never \
                 carried further (#720)"
            ));
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.push(format!(
        "  recovery: `{POLICED_TYPE}` may appear only wrapped as `SoftPath<{POLICED_TYPE}>` \
         or inside a `Path<(…)>` tuple. Convert it with `Filename::from(..)` as the first \
         statement of the handler and use `Filename` from there on; it is defined and \
         converted in {OWNER_FILE}, and nowhere else may name it in a field, parameter, or \
         return type. Encoding is not idempotent, so a second hop through this type \
         double-encodes."
    ));
    Some(lines.join("\n"))
}

/// Scan every Rust file under each of [`POLICED_ROOTS`] and push the result step. A missing
/// root is a hard failure, so a moved/renamed tree can never quietly disable the guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        match files::with_extension(Path::new(root), "rs") {
            Ok(found) => files.extend(found),
            Err(e) => {
                result.push(
                    StepResult::fail("proffered-filename-position")
                        .detail(format!("cannot scan {root}: {e}")),
                );
                return;
            }
        }
    }
    let scanned: Vec<(String, String)> = files
        .iter()
        .filter_map(|p| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| (p.display().to_string(), s))
        })
        .collect();
    let step = match problems(&scanned) {
        None => StepResult::ok("proffered-filename-position"),
        Some(detail) => StepResult::fail("proffered-filename-position").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOFT_PATH_FIELD: &str = r"
pub struct ServeParams {
    pub filename: SoftPath<ProfferedFilename>,
}
";

    const PATH_TUPLE: &str = r"
pub async fn member_get(
    Path((username, sha, filename)): Path<(Username, ContentHash, ProfferedFilename)>,
) -> Result<Response, HandlerError> { todo!() }
";

    const BARE_STRUCT_FIELD: &str = r"
pub struct MediaItem {
    pub filename: ProfferedFilename,
}
";

    const SERVER_PARAM: &str = r"
#[server]
pub async fn delete_media(filename: ProfferedFilename) -> WebResult<()> { todo!() }
";

    const BARE_RETURN: &str = r"
pub fn parse_it() -> ProfferedFilename { todo!() }
";

    const PLAIN_FN_PARAM: &str = r"
fn helper(filename: ProfferedFilename) {}
";

    const IMPORT_AND_COMMENT: &str = r"
use common::media::ProfferedFilename;
// `ProfferedFilename` is the inbound twin for URL path segments.
";

    /// The real shape in `server/src/media.rs` — the mention lands on a continuation line
    /// that starts with neither `use ` nor `//`. A prefix-only rule reports it, which is
    /// exactly what this guard did on its first run against the tree.
    const WRAPPED_IMPORT: &str = r"
use common::media::{
    detect_content_type, media_path, should_inline, ContentHash, Filename, MediaSource,
    ProfferedFilename,
};

pub struct ServeParams {
    pub filename: SoftPath<ProfferedFilename>,
}
";

    /// A braced import must not swallow the rest of the file: the `;` closes it, so a bare
    /// mention *after* one is still caught.
    const WRAPPED_IMPORT_THEN_LEAK: &str = r"
use common::media::{
    Filename,
    ProfferedFilename,
};

pub fn leak() -> ProfferedFilename {
    todo!()
}
";

    #[test]
    fn wrapped_extractor_positions_are_allowed() {
        assert!(violations(SOFT_PATH_FIELD).is_empty());
        assert!(violations(PATH_TUPLE).is_empty());
    }

    #[test]
    fn a_bare_struct_field_is_a_violation() {
        assert_eq!(violations(BARE_STRUCT_FIELD), vec![3]);
    }

    #[test]
    fn a_server_parameter_is_a_violation() {
        assert_eq!(violations(SERVER_PARAM), vec![3]);
    }

    #[test]
    fn a_return_position_is_a_violation() {
        assert_eq!(violations(BARE_RETURN), vec![2]);
    }

    #[test]
    fn a_plain_fn_parameter_is_a_violation() {
        assert_eq!(violations(PLAIN_FN_PARAM), vec![2]);
    }

    #[test]
    fn imports_and_comments_are_allowed() {
        assert!(violations(IMPORT_AND_COMMENT).is_empty());
    }

    #[test]
    fn a_mention_on_a_wrapped_import_continuation_line_is_allowed() {
        assert!(violations(WRAPPED_IMPORT).is_empty());
    }

    #[test]
    fn a_wrapped_import_does_not_swallow_a_later_leak() {
        assert_eq!(violations(WRAPPED_IMPORT_THEN_LEAK), vec![7]);
    }

    #[test]
    fn a_bare_mention_sharing_a_line_with_a_closed_extractor_is_a_violation() {
        // The hole a whole-line `contains("Path<(")` would leave: the tuple closes before
        // the mention, so this is a bare parameter riding alongside a legitimate extractor.
        let src = "\nfn f(Path((a,)): Path<(Username,)>, leaked: ProfferedFilename) {}\n";
        assert_eq!(violations(src), vec![2]);
    }

    #[test]
    fn a_longer_identifier_is_not_matched() {
        // Whole-word matching: `ProfferedFilenameList` is a different type.
        assert!(violations("pub struct X { f: ProfferedFilenameList }").is_empty());
    }

    #[test]
    fn problems_reports_the_offending_path_and_line() {
        let scanned = vec![(
            "web/src/media/api.rs".to_owned(),
            BARE_STRUCT_FIELD.to_owned(),
        )];
        let detail = problems(&scanned).expect("a violation");
        assert!(detail.contains("web/src/media/api.rs:3"), "{detail}");
        assert!(detail.contains("ProfferedFilename"), "{detail}");
        assert!(detail.contains("recovery:"), "{detail}");
    }

    #[test]
    fn the_owner_file_is_exempt() {
        let scanned = vec![("common/src/media.rs".to_owned(), BARE_RETURN.to_owned())];
        assert!(problems(&scanned).is_none());
    }

    #[test]
    fn a_clean_tree_reports_nothing() {
        let scanned = vec![
            ("server/src/media.rs".to_owned(), SOFT_PATH_FIELD.to_owned()),
            (
                "server/src/atompub/media.rs".to_owned(),
                PATH_TUPLE.to_owned(),
            ),
        ];
        assert!(problems(&scanned).is_none());
    }
}
