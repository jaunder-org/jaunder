//! The `fetch-one-guard` check (#343): asserts the `clippy.toml` ban on
//! `fetch_one` is still configured, and still configured *loudly*.
//!
//! ## Why this exists when clippy already enforces the ban
//!
//! The ban itself is `disallowed-methods` in `clippy.toml`, and it works —
//! verified by spike before #343's design was approved: it flagged 33 of 33
//! call sites. The risk this step covers is not that clippy stops rejecting a
//! `fetch_one`; it is that the *configuration* quietly stops asking it to.
//!
//! There are two ways that could happen, and they need different answers:
//!
//! 1. **The paths stop resolving** — sqlx moves or renames them in an upgrade.
//!    This one is already self-announcing: clippy emits `does not refer to a
//!    reachable function` for an unresolvable entry, which is a hard error under
//!    the gate's `-D warnings`. Nothing here needs to duplicate that... *unless*
//!    someone silences it with `allow-invalid`, which is why that key is
//!    rejected below.
//! 2. **The entries are edited away** — a merge resolves badly, a cleanup pass
//!    trims "unused" config, a path is dropped while refactoring. Nothing else
//!    in the tree would notice: a missing entry does not fail, it simply stops
//!    guarding, and the ban would decay silently.
//!
//! So this step reads the configuration rather than re-testing clippy. Running
//! clippy against a planted fixture would be the stronger check, but it costs a
//! crate compile on every gate run to re-prove a mechanism the spike already
//! established, and it would not catch (2) any better than reading the file
//! does.

use crate::result::{CommandResult, StepResult};

/// Every `fetch_one` definition in sqlx 0.8, by the path clippy resolves.
///
/// The `sqlx::` facade paths are deliberately *not* used: sqlx re-exports these
/// types from `sqlx_core`, and a facade path resolves without matching any call
/// site — a guard that silently guards nothing. Three of these have callers in
/// the tree today; the rest are listed so a future use is caught rather than
/// admitted.
const REQUIRED_PATHS: &[&str] = &[
    "sqlx_core::query::Query::fetch_one",
    "sqlx_core::query::Map::fetch_one",
    "sqlx_core::query_as::QueryAs::fetch_one",
    "sqlx_core::query_scalar::QueryScalar::fetch_one",
    "sqlx_core::raw_sql::RawSql::fetch_one",
    "sqlx_core::executor::Executor::fetch_one",
];

/// The failure detail for a `clippy.toml` that no longer bans `fetch_one`
/// loudly, or `None` when it does. Pure given the file contents, so it is
/// unit-tested directly.
pub fn problems(clippy_toml: &str) -> Option<String> {
    // Comment lines are stripped first. `clippy.toml` documents *why* the ban is
    // shaped the way it is, including the warning not to set `allow-invalid` —
    // so a naive substring scan reports the prose as a violation. Stripping also
    // makes the path check stronger: a path named only in a comment is not a
    // ban, and should read as missing.
    let effective: String = clippy_toml
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let clippy_toml = effective.as_str();

    let mut lines = Vec::new();

    for path in REQUIRED_PATHS {
        if !clippy_toml.contains(path) {
            lines.push(format!(
                "clippy.toml: `disallowed-methods` no longer lists `{path}` — the \
                 `fetch_one` ban (#343) is incomplete. Absence must stay modelled as \
                 `fetch_optional` or `storage::error::fetch_exactly_one{{,_scalar}}`; \
                 restore the entry rather than deleting it."
            ));
        }
    }

    // `allow-invalid` suppresses clippy's `does not refer to a reachable
    // function` warning. That warning is the only thing that tells us a path
    // stopped resolving after an sqlx upgrade — with it silenced, the ban would
    // pass the gate while matching nothing.
    if clippy_toml.contains("allow-invalid") {
        lines.push(
            "clippy.toml: `allow-invalid` is set — that silences clippy's \
             `does not refer to a reachable function` warning, which is what makes an \
             unresolvable `fetch_one` path a hard failure instead of a guard that \
             quietly matches nothing (#343). Fix the path instead."
                .to_string(),
        );
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Read `clippy.toml` and push the result step. A missing or unreadable file is
/// a hard failure — the ban lives there, so losing the file loses the ban.
pub fn run(result: &mut CommandResult) {
    let step = match std::fs::read_to_string("clippy.toml") {
        Ok(source) => match problems(&source) {
            None => StepResult::ok("fetch-one-guard"),
            Some(detail) => StepResult::fail("fetch-one-guard").detail(detail),
        },
        Err(e) => {
            StepResult::fail("fetch-one-guard").detail(format!("cannot read clippy.toml: {e}"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real file must pass — this is the assertion that actually guards the
    /// guard; the fixtures below only prove the checker can fail.
    #[test]
    fn the_real_clippy_toml_bans_fetch_one() {
        let source = std::fs::read_to_string("../clippy.toml")
            .or_else(|_| std::fs::read_to_string("clippy.toml"))
            .expect("clippy.toml must be readable from the xtask test cwd");
        assert_eq!(
            problems(&source),
            None,
            "the repo's clippy.toml must ban every sqlx fetch_one path"
        );
    }

    fn full_config() -> String {
        REQUIRED_PATHS
            .iter()
            .map(|p| format!("{{ path = \"{p}\", reason = \"x\" }},\n"))
            .collect()
    }

    #[test]
    fn a_complete_config_has_no_problems() {
        assert_eq!(problems(&full_config()), None);
    }

    #[test]
    fn a_dropped_path_is_reported_by_name() {
        let partial: String = full_config()
            .lines()
            .filter(|l| !l.contains("QueryScalar"))
            .collect::<Vec<_>>()
            .join("\n");
        let detail = problems(&partial).expect("a missing path must fail the step");
        assert!(
            detail.contains("sqlx_core::query_scalar::QueryScalar::fetch_one"),
            "the report must name the dropped path, got: {detail}"
        );
    }

    #[test]
    fn an_empty_config_reports_every_path() {
        let detail = problems("allow-unwrap-in-tests = true\n").expect("an empty ban must fail");
        for path in REQUIRED_PATHS {
            assert!(
                detail.contains(path),
                "every missing path must be named: {path}"
            );
        }
    }

    /// The loudness property: silencing clippy's unresolvable-path warning is
    /// how this guard would stop guarding without anyone noticing.
    #[test]
    fn allow_invalid_is_rejected_even_with_every_path_present() {
        let with_allow = format!("{}\nallow-invalid = true\n", full_config());
        let detail = problems(&with_allow).expect("allow-invalid must fail the step");
        assert!(
            detail.contains("allow-invalid"),
            "the report must explain why allow-invalid defeats the guard, got: {detail}"
        );
    }

    /// The real `clippy.toml` documents the `allow-invalid` hazard in prose, so
    /// a substring scan would report its own warning as a violation. Pins the
    /// comment-stripping that stops that.
    #[test]
    fn a_commented_mention_is_not_a_setting() {
        let commented = format!("# do not set allow-invalid here\n{}", full_config());
        assert_eq!(
            problems(&commented),
            None,
            "prose about allow-invalid must not read as allow-invalid being set"
        );
    }

    /// The mirror of the above: a path that appears only in a comment is not a
    /// ban, so it must still report as missing.
    #[test]
    fn a_path_only_in_a_comment_still_reads_as_missing() {
        let commented_path = format!(
            "# sqlx_core::query_scalar::QueryScalar::fetch_one\n{}",
            full_config()
                .lines()
                .filter(|l| !l.contains("QueryScalar"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let detail = problems(&commented_path).expect("a commented-out path is not a ban");
        assert!(detail.contains("QueryScalar"));
    }
}
