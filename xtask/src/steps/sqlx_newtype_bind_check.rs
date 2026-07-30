//! The `sqlx-newtype-bind` static check (#438, #686): forbids the
//! newtype-stripping idioms at `sqlx` bind sites in `storage/src`.
//!
//! All three newtype derives now emit an `sqlx::Encode`/`Type`/`Decode` bridge —
//! `StrNewtype` since #438, `IdNewtype` and `NumNewtype` since #686 — so
//! `.bind(newtype)` binds the typed value directly and every strip is dead weight
//! that also re-opens the transposition hazard the newtype exists to close
//! (ADR-0063 §2). Two idioms are policed:
//!
//! - **Stringly** (#438): `.bind(x.as_ref())` (via `AsRef<str>`), `.bind(&*x)` /
//!   `.bind(&**x)` deref binds, and the `Option` map-deref forms
//!   `.bind(x.map(|v| &*v))` / `&**v`.
//! - **Numeric** (#686): `.bind(i64::from(x))`, which unwrapped an id or bounded
//!   numeric to its inner `i64` before binding.
//!
//! The scan is line-based over every `.rs` under `storage/src`. A `.bind(` region
//! that contains `.as_ref()`, a `&*` deref, or `i64::from(` is a violation, unless
//! the whole line matches an [`ALLOWLIST`] needle. `String::as_str` (e.g.
//! `format.as_str()`) is a genuine owned-`String` slice, not a newtype strip, and
//! is **not** policed.

use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// A bind-expression exempt from the guard, matched by **substring** so it is
/// robust to reflow (rustfmt can move a bind across lines) — unlike a line number
/// or an inline `// allow` marker, which rustfmt can relocate.
struct Allowed {
    /// The bind-expression substring; a flagged line containing it is exempt.
    needle: &'static str,
    /// Why this bind legitimately keeps the stripping idiom.
    reason: &'static str,
}

/// The exempt bind-expressions. The `as_ref()` pair each appears in `posts.rs`,
/// `sqlite/posts.rs`, and `postgres/posts.rs`, and the substring match covers all
/// three. (`RenderedHtml` was also exempt until #502 gave it a sqlx `Encode` bridge
/// and its binds became `.bind(&input.rendered_html)`, so it is now policed like any
/// newtype.)
///
/// The `i64::from` pair is **not** a newtype strip at all: both arguments are `u32`
/// (`limit` is a bare `u32`; `PageOffset`'s declared `inner` is `u32`), and sqlx
/// implements no Postgres `Encode` for unsigned types, so the widening to `i64` is
/// forced. #696 owns the storage fetch-limit types; when it lands, these two entries
/// should go with it.
///
/// **A needle exempts every matching line under [`POLICED_ROOT`], not one site.** That
/// is what makes it reflow-proof, and it is also the cost: `i64::from(limit)` would
/// exempt a future *newtype*-typed local that happened to be named `limit`. Keep needles
/// specific enough that they cannot plausibly collide, and prefer deleting an entry over
/// broadening one.
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
    Allowed {
        needle: "i64::from(limit)",
        reason: "`limit` is a bare u32 — a forced u32→i64 widening (no Postgres \
                 unsigned Encode), not a newtype strip; owned by #696",
    },
    Allowed {
        needle: "i64::from(offset.value())",
        reason: "`PageOffset`'s inner is u32 — a forced u32→i64 widening (no Postgres \
                 unsigned Encode), not a newtype strip; owned by #696",
    },
];

/// Source root scanned recursively for `.rs` files.
const POLICED_ROOT: &str = "storage/src";

/// Whether `line` is an exempt bind — it contains an [`ALLOWLIST`] needle.
fn is_allowed(line: &str) -> bool {
    ALLOWLIST.iter().any(|a| line.contains(a.needle))
}

/// Whether `line` strips a newtype inside a `.bind(` argument: the region after the
/// first `.bind(` contains `.as_ref()` (an `AsRef<str>` strip), a `&*` deref
/// (covering `&*`, `&**`, and the `Option` map-deref `&*v`/`&**v` forms), or
/// `i64::from(` (an id/numeric strip to the inner `i64`).
///
/// `.as_str()` is deliberately not matched — it is `String::as_str` on a genuine
/// owned `String`, not a newtype. Pure, so it is unit-tested directly.
fn strips_newtype_in_bind(line: &str) -> bool {
    let Some(pos) = line.find(".bind(") else {
        return false;
    };
    let region = &line[pos + ".bind(".len()..];
    region.contains(".as_ref()") || region.contains("&*") || region.contains("i64::from(")
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
                "{path}:{ln}: `{offending}` strips a newtype at a sqlx bind — the `StrNewtype` \
                 (#438), `IdNewtype` and `NumNewtype` (#686) derives all emit an `sqlx::Encode`, \
                 so bind the typed value directly (`.bind(x)` / `.bind(&x)`), not \
                 `.as_ref()`/`&*`/`i64::from(…)`"
            ));
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.push(
        "  recovery: bind the newtype directly. A genuinely non-newtype bind — an owned `String`, \
         or a primitive widening sqlx forces (e.g. `u32` → `i64`, which has no Postgres `Encode`) \
         — must be added to this gate's ALLOWLIST with a documented reason. Currently exempt:"
            .to_string(),
    );
    for a in ALLOWLIST {
        lines.push(format!("    - `{}`: {}", a.needle, a.reason));
    }
    Some(lines.join("\n"))
}

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step. A missing
/// root is a hard failure, so a moved/renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let files = match files::with_extension(Path::new(POLICED_ROOT), "rs") {
        Ok(files) => files,
        Err(e) => {
            result.push(
                StepResult::fail("sqlx-newtype-bind")
                    .detail(format!("cannot scan {POLICED_ROOT}: {e}")),
            );
            return;
        }
    };
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
    fn i64_from_bind_is_flagged() {
        // Proves the numeric half bites (#686): an id or bounded numeric unwrapped to
        // its inner `i64` before binding is a violation, in every spelling the sweep
        // removed.
        let src = "\
    .bind(i64::from(user_id))
    .bind(i64::from(cursor.post_id))
    .bind(i64::from(*channel_id))
";
        assert_eq!(violations(src), vec![1, 2, 3]);
    }

    #[test]
    fn i64_from_outside_a_bind_is_ignored() {
        // The widening is only policed at a bind site; `i64::from` elsewhere (a format
        // argument, a comparison) is ordinary code.
        let src = "        let sql = format!(\"... {}\", i64::from(post_id));\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn allowlisted_primitive_widenings_are_clean() {
        // `limit`/`PageOffset` are `u32`, which sqlx cannot bind on Postgres, so the
        // widening is forced rather than a newtype strip (#696 owns removing it).
        let src = "\
    .bind(i64::from(limit))
    .bind(i64::from(offset.value()))
";
        assert!(violations(src).is_empty());
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
