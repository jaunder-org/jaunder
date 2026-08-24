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

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Coverage;
use crate::server_fns::ServerFn;

/// The regenerate command named in every failure message, so an author who hits
/// the gate needs no external context.
pub const REGENERATE_CMD: &str = "cargo xtask server-fn-coverage regenerate";

/// Which server fns a real browser session drove, as committed to
/// `docs/coverage/server-fns.json`. **This is the byte-compared artifact.**
///
/// Named by [`ServerFn::qualified`] — `<vertical>::<ident>` — so the artifact
/// and inventory name a fn the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The `<vertical>::<ident>` names driven by at least one e2e flow.
    ///
    /// A *set*, not a map to test titles, because the set is what [`verdict`]
    /// asserts and the set is what reproduces.
    ///
    /// `BTreeSet` rather than `Vec` so sortedness and uniqueness are properties
    /// of the type rather than of every construction site — it serializes to the
    /// same JSON array, and the artifact is byte-compared, so an unsorted or
    /// duplicated `Vec` would have been a drift failure diagnosed as coverage
    /// drift.
    pub covered: BTreeSet<String>,
    /// `<vertical>::<ident>` → the distinct reasons its unattributed hits ended
    /// with. Reported, not failed: a non-empty bucket means the harness stopped
    /// attributing somewhere, which is worth seeing — and the reason is what makes
    /// it diagnosable rather than merely visible (spec AC5).
    ///
    /// Deliberately not counts: a count tracks how many tests ran, so it would make
    /// this byte-compared artifact churn every time anyone adds an e2e test. See
    /// `Coverage::orphans`.
    #[serde(default)]
    pub orphans: BTreeMap<String, BTreeSet<String>>,
}

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

/// Every reason the gate should fail, one message per violation, sorted. Empty
/// means the gate passes. Pure given its inputs, so it is unit-tested directly.
pub fn verdict(inventory: &[ServerFn], snapshot: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    // Every name the gate speaks is qualified (`<vertical>::<ident>`): fifteen fns
    // share six idents across verticals since #684, so an ident-keyed set would
    // treat `audiences::create` as accounted for by `posts::create`.
    let inventory_names: BTreeSet<String> = inventory.iter().map(ServerFn::qualified).collect();

    for f in inventory {
        let qualified = f.qualified();
        // There is deliberately no endpoint-drift check here — a comparison of a
        // computed value against itself would pass for the wrong reason; the seed
        // cross-check verifies the endpoint against real traffic instead
        // (docs/adr/0120-no-endpoint-drift-check.md).
        if !snapshot.covered.contains(&qualified) {
            out.push(format!(
                "{qualified}: no e2e flow drives this server fn. Add a browser flow and \
                 regenerate (`{REGENERATE_CMD}`)"
            ));
        }
    }

    // A snapshot naming something the inventory does not is stale.
    for name in &snapshot.covered {
        if !inventory_names.contains(name) {
            out.push(format!(
                "{name}: present in the snapshot but is not a #[server] fn — regenerate \
                 (`{REGENERATE_CMD}`)"
            ));
        }
    }

    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vertical the helpers below write their fns in. Everything the gate
    /// names is qualified by it: the key is `<vertical>::<ident>` and the derived
    /// endpoint `<vertical>/<ident>`.
    const V: &str = "posts";

    /// A fn in [`V`] whose declared endpoint matches what the gate derives, so a
    /// test that is not about endpoint drift does not trip over it.
    fn fnf(ident: &str) -> ServerFn {
        ServerFn {
            ident: ident.to_string(),
            endpoint: Some(format!("{V}/{ident}")),
            module: format!("{V}::api"),
            line: 1,
        }
    }

    /// The coverage key of an ident written in [`V`].
    fn qual(ident: &str) -> String {
        format!("{V}::{ident}")
    }

    fn inv(idents: &[&str]) -> Vec<ServerFn> {
        idents.iter().map(|i| fnf(i)).collect()
    }

    fn covered_with(idents: &[&str]) -> Snapshot {
        Snapshot {
            covered: idents.iter().map(|i| qual(i)).collect(),
            orphans: BTreeMap::new(),
        }
    }

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

    #[test]
    fn a_fully_covered_inventory_passes() {
        let v = verdict(&inv(&["create_post"]), &covered_with(&["create_post"]));
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn uncovered_fn_is_a_violation() {
        let v = verdict(&inv(&["delete_media"]), &Snapshot::default());
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("delete_media"));
        assert!(v[0].contains(REGENERATE_CMD), "names the remedy: {}", v[0]);
        assert!(
            !v[0].contains("allowlist"),
            "does not name an exemption path: {}",
            v[0]
        );
    }

    /// The two endpoint-drift tests that stood here were deleted with the check
    /// they covered (#714). They built a `ServerFn` by hand with a deliberately
    /// wrong `endpoint`, which no enumeration can now produce — the inventory
    /// derives that field. Keeping them would have meant asserting that a
    /// hand-corrupted value is reported, while the real code path could not reach
    /// the branch: a green test proving nothing about the tree.
    #[test]
    fn snapshot_entry_for_an_unknown_fn_is_stale() {
        let v = verdict(&inv(&["a"]), &covered_with(&["a", "ghost"]));
        assert!(v.iter().any(|m| m.contains("ghost")), "{v:?}");
    }

    #[test]
    fn gate_bites_on_a_newly_added_uncovered_fn() {
        // The enforcement proof lives in the repo, not in PR prose: a new
        // #[server] fn that no e2e flow drives must redden the build.
        let inventory = inv(&["create_post", "brand_new_uncovered_fn"]);
        let v = verdict(&inventory, &covered_with(&["create_post"]));
        assert!(
            v.iter().any(|m| m.contains("brand_new_uncovered_fn")),
            "{v:?}"
        );
    }

    #[test]
    fn covering_one_verticals_fn_leaves_another_verticals_same_named_fn_uncovered() {
        // The #684 collision, at the layer that decides red or green: three
        // verticals declare `create`, and an ident-keyed gate would have passed all
        // three on the strength of one covered flow.
        let one_create_per_vertical: Vec<ServerFn> = ["posts", "audiences", "invites"]
            .iter()
            .map(|v| ServerFn {
                ident: "create".to_string(),
                endpoint: Some(format!("{v}/create")),
                module: format!("{v}::api"),
                line: 1,
            })
            .collect();
        let snapshot = Snapshot {
            covered: BTreeSet::from(["posts::create".to_string()]),
            orphans: BTreeMap::new(),
        };

        let v = verdict(&one_create_per_vertical, &snapshot);
        assert!(
            v.iter().any(|m| m.starts_with("audiences::create:")),
            "audiences::create is undriven and must be reported: {v:?}"
        );
        assert!(
            v.iter().any(|m| m.starts_with("invites::create:")),
            "invites::create is undriven and must be reported: {v:?}"
        );
        assert!(
            !v.iter().any(|m| m.starts_with("posts::create:")),
            "posts::create IS covered and must not be reported: {v:?}"
        );
    }
}
