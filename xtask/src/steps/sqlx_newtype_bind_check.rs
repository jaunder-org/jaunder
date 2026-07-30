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
//!
//! **The hoisted form is policed too (#696).** Assigning the conversion to a local and
//! binding that — `let x = i64::from(y); … .bind(x)` — used to evade the scan entirely,
//! because it only ever looked at the text *after* `.bind(`. `violations` now also
//! tracks locals bound to a stripping conversion within the enclosing item, so the
//! evasion closes. Scope resets at a column-0 `}`.
//!
//! **What it still cannot see: a strip laundered through a function parameter.** If the
//! conversion happens in one function and the `.bind` in another, the binding function
//! sees only an `i64` argument and there is nothing left to detect. That is **#716**,
//! and two live instances are recorded there.
//!
//! **This gate does not conform to the "enumerate, don't search" decision**
//! (`docs/adr/0085-static-type-safety-gates-enumerate.md`, #715), on two counts, and
//! the laundering above is a symptom rather than the disease:
//!
//! - It decides a violation by *searching* for three strip spellings (`.as_ref()`,
//!   `&*`, `i64::from(`). A fourth spelling — including passing the value through a
//!   parameter — passes green, because the scan recognises nothing and treats that as
//!   clean. A conforming gate defines its population structurally and denies by
//!   default, so an unanticipated construct fails.
//! - Its [`ALLOWLIST`] is **region-scoped**: as the doc below says, a needle exempts
//!   every matching line under [`POLICED_ROOT`], not one site. A new violation that
//!   happens to match an existing needle is absorbed silently.
//!
//! `sqlx-newtype-decode` is the conforming sibling; rebuilding this one the same way —
//! deny bare-primitive binds unless an entry names the site and its multiplicity — is
//! **#716**'s scope.

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
/// **Nothing numeric is exempt.** #686 added two entries for forced `u32 → i64`
/// widenings (`i64::from(limit)`, `i64::from(offset.value())`); #696 gave those values
/// `i64`-backed newtypes with declared bounds, so the widenings — and their exemptions —
/// are gone. The numeric half of this rule is now absolute, not absolute-with-footnotes.
///
/// **A needle exempts every matching line under [`POLICED_ROOT`], not one site.** That
/// is what makes it reflow-proof, and it is also the cost: a needle like
/// `i64::from(limit)` would exempt any future *newtype*-typed local that happened to be
/// named `limit`. Keep needles specific enough that they cannot plausibly collide, and
/// prefer deleting an entry over broadening one.
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

/// The identifier a line binds a stripping conversion to, if any: the `x` in
/// `let x = i64::from(y);`.
///
/// Deliberately narrow. It matches the **hoist** shape only — a `let` whose
/// right-hand side is a conversion this gate already policies — because the point is to
/// close the "assign then bind" evasion, not to track every local. `let x: i64 = …`,
/// `as i64`, and `i64::try_from(…)` are **not** matched: the first two are not strips
/// this gate defines, and the third is laundered across a function boundary in the one
/// place it occurs, which no line scan can see (#716).
fn hoisted_strip_binding(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("let ")?;
    let (name, value) = rest.split_once('=')?;
    if !value.contains("i64::from(") {
        return None;
    }
    // `let mut x` / `let x: T` both reduce to the identifier.
    let name = name.trim().trim_start_matches("mut ").trim();
    let name = name.split(':').next()?.trim();
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(name)
}

/// Whether `line` binds `ident` — `.bind(ident)`, exactly, not merely containing it.
///
/// The closing paren matters: without it, `.bind(limit)` would match a hoisted
/// `limit_i64` by prefix and flag an unrelated bind.
fn binds_ident(line: &str, ident: &str) -> bool {
    line.contains(&format!(".bind({ident})"))
}

/// Whether `line` begins a function item — the boundary at which locals go out of scope.
///
/// Scoping on braces alone is not enough, and the failure is a **false positive**, which
/// is the expensive direction for a gate: methods inside an `impl` close at an indented
/// `}`, so a hoist in one method would still be "in scope" at the next method's
/// parameter bind. Every storage type is an `impl` block, so that is the common shape.
fn starts_a_function(line: &str) -> bool {
    let mut rest = line.trim_start();
    loop {
        let Some((word, tail)) = rest.split_once(' ') else {
            return false;
        };
        match word {
            "fn" => return true,
            "pub" | "async" | "const" | "unsafe" | "extern" | "default" => rest = tail.trim_start(),
            w if w.starts_with("pub(") => rest = tail.trim_start(),
            _ => return false,
        }
    }
}

/// 1-based line numbers of every bind that strips a newtype and is not allowlisted.
///
/// Two passes in one walk: the direct form (a conversion inside the `.bind(` itself),
/// and the **hoisted** form (a conversion assigned to a local earlier in the same
/// function, then bound by name). Hoisted names are dropped at a top-level `}` so a
/// strip in one function cannot taint an identically-named bind in the next.
///
/// Pure given the source, so it is unit-tested directly.
fn violations(source: &str) -> Vec<usize> {
    let mut hoisted: Vec<&str> = Vec::new();
    let mut out = Vec::new();

    for (i, line) in source.lines().enumerate() {
        // Locals do not outlive their function, so each `fn` starts a fresh scope. A
        // column-0 `}` clears too, for a free item that ends without another following.
        if starts_a_function(line) || line.starts_with('}') {
            hoisted.clear();
        }
        if let Some(name) = hoisted_strip_binding(line) {
            hoisted.push(name);
        }
        let hoisted_here = hoisted.iter().any(|name| binds_ident(line, name));
        if (strips_newtype_in_bind(line) || hoisted_here) && !is_allowed(line) {
            out.push(i + 1);
        }
    }
    out
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
                 `.as_ref()`/`&*`/`i64::from(…)`. This includes hoisting the conversion into \
                 a local and binding that (#696) — move the type, don't move the strip."
            ));
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.push(
        "  recovery: bind the newtype directly. If the value is genuinely not a newtype, prefer \
         giving it one — a `u32` that has to be widened at every bind (sqlx has no Postgres \
         `Encode` for unsigned types) is a type-design smell, and #696 removed the last two such \
         exemptions rather than keeping them. Only add an ALLOWLIST entry, with a documented \
         reason, when that is genuinely wrong. Currently exempt:"
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
    fn hoisted_i64_from_bind_is_flagged() {
        // The blind spot #696 found: hoisting the conversion into a local made the
        // strip invisible, because the scan only ever looked after `.bind(`. Both the
        // assignment and the bind are needed to flag — see the two tests below for the
        // halves that must NOT flag on their own.
        let src = "\
    let limit_i64 = i64::from(limit);
    sqlx::query(SQL)
        .bind(limit_i64)
";
        assert_eq!(violations(src), vec![3]);
    }

    #[test]
    fn hoisted_local_not_from_a_strip_is_clean() {
        // A local that was never a newtype is an ordinary value; binding it is fine.
        let src = "\
    let count = row.count;
        .bind(count)
";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn bind_of_an_unrelated_ident_is_clean() {
        // No assignment in scope at all — the bind names a parameter or field.
        let src = "        .bind(limit_i64)\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn a_hoist_does_not_taint_a_later_method_in_the_same_impl() {
        // The false-positive case that matters: methods close at an indented `}`, not a
        // column-0 one, so scoping on top-level braces alone would let a hoist in one
        // method flag a legitimate parameter bind in the next. Every storage type is an
        // `impl` block, so this is the common shape, not an exotic one.
        let src = "\
impl Store {
    async fn a(&self) {
        let limit_i64 = i64::from(limit);
        q.bind(limit_i64);
    }

    async fn b(&self, limit_i64: i64) {
        q.bind(limit_i64);
    }
}
";
        assert_eq!(violations(src), vec![4]);
    }

    #[test]
    fn a_hoist_does_not_taint_a_later_function() {
        // Scope resets at a top-level `}`: a strip hoisted in one fn must not make an
        // identically-named bind in the next fn a violation.
        let src = "\
fn a() {
    let limit_i64 = i64::from(limit);
    q.bind(limit_i64);
}
fn b(limit_i64: i64) {
    q.bind(limit_i64);
}
";
        assert_eq!(violations(src), vec![3]);
    }

    #[test]
    fn i64_from_outside_a_bind_is_ignored() {
        // The widening is only policed at a bind site; `i64::from` elsewhere (a format
        // argument, a comparison) is ordinary code.
        let src = "        let sql = format!(\"... {}\", i64::from(post_id));\n";
        assert!(violations(src).is_empty());
    }

    #[test]
    fn numeric_widenings_are_no_longer_exempt() {
        // #686 exempted these two as forced `u32 → i64` widenings. #696 gave `limit` and
        // the offset `i64`-backed newtypes with declared bounds, so the widenings are
        // gone and so are their entries — this asserts the exemption was actually
        // removed, not merely that no site trips it today.
        let src = "\
    .bind(i64::from(limit))
    .bind(i64::from(offset.value()))
";
        assert_eq!(violations(src), vec![1, 2]);
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
