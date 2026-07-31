# Server-fn coverage determinism (#745) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `docs/coverage/server-fns.json` byte-reproducible by comparing
only what the gate asserts (covered fn keys + orphan reason sets) and moving the
per-test titles to an uncompared evidence file.

**Architecture:** `Coverage` (unchanged, derived from traces) gains a `split()`
into two committed artifacts: `Snapshot` — a sorted `Vec<String>` of qualified
names plus the orphan reason map, byte-compared as today — and `Evidence`, the
qualified-name → test-title map, written by `regenerate` and never compared. A
new static-lane check keeps the evidence file's key set in step with the
snapshot's. No change to trace parsing, span identification, or attribution.

**Tech Stack:** Rust (`xtask` crate), `serde`/`serde_json`, `anyhow`,
`cargo nextest`.

**Spec:**
[`docs/superpowers/specs/2026-07-31-issue-745-server-fn-coverage-determinism.md`](../specs/2026-07-31-issue-745-server-fn-coverage-determinism.md)
— read D1–D7 and AC1–AC12 before starting. This plan is "how"; the spec is
"what/why" and is not restated here.

## Global Constraints

- Compared artifact: `docs/coverage/server-fns.json` (`io::SNAPSHOT_PATH`).
- Evidence artifact: `docs/coverage/server-fns-evidence.json` — new
  `io::EVIDENCE_PATH`.
- Both render as `serde_json::to_string_pretty` + one trailing newline, ordered
  by `BTreeMap`/`BTreeSet`/sorted `Vec` (spec D3).
- Coverage keys are always `ServerFn::qualified()` — `<vertical>::<ident>`,
  never a bare ident.
- Everything on the read path fails closed: missing/unparseable = error, never
  "nothing uncovered" (spec AC5). The `read_allowlist` "missing = empty"
  template is deliberately **not** followed for the evidence file.
- No `Co-Authored-By` trailer on any commit.
- Run `cargo xtask check` before each commit so the pre-commit hook passes clean
  (**jaunder-commit**). Note this runs `server_fn_coverage_check::run` against
  the **real** committed artifacts (`lib.rs:403,443`) as well as the xtask unit
  tests — which is why Task 2 must leave those artifacts in the new format.

## Tasks

1. File the out-of-scope evidence-titles issue (separable concern, filed up
   front)
2. Split the artifact **and regenerate both files in the same commit**
3. Commit the determinism evidence as testdata with a projection test
4. Cross-check the evidence key set in the static lane
5. Make the byte-compare reachable without a capture, and test it
6. Docs: `CONTRIBUTING.md`, `docs/observability.md`, ADR-0081 amendment
7. Answer #745 with the corrected mechanism

**Key risks / decisions**

- **The type change and the regenerated artifacts are one atomic commit.** This
  was a defect in the first draft of this plan: `check()` and
  `seed_capture_covers_the_committed_snapshots_fns` both read the **real**
  `docs/coverage/server-fns.json`, so the moment `Snapshot.covered` becomes a
  `Vec<String>`, the still-object-shaped committed file fails to deserialize —
  the seed test panics and the static lane goes red. Splitting "change the type"
  from "regenerate the file" leaves the tree un-buildable in between, so Task 2
  does both.
- **Task 2 is wide by necessity**, for the reason above. Its call-site list is
  exhaustive; a missed one is a compile error, not a silent wrong answer.
- **Task 3's test is vacuous unless the inputs differ.** It must assert the
  three run fixtures are pairwise distinct _before_ asserting their projections
  are identical.
- **Task 3's fixtures are in the pre-split combined format**, which Task 2's
  code no longer parses. That is deliberate — they are historical evidence — so
  the test carries its own `CombinedRun` deserialize struct. It also means they
  cannot be regenerated after Task 2 lands without checking out an older tree,
  which is why Step 0 below stages them first.
- **Task 2 cannot be verified by re-running the e2e combo** — that replays the
  cached derivation in ~4 s and asserts a file equals itself (spec AC8).

**Re-slices discovered during execution** (recorded rather than silently
absorbed):

- **`read_evidence` and its fail-closed tests moved from Task 2 to Task 4.** The
  crate builds with `-D dead-code`, so a reader with no caller fails the gate,
  and its only caller is Task 4's `check` wiring. Suppressing the lint would
  have needed explicit approval and would have been the wrong fix. Task 2 keeps
  `EVIDENCE_PATH`, `write_evidence`, and the types.
- **`compare_rendered` (Task 5's seam) landed in Task 2.** Extracting it was
  entangled with routing `regenerate_or_verify` through the new `split()` — the
  same lines change either way. Task 5 is now its tests only, which is where its
  value was regardless: the seam without the tests proves nothing.

---

### Task 0 (already done — verify, do not redo)

The inputs Tasks 2 and 3 consume were staged into the worktree before planning,
because they existed only in a session-scoped scratchpad and reproducing them
costs four ~7-minute e2e executions _on a pre-split tree_:

- `xtask/src/server_fn_coverage/testdata/determinism/run-{a,b,c}.json` —
  untracked, 66851 / 66760 / 66942 bytes.
- A capture at
  `/tmp/claude-1000/-home-mdorman-src-jaunder/495b7307-95cc-43f1-a2cd-a1fa156a88ef/scratchpad/keep/capture-for-regenerate.tar.gz`
  (2.2 MB, read-only).

- [x] **Verify both are present** before starting Task 2:

```bash
ls -l xtask/src/server_fn_coverage/testdata/determinism/
ls -l /tmp/claude-1000/-home-mdorman-src-jaunder/495b7307-95cc-43f1-a2cd-a1fa156a88ef/scratchpad/keep/capture-for-regenerate.tar.gz
```

If the capture is gone, produce a fresh one with
`nix build --rebuild --keep-failed --accept-flake-config .#checks.x86_64-linux.e2e-sqlite-chromium`
and take `<out>.check/capture-sqlite.tar.gz` — any capture works, since the key
set is stable across runs. If the **fixtures** are gone, stop and re-plan Task
3: they cannot be reproduced on a post-Task-2 tree.

---

### Task 1: File the out-of-scope evidence-titles issue

**Files:** none in-tree (tracker only).

**Interfaces:**

- Consumes: nothing.
- Produces: an issue number, cited in Task 6's ADR amendment.

- [x] **Step 1: File the issue** via **jaunder-issues** in
      `jaunder-org/jaunder`.

Title: `server-fn coverage evidence: should it carry per-test titles at all?`

Body must state: the split (#745) removed the _gate_ consequence of the
964-title list, but `docs/coverage/server-fns-evidence.json` still rewrites ~66
KB whenever anyone adds, renames, or deletes an e2e test, and its titles can go
stale unnoticed because only its key set is checked (spec D4's stated limit).
Link #745, #681, and ADR-0081. Labels: `tooling`, `coverage`.

- [x] **Step 2: Record the number** — Task 6 cites it. Filed as
      [#757](https://github.com/jaunder-org/jaunder/issues/757) (Task,
      `tooling` + `coverage`, added to Jaunder Backlog).

No commit (tracker-only task).

---

### Task 2: Split the artifact and regenerate both files

**Files:**

- Modify: `xtask/src/server_fn_coverage/snapshot.rs` (the `Snapshot` type, the
  new `Evidence` type, `Coverage::split`, `render`, `verdict`, and the test
  fixtures)
- Modify: `xtask/src/server_fn_coverage/mod.rs` (re-export `Evidence`)
- Modify: `xtask/src/server_fn_coverage/io.rs` (`EVIDENCE_PATH`,
  `read_evidence`, `write_evidence`)
- Modify: `xtask/src/steps/server_fn_coverage_check.rs` (`regenerate_or_verify`
  gains an evidence path; `from_capture`; the inline fixtures; the seed test)
- Modify: `docs/coverage/server-fns.json` (regenerated, new format)
- Create: `docs/coverage/server-fns-evidence.json`
- Test: in-file `#[cfg(test)]` in `snapshot.rs` and `io.rs`

**Interfaces:**

- Consumes:
  `Coverage { covered: BTreeMap<String, BTreeSet<String>>, orphans: BTreeMap<String, BTreeSet<String>> }`
  (`extract.rs:85-101`) — unchanged.
- Produces:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Sorted `<vertical>::<ident>` names. The gate's assertion.
    pub covered: Vec<String>,
    #[serde(default)]
    pub orphans: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// `<vertical>::<ident>` → the tests that drove it, sorted.
    pub covered: BTreeMap<String, BTreeSet<String>>,
}

impl Coverage {
    /// The two committed artifacts. One constructor, so neither can be built
    /// without the other and drift into disagreement.
    pub fn split(self) -> (Snapshot, Evidence);
}

pub fn render<T: serde::Serialize>(value: &T) -> Result<String>;

// io.rs
pub const EVIDENCE_PATH: &str = "docs/coverage/server-fns-evidence.json";
pub fn read_evidence(path: &Path) -> Result<Evidence>;
pub fn write_evidence(path: &Path, evidence: &Evidence) -> Result<()>;

// server_fn_coverage_check.rs — gains a 5th parameter, keeping its
// "over explicit paths so it is testable without the repo" contract (:89)
fn regenerate_or_verify(
    web_src: &Path,
    capture: &Path,
    snapshot_path: &Path,
    evidence_path: &Path,
    regenerate: bool,
) -> Result<StepResult>;
```

- [x] **Step 1: Write the failing tests**

In `snapshot.rs`'s test module, replacing
`render_is_byte_stable_across_equal_snapshots`,
`render_sorts_keys_regardless_of_insertion_order`, `render_ends_with_a_newline`
and `snapshot_round_trips_through_json`:

```rust
fn coverage_of(pairs: &[(&str, &[&str])]) -> Coverage {
    Coverage {
        covered: pairs
            .iter()
            .map(|(k, ts)| (k.to_string(), ts.iter().map(|t| t.to_string()).collect()))
            .collect(),
        orphans: BTreeMap::new(),
    }
}

#[test]
fn split_puts_keys_in_the_snapshot_and_titles_in_the_evidence() {
    let (s, e) = coverage_of(&[("posts::create", &["a test"])]).split();
    assert_eq!(s.covered, vec!["posts::create".to_string()]);
    assert_eq!(
        e.covered.get("posts::create").expect("key present"),
        &BTreeSet::from(["a test".to_string()])
    );
}

#[test]
fn the_compared_snapshot_carries_no_test_titles() {
    // AC1. A title and a qualified name are the same TYPE, so the guard is a
    // SHAPE assertion: every entry must be `<vertical>::<ident>`. The regex is
    // the spec's, not a looser paraphrase — a title containing `::` and no
    // space would slip past a mere space/`::`-count check.
    let (s, _) =
        coverage_of(&[("posts::create", &["authenticated user can create a post"])]).split();
    let rendered = render(&s).expect("renders");
    assert!(!rendered.contains("authenticated user"), "{rendered}");
    for name in &s.covered {
        assert!(is_qualified(name), "not a qualified name: {name}");
    }
}

/// `^[a-z_][a-z0-9_]*::[a-z0-9_]+$` — the spec's AC1 shape, hand-rolled because
/// `xtask` has no `regex` dependency and one shape assertion does not justify
/// adding one. Rejects a space, a capital, and any `::` count but one.
fn is_qualified(name: &str) -> bool {
    let Some((vertical, ident)) = name.split_once("::") else {
        return false;
    };
    let word = |s: &str| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    word(vertical)
        && word(ident)
        && vertical.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
}

#[test]
fn the_qualified_name_guard_rejects_what_it_must() {
    // The guard is only worth having if it bites: a test title, a capitalised
    // name, and a nested path must all fail it.
    assert!(is_qualified("posts::create"));
    assert!(is_qualified("_private::fn2"));
    assert!(!is_qualified("authenticated user can create a post"));
    assert!(!is_qualified("Posts::create"));
    assert!(!is_qualified("posts::api::create"));
    assert!(!is_qualified("create"));
    assert!(!is_qualified("posts::"));
}

#[test]
fn split_orders_snapshot_keys_regardless_of_insertion_order() {
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
    let (s, e) = coverage_of(&[("a::fn", &["t"])]).split();
    assert!(render(&s).expect("renders").ends_with('\n'));
    assert!(render(&e).expect("renders").ends_with('\n'));
}

#[test]
fn both_artifacts_round_trip_through_json() {
    let (s, e) = coverage_of(&[("a::fn", &["t"]), ("b::fn", &["u"])]).split();
    let s2: Snapshot = serde_json::from_str(&render(&s).expect("renders")).expect("round-trips");
    let e2: Evidence = serde_json::from_str(&render(&e).expect("renders")).expect("round-trips");
    assert_eq!(s, s2);
    assert_eq!(e, e2);
}
```

In `io.rs`'s test module (note `io.rs:9` imports only `std::path::Path`, so the
collection imports are new):

```rust
use std::collections::{BTreeMap, BTreeSet};

fn one_entry_coverage() -> Coverage {
    Coverage {
        covered: BTreeMap::from([(
            "posts::create".to_string(),
            BTreeSet::from(["a test".to_string()]),
        )]),
        orphans: BTreeMap::new(),
    }
}

#[test]
fn missing_evidence_fails_closed_rather_than_reading_as_empty() {
    // Deliberately NOT the `read_allowlist` template above, where missing means
    // empty means pass: a missing evidence file must not look like agreement.
    let err = read_evidence(Path::new("/nonexistent-evidence.json")).unwrap_err();
    let chain = format!("{err:#}");
    assert!(chain.contains(super::super::REGENERATE_CMD), "{chain}");
}

#[test]
fn unparseable_evidence_fails_closed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("evidence.json");
    std::fs::write(&path, "{not json").expect("write");
    assert!(read_evidence(&path).is_err());
}

#[test]
fn write_evidence_creates_the_directory_and_renders_stably() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("nested").join("evidence.json");
    let (_, e) = one_entry_coverage().split();
    write_evidence(&path, &e).expect("writes");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        render(&e).expect("renders")
    );
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: FAIL —
`Coverage::split`, `Evidence`, `read_evidence`, `write_evidence`,
`EVIDENCE_PATH` not defined.

- [x] **Step 3: Implement against the tests**

Write the types, `Coverage::split`, the generic `render`, and the three `io.rs`
items to the signatures in **Interfaces**. The tests pin every branch —
ordering, newline, round-trip, shape, and both fail-closed paths — so the bodies
follow.

Two things the tests cannot pin, so write them out:

- `render`'s error context becomes artifact-neutral:
  `.context("serializing the server-fn coverage artifact")`. Keep it fallible;
  `snapshot.rs:73-79` records why a lossy fallback is the exact false verdict
  this gate exists to prevent.
- `read_evidence` mirrors `read_snapshot`'s context, naming `REGENERATE_CMD` as
  the remedy — not `read_allowlist`'s missing-is-empty shortcut.

Then update every caller the type change breaks. Exhaustively:

- `snapshot.rs:125` — `snapshot.covered.contains_key(&qualified)` →
  `snapshot.covered.iter().any(|c| c == &qualified)`.
- `snapshot.rs:152` — `for name in snapshot.covered.keys()` →
  `for name in &snapshot.covered`.
- `snapshot.rs:202-210` — the `covered_with()` helper returns the new shape.
- `snapshot.rs` — the `posts::create` fixture inside
  `covering_one_verticals_fn_leaves_another_verticals_same_named_fn_uncovered`.
- `mod.rs:15` — re-export `Evidence` alongside `Snapshot`.
- `server_fn_coverage_check.rs:72` and `:108` — `snapshot.covered.len()` still
  compiles as `Vec::len`; confirm both detail strings still read correctly.
- `server_fn_coverage_check.rs:198,215,229` — the three inline JSON fixtures
  become `{"covered":["posts::create_post"],"orphans":{}}` and equivalents.
- `server_fn_coverage_check.rs:362` — `snapshot.covered.keys().filter(...)` →
  `snapshot.covered.iter().filter(...)`. This is the one seed test that does
  **not** compile unmodified (spec AC7).
- `server_fn_coverage_check.rs:94-130` — `regenerate_or_verify` gains
  `evidence_path` as its 4th parameter (see the signature above);
  `Snapshot::from(coverage)` → `coverage.split()`; the regenerate branch writes
  **both** files and its detail names both paths.
- Its three callers, all of which must pass the new argument: `from_capture`
  (`:136-141`, passing `Path::new(EVIDENCE_PATH)`),
  `verify_from_a_missing_capture_is_an_error` (`:651-657`), and
  `an_unscannable_web_src_is_an_error_not_an_empty_inventory` (`:665-671`).

- [x] **Step 4: Regenerate both committed artifacts**

The tree is not buildable until this happens — the committed snapshot is still
an object of arrays and will not deserialize into the new `Snapshot`.

```bash
mkdir -p .xtask/diagnostics/e2e-sqlite-chromium
cp /tmp/claude-1000/-home-mdorman-src-jaunder/495b7307-95cc-43f1-a2cd-a1fa156a88ef/scratchpad/keep/capture-for-regenerate.tar.gz .xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz
cargo xtask server-fn-coverage regenerate
```

Expected: PASS, detail naming both written paths.

- [x] **Step 5: Run the tests, verify they pass**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: PASS — including
the seed tests that read the real artifacts
(`seed_capture_covers_the_committed_snapshots_fns`,
`every_allowlist_entry_is_absent_from_the_seed_captures_hit_set`,
`each_signal_finds_fns_on_its_own_in_the_real_capture` — spec AC7).

Then confirm the committed bytes are exactly what regeneration produced (spec
AC8):

Run: `cargo xtask server-fn-coverage verify` Expected: PASS —
`54 covered; snapshot current`.

Do **not** substitute `cargo xtask e2e sqlite chromium`: on an unchanged tree it
replays the cached derivation in ~4 s and asserts a file equals itself.

- [x] **Step 6: Commit**

```bash
git add xtask/src/server_fn_coverage/snapshot.rs xtask/src/server_fn_coverage/io.rs xtask/src/server_fn_coverage/mod.rs xtask/src/steps/server_fn_coverage_check.rs docs/coverage/server-fns.json docs/coverage/server-fns-evidence.json
git commit -m "refactor(coverage): compare the covered fn set, not the test titles (#745)"
```

The message body must record that this also undoes `6b0fad2c`'s CI-derived
workaround snapshot, which is no longer needed because the compared file now
converges (spec D6). Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 3: Commit the determinism evidence as testdata

**Files:**

- Add (already on disk, untracked — see Task 0):
  `xtask/src/server_fn_coverage/testdata/determinism/run-{a,b,c}.json`
- Create: `xtask/src/server_fn_coverage/testdata/determinism/README.md`
- Test: in-file `#[cfg(test)]` in `snapshot.rs`

**Interfaces:**

- Consumes: `Coverage`, `Coverage::split`, `render` (Task 2).
- Produces: nothing consumed by later tasks.

The three fixtures are the **distinct** snapshots that
`cargo xtask server-fn-coverage regenerate` produced from four forced
re-executions of `.#checks.x86_64-linux.e2e-sqlite-chromium` on this tree. Runs
2 and 3 were byte-identical, hence three files. They are in the **pre-split
combined format**, which the post-Task-2 code no longer parses — deliberate, as
historical evidence, so the test brings its own deserialize struct.

- [x] **Step 1: Write the README**

`README.md` records provenance so the files are not mistaken for hand-authored:
the commit they came from, the method (one `nix build` plus three
`nix build --rebuild` of `.#checks.x86_64-linux.e2e-sqlite-chromium`,
regenerating from each output's `capture-sqlite.tar.gz`), that runs 2 and 3 were
byte-identical, that the format is the pre-#745 combined one, and that the
captures themselves (~2.2 MB compressed, ~26 MB of JSONL extracted) are not
committed.

- [x] **Step 2: Write the failing tests**

```rust
/// Real `regenerate` output from three distinct forced re-executions of the
/// authoritative e2e check on one tree. See testdata/determinism/README.md.
const RUN_A: &str = include_str!("testdata/determinism/run-a.json");
const RUN_B: &str = include_str!("testdata/determinism/run-b.json");
const RUN_C: &str = include_str!("testdata/determinism/run-c.json");

/// The combined shape `regenerate` wrote before #745 — the fixtures' format.
#[derive(serde::Deserialize)]
struct CombinedRun {
    covered: BTreeMap<String, BTreeSet<String>>,
    orphans: BTreeMap<String, BTreeSet<String>>,
}

fn run_coverage(raw: &str) -> Coverage {
    let run: CombinedRun = serde_json::from_str(raw).expect("fixture parses");
    Coverage { covered: run.covered, orphans: run.orphans }
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
    // AC9, and the whole basis of #745's fix: the covered key set and the orphan
    // reason sets are what the gate asserts, and they do not move between runs —
    // only the test titles do.
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
    // entirely in the uncompared artifact.
    let evidence: Vec<String> = [RUN_A, RUN_B, RUN_C]
        .iter()
        .map(|raw| render(&run_coverage(raw).split().1).expect("renders"))
        .collect();
    assert_ne!(evidence[0], evidence[1]);
}
```

- [x] **Step 3: Run the tests, verify they pass**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: PASS.

> Executed as `cargo test --manifest-path xtask/Cargo.toml server_fn_coverage` —
> `xtask` lives outside the workspace, so `-p xtask` resolves to no package. 63
> passed. The same correction applies to every other task's run step.

There is deliberately **no red phase** here. The fixtures are pre-staged
(Task 0) and `include_str!` is compile-time, so a missing fixture is a build
failure rather than a failing assertion. The meaningful guard is
`the_three_runs_really_do_disagree`, which fails if the fixtures are ever
replaced by copies of one run.

- [x] **Step 4: Commit**

```bash
git add xtask/src/server_fn_coverage/testdata/determinism xtask/src/server_fn_coverage/snapshot.rs
git commit -m "test(coverage): pin that runs disagreeing on titles render one snapshot (#745)"
```

---

### Task 4: Cross-check the evidence key set in the static lane

**Files:**

- Modify: `xtask/src/server_fn_coverage/snapshot.rs` (new `evidence_verdict`)
- Modify: `xtask/src/server_fn_coverage/mod.rs` (re-export `evidence_verdict`)
- Modify: `xtask/src/steps/server_fn_coverage_check.rs` (`check()` reads and
  checks the evidence file; `run()` passes its path)
- Test: in-file `#[cfg(test)]` in both

**Interfaces:**

- Consumes: `Snapshot`, `Evidence`, `io::EVIDENCE_PATH`, `io::read_evidence`
  (Task 2).
- Produces:

```rust
/// Every way the evidence file disagrees with the snapshot's `covered` array,
/// one message per key, sorted. Empty means they agree.
pub fn evidence_verdict(snapshot: &Snapshot, evidence: &Evidence) -> Vec<String>;
```

and `check` gains a path:

```rust
fn check(
    web_src: &Path,
    snapshot_path: &Path,
    allowlist_path: &Path,
    evidence_path: &Path,
) -> StepResult;
```

- [x] **Step 1: Write the failing tests**

In `snapshot.rs`'s test module:

```rust
fn evidence_of(keys: &[&str]) -> Evidence {
    Evidence {
        covered: keys
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
    assert!(v[0].contains("missing"), "says which way it drifted: {}", v[0]);
    assert!(v[0].contains(REGENERATE_CMD), "names the remedy: {}", v[0]);
    assert!(
        v[0].contains("cargo xtask e2e sqlite chromium"),
        "the remedy is two steps — regenerate needs a capture first: {}",
        v[0]
    );
}

#[test]
fn evidence_naming_a_fn_the_snapshot_does_not_cover_is_a_violation() {
    // The other direction: a stale evidence file left behind after a fn stopped
    // being covered. Both directions, or half the drift is invisible.
    let (s, _) = coverage_of(&[("posts::create", &["a test"])]).split();
    let v = evidence_verdict(&s, &evidence_of(&["posts::create", "ghost::fn"]));
    assert_eq!(v.len(), 1, "{v:?}");
    assert!(v[0].contains("ghost::fn"), "{}", v[0]);
    assert!(
        v[0].contains("not covered"),
        "distinguishable from the missing-key message: {}",
        v[0]
    );
}

#[test]
fn evidence_verdict_ignores_orphan_only_keys() {
    // The evidence file mirrors `covered`, not `orphans` (spec D4). A fn hit only
    // during the `_autoPerfSpan` warmup has an orphan key and no covered key, and
    // must not be required in the evidence file.
    let s = Snapshot {
        covered: vec!["posts::create".to_string()],
        orphans: BTreeMap::from([(
            "auth::get_session".to_string(),
            BTreeSet::from(["unknown-parent:1111111111111111".to_string()]),
        )]),
    };
    assert!(evidence_verdict(&s, &evidence_of(&["posts::create"])).is_empty());
}
```

In `server_fn_coverage_check.rs`'s test module — **both** directions at the
fixture-file level, per spec AC4:

```rust
/// A snapshot + evidence pair on disk, so `check()` is exercised the way `run()`
/// calls it.
fn write_artifacts(dir: &Path, snapshot: &str, evidence: &str) -> (PathBuf, PathBuf) {
    let snap = dir.join("snap.json");
    let ev = dir.join("evidence.json");
    write_json(&snap, snapshot);
    write_json(&ev, evidence);
    (snap, ev)
}

#[test]
fn static_lane_fails_when_the_evidence_is_missing_a_covered_fn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    web_src_with(tmp.path(), &["create_post"]);
    let (snap, ev) = write_artifacts(
        tmp.path(),
        r#"{"covered":["posts::create_post"],"orphans":{}}"#,
        r#"{"covered":{}}"#,
    );
    let step = check(tmp.path(), &snap, &tmp.path().join("absent-allowlist.json"), &ev);
    assert!(!step.ok);
    assert!(step.detail.unwrap_or_default().contains("posts::create_post"));
}

#[test]
fn static_lane_fails_when_the_evidence_names_an_uncovered_fn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    web_src_with(tmp.path(), &["create_post"]);
    let (snap, ev) = write_artifacts(
        tmp.path(),
        r#"{"covered":["posts::create_post"],"orphans":{}}"#,
        r#"{"covered":{"posts::create_post":["t"],"posts::ghost":["t"]}}"#,
    );
    let step = check(tmp.path(), &snap, &tmp.path().join("absent-allowlist.json"), &ev);
    assert!(!step.ok);
    assert!(step.detail.unwrap_or_default().contains("posts::ghost"));
}

#[test]
fn static_lane_fails_closed_on_a_missing_evidence_file() {
    // AC5: the plumbing's own failure must not look like "nothing uncovered".
    let tmp = tempfile::tempdir().expect("tempdir");
    web_src_with(tmp.path(), &["create_post"]);
    let snap = tmp.path().join("snap.json");
    write_json(&snap, r#"{"covered":["posts::create_post"],"orphans":{}}"#);
    let step = check(
        tmp.path(),
        &snap,
        &tmp.path().join("absent-allowlist.json"),
        &tmp.path().join("absent-evidence.json"),
    );
    assert!(!step.ok);
}
```

The **five** existing `check()` tests —
`static_lane_passes_when_every_fn_is_covered`,
`static_lane_bites_on_an_uncovered_fn`,
`static_lane_accepts_a_substantive_allowlist_entry`,
`static_lane_fails_closed_on_a_missing_snapshot`,
`static_lane_fails_closed_on_an_unparseable_snapshot` — each gain an evidence
file matching their snapshot's `covered` array, so they keep testing what they
were written to test (spec AC6).

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: FAIL —
`evidence_verdict` not defined; `check` takes 3 arguments.

- [x] **Step 3: Implement against the tests**

Write `evidence_verdict` to the signature above and thread `evidence_path`
through `check` and `run`. The tests pin both directions, the orphan-only
exclusion, and the fail-closed read.

The message text is pinned only by its required substrings, so write both out.
They must be **distinguishable** — a reader hitting one has to know which file
is stale — and the remedy is genuinely two steps, because `REGENERATE_CMD` fails
immediately without a capture (`io.rs:68-74`):

```rust
// evidence missing a key the snapshot covers
format!(
    "{name}: missing from the evidence file but covered by the snapshot. Regenerate \
     both: run `cargo xtask e2e sqlite chromium` to produce a capture, then \
     `{REGENERATE_CMD}`"
)
// evidence naming a key the snapshot does not cover
format!(
    "{name}: named by the evidence file but not covered by the snapshot — stale \
     evidence. Regenerate both: run `cargo xtask e2e sqlite chromium` to produce a \
     capture, then `{REGENERATE_CMD}`"
)
```

In `check`, fold `evidence_verdict`'s messages into the same `violations` vector
as `verdict`'s, so one failing step reports every reason at once, and extend the
existing 3-way error `match` (`server_fn_coverage_check.rs:50-65`) to a 4-way
one including `read_evidence`.

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add xtask/src/server_fn_coverage/snapshot.rs xtask/src/server_fn_coverage/mod.rs xtask/src/steps/server_fn_coverage_check.rs
git commit -m "feat(coverage): fail when the evidence file's key set drifts from the snapshot (#745)"
```

---

### Task 5: Make the byte-compare reachable without a capture

**Files:**

- Modify: `xtask/src/steps/server_fn_coverage_check.rs:94-130`
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Consumes: `Snapshot` (Task 2).
- Produces:

```rust
/// The verify verdict for bytes already derived — pure over its inputs, so the
/// drift branch is testable without a capture tarball. `name` is the step name
/// (`VERIFY_STEP`), passed rather than hardcoded so the caller stays the single
/// place that decides which step this is.
fn compare_rendered(
    name: &'static str,
    committed: &str,
    rendered: &str,
    snapshot_path: &Path,
    covered: usize,
) -> StepResult;
```

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn identical_bytes_verify_clean() {
    let step = compare_rendered(
        VERIFY_STEP,
        "same\n",
        "same\n",
        Path::new("docs/coverage/server-fns.json"),
        54,
    );
    assert!(step.ok, "{:?}", step.detail);
    assert!(step.detail.unwrap_or_default().contains("54 covered"));
}

#[test]
fn any_byte_difference_is_drift() {
    // AC3. Byte equality, not parsed equality: a hand-edit that happens to parse
    // equal is still drift.
    let step = compare_rendered(
        VERIFY_STEP,
        "a\n",
        "b\n",
        Path::new("docs/coverage/server-fns.json"),
        54,
    );
    assert!(!step.ok);
    let detail = step.detail.unwrap_or_default();
    assert!(detail.contains("docs/coverage/server-fns.json"), "{detail}");
    assert!(detail.contains(REGENERATE_CMD), "names the remedy: {detail}");
}

#[test]
fn a_missing_committed_file_reads_as_empty_and_therefore_drifts() {
    // `regenerate_or_verify` passes `unwrap_or_default()` for an unreadable file;
    // empty never equals rendered output, so it fails — the strict reading.
    let step = compare_rendered(
        VERIFY_STEP,
        "",
        "anything\n",
        Path::new("docs/coverage/server-fns.json"),
        0,
    );
    assert!(!step.ok);
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: FAIL —
`compare_rendered` not defined.

- [x] **Step 3: Implement against the tests**

Extract the existing comparison from `regenerate_or_verify`
(`server_fn_coverage_check.rs:121-129`) into `compare_rendered` and call it from
there. Behaviour is unchanged — this is a seam, not new logic.

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p xtask server_fn_coverage` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add xtask/src/steps/server_fn_coverage_check.rs
git commit -m "test(coverage): make the verify byte-compare reachable without a capture (#745)"
```

---

### Task 6: Docs and the ADR-0081 amendment

**Files:**

- Modify: `docs/adr/0081-empirical-server-fn-flow-coverage.md`
- Modify: `CONTRIBUTING.md:533-534,545,548`
- Modify: `docs/observability.md:71-72,124-125`

**Interfaces:**

- Consumes: the issue number from Task 1.
- Produces: nothing consumed by later tasks.

- [x] **Step 1: Amend ADR-0081**

Per spec D7 — amend in place, do not supersede; it is still `Status: proposed`.
Record:

- The Decision now names two artifacts: the compared snapshot (keys + orphan
  reasons) and the uncompared evidence file.
- The existing Consequence that title churn is "accepted as the signal working,
  **confined to one file**" is **false as written** and is replaced: the churn
  is confined to the _uncompared_ file and no longer reddens a build.
- The measured basis: 54 covered keys and identical orphan reason sets across
  four forced re-executions; only titles moved.
- **No misattribution exists** — every hit chains to the test whose context
  issued it. The instability was post-assertion trailing traffic. A future
  reader must not go hunting for a propagation bug.
- D5: the time-window rule was implemented, measured, and rejected; ADR-0081's
  own rejection of time-window correlation is the same underlying reason.
- D4's limit: the evidence file's titles can rot unnoticed — follow-up is the
  Task 1 issue number.

Use **jaunder-adr** for the mechanics. This edits an existing numbered ADR, so
the numberless-draft flow does not apply.

- [x] **Step 2: Update `CONTRIBUTING.md`**

`CONTRIBUTING.md:533-534,545,548` — the coverage-policy section must name both
files, state plainly which one the gate compares and which is evidence, and note
that `regenerate` writes both.

- [x] **Step 3: Update `docs/observability.md`**

Two edits, not one:

- `:71-72` — the table row for `docs/coverage/server-fns.json` currently reads
  "server fn → the named tests that drove it". That becomes **wrong** on
  landing: the snapshot row becomes the covered fn set, and a **new row**
  describes the evidence file.
- `:124-125` — the surrounding prose describing how the snapshot is produced and
  read must name both artifacts too, or it contradicts the amended table.

- [x] **Step 4: Commit**

```bash
git add docs/adr/0081-empirical-server-fn-flow-coverage.md CONTRIBUTING.md docs/observability.md
git commit -m "docs(coverage): record the compared/evidence split and correct #745's premise (#745)"
```

---

### Task 7: Answer #745 with the corrected mechanism

**Files:** none in-tree (tracker only).

**Interfaces:**

- Consumes: the spec's Problem section.
- Produces: nothing.

- [x] **Step 1: Comment on #745**

Post the corrected mechanism before the PR merges (spec AC12), stating:

- No misattribution exists; every hit chains to the test whose browser context
  issued it.
- The instability is post-assertion trailing traffic: `authed-flash.spec.ts:64`
  asserts a redirect and ends while its page is still booting, and the boot is
  progressively truncated at a different point each run.
- The `posts::create` in the issue's diff hunk is a mislabelled key. The real
  `19368ffe` → `6b0fad2c` delta is exactly one pair —
  `posts::get_default_audience_selection` gaining the redirect test's title —
  and `posts::create` carries that title on neither commit.
- Nix caches the e2e derivation, which is why no regeneration converges: a dev
  box replays one capture while CI produces its own.

Keep it factual and short; the spec and ADR carry the full record.

No commit (tracker-only task).

---

## Self-review

**Spec coverage.** AC1 → Task 2 (`the_compared_snapshot_carries_no_test_titles`,
using the spec's regex). AC2 → Task 2 (render contract, `write_evidence`, and
the committed file from Step 4). AC3 → Task 5. AC4 → Task 4 (both directions, at
both the pure-function and fixture-file levels). AC5 → Task 2 (`read_evidence`)
and Task 4 (static lane). AC6 → Task 2 (the fixtures at `snapshot.rs:202-210`
and `check.rs:198,215,229` are rewritten there, because the type change forces
it) and Task 4 (the evidence sidecar those tests gain). AC7 → Task 2 Step 3
(names the one seed test that changes) and Step 5 (runs the other two). AC8 →
Task 2 Step 5. AC9 → Task 3. AC10 → Task 6 Steps 2–3. AC11 → Task 6 Step 1. AC12
→ Task 7. D6 → Task 2 Steps 4 and 6. Out-of-scope issue → Task 1.

**Ordering.** Task 2 is the only task that may leave the tree un-buildable
mid-task, and it closes that window itself by regenerating before its gate run.
Every later task consumes only what Task 2 produced. Task 3's fixtures are
staged in Task 0 because they cannot be reproduced on a post-Task-2 tree.

**Type consistency.** `Snapshot.covered: Vec<String>` is used as a `Vec` in
every later reference (`iter().any`, `for name in &`, `.len()` at `check.rs:72`
and `:108`, `evidence_verdict`).
`Evidence.covered: BTreeMap<String, BTreeSet<String>>` matches
`Coverage.covered`, so `split()` moves it without conversion. Both derive
`Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize` — required by
`assert_eq!`, `serde_json::from_str`, and `render`. `render` is generic from
Task 2 onward and every later call passes `&Snapshot` or `&Evidence`.
`evidence_verdict(&Snapshot, &Evidence)` matches at its definition and its only
caller. `regenerate_or_verify` takes `evidence_path` in its definition and all
three call sites. `compare_rendered` takes `name` and is called only with
`VERIFY_STEP`.

**Placeholders.** None: every test is written out, every signature and derive
given, and the four bodies the tests cannot pin (`render`'s error context, the
two drift messages, and `is_qualified`) are stated verbatim. `is_qualified` is
itself pinned by `the_qualified_name_guard_rejects_what_it_must`, so the guard
cannot silently degrade into one that accepts everything.
