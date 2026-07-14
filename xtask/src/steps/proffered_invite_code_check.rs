//! The `proffered-invite-code` static check (#400): pins
//! `common::invite::ProfferedInviteCode` to `#[server]` function **parameter**
//! positions.
//!
//! `ProfferedInviteCode` is the serde-capable *inbound* half of the invite-code
//! type split (ADR-0063 inbound-secret variant): a client submits it, but a raw
//! capability token must never be sent server→client. Serde traits encode
//! *operations*, not a *direction*, so the type itself cannot express "inbound
//! only" — this guard enforces it structurally. In any policed source file, a
//! mention of the type is a violation unless it is an `use` import, a comment, or
//! sits in the parameter list of a `#[server]`-attributed `fn`. A return-type
//! position (`-> … ProfferedInviteCode`) is always a violation, even inside a
//! `#[server]` fn. The two owner files that define and convert the type are
//! exempt.

use std::path::{Path, PathBuf};

use crate::result::{CommandResult, StepResult};

/// The type this guard contains.
const TYPE_NAME: &str = "ProfferedInviteCode";

/// Source roots scanned recursively for `.rs` files.
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
];

/// The type's home (definition + serde trailer) and its conversion into the
/// domain `InviteCode` — the two places the type is *supposed* to appear outside a
/// `#[server]` parameter list, so they are exempt wholesale.
fn is_owner_file(path: &str) -> bool {
    path.ends_with("common/src/invite.rs") || path.ends_with("host/src/invite.rs")
}

/// Byte index of the first **whole-word** `ProfferedInviteCode` occurrence in
/// `line` — a match whose neighbours are not identifier characters, so a longer
/// identifier like `ProfferedInviteCodeList` is not matched. `None` when absent.
fn type_index(line: &str) -> Option<usize> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(TYPE_NAME).find_map(|(i, _)| {
        let before_ok = line[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        let after_ok = line[i + TYPE_NAME.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident(c));
        (before_ok && after_ok).then_some(i)
    })
}

/// 1-based line numbers of every whole-word `ProfferedInviteCode` mention that is
/// not an allowed occurrence: a `use` import, a comment, or a `#[server]` fn
/// parameter.
///
/// The scan tracks a small state machine — a `#[server]` attribute arms
/// `pending_server`; the next `fn` signature opens the parameter region, which
/// closes at the first `)`, `->`, or `{`. A mention that sits **after** a `->` on
/// its line is a return position (a parameter never does), so a single-line
/// `#[server]` fn that *returns* the type is still caught while one that takes it
/// as a *parameter* stays clean. Pure given the source, so it is unit-tested
/// directly.
///
/// Known limitation (accepted, #400): matching is on the literal type name, so a
/// deliberate `use …::ProfferedInviteCode as Alias;` rename evades the guard. It
/// is a guardrail against accidental leaks, not a determined adversary — an alias
/// rename is as visible in review as adding a file to an allowlist.
fn violations(source: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pending_server = false;
    let mut in_server_params = false;
    for (i, raw) in source.lines().enumerate() {
        let t = raw.trim();
        if t.starts_with("#[server") {
            pending_server = true;
        }
        if pending_server && t.contains("fn ") {
            pending_server = false;
            in_server_params = true;
        }
        if let Some(type_at) = type_index(raw) {
            if !t.starts_with("//") && !t.starts_with("use ") && !t.starts_with("pub use ") {
                // A return position — the type appears after this line's `->` — is
                // always a violation; otherwise a mention is a violation only outside
                // a server parameter region.
                let is_return = raw.find("->").is_some_and(|arrow| type_at > arrow);
                if is_return || !in_server_params {
                    out.push(i + 1);
                }
            }
        }
        if in_server_params && (t.contains(')') || t.contains("->") || t.ends_with('{')) {
            in_server_params = false;
        }
    }
    out
}

/// The failure detail for all offending mentions across the scanned files, or
/// `None` when every mention is allowed. Owner files are skipped. Pure given the
/// `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        if is_owner_file(path) {
            continue;
        }
        for ln in violations(source) {
            lines.push(format!(
                "{path}:{ln}: `ProfferedInviteCode` outside a #[server] fn parameter — a raw \
                 invite code must never be returned or stored where it can reach a client (#400)"
            ));
        }
    }
    if !lines.is_empty() {
        lines.push(
            "  recovery: `ProfferedInviteCode` may appear only as a #[server] fn parameter (convert \
             it to `host::invite::InviteCode` inside the boundary). It is defined and converted in \
             common/src/invite.rs and host/src/invite.rs; nowhere else may name it in a return \
             type, field, or binding."
                .to_string(),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Collect every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Scan every Rust file under each of [`POLICED_ROOTS`] and push the result step.
/// A missing root is a hard failure, so a moved/renamed tree can never quietly
/// disable the guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        if let Err(e) = rust_files(Path::new(root), &mut files) {
            result.push(
                StepResult::fail("proffered-invite-code")
                    .detail(format!("cannot scan {root}: {e}")),
            );
            return;
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
        None => StepResult::ok("proffered-invite-code"),
        Some(detail) => StepResult::fail("proffered-invite-code").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER_PARAM: &str = "\
#[server(endpoint = \"/register\")]
pub async fn register(
    username: Username,
    invite_code: Option<ProfferedInviteCode>,
) -> WebResult<String> {
    todo!()
}
";
    const SERVER_RETURN: &str = "\
#[server(endpoint = \"/mint\")]
pub async fn mint() -> WebResult<ProfferedInviteCode> {
    todo!()
}
";
    const STRUCT_FIELD: &str = "\
pub struct Dto {
    pub code: ProfferedInviteCode,
}
";
    const PLAIN_FN_PARAM: &str = "\
fn helper(code: ProfferedInviteCode) {}
";
    const IMPORT_AND_COMMENT: &str = "\
use common::invite::ProfferedInviteCode;
// `ProfferedInviteCode` is the inbound wire type.
";

    #[test]
    fn server_fn_parameter_is_clean() {
        assert!(violations(SERVER_PARAM).is_empty());
    }

    #[test]
    fn server_fn_return_is_flagged() {
        assert_eq!(violations(SERVER_RETURN), vec![2]);
    }

    #[test]
    fn struct_field_is_flagged() {
        assert_eq!(violations(STRUCT_FIELD), vec![2]);
    }

    #[test]
    fn non_server_fn_parameter_is_flagged() {
        // No `#[server]` attribute, so the parameter region never opens.
        assert_eq!(violations(PLAIN_FN_PARAM), vec![1]);
    }

    #[test]
    fn import_and_comment_are_clean() {
        assert!(violations(IMPORT_AND_COMMENT).is_empty());
    }

    #[test]
    fn single_line_server_fn_parameter_is_clean() {
        // The `->` is present but the type sits *before* it (a parameter), so the
        // return-position heuristic must not flag it.
        let src = "\
#[server(endpoint = \"/f\")]
pub async fn f(code: ProfferedInviteCode) -> WebResult<()> { todo!() }
";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn longer_identifier_is_not_matched() {
        // A distinct type whose name merely starts with ours must not be flagged.
        let src = "\
pub struct Dto {
    pub codes: ProfferedInviteCodeList,
}
";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn owner_files_are_exempt() {
        assert_eq!(
            problems(&[("host/src/invite.rs".to_string(), STRUCT_FIELD.to_string())]),
            None
        );
    }

    #[test]
    fn problem_detail_names_file_line_and_recovery() {
        let detail = problems(&[("web/src/x.rs".to_string(), SERVER_RETURN.to_string())])
            .expect("a problem");
        assert!(detail.contains("web/src/x.rs:2"));
        assert!(detail.contains("only as a #[server] fn parameter"));
    }

    #[test]
    fn clean_file_reports_no_problems() {
        assert_eq!(
            problems(&[("web/src/auth/mod.rs".to_string(), SERVER_PARAM.to_string())]),
            None
        );
    }
}
