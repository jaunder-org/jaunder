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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// fn ident → the tests that drove it, sorted.
    pub covered: BTreeMap<String, Vec<String>>,
    /// fn ident → the distinct reasons its unattributed hits ended with. Reported,
    /// not failed: a non-empty bucket means the harness stopped attributing
    /// somewhere, which is worth seeing — and the reason is what makes it
    /// diagnosable rather than merely visible (spec AC5).
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
    let inventory_idents: BTreeSet<&str> = inventory.iter().map(|f| f.ident.as_str()).collect();

    for f in inventory {
        // Endpoint/fn-name drift. A bare `#[server]` (no endpoint) counts: its
        // generated path carries a hash suffix, so it is NOT `/api/<fn_name>` and
        // the URI signal would silently never match it.
        match f.endpoint.as_deref() {
            None => out.push(format!(
                "{}: bare #[server] with no `endpoint = \"…\"` — its generated path carries a \
                 hash suffix, so coverage cannot match it by URI; add `endpoint = \"/{}\"`",
                f.ident, f.ident
            )),
            Some(ep) if ep != f.ident => out.push(format!(
                "{}: declared endpoint `{ep}` does not match the fn name — intentional renames \
                 are fine, but update this gate's expectations deliberately rather than by \
                 accident",
                f.ident
            )),
            Some(_) => {}
        }

        let covered = snapshot.covered.contains_key(&f.ident);
        let entry = allowed.get(f.ident.as_str());

        match (covered, entry) {
            // The ratchet must not loosen: a stale entry for a now-covered fn is
            // itself a failure, so entries cannot quietly become write-only.
            (true, Some(_)) => out.push(format!(
                "{}: allowlisted but the snapshot shows it covered — the entry is no longer \
                 needed; delete it from the allowlist",
                f.ident
            )),
            (false, None) => out.push(format!(
                "{}: no e2e flow drives this server fn. Either add one and regenerate \
                 (`{REGENERATE_CMD}`), or add an allowlist entry with a reason and an issue link",
                f.ident
            )),
            (false, Some(e)) => {
                if e.reason.trim().is_empty() || e.issue.trim().is_empty() {
                    out.push(format!(
                        "{}: allowlist entry needs both a non-empty `reason` and `issue` — an \
                         entry without them is a silent bypass",
                        f.ident
                    ));
                }
            }
            (true, None) => {}
        }
    }

    // A snapshot or allowlist naming something the inventory does not is stale.
    for ident in snapshot.covered.keys() {
        if !inventory_idents.contains(ident.as_str()) {
            out.push(format!(
                "{ident}: present in the snapshot but is not a #[server] fn — regenerate \
                 (`{REGENERATE_CMD}`)"
            ));
        }
    }
    for e in allowlist {
        if !inventory_idents.contains(e.server_fn.as_str()) {
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

    fn fnf(ident: &str) -> ServerFn {
        ServerFn {
            ident: ident.to_string(),
            endpoint: Some(ident.to_string()),
            module: "x::api".to_string(),
            line: 1,
        }
    }

    fn inv(idents: &[&str]) -> Vec<ServerFn> {
        idents.iter().map(|i| fnf(i)).collect()
    }

    fn covered_with(idents: &[&str]) -> Snapshot {
        Snapshot {
            covered: idents
                .iter()
                .map(|i| ((*i).to_string(), vec!["a test".to_string()]))
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
            "delete_media",
            "covered by server integration tests, no browser flow",
            "https://github.com/jaunder-org/jaunder/issues/700",
        )];
        assert!(verdict(&inv(&["delete_media"]), &Snapshot::default(), &al).is_empty());
    }

    #[test]
    fn allowlist_entry_without_reason_is_rejected() {
        let al = vec![entry("delete_media", "  ", "https://example.invalid/1")];
        let v = verdict(&inv(&["delete_media"]), &Snapshot::default(), &al);
        assert!(!v.is_empty(), "a hollow entry must not satisfy the gate");
    }

    #[test]
    fn allowlist_entry_without_issue_is_rejected() {
        let al = vec![entry("delete_media", "a real reason", "")];
        let v = verdict(&inv(&["delete_media"]), &Snapshot::default(), &al);
        assert!(!v.is_empty(), "a hollow entry must not satisfy the gate");
    }

    #[test]
    fn allowlist_entry_for_a_covered_fn_is_a_violation() {
        // The ratchet must not loosen: stale entries are removed, not accumulated.
        let al = vec![entry("delete_media", "r", "i")];
        let v = verdict(
            &inv(&["delete_media"]),
            &covered_with(&["delete_media"]),
            &al,
        );
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("no longer needed"), "{}", v[0]);
    }

    #[test]
    fn endpoint_not_matching_fn_name_is_a_violation() {
        let mut renamed = fnf("get_post");
        renamed.endpoint = Some("fetch_post".into());
        let v = verdict(&[renamed], &covered_with(&["get_post"]), &[]);
        assert!(v.iter().any(|m| m.contains("get_post")), "{v:?}");
    }

    #[test]
    fn bare_server_attr_without_endpoint_is_drift() {
        let mut bare = fnf("thing");
        bare.endpoint = None;
        let v = verdict(&[bare], &covered_with(&["thing"]), &[]);
        assert!(v.iter().any(|m| m.contains("thing")), "{v:?}");
    }

    #[test]
    fn snapshot_entry_for_an_unknown_fn_is_stale() {
        let v = verdict(&inv(&["a"]), &covered_with(&["a", "ghost"]), &[]);
        assert!(v.iter().any(|m| m.contains("ghost")), "{v:?}");
    }

    #[test]
    fn allowlist_entry_for_an_unknown_fn_is_stale() {
        let al = vec![entry("ghost", "r", "i")];
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
    fn snapshot_round_trips_through_json() {
        let s = covered_with(&["a", "b"]);
        let back: Snapshot =
            serde_json::from_str(&render(&s).expect("renders")).expect("round-trips");
        assert_eq!(s, back);
    }
}
