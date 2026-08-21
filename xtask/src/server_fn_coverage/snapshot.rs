//! The committed coverage artifacts and the verdict that turns them into build
//! failures (#681).
//!
//! **Two files, deliberately separate.** The generator rewrites the snapshot and
//! the evidence on every e2e run.
//!
//! **The snapshot is what the gate asserts; the evidence is what a reader wants
//! (#745).** [`verdict`] has only ever consulted the *set* of covered fns —
//! never a test title — so the titles were load-bearing for red/green solely
//! because the whole file was byte-compared. And the titles are the one part
//! that does not reproduce: measured across four forced re-executions of the
//! authoritative e2e check on one tree, the covered key set and the orphan
//! reason sets were identical every time while the title sets moved. The cause
//! is not misattribution — every hit really is attributed to the test whose
//! browser context issued it — but *post-assertion trailing traffic*: a test
//! that ends mid-navigation leaves its page booting, and how far that boot gets
//! before teardown differs run to run.
//!
//! So [`Snapshot`] carries the covered names and the orphan reasons and is
//! byte-compared; [`Evidence`] carries the titles and is regenerated but never
//! compared. [`verdict`] cross-checks their key sets, which *are* stable, so the
//! evidence cannot silently fall out of step — see `evidence_verdict`.
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
    /// asserts and the set is what reproduces. The titles live in [`Evidence`].
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

/// Which tests drove each server fn, as committed to
/// `docs/coverage/server-fns-evidence.json`. **Never compared** — regenerated
/// alongside the snapshot and read only by humans.
///
/// It is uncompared because it does not reproduce (see the module docs), and it
/// is still *committed* because ADR-0081 leans on being able to see which flow
/// backs each fn without running the suite. The cost of that trade is that a
/// title can go stale unnoticed: `evidence_verdict` checks only the key set.
/// Whether it is worth its weight at all is #757.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// `<vertical>::<ident>` → the tests that drove it, sorted.
    pub covered: BTreeMap<String, BTreeSet<String>>,
}

impl Coverage {
    /// The two committed artifacts. One constructor rather than two `From` impls,
    /// so neither can be built without the other and drift into disagreement —
    /// the disagreement `evidence_verdict` exists to catch is then only reachable
    /// through the filesystem, never through this seam.
    pub fn split(self) -> (Snapshot, Evidence) {
        (
            Snapshot {
                covered: self.covered.keys().cloned().collect(),
                orphans: self.orphans,
            },
            Evidence {
                covered: self.covered,
            },
        )
    }
}

/// Render a coverage artifact to the exact bytes committed: stably-ordered,
/// pretty-printed JSON with a trailing newline.
///
/// Generic over both artifacts so they share one rendering contract. The evidence
/// file is not compared, but an unstable rendering would still churn its git diff
/// on every regenerate with no gate to catch it.
///
/// Fallible rather than lossy. A serialization failure must not fall back to an
/// empty `{"covered":{},"orphans":{}}` — `regenerate` would *write* it, the
/// committed snapshot would say nothing is covered, and the next `verify` would
/// agree. That is the exact false verdict this gate exists to prevent, so the
/// error propagates.
pub fn render<T: Serialize>(value: &T) -> Result<String> {
    // Every collection in both artifacts is a `BTreeMap`/`BTreeSet`, so they
    // serialize in key order, and `to_string_pretty` is itself deterministic —
    // equal values render byte-identically without anyone remembering to sort.
    let mut out = serde_json::to_string_pretty(value)
        .context("serializing the server-fn coverage artifact")?;
    out.push('\n');
    Ok(out)
}

/// The remedy for either artifact being out of step, as the two steps it really
/// is: [`REGENERATE_CMD`] fails immediately without a capture, so naming it alone
/// sends an author into an error instead of a fix.
const REGENERATE_BOTH: &str = "regenerate both: run `cargo xtask e2e sqlite chromium` to produce a \
                               capture, then `cargo xtask server-fn-coverage regenerate`";

/// Every way the evidence file disagrees with the snapshot's `covered` names, one
/// message per name, sorted. Empty means they agree.
///
/// Compares **key sets only, in both directions** — never titles. The titles are
/// the part that does not reproduce (module docs), so comparing them is the bug
/// #745 fixed; the key sets are measured stable, so comparing them is free.
///
/// It mirrors `covered` and not `orphans`. Today every orphan name is also a
/// covered name, so the distinction is latent — but a fn hit only during the
/// `_autoPerfSpan` warmup would have an orphan entry and no covered entry, and
/// requiring evidence for it would demand titles that by definition do not exist.
///
/// What this cannot catch is titles that went stale while the key set held —
/// a renamed or deleted test. That is a known, accepted gap (#757).
pub fn evidence_verdict(snapshot: &Snapshot, evidence: &Evidence) -> Vec<String> {
    let mut out = Vec::new();

    for name in &snapshot.covered {
        if !evidence.covered.contains_key(name) {
            out.push(format!(
                "{name}: covered by the snapshot but missing from the evidence file — \
                 {REGENERATE_BOTH}"
            ));
        }
    }
    for name in evidence.covered.keys() {
        if !snapshot.covered.contains(name) {
            out.push(format!(
                "{name}: named by the evidence file but not covered by the snapshot — stale \
                 evidence; {REGENERATE_BOTH}"
            ));
        }
    }

    out.sort();
    out
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

    /// A [`Coverage`] over `(qualified name, test titles)` pairs, the input side
    /// of [`Coverage::split`].
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

    // ── The determinism evidence (#745 AC9) ─────────────────────────────────
    //
    // Real `regenerate` output from three distinct executions of the same e2e
    // derivation on one tree. See testdata/determinism/README.md for provenance.
    // These pin the claim the split rests on: runs disagree about titles and
    // agree about everything the gate asserts.

    const RUN_A: &str = include_str!("testdata/determinism/run-a.json");
    const RUN_B: &str = include_str!("testdata/determinism/run-b.json");
    const RUN_C: &str = include_str!("testdata/determinism/run-c.json");

    /// The combined shape `regenerate` wrote before #745 — the fixtures' format,
    /// which today's [`Snapshot`] deliberately cannot parse.
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
    fn the_three_runs_really_do_disagree() {
        // Without this the test below is vacuous: three identical inputs would
        // project identically and prove nothing about determinism.
        assert_ne!(RUN_A, RUN_B);
        assert_ne!(RUN_B, RUN_C);
        assert_ne!(RUN_A, RUN_C);
    }

    #[test]
    fn runs_that_disagree_on_titles_still_render_one_compared_snapshot() {
        // AC9, and the whole basis of #745's fix.
        let rendered: Vec<String> = [RUN_A, RUN_B, RUN_C]
            .iter()
            .map(|raw| render(&run_coverage(raw).split().0).expect("renders"))
            .collect();
        assert_eq!(rendered[0], rendered[1]);
        assert_eq!(rendered[1], rendered[2]);
    }

    #[test]
    fn the_runs_disagree_only_in_the_evidence() {
        // The complement: prove the difference asserted above is real and lands
        // entirely in the uncompared artifact. Pairwise, not just [0] vs [1] —
        // otherwise two of the three fixtures could be evidence-identical and
        // this would still pass, leaving the third carrying no weight.
        let evidence: Vec<String> = [RUN_A, RUN_B, RUN_C]
            .iter()
            .map(|raw| render(&run_coverage(raw).split().1).expect("renders"))
            .collect();
        assert_ne!(evidence[0], evidence[1]);
        assert_ne!(evidence[1], evidence[2]);
        assert_ne!(evidence[0], evidence[2]);
    }

    fn evidence_of(names: &[&str]) -> Evidence {
        Evidence {
            covered: names
                .iter()
                .map(|k| (k.to_string(), BTreeSet::from(["a test".to_string()])))
                .collect(),
        }
    }

    #[test]
    fn agreeing_key_sets_pass() {
        let (s, _) = coverage_of(&[("posts::create", &["a test"])]).split();
        assert!(evidence_verdict(&s, &evidence_of(&["posts::create"])).is_empty());
    }

    #[test]
    fn evidence_missing_a_covered_fn_is_a_violation() {
        let (s, _) =
            coverage_of(&[("posts::create", &["a test"]), ("tags::list", &["a test"])]).split();
        let v = evidence_verdict(&s, &evidence_of(&["posts::create"]));
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("tags::list"), "{}", v[0]);
        assert!(
            v[0].contains("missing"),
            "says which way it drifted: {}",
            v[0]
        );
        assert!(v[0].contains(REGENERATE_CMD), "names the remedy: {}", v[0]);
        assert!(
            v[0].contains("cargo xtask e2e sqlite chromium"),
            "the remedy is two steps — regenerate needs a capture first: {}",
            v[0]
        );
    }

    #[test]
    fn evidence_naming_a_fn_the_snapshot_does_not_cover_is_a_violation() {
        // The other direction: stale evidence left behind after a fn stopped
        // being covered. Both directions, or half the drift is invisible.
        let (s, _) = coverage_of(&[("posts::create", &["a test"])]).split();
        let v = evidence_verdict(&s, &evidence_of(&["posts::create", "ghost::fn"]));
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("ghost::fn"), "{}", v[0]);
        assert!(
            v[0].contains("stale"),
            "distinguishable from the missing-key message: {}",
            v[0]
        );
    }

    #[test]
    fn evidence_verdict_ignores_orphan_only_names() {
        // The evidence file mirrors `covered`, not `orphans`. A fn hit only during
        // the `_autoPerfSpan` warmup has an orphan entry and no covered entry, and
        // requiring evidence for it would demand titles that do not exist.
        let s = Snapshot {
            covered: BTreeSet::from(["posts::create".to_string()]),
            orphans: BTreeMap::from([(
                "auth::get_session".to_string(),
                BTreeSet::from(["unknown-parent:1111111111111111".to_string()]),
            )]),
        };
        assert!(evidence_verdict(&s, &evidence_of(&["posts::create"])).is_empty());
    }

    #[test]
    fn the_committed_artifacts_agree_with_each_other() {
        // The rule applied to the real files, not just to fixtures: whatever is in
        // `docs/coverage/` right now must satisfy the check the static lane runs.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let snapshot = crate::server_fn_coverage::io::read_artifact::<Snapshot>(
            &root.join(crate::server_fn_coverage::io::SNAPSHOT_PATH),
        )
        .expect("the committed snapshot parses");
        let evidence = crate::server_fn_coverage::io::read_artifact::<Evidence>(
            &root.join(crate::server_fn_coverage::io::EVIDENCE_PATH),
        )
        .expect("the committed evidence parses");
        let v = evidence_verdict(&snapshot, &evidence);
        assert!(v.is_empty(), "committed artifacts disagree: {v:?}");

        // AC1 against the REAL artifact, not only a synthetic fixture: the shape
        // guard is worthless if it is never pointed at the file that could
        // actually acquire a title.
        for name in &snapshot.covered {
            assert!(
                is_qualified(name),
                "the committed snapshot holds a non-qualified name: {name}"
            );
        }
    }

    #[test]
    fn split_puts_names_in_the_snapshot_and_titles_in_the_evidence() {
        let (s, e) = coverage_of(&[("posts::create", &["a test"])]).split();
        assert_eq!(s.covered, BTreeSet::from(["posts::create".to_string()]));
        assert_eq!(
            e.covered.get("posts::create").expect("key present"),
            &BTreeSet::from(["a test".to_string()])
        );
    }

    #[test]
    fn the_compared_snapshot_carries_no_test_titles() {
        // AC1, and the point of the whole split. A title and a qualified name are
        // the same TYPE, so nothing at the `render` seam can tell them apart by
        // type — the guard has to be about SHAPE.
        let (s, _) =
            coverage_of(&[("posts::create", &["authenticated user can create a post"])]).split();
        let rendered = render(&s).expect("renders");
        assert!(!rendered.contains("authenticated user"), "{rendered}");
        for name in &s.covered {
            assert!(is_qualified(name), "not a qualified name: {name}");
        }
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

    // There was a `split_orders_snapshot_names_regardless_of_insertion_order`
    // test here while `Snapshot.covered` was a `Vec`. `BTreeSet` makes ordering a
    // property of the type, so the test policed nothing the compiler did not
    // already guarantee. `render_is_byte_stable_across_equal_values` still pins
    // the property that actually matters to a byte-compared artifact.

    #[test]
    fn render_is_byte_stable_across_equal_values() {
        let (a, _) = coverage_of(&[("b::fn", &["t"]), ("a::fn", &["t"])]).split();
        let (b, _) = coverage_of(&[("a::fn", &["t"]), ("b::fn", &["t"])]).split();
        assert_eq!(render(&a).expect("renders"), render(&b).expect("renders"));
    }

    #[test]
    fn render_ends_with_a_newline() {
        // Both artifacts: the evidence file is not compared, but an unstable
        // rendering would still churn its git diff on every regenerate.
        let (s, e) = coverage_of(&[("a::fn", &["t"])]).split();
        assert!(render(&s).expect("renders").ends_with('\n'));
        assert!(render(&e).expect("renders").ends_with('\n'));
    }

    #[test]
    fn both_artifacts_round_trip_through_json() {
        let (s, e) = coverage_of(&[("a::fn", &["t"]), ("b::fn", &["u"])]).split();
        let s2: Snapshot =
            serde_json::from_str(&render(&s).expect("renders")).expect("round-trips");
        let e2: Evidence =
            serde_json::from_str(&render(&e).expect("renders")).expect("round-trips");
        assert_eq!(s, s2);
        assert_eq!(e, e2);
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
