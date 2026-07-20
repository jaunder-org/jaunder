//! The `sqlx-newtype-bind` static check (#438): forbids the stringly
//! newtype-stripping idiom at `sqlx` bind sites in `storage/src`.
//!
//! The `StrNewtype` derive now emits an `sqlx::Encode`/`Type`/`Decode` bridge for
//! every string newtype, so `.bind(newtype)` binds the typed value directly. The
//! pre-bridge idiom stripped a newtype down to its `&str` before binding —
//! `.bind(x.as_ref())` (via `AsRef<str>`), `.bind(&*x)` / `.bind(&**x)` deref
//! binds, and the `Option` map-deref forms `.bind(x.map(|v| &*v))` / `&**v`. All
//! storage bind sites now bind the typed value — the derive newtypes via their
//! bridge, and `RenderedHtml` via its hand-written write-only bridge (#502) — so
//! this guard is the regression guard against the stringly idiom silently returning.
//!
//! The scan is line-based over every `.rs` under `storage/src`. A `.bind(` region
//! that contains `.as_ref()` or a `&*` deref is a violation, unless the whole line
//! matches an [`ALLOWLIST`] needle. `String::as_str` (e.g. `format.as_str()`) is a
//! genuine owned-`String` slice, not a newtype strip, and is **not** policed.

use std::path::{Path, PathBuf};

use crate::result::{CommandResult, StepResult};

/// A bind-expression exempt from the guard, matched by **substring** so it is
/// robust to reflow (rustfmt can move a bind across lines) — unlike a line number
/// or an inline `// allow` marker, which rustfmt can relocate.
struct Allowed {
    /// The bind-expression substring; a flagged line containing it is exempt.
    needle: &'static str,
    /// Why this bind legitimately keeps the `.as_ref()` idiom.
    reason: &'static str,
}

/// The two exempt bind-expressions; each appears in `posts.rs`, `sqlite/posts.rs`, and
/// `postgres/posts.rs`, and the substring match covers all three. (`RenderedHtml` was
/// also exempt until #502 gave it a sqlx `Encode` bridge and its binds became
/// `.bind(&input.rendered_html)`, so it is now policed like any newtype.)
const ALLOWLIST: &[Allowed] = &[
    Allowed {
        needle: "input.title.as_ref()",
        // `title` is `Option<PostTitle>`, so this is `Option::as_ref()` →
        // `Option<&PostTitle>` (a typed bind), NOT an `AsRef<str>` str-strip.
        reason: "Option<PostTitle>::as_ref() — a typed Option bind, not an AsRef<str> strip",
    },
    Allowed {
        needle: "input.summary.as_ref()",
        // `summary` is `Option<PostSummary>`, so this is `Option::as_ref()` →
        // `Option<&PostSummary>` (a typed bind), NOT an `AsRef<str>` str-strip.
        reason: "Option<PostSummary>::as_ref() — a typed Option bind, not an AsRef<str> strip",
    },
];

/// Source root scanned recursively for `.rs` files.
const POLICED_ROOT: &str = "storage/src";

/// Whether `line` is an exempt bind — it contains an [`ALLOWLIST`] needle.
fn is_allowed(line: &str) -> bool {
    ALLOWLIST.iter().any(|a| line.contains(a.needle))
}

/// Whether `line` strips a newtype to `&str` inside a `.bind(` argument: the region
/// after the first `.bind(` contains `.as_ref()` (an `AsRef<str>` strip) or a `&*`
/// deref (covering `&*`, `&**`, and the `Option` map-deref `&*v`/`&**v` forms).
///
/// `.as_str()` is deliberately not matched — it is `String::as_str` on a genuine
/// owned `String`, not a newtype. Pure, so it is unit-tested directly.
fn strips_newtype_in_bind(line: &str) -> bool {
    let Some(pos) = line.find(".bind(") else {
        return false;
    };
    let region = &line[pos + ".bind(".len()..];
    region.contains(".as_ref()") || region.contains("&*")
}

/// 1-based line numbers of every bind that strips a newtype and is not allowlisted.
/// Pure given the source, so it is unit-tested directly.
fn violations(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| strips_newtype_in_bind(line) && !is_allowed(line))
        .map(|(i, _)| i + 1)
        .collect()
}

/// The failure detail for every offending bind across the scanned files, or `None`
/// when every bind is typed or allowlisted. Pure given the `(path, source)` pairs,
/// so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for ln in violations(source) {
            let offending = source.lines().nth(ln - 1).unwrap_or("").trim();
            lines.push(format!(
                "{path}:{ln}: `{offending}` strips a string newtype to `&str` at a sqlx bind — \
                 the `StrNewtype` derive gives every newtype an `sqlx::Encode`, so bind the typed \
                 value directly (`.bind(x)` / `.bind(&x)`), not `.as_ref()`/`&*` (#438)"
            ));
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.push(
        "  recovery: bind the newtype directly. A genuinely non-newtype `String` bind must be \
         added to this gate's ALLOWLIST with a documented reason. Currently exempt:"
            .to_string(),
    );
    for a in ALLOWLIST {
        lines.push(format!("    - `{}`: {}", a.needle, a.reason));
    }
    Some(lines.join("\n"))
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

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step. A missing
/// root is a hard failure, so a moved/renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    if let Err(e) = rust_files(Path::new(POLICED_ROOT), &mut files) {
        result.push(
            StepResult::fail("sqlx-newtype-bind")
                .detail(format!("cannot scan {POLICED_ROOT}: {e}")),
        );
        return;
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
        None => StepResult::ok("sqlx-newtype-bind"),
        Some(detail) => StepResult::fail("sqlx-newtype-bind").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_binds_are_clean() {
        let src = "\
    .bind(slug)
    .bind(&code)
    .bind(now)
";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn as_str_on_owned_string_is_clean() {
        // `String::as_str` is a genuine owned-String slice, not a newtype strip.
        let src = "    .bind(date_str.as_str())\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn as_ref_strip_is_flagged() {
        // Proves the gate bites: a non-allowlisted newtype strip is a violation.
        let src = "    .bind(username.as_ref())\n";
        assert_eq!(violations(src), vec![1]);
    }

    #[test]
    fn deref_binds_are_flagged() {
        let src = "\
    .bind(&*value)
    .bind(&**value)
    .bind(display.map(|v| &*v))
";
        assert_eq!(violations(src), vec![1, 2, 3]);
    }

    #[test]
    fn allowlisted_title_is_clean() {
        let src = "    .bind(input.title.as_ref())\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn rendered_html_as_ref_bind_is_now_flagged() {
        // #502 retired `RenderedHtml`'s allowlist entry once it gained a sqlx `Encode`
        // bridge; the stringly `.as_ref()` bind must now be flagged like any newtype strip.
        assert_eq!(
            violations("    .bind(input.rendered_html.as_ref())\n"),
            vec![1]
        );
    }

    #[test]
    fn non_bind_deref_is_ignored() {
        // A `&*` outside any `.bind(` (e.g. a test `let posts = &*env...`) is fine.
        let src = "        let posts = &*env.state.posts;\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn problem_detail_names_file_line_and_recovery() {
        let detail = problems(&[(
            "storage/src/users.rs".to_string(),
            "    .bind(username.as_ref())\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("storage/src/users.rs:1"));
        assert!(detail.contains("username.as_ref()"));
        assert!(detail.contains("ALLOWLIST"));
    }

    #[test]
    fn clean_scan_reports_no_problems() {
        assert_eq!(
            problems(&[(
                "storage/src/posts.rs".to_string(),
                "    .bind(slug)\n    .bind(input.title.as_ref())\n".to_string(),
            )]),
            None
        );
    }
}
