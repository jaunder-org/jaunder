//! The `no-full-reload` static check (#592): forbids raw `window.location` navigation in
//! `web/src` + `client/src`, so the post-lifecycle flows stay client-side SPA navigation
//! (`leptos_router` `use_navigate()`). It is a host source-scan, not clippy
//! `disallowed-methods`, because these are wasm-target-gated call sites the default
//! `cargo xtask check` clippy pass never lints. No allowlist: after #592 there are zero
//! legitimate callers (the pre-paint `/`→`/app` redirect is a JS *string*, and leptos's
//! `use_location()` is a free fn — neither is a `.location()` call-chain).
//!
//! Accepted limitation (as in [`super::proffered_secret_check`]): matching is per-line,
//! so a chain split across lines by the formatter could evade it — a guardrail against
//! accidental reintroduction, not a determined adversary.

use std::path::Path;

use anyhow::{Context, Result};

use crate::files;
use crate::result::{CommandResult, StepResult};

/// Navigation methods on a `web_sys::Location` that trigger a full document load.
const NAV_METHODS: &[&str] = &[".replace(", ".assign(", ".reload(", ".set_href("];

/// Source roots scanned recursively for `.rs` files — every crate that can name a raw
/// `window().location()` navigation. `web` consumes browser APIs; `client` is their home.
const POLICED_ROOTS: &[&str] = &["web/src", "client/src"];

/// 1-based line numbers where a `.location()` receiver is navigated (`replace`/`assign`/
/// `reload`/`set_href` after the `.location()` on the same line). Comment lines are
/// skipped. `String::replace` is not flagged (no `.location()` on the line), nor is
/// leptos's `use_location()` (a free fn, not a `.location()` chain). Pure — unit-tested.
fn violations(source: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        if raw.trim_start().starts_with("//") {
            continue;
        }
        if let Some(loc) = raw.find(".location()")
            && NAV_METHODS.iter().any(|m| raw[loc..].contains(m))
        {
            out.push(i + 1);
        }
    }
    out
}

/// The failure detail for every offending line across the scanned files, or `None` when
/// clean. Pure given the `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for ln in violations(source) {
            lines.push(format!(
                "{path}:{ln}: raw `window.location` navigation is forbidden — use \
                 `leptos_router` `use_navigate()` for client-side router navigation (#592)"
            ));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn read_sources_with(
    paths: &[std::path::PathBuf],
    mut read: impl FnMut(&Path) -> std::io::Result<String>,
) -> Result<Vec<(String, String)>> {
    paths
        .iter()
        .map(|path| {
            read(path)
                .with_context(|| format!("reading {}", path.display()))
                .map(|source| (path.display().to_string(), source))
        })
        .collect::<Result<_>>()
}

/// Scan every Rust file under each of [`POLICED_ROOTS`] and push the result step. A
/// missing root is a hard failure, so a moved/renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        match files::with_extension(Path::new(root), "rs") {
            Ok(found) => files.extend(found),
            Err(e) => {
                result.push(
                    StepResult::fail("no-full-reload").detail(format!("cannot scan {root}: {e}")),
                );
                return;
            }
        }
    }
    let scanned = match read_sources_with(&files, |path| std::fs::read_to_string(path)) {
        Ok(scanned) => scanned,
        Err(error) => {
            result.push(
                StepResult::fail("no-full-reload")
                    .detail(format!("cannot read source population: {error:#}")),
            );
            return;
        }
    };
    let step = match problems(&scanned) {
        None => StepResult::ok("no-full-reload"),
        Some(detail) => StepResult::fail("no-full-reload").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{problems, read_sources_with, violations};

    #[test]
    fn flags_location_replace_assign_reload_set_href() {
        assert_eq!(
            violations("    window().location().replace(&url);\n"),
            vec![1]
        );
        assert_eq!(
            violations("    window().location().assign(&url);\n"),
            vec![1]
        );
        assert_eq!(
            violations("    window().location().set_href(&url);\n"),
            vec![1]
        );
        assert_eq!(
            violations("    let _ = window().location().reload();\n"),
            vec![1]
        );
    }

    #[test]
    fn ignores_string_replace_and_use_location() {
        // `String::replace` — no `.location()` on the line.
        assert!(violations(r#"    let s = json.replace("a", "b");"#).is_empty());
        // leptos_router `use_location()` — a free fn, not a `.location()` chain.
        assert!(violations("    let loc = use_location();\n").is_empty());
    }

    #[test]
    fn ignores_comment_lines() {
        assert!(violations("    // window().location().replace(x) is forbidden\n").is_empty());
    }

    #[test]
    fn problems_reports_path_line_and_recovery() {
        let detail = problems(&[(
            "web/src/x.rs".to_string(),
            "    window().location().replace(&url);\n".to_string(),
        )])
        .expect("a problem");

        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("use_navigate()"));
    }
    #[test]
    fn fail_closed_population_unreadable_source() {
        let path = std::path::PathBuf::from("web/src/unreadable.rs");
        let error = read_sources_with(&[path], |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected",
            ))
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("web/src/unreadable.rs"));
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(
            problems(&[(
                "web/src/x.rs".to_string(),
                "    navigate(&url, opts);\n".to_string()
            )]),
            None
        );
    }
}
