//! The committed coverage snapshot, the hand-maintained allowlist, and the
//! verdict that turns them into build failures (#681).
//!
//! **Two files, deliberately separate.** The generator rewrites the snapshot on
//! every e2e run; the allowlist is hand-written. Folding the allowlist into the
//! snapshot would let regeneration clobber it.
//!
//! **No provenance fields.** The snapshot records coverage and nothing else — no
//! commit, no timestamp. A recorded commit is necessarily an ancestor of the
//! commit under test, so with fail-on-any-difference the gate would be red
//! forever. Keys are `BTreeMap`-ordered and rendering is stable, so an unchanged
//! run is byte-identical and drift comparison can be total.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Coverage;
use crate::server_fns::ServerFn;

/// The regenerate command named in every failure message, so an author who hits
/// the gate needs no external context.
pub const REGENERATE_CMD: &str = "cargo xtask server-fn-coverage regenerate";

/// Which tests exercised each server fn, as committed to `docs/coverage/`.
///
/// Keyed by [`ServerFn::qualified`] — `<vertical>::<ident>` — as is the
/// allowlist's `server_fn` field, so the two files and the inventory all name a fn
/// the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// `<vertical>::<ident>` → the tests that drove it, sorted.
    pub covered: BTreeMap<String, Vec<String>>,
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

impl From<Coverage> for Snapshot {
    fn from(c: Coverage) -> Self {
        Self {
            covered: c
                .covered
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().collect::<Vec<_>>()))
                .collect(),
            orphans: c.orphans,
        }
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

/// Render a snapshot to the exact bytes committed: stably-ordered, pretty-printed
/// JSON with a trailing newline.
///
/// Fallible rather than lossy. A serialization failure used to fall back to an empty
/// `{"covered":{},"orphans":{}}`, which `regenerate` would then *write*: the committed
/// snapshot would say nothing is covered, and the next `verify` would agree with it.
/// That is the exact false verdict this gate exists to prevent, so the error
/// propagates.
pub fn render(snapshot: &Snapshot) -> Result<String> {
    // BTreeMap serializes in key order, and `to_string_pretty` is deterministic,
    // so equal snapshots render byte-identically.
    let mut out = serde_json::to_string_pretty(snapshot)
        .context("serializing the server-fn coverage snapshot")?;
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
        let covered = snapshot.covered.contains_key(&qualified);
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
    for name in snapshot.covered.keys() {
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
            covered: idents
                .iter()
                .map(|i| (qual(i), vec!["a test".to_string()]))
                .collect(),
            orphans: BTreeMap::new(),
        }
    }

    fn entry(server_fn: &str, reason: &str, issue: &str) -> AllowlistEntry {
        AllowlistEntry {
            server_fn: server_fn.to_string(),
            reason: reason.to_string(),
            issue: issue.to_string(),
        }
    }

    #[test]
    fn render_is_byte_stable_across_equal_snapshots() {
        let a = covered_with(&["b_fn", "a_fn"]);
        let b = covered_with(&["a_fn", "b_fn"]);
        assert_eq!(render(&a).expect("renders"), render(&b).expect("renders"));
    }

    #[test]
    fn render_sorts_keys_regardless_of_insertion_order() {
        let mut s = Snapshot::default();
        s.covered.insert("z_fn".into(), vec!["t".into()]);
        s.covered.insert("a_fn".into(), vec!["t".into()]);
        let out = render(&s).expect("renders");
        let (a, z) = (out.find("a_fn"), out.find("z_fn"));
        assert!(a < z, "keys must render in sorted order: {out}");
    }

    #[test]
    fn render_ends_with_a_newline() {
        assert!(render(&Snapshot::default())
            .expect("renders")
            .ends_with('\n'));
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
            covered: [(
                "posts::create".to_string(),
                vec!["creates a post".to_string()],
            )]
            .into_iter()
            .collect(),
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

    #[test]
    fn snapshot_round_trips_through_json() {
        let s = covered_with(&["a", "b"]);
        let back: Snapshot =
            serde_json::from_str(&render(&s).expect("renders")).expect("round-trips");
        assert_eq!(s, back);
    }
}
