//! The `e2e-scaffold` static check (#792): forbids committing a non-empty `e2eSalt` in
//! `flake.nix`.
//!
//! The salt exists so an e2e measurement run can be built without nix serving a cached
//! suite result — see `docs/observability.md` §"#792 — the per-test warmup A/B". Left
//! set, it is **silent**: it changes every e2e derivation hash, so CI misses cache on
//! all four combos and rebuilds them from scratch. Nothing fails; CI just gets slower,
//! and "CI got slow" is not a symptom anyone traces back to a one-line diff.
//!
//! Deliberately a **host-side** xtask step, never wired into the e2e derivations: a
//! salted local `nix build`/`traces run` must keep working, since that is the whole
//! point of the salt. (`flake.nix` excludes `/xtask/` from its source filter, so a
//! derivation could not invoke this even by accident.)
//!
//! Accepted limitation: matching is per-line, so a reformatting that splits a literal across
//! lines would evade it. A guardrail against accidental reintroduction, not a determined
//! adversary — but note the missing-literal case below fails loudly rather than silently
//! passing.

use crate::result::{CommandResult, StepResult};

/// Where the scaffolding literals live.
const FLAKE: &str = "flake.nix";

/// The committed (safe) form of the literal, matched against a whitespace-trimmed line.
const SAFE_SALT: &str = r#"e2eSalt = "";"#;

/// Prefix identifying the declaration line, so a *changed* value is distinguishable from
/// an *absent* one. Comment lines are skipped by the caller, which is what keeps the
/// explanatory comment block beside the literal from matching.
const SALT_DECL: &str = "e2eSalt =";

/// The failure detail when the measurement salt is left set, or `None` when it is at its
/// committed default. Pure given `flake.nix`'s source, so it is unit-tested directly.
///
/// A literal that is **missing entirely** is a failure, not a pass: renaming or deleting it
/// must not quietly disable the guard.
pub fn problems(source: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut saw_salt = false;

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
    }

    if !saw_salt {
        lines.push(format!(
            "{FLAKE}: `e2eSalt` declaration not found — the guard cannot verify the #792 \
             measurement salt is unset. If the literal was renamed or removed, update this \
             check rather than leaving it matching nothing"
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

    /// The literal at its committed value, in the shape `flake.nix` actually uses.
    const CLEAN: &str = r#"
        # Cache-busting salt for e2e measurement runs (#792).
        e2eSalt = "";
    "#;

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(problems(CLEAN), None);
    }

    #[test]
    fn flags_non_empty_salt() {
        let detail = problems(&CLEAN.replace(r#"e2eSalt = "";"#, r#"e2eSalt = "run1";"#))
            .expect("a problem");
        assert!(detail.contains("`e2eSalt` is set"), "{detail}");
        assert!(detail.contains("#792"), "{detail}");
    }

    /// A renamed or deleted literal must fail loudly — a guard that matches nothing is
    /// indistinguishable from a guard that passes.
    #[test]
    fn missing_literal_fails_loudly() {
        let detail = problems("        vmMemory = 3072;\n").expect("a problem");
        assert!(
            detail.contains("`e2eSalt` declaration not found"),
            "{detail}"
        );
    }

    /// The explanatory comment block beside the literal names it, and a commented-out
    /// leftover must not be mistaken for a declaration.
    #[test]
    fn ignores_comment_mentions() {
        let commented = r#"
        # a committed non-empty `e2eSalt` costs CI its cache
        # e2eSalt = "leftover";
        e2eSalt = "";
        "#;
        assert_eq!(problems(commented), None);
    }
}
