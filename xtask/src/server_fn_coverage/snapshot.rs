//! The committed coverage artifacts, the hand-maintained allowlist, and the
//! verdict that turns them into build failures (#681).
//!
//! **Three files, deliberately separate.** The generator rewrites the snapshot
//! and the evidence on every e2e run; the allowlist is hand-written. Folding the
//! allowlist into either would let regeneration clobber it.
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
//! forever. Keys are ordered and rendering is stable, so an unchanged run is
//! byte-identical and drift comparison can be total.

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
/// Named by [`ServerFn::qualified`] — `<vertical>::<ident>` — as is the
/// allowlist's `server_fn` field, so all three files and the inventory name a fn
/// the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The `<vertical>::<ident>` names driven by at least one e2e flow, sorted.
    ///
    /// A *set*, not a map to test titles, because the set is what [`verdict`]
    /// asserts and the set is what reproduces. The titles live in [`Evidence`].
    pub covered: Vec<String>,
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
                // `BTreeMap` keys already iterate in sorted order, so this is a
                // move into a `Vec` that happens to be sorted, not a re-sort.
                covered: self.covered.keys().cloned().collect(),
                orphans: self.orphans,
            },
            Evidence {
                covered: self.covered,
            },
        )
    }
}

/// One knowingly-uncovered server fn. Both fields are mandatory in substance, not
/// just in shape: a blank reason or issue is rejected, so an entry cannot be a
/// silent bypass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowlistEntry {
    /// The fn, as `<vertical>::<ident>` ([`ServerFn::qualified`]). A bare ident
    /// would excuse every vertical's fn of that name at once.
    pub server_fn: String,
    pub reason: String,
    pub issue: String,
}

/// Render a coverage artifact to the exact bytes committed: stably-ordered,
/// pretty-printed JSON with a trailing newline.
///
/// Generic over both artifacts so they share one rendering contract. The evidence
/// file is not compared, but an unstable rendering would still churn its git diff
/// on every regenerate with no gate to catch it.
///
/// Fallible rather than lossy. A serialization failure used to fall back to an empty
/// `{"covered":{},"orphans":{}}`, which `regenerate` would then *write*: the committed
/// snapshot would say nothing is covered, and the next `verify` would agree with it.
/// That is the exact false verdict this gate exists to prevent, so the error
/// propagates.
pub fn render<T: Serialize>(value: &T) -> Result<String> {
    // `BTreeMap`/`BTreeSet` serialize in key order and `covered` is sorted on
    // construction, so `to_string_pretty` — itself deterministic — renders equal
    // values byte-identically.
    let mut out = serde_json::to_string_pretty(value)
        .context("serializing the server-fn coverage artifact")?;
    out.push('\n');
    Ok(out)
}

/// Every reason the gate should fail, one message per violation, sorted. Empty
/// means the gate passes. Pure given its inputs, so it is unit-tested directly.
pub fn verdict(
    inventory: &[ServerFn],
    snapshot: &Snapshot,
    allowlist: &[AllowlistEntry],
) -> Vec<String> {
    let mut out = Vec::new();

    let allowed: BTreeMap<&str, &AllowlistEntry> = allowlist
        .iter()
        .map(|e| (e.server_fn.as_str(), e))
        .collect();
    // Every name the gate speaks is qualified (`<vertical>::<ident>`): fifteen fns
    // share six idents across verticals since #684, so an ident-keyed set would
    // treat `audiences::create` as accounted for by `posts::create`.
    let inventory_names: BTreeSet<String> = inventory.iter().map(ServerFn::qualified).collect();

    for f in inventory {
        let qualified = f.qualified();
        // There is no endpoint-drift check here any more, and its absence is
        // deliberate rather than an oversight.
        //
        // It used to compare each fn's *declared* `endpoint = "…"` against the
        // derived `<vertical>/<ident>`, which was a real cross-check while an author
        // wrote that literal by hand. Since #714 nothing declares it: the inventory
        // computes `endpoint` with the very expression this check compared it to
        // (`server_fns.rs`), so both arms — a missing endpoint, and a declared one
        // that disagrees — became unreachable by construction. A comparison of a
        // value against itself passes for the wrong reason, which is worse than no
        // comparison at all.
        //
        // What replaces it is `server_fn_coverage_check`'s seed cross-check, which
        // compares the computed endpoint against URIs observed in a real captured
        // run — ground truth produced by the macro's actual expansion, not a second
        // restatement of the rule.
        let covered = snapshot.covered.iter().any(|c| c == &qualified);
        let entry = allowed.get(qualified.as_str());

        match (covered, entry) {
            // The ratchet must not loosen: a stale entry for a now-covered fn is
            // itself a failure, so entries cannot quietly become write-only.
            (true, Some(_)) => out.push(format!(
                "{qualified}: allowlisted but the snapshot shows it covered — the entry is no \
                 longer needed; delete it from the allowlist"
            )),
            (false, None) => out.push(format!(
                "{qualified}: no e2e flow drives this server fn. Either add one and regenerate \
                 (`{REGENERATE_CMD}`), or add an allowlist entry with a reason and an issue link"
            )),
            (false, Some(e)) => {
                if e.reason.trim().is_empty() || e.issue.trim().is_empty() {
                    out.push(format!(
                        "{qualified}: allowlist entry needs both a non-empty `reason` and \
                         `issue` — an entry without them is a silent bypass"
                    ));
                }
            }
            (true, None) => {}
        }
    }

    // A snapshot or allowlist naming something the inventory does not is stale.
    for name in &snapshot.covered {
        if !inventory_names.contains(name) {
            out.push(format!(
                "{name}: present in the snapshot but is not a #[server] fn — regenerate \
                 (`{REGENERATE_CMD}`)"
            ));
        }
    }
    for e in allowlist {
        if !inventory_names.contains(&e.server_fn) {
            out.push(format!(
                "{}: allowlisted but is not a #[server] fn — delete the stale entry",
                e.server_fn
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
            covered: {
                let mut names: Vec<String> = idents.iter().map(|i| qual(i)).collect();
                names.sort();
                names
            },
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

    fn entry(server_fn: &str, reason: &str, issue: &str) -> AllowlistEntry {
        AllowlistEntry {
            server_fn: server_fn.to_string(),
            reason: reason.to_string(),
            issue: issue.to_string(),
        }
    }

    #[test]
    fn split_puts_names_in_the_snapshot_and_titles_in_the_evidence() {
        let (s, e) = coverage_of(&[("posts::create", &["a test"])]).split();
        assert_eq!(s.covered, vec!["posts::create".to_string()]);
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

    #[test]
    fn split_orders_snapshot_names_regardless_of_insertion_order() {
        let (s, _) = coverage_of(&[("z::fn", &["t"]), ("a::fn", &["t"])]).split();
        assert_eq!(s.covered, vec!["a::fn".to_string(), "z::fn".to_string()]);
    }

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
        let v = verdict(&inv(&["create_post"]), &covered_with(&["create_post"]), &[]);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn uncovered_and_unallowlisted_fn_is_a_violation() {
        let v = verdict(&inv(&["delete_media"]), &Snapshot::default(), &[]);
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("delete_media"));
        assert!(v[0].contains(REGENERATE_CMD), "names the remedy: {}", v[0]);
        assert!(
            v[0].contains("allowlist"),
            "names the other remedy: {}",
            v[0]
        );
    }

    #[test]
    fn allowlisted_uncovered_fn_passes() {
        let al = vec![entry(
            &qual("delete_media"),
            "covered by server integration tests, no browser flow",
            "https://github.com/jaunder-org/jaunder/issues/700",
        )];
        assert!(verdict(&inv(&["delete_media"]), &Snapshot::default(), &al).is_empty());
    }

    #[test]
    fn allowlist_entry_without_reason_is_rejected() {
        let al = vec![entry(
            &qual("delete_media"),
            "  ",
            "https://example.invalid/1",
        )];
        let v = verdict(&inv(&["delete_media"]), &Snapshot::default(), &al);
        assert!(!v.is_empty(), "a hollow entry must not satisfy the gate");
    }

    #[test]
    fn allowlist_entry_without_issue_is_rejected() {
        let al = vec![entry(&qual("delete_media"), "a real reason", "")];
        let v = verdict(&inv(&["delete_media"]), &Snapshot::default(), &al);
        assert!(!v.is_empty(), "a hollow entry must not satisfy the gate");
    }

    #[test]
    fn allowlist_entry_for_a_covered_fn_is_a_violation() {
        // The ratchet must not loosen: stale entries are removed, not accumulated.
        let al = vec![entry(&qual("delete_media"), "r", "i")];
        let v = verdict(
            &inv(&["delete_media"]),
            &covered_with(&["delete_media"]),
            &al,
        );
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("no longer needed"), "{}", v[0]);
    }

    /// The two endpoint-drift tests that stood here were deleted with the check
    /// they covered (#714). They built a `ServerFn` by hand with a deliberately
    /// wrong `endpoint`, which no enumeration can now produce — the inventory
    /// derives that field. Keeping them would have meant asserting that a
    /// hand-corrupted value is reported, while the real code path could not reach
    /// the branch: a green test proving nothing about the tree.
    #[test]
    fn snapshot_entry_for_an_unknown_fn_is_stale() {
        let v = verdict(&inv(&["a"]), &covered_with(&["a", "ghost"]), &[]);
        assert!(v.iter().any(|m| m.contains("ghost")), "{v:?}");
    }

    #[test]
    fn allowlist_entry_for_an_unknown_fn_is_stale() {
        let al = vec![entry(&qual("ghost"), "r", "i")];
        let v = verdict(&inv(&["a"]), &covered_with(&["a"]), &al);
        assert!(v.iter().any(|m| m.contains("ghost")), "{v:?}");
    }

    #[test]
    fn gate_bites_on_a_newly_added_uncovered_fn() {
        // The enforcement proof lives in the repo, not in PR prose: a new
        // #[server] fn that no e2e flow drives must redden the build.
        let inventory = inv(&["create_post", "brand_new_uncovered_fn"]);
        let v = verdict(&inventory, &covered_with(&["create_post"]), &[]);
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
            covered: vec!["posts::create".to_string()],
            orphans: BTreeMap::new(),
        };

        let v = verdict(&one_create_per_vertical, &snapshot, &[]);
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

    #[test]
    fn an_allowlist_entry_excuses_only_its_own_vertical() {
        // The mirror of the above: excusing `media::delete` must not excuse
        // `posts::delete`, which a bare `delete` entry would have.
        let inventory: Vec<ServerFn> = ["media", "posts"]
            .iter()
            .map(|v| ServerFn {
                ident: "delete".to_string(),
                endpoint: Some(format!("{v}/delete")),
                module: format!("{v}::api"),
                line: 1,
            })
            .collect();
        let al = vec![entry("media::delete", "no browser flow", "#706")];

        let v = verdict(&inventory, &Snapshot::default(), &al);
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].starts_with("posts::delete:"), "{}", v[0]);
    }
}
