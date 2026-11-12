//! Read-only server-function inventory and committed snapshot contracts.
//! Runtime trace extraction, producer verification and regeneration have separate owners.
use crate::files;
use crate::server_fns::{ServerFn, module_path_of, server_fns_in};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
/// The `web` crate source root, scanned for the `#[macros::server]` inventory.
pub const WEB_SRC: &str = "web/src";
/// The committed, generated coverage snapshot — the byte-compared artifact.
pub const SNAPSHOT_PATH: &str = "docs/coverage/server-fns.json";

/// Every `#[macros::server]` fn under `web/src`, sorted by qualified name. A file
/// that cannot be enumerated is a hard error: a file we cannot read could hide a fn,
/// and an under-counted inventory is exactly a false pass.
pub fn inventory(root: &Path) -> Result<Vec<ServerFn>> {
    let files = files::with_extension(root, "rs")
        .with_context(|| format!("scanning {} for #[macros::server] fns", root.display()))?;

    let mut out = Vec::new();
    for path in &files {
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let rel = path.strip_prefix(root).unwrap_or(path);
        match server_fns_in(&src, &module_path_of(rel)) {
            Ok(fns) => out.extend(fns),
            Err(msg) => bail!("{}: {msg}", path.display()),
        }
    }
    // By the coverage key, not the ident: idents collide across verticals (#684),
    // so an ident sort leaves the order of a collision set up to directory walk
    // order.
    out.sort_by_key(ServerFn::qualified);
    Ok(out)
}

/// Read the generated coverage snapshot or fail naming the remedy.
///
/// Missing or unparseable input is an error, never an empty value: static
/// verification must fail closed when the sole artifact is absent or malformed.
pub fn read_snapshot(path: &Path) -> Result<Snapshot> {
    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "reading {} — if it does not exist yet, generate it with `{}`",
            path.display(),
            REGENERATE_CMD
        )
    })?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

/// The regenerate command named in every failure message, so an author who hits
/// the gate needs no external context.
pub const REGENERATE_CMD: &str = "just e2e-regenerate-coverage";

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

/// The static lane's step name.
/// The static-lane check, over explicit paths so it is testable without the repo.
///
/// A missing snapshot, an unscannable `web/src`, or an unparseable snapshot is a
/// **failure**, never a pass: the failure mode this gate guards against and the
/// failure mode of its own plumbing would otherwise look identical.
fn check(web_src: &Path, snapshot_path: &Path) -> std::result::Result<Option<String>, String> {
    let (inventory, snapshot) = match (inventory(web_src), read_snapshot(snapshot_path)) {
        (Ok(inventory), Ok(snapshot)) => (inventory, snapshot),
        (inventory, snapshot) => {
            let detail = [inventory.err(), snapshot.err()]
                .into_iter()
                .flatten()
                .map(|e| format!("{e:#}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(detail);
        }
    };

    let violations = verdict(&inventory, &snapshot);
    if violations.is_empty() {
        return Ok(Some(format!(
            "{} server fn(s) accounted for ({} covered)",
            inventory.len(),
            snapshot.covered.len()
        )));
    }
    Err(violations.join("\n"))
}

/// The static lane over the repo's real snapshot. Runs in `check` and
/// `just validate-no-e2e`.
pub fn run() -> std::result::Result<Option<String>, String> {
    check(Path::new(WEB_SRC), Path::new(SNAPSHOT_PATH))
}

#[cfg(test)]
mod verdict_tests {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The vertical the fixture tree writes its fns in.
    const VERTICAL: &str = "posts";

    /// A `web/src`-shaped tree with one `#[macros::server]` fn per ident given, all
    /// in the [`VERTICAL`] vertical — a fn at the crate root has no vertical, so its
    /// coverage key would be a degenerate `::<ident>` and pin nothing about the
    /// real snapshot. The endpoints are the `<vertical>/<ident>` the gate derives
    /// from that placement.
    fn web_src_with(dir: &Path, idents: &[&str]) {
        let src: String = idents
            .iter()
            .map(|i| format!("#[macros::server]\npub async fn {i}() {{}}\n"))
            .collect();
        let vertical = dir.join(VERTICAL);
        std::fs::create_dir_all(&vertical).expect("create the vertical's dir");
        std::fs::write(vertical.join("api.rs"), src).expect("write source");
    }

    fn write_json(path: &Path, json: &str) {
        std::fs::write(path, json).expect("write json");
    }

    #[test]
    fn static_lane_passes_when_every_fn_is_covered() {
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snapshot = tmp.path().join("snap.json");
        write_json(
            &snapshot,
            r#"{"covered":["posts::create_post"],"orphans":{}}"#,
        );

        let step = check(tmp.path(), &snapshot);
        assert!(step.is_ok(), "{:?}", step.clone().unwrap_or_else(Some));
        assert!(
            step.clone()
                .unwrap_or_else(Some)
                .unwrap_or_default()
                .contains("1 server fn")
        );
    }

    #[test]
    fn static_lane_bites_on_an_uncovered_fn() {
        // `verdict`'s own bite test proves the rule; this proves the lane applies
        // it to the real snapshot and source-derived inventory.
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post", "brand_new_uncovered_fn"]);
        let snapshot = tmp.path().join("snap.json");
        write_json(
            &snapshot,
            r#"{"covered":["posts::create_post"],"orphans":{}}"#,
        );

        let step = check(tmp.path(), &snapshot);
        assert!(!step.is_ok());
        let detail = step.clone().unwrap_or_else(Some).unwrap_or_default();
        assert!(detail.contains("brand_new_uncovered_fn"), "{detail}");
    }

    #[test]
    fn static_lane_ignores_the_retired_evidence_path() {
        // Issue #757 removed this input. A malformed file at the retired path
        // must be inert, so a future evidence read cannot slip back into the
        // static verdict unnoticed.
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snapshot = tmp.path().join("snap.json");
        write_json(
            &snapshot,
            r#"{"covered":["posts::create_post"],"orphans":{}}"#,
        );
        write_json(
            &tmp.path().join("server-fns-evidence.json"),
            "{ definitely not json",
        );

        let step = check(tmp.path(), &snapshot);
        assert!(step.is_ok(), "{:?}", step.clone().unwrap_or_else(Some));
    }

    #[test]
    fn static_lane_fails_closed_on_a_missing_snapshot() {
        // The plumbing's own failure must not look like "nothing uncovered".
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);

        let step = check(tmp.path(), &tmp.path().join("absent-snapshot.json"));
        assert!(!step.is_ok());
        let detail = step.clone().unwrap_or_else(Some).unwrap_or_default();
        assert!(
            detail.contains(REGENERATE_CMD),
            "names the remedy: {detail}"
        );
    }

    #[test]
    fn static_lane_fails_closed_on_an_unparseable_snapshot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snapshot = tmp.path().join("snap.json");
        write_json(&snapshot, "{not json");

        let step = check(tmp.path(), &snapshot);
        assert!(!step.is_ok());
    }
}
