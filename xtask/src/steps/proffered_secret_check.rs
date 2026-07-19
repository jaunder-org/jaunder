//! The `proffered-secret` static check (#400, #315): pins each **inbound-secret**
//! newtype — `common::invite::ProfferedInviteCode` and
//! `common::password::ProfferedPassword` — to `#[server]` function **parameter**
//! positions.
//!
//! An inbound secret is the serde-capable *inbound* half of an ADR-0063
//! inbound-secret type split (`#[str_newtype(secret, serde)]`): a client submits
//! it, but the raw value (a capability token, a plaintext password) must never be
//! sent server→client. Serde traits encode *operations*, not a *direction*, so the
//! type itself cannot express "inbound only" — this guard enforces it structurally.
//! In any policed source file, a mention of such a type is a violation unless it is
//! a `use` import, a comment, or sits in the parameter list of a `#[server]`-
//! attributed `fn`. A return-type position (`-> … ProfferedPassword`) is always a
//! violation, even inside a `#[server]` fn. The owner files that define and convert
//! each type are exempt.

use std::path::{Path, PathBuf};

use crate::result::{CommandResult, StepResult};

/// An inbound-secret type the guard pins to `#[server]` parameter positions.
struct PolicedType {
    /// The type's identifier, matched whole-word.
    name: &'static str,
    /// The file(s) that define and convert the type — the places it is *supposed*
    /// to appear outside a `#[server]` parameter list, so they are exempt wholesale.
    owner_files: &'static [&'static str],
}

/// The inbound-secret types under guard. Each pairs a serde-capable inbound newtype
/// with a serde-free domain type it converts into (ADR-0063 inbound-secret variant).
const POLICED_TYPES: &[PolicedType] = &[
    PolicedType {
        name: "ProfferedInviteCode",
        owner_files: &["common/src/invite.rs", "host/src/invite.rs"],
    },
    PolicedType {
        name: "ProfferedPassword",
        owner_files: &["common/src/password.rs"],
    },
];

/// Source roots scanned recursively for `.rs` files. Must cover **every** crate
/// that can name an inbound-secret type — i.e. anything depending on `common`
/// (defines `ProfferedInviteCode` / `ProfferedPassword`) or `web` (where they are
/// consumed) — so a leak can't hide in an unscanned member. The client bundle
/// (`csr`) and the `test-support` binary are easy to overlook: both pull in
/// `common`/`web` and would otherwise be blind spots.
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "test-support/src",
];

/// Byte index of the first **whole-word** `type_name` occurrence in `line` — a match
/// whose neighbours are not identifier characters, so a longer identifier like
/// `ProfferedInviteCodeList` is not matched. `None` when absent.
fn type_index(line: &str, type_name: &str) -> Option<usize> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(type_name).find_map(|(i, _)| {
        let before_ok = line[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        let after_ok = line[i + type_name.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident(c));
        (before_ok && after_ok).then_some(i)
    })
}

/// 1-based line numbers of every whole-word `type_name` mention that is not an
/// allowed occurrence: a `use` import, a comment, or a `#[server]` fn parameter.
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
/// deliberate `use …::ProfferedPassword as Alias;` rename evades the guard. It is a
/// guardrail against accidental leaks, not a determined adversary — an alias rename
/// is as visible in review as adding a file to an allowlist.
fn violations(source: &str, type_name: &str) -> Vec<usize> {
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
        if let Some(type_at) = type_index(raw, type_name) {
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

/// The failure detail for all offending mentions of every [`POLICED_TYPES`] entry
/// across the scanned files, or `None` when every mention is allowed. Each type's
/// owner files are skipped for that type. Pure given the `(path, source)` pairs, so
/// it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for policed in POLICED_TYPES {
        let mut type_lines = Vec::new();
        for (path, source) in scanned {
            if policed
                .owner_files
                .iter()
                .any(|owner| path.ends_with(owner))
            {
                continue;
            }
            for ln in violations(source, policed.name) {
                type_lines.push(format!(
                    "{path}:{ln}: `{}` outside a #[server] fn parameter — an inbound secret must \
                     never be returned or stored where it can reach a client (ADR-0063)",
                    policed.name
                ));
            }
        }
        if !type_lines.is_empty() {
            type_lines.push(format!(
                "  recovery: `{}` may appear only as a #[server] fn parameter (convert it to its \
                 serde-free domain type inside the boundary). It is defined and converted in {}; \
                 nowhere else may name it in a return type, field, or binding.",
                policed.name,
                policed.owner_files.join(" + "),
            ));
            lines.extend(type_lines);
        }
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
                StepResult::fail("proffered-secret").detail(format!("cannot scan {root}: {e}")),
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
        None => StepResult::ok("proffered-secret"),
        Some(detail) => StepResult::fail("proffered-secret").detail(detail),
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
    password: ProfferedPassword,
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
    const PASSWORD_RETURN: &str = "\
#[server(endpoint = \"/echo\")]
pub async fn echo() -> WebResult<ProfferedPassword> {
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
        assert!(violations(SERVER_PARAM, "ProfferedInviteCode").is_empty());
        assert!(violations(SERVER_PARAM, "ProfferedPassword").is_empty());
    }

    #[test]
    fn server_fn_return_is_flagged() {
        assert_eq!(violations(SERVER_RETURN, "ProfferedInviteCode"), vec![2]);
        assert_eq!(violations(PASSWORD_RETURN, "ProfferedPassword"), vec![2]);
    }

    #[test]
    fn struct_field_is_flagged() {
        assert_eq!(violations(STRUCT_FIELD, "ProfferedInviteCode"), vec![2]);
    }

    #[test]
    fn non_server_fn_parameter_is_flagged() {
        // No `#[server]` attribute, so the parameter region never opens.
        assert_eq!(violations(PLAIN_FN_PARAM, "ProfferedInviteCode"), vec![1]);
    }

    #[test]
    fn import_and_comment_are_clean() {
        assert!(violations(IMPORT_AND_COMMENT, "ProfferedInviteCode").is_empty());
    }

    #[test]
    fn single_line_server_fn_parameter_is_clean() {
        // The `->` is present but the type sits *before* it (a parameter), so the
        // return-position heuristic must not flag it.
        let src = "\
#[server(endpoint = \"/f\")]
pub async fn f(code: ProfferedPassword) -> WebResult<()> { todo!() }
";
        assert!(violations(src, "ProfferedPassword").is_empty());
    }

    #[test]
    fn longer_identifier_is_not_matched() {
        // A distinct type whose name merely starts with ours must not be flagged.
        let src = "\
pub struct Dto {
    pub codes: ProfferedInviteCodeList,
}
";
        assert!(violations(src, "ProfferedInviteCode").is_empty());
    }

    #[test]
    fn owner_files_are_exempt() {
        assert_eq!(
            problems(&[("host/src/invite.rs".to_string(), STRUCT_FIELD.to_string())]),
            None
        );
        // `ProfferedPassword`'s owner file may name it in a return/field position.
        assert_eq!(
            problems(&[(
                "common/src/password.rs".to_string(),
                PASSWORD_RETURN.to_string()
            )]),
            None
        );
    }

    #[test]
    fn problem_detail_names_file_line_and_recovery() {
        let detail = problems(&[("web/src/x.rs".to_string(), PASSWORD_RETURN.to_string())])
            .expect("a problem");
        assert!(detail.contains("web/src/x.rs:2"));
        assert!(detail.contains("`ProfferedPassword`"));
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
