//! The `e2e-scaffold` static check (#792): forbids committing the e2e measurement
//! scaffolding in `flake.nix` — a non-empty `e2eSalt`, or `e2eWarmup` set to anything
//! but `true`.
//!
//! Both literals exist so an A/B measurement run can be built at gate-identical
//! settings (see `docs/superpowers/specs/2026-08-04-issue-792-e2e-warmup.md`), and both
//! are **silent** when left set — which is exactly why they need a guard:
//!
//! - a non-empty `e2eSalt` changes every e2e derivation hash, so CI misses cache on all
//!   four combos and rebuilds them from scratch. Nothing fails; CI just gets slower, and
//!   "CI got slow" is not a symptom anyone traces back to a one-line diff.
//! - `e2eWarmup = false` disables the per-test warmup on all four gate checks. Nothing
//!   fails there either — the gate simply stops testing what it claims to test.
//!
//! Deliberately a **host-side** xtask step, never wired into the e2e derivations: a
//! salted local `nix build`/`traces run` must keep working, since that is the whole
//! point of the salt. (`flake.nix` excludes `/xtask/` from its source filter, so a
//! derivation could not invoke this even by accident.)
//!
//! Accepted limitation (as in [`super::no_full_reload_check`]): matching is per-line, so
//! a reformatting that splits a literal across lines would evade it. A guardrail against
//! accidental reintroduction, not a determined adversary — but note the missing-literal
//! case below fails loudly rather than silently passing.

use crate::result::{CommandResult, StepResult};

/// Where the scaffolding literals live.
const FLAKE: &str = "flake.nix";

/// The committed (safe) form of each literal, matched against a whitespace-trimmed line.
const SAFE_SALT: &str = r#"e2eSalt = "";"#;
const SAFE_WARMUP: &str = "e2eWarmup = true;";

/// Prefixes identifying a declaration line for each literal, so a *changed* value is
/// distinguishable from an *absent* one. Comment lines are skipped by the caller, which
/// is what keeps the explanatory comment block next to the literals from matching.
const SALT_DECL: &str = "e2eSalt =";
const WARMUP_DECL: &str = "e2eWarmup =";

/// The failure detail when #792's measurement scaffolding is left set, or `None` when
/// both literals are present at their committed defaults. Pure given `flake.nix`'s
/// source, so it is unit-tested directly.
///
/// A literal that is **missing entirely** is a failure, not a pass: renaming or deleting
/// one must not quietly disable the guard (same reasoning as
/// [`super::no_full_reload_check`]'s missing-root hard failure).
pub fn problems(source: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut saw_salt = false;
    let mut saw_warmup = false;

    for (i, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with(SALT_DECL) {
            saw_salt = true;
            if line != SAFE_SALT {
                lines.push(format!(
                    "{FLAKE}:{}: `e2eSalt` is set — revert it to `\"\"` before committing. \
                     A non-empty salt changes every e2e derivation hash, so CI rebuilds all \
                     four combos from scratch with no cache hit and nothing fails loudly (#792)",
                    i + 1
                ));
            }
        }
        if line.starts_with(WARMUP_DECL) {
            saw_warmup = true;
            if line != SAFE_WARMUP {
                lines.push(format!(
                    "{FLAKE}:{}: `e2eWarmup` is not `true` — restore it before committing. \
                     It disables the per-test warmup on all four gate checks, silently \
                     changing what the gate tests (#792)",
                    i + 1
                ));
            }
        }
    }

    if !saw_salt {
        lines.push(format!(
            "{FLAKE}: `e2eSalt` declaration not found — the guard cannot verify #792's \
             measurement scaffolding is unset. If the literal was renamed or removed, update \
             this check rather than leaving it matching nothing"
        ));
    }
    if !saw_warmup {
        lines.push(format!(
            "{FLAKE}: `e2eWarmup` declaration not found — see the `e2eSalt` note above (#792)"
        ));
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Read `flake.nix` and push the result step. An unreadable `flake.nix` is a hard
/// failure, so a moved/renamed file can never quietly disable the guard.
pub fn run(result: &mut CommandResult) {
    let step = match std::fs::read_to_string(FLAKE) {
        Err(e) => StepResult::fail("e2e-scaffold").detail(format!("cannot read {FLAKE}: {e}")),
        Ok(source) => match problems(&source) {
            None => StepResult::ok("e2e-scaffold"),
            Some(detail) => StepResult::fail("e2e-scaffold").detail(detail),
        },
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::problems;

    /// Both literals at their committed values, in the shape `flake.nix` actually uses.
    const CLEAN: &str = r#"
        # Cache-busting salt for e2e measurement runs (#792).
        e2eSalt = "";

        # A/B scaffolding for #792.
        e2eWarmup = true;
    "#;

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(problems(CLEAN), None);
    }

    #[test]
    fn flags_non_empty_salt() {
        let detail = problems(&CLEAN.replace(r#"e2eSalt = "";"#, r#"e2eSalt = "run1";"#))
            .expect("a problem");
        assert!(detail.contains("e2eSalt"), "{detail}");
        assert!(detail.contains("#792"), "{detail}");
        assert!(!detail.contains("e2eWarmup"), "{detail}");
    }

    #[test]
    fn flags_disabled_warmup() {
        let detail =
            problems(&CLEAN.replace("e2eWarmup = true;", "e2eWarmup = false;")).expect("a problem");
        assert!(detail.contains("e2eWarmup"), "{detail}");
        assert!(!detail.contains("`e2eSalt` is set"), "{detail}");
    }

    #[test]
    fn flags_both_when_both_set() {
        let both = CLEAN
            .replace(r#"e2eSalt = "";"#, r#"e2eSalt = "run1";"#)
            .replace("e2eWarmup = true;", "e2eWarmup = false;");
        let detail = problems(&both).expect("a problem");
        assert!(detail.contains("`e2eSalt` is set"), "{detail}");
        assert!(detail.contains("`e2eWarmup` is not `true`"), "{detail}");
    }

    /// A renamed or deleted literal must fail loudly — a guard that matches nothing is
    /// indistinguishable from a guard that passes.
    #[test]
    fn missing_literals_fail_loudly() {
        let detail = problems("        vmMemory = 3072;\n").expect("a problem");
        assert!(
            detail.contains("`e2eSalt` declaration not found"),
            "{detail}"
        );
        assert!(
            detail.contains("`e2eWarmup` declaration not found"),
            "{detail}"
        );
    }

    /// The explanatory comment block beside the literals mentions both by name and must
    /// not be mistaken for a declaration.
    #[test]
    fn ignores_comment_mentions() {
        let commented = r#"
        # a committed `e2eWarmup = false` silently disables warmup
        # e2eSalt = "leftover";
        e2eSalt = "";
        e2eWarmup = true;
        "#;
        assert_eq!(problems(commented), None);
    }
}
