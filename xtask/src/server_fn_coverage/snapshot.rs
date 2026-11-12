//! The committed coverage snapshot and the verdict that turns it into build
//! failures (#681).
//!
//! [`Snapshot`] is the sole durable generated artifact. It carries only the
//! reproducible covered-function and orphan-reason sets, so regeneration is
//! byte-identical when those observable facts are unchanged. Per-test titles
//! remain in [`Coverage`] while traces are classified, because attribution
//! distinguishes browser traffic driven by a test from orphan traffic, but
//! those timing-dependent titles are discarded at the snapshot boundary.
//!
//! **No provenance fields.** The snapshot records coverage and nothing else — no
//! commit, no timestamp. A recorded commit is necessarily an ancestor of the
//! commit under test, so with fail-on-any-difference the gate would be red
//! forever. Every collection is `BTreeMap`/`BTreeSet` and rendering is stable,
//! so an unchanged run is byte-identical and drift comparison can be total.

use anyhow::{Context, Result};

use super::Coverage;
pub use repository_policy::source_inventory::{REGENERATE_CMD, Snapshot};

impl Coverage {
    /// Project trace-derived coverage into the sole committed artifact.
    ///
    /// `covered` remains a title-attributed map until this boundary because
    /// extraction needs that attribution to separate covered requests from
    /// orphan traffic. Only its keys are durable.
    pub fn into_snapshot(self) -> Snapshot {
        Snapshot {
            covered: self.covered.into_keys().collect(),
            orphans: self.orphans,
        }
    }
}

/// Render the coverage snapshot to the exact bytes committed: stably-ordered,
/// pretty-printed JSON with a trailing newline.
///
/// Fallible rather than lossy. A serialization failure must not fall back to an
/// empty `{"covered":[],"orphans":{}}` — `regenerate` would *write* it, the
/// committed snapshot would say nothing is covered, and the next `verify` would
/// agree. That is the exact false verdict this gate exists to prevent, so the
/// error propagates.
pub fn render(snapshot: &Snapshot) -> Result<String> {
    // Every collection is a `BTreeMap`/`BTreeSet`, so it serializes in key order,
    // and `to_string_pretty` is itself deterministic — equal values render
    // byte-identically without anyone remembering to sort.
    let mut out = serde_json::to_string_pretty(snapshot)
        .context("serializing the server-fn coverage snapshot")?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::{BTreeMap, BTreeSet};

    /// A [`Coverage`] over `(qualified name, test titles)` pairs.
    fn coverage_of(pairs: &[(&str, &[&str])]) -> Coverage {
        Coverage {
            covered: pairs
                .iter()
                .map(|(k, ts)| (k.to_string(), ts.iter().map(|t| t.to_string()).collect()))
                .collect(),
            orphans: BTreeMap::new(),
        }
    }

    /// `^[a-z_][a-z0-9_]*::[a-z0-9_]+$` — hand-rolled because `xtask` has no
    /// `regex` dependency and one shape assertion does not justify adding one.
    fn is_qualified(name: &str) -> bool {
        let Some((vertical, ident)) = name.split_once("::") else {
            return false;
        };
        let word = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        word(vertical)
            && word(ident)
            && vertical.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
    }

    // Real `regenerate` inputs from three distinct executions of the same e2e
    // derivation on one tree. The covered keys and orphan reasons agree while
    // internal title attribution differs.
    const RUN_A: &str = include_str!("testdata/determinism/run-a.json");
    const RUN_B: &str = include_str!("testdata/determinism/run-b.json");
    const RUN_C: &str = include_str!("testdata/determinism/run-c.json");

    /// The combined shape regeneration wrote before #745. Retaining this
    /// deserialize-only fixture shape lets the test project historical captures
    /// through today's single-artifact boundary.
    #[derive(Deserialize)]
    struct CombinedRun {
        covered: BTreeMap<String, BTreeSet<String>>,
        orphans: BTreeMap<String, BTreeSet<String>>,
    }

    fn run_coverage(raw: &str) -> Coverage {
        let run: CombinedRun = serde_json::from_str(raw).expect("fixture parses");
        Coverage {
            covered: run.covered,
            orphans: run.orphans,
        }
    }

    #[test]
    fn the_runs_really_do_disagree_in_internal_title_attribution() {
        // Without this, equal inputs would make the projection test below
        // vacuous and would stop proving titles are excluded at the boundary.
        let titles: Vec<BTreeMap<String, BTreeSet<String>>> = [RUN_A, RUN_B, RUN_C]
            .iter()
            .map(|raw| run_coverage(raw).covered)
            .collect();
        assert_ne!(titles[0], titles[1]);
        assert_ne!(titles[1], titles[2]);
        assert_ne!(titles[0], titles[2]);
    }

    #[test]
    fn runs_that_disagree_on_titles_render_one_snapshot() {
        // Issue #757's durable contract: title-only timing variation cannot
        // alter the sole generated artifact.
        let rendered: Vec<String> = [RUN_A, RUN_B, RUN_C]
            .iter()
            .map(|raw| render(&run_coverage(raw).into_snapshot()).expect("renders"))
            .collect();
        assert_eq!(rendered[0], rendered[1]);
        assert_eq!(rendered[1], rendered[2]);
    }

    #[test]
    fn the_committed_snapshot_has_only_qualified_coverage_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let snapshot = crate::server_fn_coverage::io::read_snapshot(
            &root.join(crate::server_fn_coverage::io::SNAPSHOT_PATH),
        )
        .expect("the committed snapshot parses");

        for name in &snapshot.covered {
            assert!(
                is_qualified(name),
                "the committed snapshot holds a non-qualified name: {name}"
            );
        }
    }

    #[test]
    fn snapshot_projection_discards_test_titles() {
        // The title map remains load-bearing inside Coverage for classification,
        // but no title may cross the sole-artifact projection boundary.
        let snapshot = coverage_of(&[("posts::create", &["authenticated user can create a post"])])
            .into_snapshot();
        let rendered = render(&snapshot).expect("renders");
        assert!(!rendered.contains("authenticated user"), "{rendered}");
        assert_eq!(
            snapshot.covered,
            BTreeSet::from(["posts::create".to_string()])
        );
    }

    #[test]
    fn the_qualified_name_guard_rejects_what_it_must() {
        // The guard above is only worth having if it bites.
        assert!(is_qualified("posts::create"));
        assert!(is_qualified("_private::fn2"));
        assert!(!is_qualified("authenticated user can create a post"));
        assert!(!is_qualified("Posts::create"));
        assert!(!is_qualified("posts::api::create"));
        assert!(!is_qualified("create"));
        assert!(!is_qualified("posts::"));
    }

    #[test]
    fn render_is_byte_stable_across_equal_values() {
        let a = coverage_of(&[("b::fn", &["t"]), ("a::fn", &["t"])]).into_snapshot();
        let b = coverage_of(&[("a::fn", &["t"]), ("b::fn", &["t"])]).into_snapshot();
        assert_eq!(render(&a).expect("renders"), render(&b).expect("renders"));
    }

    #[test]
    fn render_ends_with_a_newline() {
        let snapshot = coverage_of(&[("a::fn", &["t"])]).into_snapshot();
        assert!(render(&snapshot).expect("renders").ends_with('\n'));
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let snapshot = coverage_of(&[("a::fn", &["t"]), ("b::fn", &["u"])]).into_snapshot();
        let round_trip: Snapshot =
            serde_json::from_str(&render(&snapshot).expect("renders")).expect("round-trips");
        assert_eq!(snapshot, round_trip);
    }
}
