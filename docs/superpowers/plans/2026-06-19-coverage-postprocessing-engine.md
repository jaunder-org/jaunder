# Coverage Post-Processing Engine — Implementation Plan (Plan B2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move coverage gating off the Nix check and into host xtask: the check becomes *produce-data-only* and emits the complete per-line report; xtask parses it, classifies each delta by line identity, gates (strict ratchet), auto-heals the safe deltas, and emits a `.coverage` JSON block.

**Architecture:** The Nix `coverage` check stops gating coverage/CRAP regressions (it still fails on test failures) and copies the **rich, complete data** it already generates — `cargo llvm-cov report --text` (`.coverage-report.txt`: per-line number, hit count, executable flag, and source text) plus `.crap-report.json` — into its `$out`. Host xtask reads that output, compares against a committed **accepted-uncovered baseline** (per file: the grandfathered uncovered executable lines + their source text), maps committed→working lines through `git diff`, classifies (`regression`/`new_uncovered`/`structural`/`improvement`), gates, auto-heals (shrink-only), and reports the `.coverage` block in the result envelope.

**Tech Stack:** Rust (`xtask`: `serde`, existing deps; no new heavy deps), `git diff`, `cargo-llvm-cov` text report, `cargo-crap` JSON.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md` (coverage model + auto-heal + CRAP sections).
- **No CI changes in B2.** CI is untouched until the B4 cutover; nobody relies on CI this cycle. B2 only changes the local gate (xtask) and the Nix check's *outputs* (it stops gating coverage but CI's coverage job will still pass — it just stops catching coverage regressions, which is fine until B4).
- **The Nix `coverage` check must still fail on test failures** (the instrumented `nextest` passes). Only the coverage/CRAP *regression* gating moves to xtask.
- **Strict ratchet, shrink-only:** a currently-uncovered executable line fails unless it maps to an unchanged baseline accepted-uncovered line. `new_uncovered` (new line, uncovered) and `regression` (a previously-covered line now uncovered) both **fail**. The accepted-uncovered baseline only ever shrinks (heal removes covered/deleted gaps); it never auto-grows.
- **Auto-heal with notification, narrow:** heal (rewrite the committed baseline) only for `improvement` (a baseline gap now covered) and `structural` (a baseline gap whose line was deleted/moved away). Never heal a `regression`/`new_uncovered`. Report `coverage.healed` in the JSON and loudly in human output.
- **CRAP kept, gating, host-side:** xtask performs the same numeric per-(crate|file|function|line) comparison the script does today (worse score by >0.01 → regression), reading the check's `.crap-report.json` against the committed `.crap-manifest.json`.
- **Exclusion escape-hatch = `// cov:ignore`.** A line whose source text contains the comment `// cov:ignore` is dropped from the executable set by the report parser (Task 2) — the rich per-line source text the check emits makes this trivial, so a genuinely-uncoverable line is never a gap. `cargo-llvm-cov` 0.8.3 on stable has **no** line-level exclusion of its own; this is our convention, honored entirely host-side.
- **Commit after every task**, branch `testing-coverage-orchestration` (never `main`). `.beads/issues.jsonl` may ride along if pre-staged.
- **Environment:** Bash tool blocks `sed`/`grep`/`head`/`tail`/`awk` and complex compound commands — use `rg`, `jq`, Read/Serena, simple commands. Run xtask via `cargo xtask …`; gate via the bare command through context-mode. Inspect Rust via Serena.
- **Heavy Nix coverage runs are slow** — verify parsers/classifiers with Rust unit tests over fixture strings, not by running the coverage build. The full end-to-end run happens once in Task 8.

---

## The text-report format (reference for the parser)

`cargo llvm-cov report --text` emits, per file:

```
/abs/path/to/file.rs:
    1|      |use std::foo;            ← blank count column = NON-executable
    2|    36|pub fn bar() {           ← count "36" = executable, COVERED
    3|     0|    unreachable!()        ← count "0" = executable, UNCOVERED
   ...
```

A line is **executable** iff its second pipe-delimited column is non-blank; **covered** iff that column is a non-zero number (counts may be suffixed, e.g. `1.36k`). File header lines end in `.rs:`. (This is exactly the format `scripts/check-coverage`'s awk already parses.)

---

## File structure

- `scripts/check-coverage` — add a produce-only mode (skip regression gating; still test-gated).
- `flake.nix` — `coverage` check `installPhaseCommand`: also copy `.coverage-report.txt` + `.crap-report.json` to `$out`.
- `xtask/src/coverage/mod.rs` — module wiring + the public `run(result, opts)` entry called by `check`/`validate`.
- `xtask/src/coverage/report.rs` — parse the llvm-cov text report → `FileCoverage { path, lines: Vec<LineCov> }`.
- `xtask/src/coverage/diffmap.rs` — parse `git diff` → per-file old→new line map.
- `xtask/src/coverage/baseline.rs` — the accepted-uncovered baseline: load/save `.coverage-baseline.json`.
- `xtask/src/coverage/classify.rs` — produce `CoverageVerdict { regressions, new_uncovered, structural, improvements }`.
- `xtask/src/coverage/crap.rs` — host-side CRAP comparison.
- `xtask/src/result.rs` — add the `coverage: Option<CoverageReport>` field to the envelope.

---

## Task 1: `check-coverage` produce-only mode + expose the rich report in `$out`

**Files:**
- Modify: `scripts/check-coverage` (add `--emit` mode), `flake.nix` (coverage check `installPhaseCommand`)

**Interfaces:**
- Produces: when run with `--emit`, `check-coverage` runs the two instrumented passes, writes `.coverage-report.txt` + `.coverage-report.lcov` + `.crap-report.json`, and exits 0 on test success **without** the coverage/CRAP regression jq gating. The Nix `coverage` check's `$out` gains `coverage-report.txt` and `crap-report.json`.

- [ ] **Step 1: Add the `--emit` flag to `check-coverage`**

In the arg loop, add `--emit) EMIT=true ;;` (init `EMIT=false`). After the report/LCOV/crap-report are generated (the non-`--investigate` branch already produces `$REPORT_FILE`, `$LCOV_FILE`, `$CRAP_REPORT_FILE`), short-circuit before the gating jq:

```bash
if [[ "$EMIT" == "true" ]]; then
    echo "--- coverage: emit mode — report + CRAP produced, skipping regression gate ---"
    exit 0
fi
```

Place this immediately after `normalize_crap_report "$RAW_CRAP_REPORT" "$CRAP_REPORT_FILE"` and before the manifest/CRAP comparison logic. Test failures still abort earlier (the `cargo llvm-cov nextest` passes run under `set -e`).

- [ ] **Step 2: Point the Nix `coverage` check at `--emit` and copy the rich outputs**

In `flake.nix`, the `coverage` check (~line 1068) currently runs `bash ./scripts/check-coverage --update` and copies the manifests. Change its `buildPhaseCargoCommand` to `bash ./scripts/check-coverage --emit` and its `installPhaseCommand` to:

```nix
                installPhaseCommand = ''
                  mkdir -p $out
                  cp .coverage-report.txt .crap-report.json $out/
                '';
```

(Leave `coverage-update` ~line 931 as-is — it stays `--update` for regenerating committed baselines in Task 8.)

- [ ] **Step 3: Verify the check resolves and emits**

Run: `nix build --dry-run --accept-flake-config .#checks.x86_64-linux.coverage`
Expected: resolves cleanly.
(Do NOT run the full build here; Task 8 exercises it end-to-end.)

- [ ] **Step 4: Commit**

```bash
git add scripts/check-coverage flake.nix
git commit -m "build(coverage): produce-only --emit mode, expose text + CRAP report in $out"
```

---

## Task 2: Text-report parser (`coverage/report.rs`)

**Files:**
- Create: `xtask/src/coverage/mod.rs` (with `pub mod report;` and the shared types), `xtask/src/coverage/report.rs`
- Modify: `xtask/src/lib.rs` (add `mod coverage;`)
- Test: inline `#[cfg(test)]` in `report.rs`

**Interfaces:**
- Produces (in `coverage/mod.rs`):
  - `pub struct LineCov { pub line: u32, pub covered: bool, pub text: String }` (only executable lines)
  - `pub struct FileCoverage { pub path: String, pub lines: Vec<LineCov> }`
  - `pub fn parse_text_report(report: &str, repo_root: &str) -> Vec<FileCoverage>` (in `report.rs`) — file paths normalized to repo-relative by stripping `repo_root` + `/`.

- [ ] **Step 1: Write the failing test**

`xtask/src/coverage/report.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
/repo/server/src/x.rs:
    1|      |use std::foo;
    2|    36|pub fn bar() {
    3|     0|    fail()
    4|  1.36k|    ok()
    5|     0|    impossible() // cov:ignore
";

    #[test]
    fn parses_executable_lines_with_covered_flag_and_text() {
        let files = parse_text_report(SAMPLE, "/repo");
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "server/src/x.rs");
        // line 1 non-executable (blank count) → omitted; line 5 has `// cov:ignore` → omitted.
        assert_eq!(f.lines.iter().map(|l| l.line).collect::<Vec<_>>(), vec![2, 3, 4]);
        assert_eq!(f.lines[0].covered, true);  // 36
        assert_eq!(f.lines[1].covered, false); // 0
        assert_eq!(f.lines[2].covered, true);  // 1.36k (non-zero)
        assert_eq!(f.lines[0].text, "pub fn bar() {");
        assert!(!f.lines.iter().any(|l| l.line == 5)); // excluded by marker
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::report`
Expected: FAIL (function not defined).

- [ ] **Step 3: Implement the parser**

`xtask/src/coverage/report.rs`:

```rust
use crate::coverage::{FileCoverage, LineCov};

/// Parse `cargo llvm-cov report --text` output. A line is executable iff its
/// second pipe-delimited column is non-blank; covered iff that column is a
/// non-zero count (counts may carry a k/M suffix). File headers end in `.rs:`.
/// A line whose source text contains `// cov:ignore` is treated as
/// non-executable (our explicit exclusion escape-hatch) and omitted.
pub fn parse_text_report(report: &str, repo_root: &str) -> Vec<FileCoverage> {
    let prefix = format!("{}/", repo_root.trim_end_matches('/'));
    let mut files: Vec<FileCoverage> = Vec::new();
    for line in report.lines() {
        if let Some(path) = line.strip_suffix(".rs:") {
            let rel = path.strip_prefix(&prefix).unwrap_or(path).to_string();
            files.push(FileCoverage { path: rel, lines: Vec::new() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        // Format: "<lineno>|<count>|<source...>". Split into at most 3.
        let mut parts = line.splitn(3, '|');
        let (Some(num_col), Some(count_col)) = (parts.next(), parts.next()) else { continue };
        let Ok(lineno) = num_col.trim().parse::<u32>() else { continue };
        let count = count_col.trim();
        if count.is_empty() {
            continue; // non-executable
        }
        let covered = !is_zero_count(count);
        let text = parts.next().unwrap_or("").to_string();
        if text.contains("// cov:ignore") {
            continue; // explicit exclusion marker — drop from the executable set
        }
        file.lines.push(LineCov { line: lineno, covered, text });
    }
    files
}

/// A count column is "zero" only if it is literally 0 (covered iff non-zero).
fn is_zero_count(count: &str) -> bool {
    count == "0"
}
```

And `xtask/src/coverage/mod.rs`:

```rust
pub mod report;

#[derive(Clone, Debug, PartialEq)]
pub struct LineCov {
    pub line: u32,
    pub covered: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileCoverage {
    pub path: String,
    pub lines: Vec<LineCov>,
}
```

Add `mod coverage;` to `xtask/src/lib.rs`.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::report`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/mod.rs xtask/src/coverage/report.rs xtask/src/lib.rs
git commit -m "feat(xtask): parse llvm-cov text report into per-line FileCoverage"
```

---

## Task 3: `git diff` line-mapper (`coverage/diffmap.rs`)

Maps committed (HEAD) line numbers → working-tree line numbers per file, so a baseline gap can be located in the current file (or found deleted).

**Files:**
- Create: `xtask/src/coverage/diffmap.rs`
- Modify: `xtask/src/coverage/mod.rs` (`pub mod diffmap;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub struct LineMap { /* old_line -> Option<new_line> */ }` with `pub fn map(&self, old_line: u32) -> Option<u32>` (None = the old line was deleted).
  - `pub fn parse_unified_diff(diff: &str) -> std::collections::HashMap<String, LineMap>` — keyed by repo-relative new path.
  - `pub fn empty_map() -> LineMap` — identity map (for files with no diff: old N → new N).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // One line deleted at old line 2, one added after old line 3.
    const DIFF: &str = "\
diff --git a/server/src/x.rs b/server/src/x.rs
--- a/server/src/x.rs
+++ b/server/src/x.rs
@@ -1,4 +1,4 @@
 line1
-line2_old
 line3
+line_new
 line4
";

    #[test]
    fn maps_unchanged_deleted_and_shifts() {
        let maps = parse_unified_diff(DIFF);
        let m = maps.get("server/src/x.rs").unwrap();
        assert_eq!(m.map(1), Some(1)); // unchanged context
        assert_eq!(m.map(2), None);    // deleted
        assert_eq!(m.map(3), Some(3)); // context (added line lands after it)
        assert_eq!(m.map(4), Some(5)); // shifted down by the insertion
    }

    #[test]
    fn empty_map_is_identity() {
        assert_eq!(empty_map().map(42), Some(42));
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::diffmap`
Expected: FAIL.

- [ ] **Step 3: Implement the mapper**

`xtask/src/coverage/diffmap.rs`:

```rust
use std::collections::HashMap;

/// Maps old (HEAD) line numbers to new (working-tree) line numbers for one file.
/// Built by walking unified-diff hunks: context lines map 1:1 (with the running
/// offset), deleted lines map to None, added lines advance the new counter only.
#[derive(Default)]
pub struct LineMap {
    map: HashMap<u32, Option<u32>>,
    // For old lines outside any hunk, apply the cumulative offset after the
    // last hunk that precedes them. We store hunk boundaries to compute this.
    offset_after: Vec<(u32, i64)>, // (old_line_exclusive_upper, cumulative_offset)
}

impl LineMap {
    pub fn map(&self, old_line: u32) -> Option<u32> {
        if let Some(v) = self.map.get(&old_line) {
            return *v;
        }
        // Outside all hunks: new = old + offset accumulated before this line.
        let mut offset = 0i64;
        for (upper, off) in &self.offset_after {
            if old_line >= *upper {
                offset = *off;
            } else {
                break;
            }
        }
        Some((old_line as i64 + offset) as u32)
    }
}

pub fn empty_map() -> LineMap {
    LineMap::default()
}

pub fn parse_unified_diff(diff: &str) -> HashMap<String, LineMap> {
    let mut out: HashMap<String, LineMap> = HashMap::new();
    let mut cur_path: Option<String> = None;
    let mut lm = LineMap::default();
    let mut old_ln = 0u32;
    let mut new_ln = 0u32;
    let mut cum_offset = 0i64;

    let flush = |out: &mut HashMap<String, LineMap>, path: &mut Option<String>, lm: &mut LineMap| {
        if let Some(p) = path.take() {
            out.insert(p, std::mem::take(lm));
        }
    };

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            flush(&mut out, &mut cur_path, &mut lm);
            cur_path = Some(rest.to_string());
            old_ln = 0;
            new_ln = 0;
            cum_offset = 0;
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
            continue;
        }
        if let Some(h) = line.strip_prefix("@@") {
            // @@ -oldStart,oldCount +newStart,newCount @@
            if let Some((os, ns)) = parse_hunk_header(h) {
                old_ln = os;
                new_ln = ns;
            }
            continue;
        }
        if cur_path.is_none() {
            continue;
        }
        match line.chars().next() {
            Some(' ') => {
                lm.map.insert(old_ln, Some(new_ln));
                old_ln += 1;
                new_ln += 1;
            }
            Some('-') => {
                lm.map.insert(old_ln, None);
                old_ln += 1;
                cum_offset -= 1;
                lm.offset_after.push((old_ln, cum_offset));
            }
            Some('+') => {
                new_ln += 1;
                cum_offset += 1;
                lm.offset_after.push((old_ln, cum_offset));
            }
            _ => {}
        }
    }
    flush(&mut out, &mut cur_path, &mut lm);
    out
}

fn parse_hunk_header(h: &str) -> Option<(u32, u32)> {
    // h like " -1,4 +1,4 @@ ..."
    let h = h.trim_start();
    let mut it = h.split_whitespace();
    let old = it.next()?.strip_prefix('-')?;
    let new = it.next()?.strip_prefix('+')?;
    let old_start = old.split(',').next()?.parse().ok()?;
    let new_start = new.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}
```

Add `pub mod diffmap;` to `coverage/mod.rs`.

> Note: this is the core algorithm; the implementer should add a couple more tests (multiple hunks, pure-add file, pure-delete) and adjust `offset_after`/boundary handling if any fails — the property to hold is: context lines map exactly, deleted → None, and out-of-hunk lines shift by the net insertions/deletions before them.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::diffmap`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/diffmap.rs xtask/src/coverage/mod.rs
git commit -m "feat(xtask): unified-diff line-mapper (HEAD -> working tree)"
```

---

## Task 4: Accepted-uncovered baseline (`coverage/baseline.rs`)

**Files:**
- Create: `xtask/src/coverage/baseline.rs`
- Modify: `xtask/src/coverage/mod.rs` (`pub mod baseline;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub struct Gap { pub line: u32, pub text: String }`
  - `pub struct Baseline { /* path -> Vec<Gap> (accepted-uncovered lines at HEAD) */ }` with `pub fn gaps(&self, path: &str) -> &[Gap]`, `pub fn load(path: &str) -> anyhow::Result<Baseline>`, `pub fn save(&self, path: &str) -> anyhow::Result<()>`, `pub fn from_files(files: &[FileCoverage]) -> Baseline` (the uncovered lines of a full coverage set — used to regenerate the baseline in Task 8), and `pub fn set_gaps(&mut self, path: &str, gaps: Vec<Gap>)`.
  - Serialized as `.coverage-baseline.json`: `{ "<path>": [ {"line": N, "text": "..."}, ... ], ... }` with sorted keys + lines for stable diffs.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{FileCoverage, LineCov};

    #[test]
    fn from_files_collects_uncovered_lines_with_text() {
        let files = vec![FileCoverage {
            path: "a.rs".into(),
            lines: vec![
                LineCov { line: 1, covered: true, text: "ok".into() },
                LineCov { line: 2, covered: false, text: "gap".into() },
            ],
        }];
        let b = Baseline::from_files(&files);
        assert_eq!(b.gaps("a.rs"), &[Gap { line: 2, text: "gap".into() }]);
        assert_eq!(b.gaps("missing.rs"), &[] as &[Gap]);
    }

    #[test]
    fn round_trips_through_json_stably() {
        let mut b = Baseline::default();
        b.set_gaps("z.rs", vec![Gap { line: 3, text: "x".into() }]);
        b.set_gaps("a.rs", vec![Gap { line: 1, text: "y".into() }]);
        let json = b.to_json();
        // keys sorted for stable diffs
        assert!(json.find("\"a.rs\"").unwrap() < json.find("\"z.rs\"").unwrap());
        let b2 = Baseline::from_json(&json).unwrap();
        assert_eq!(b2.gaps("a.rs"), b.gaps("a.rs"));
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::baseline`
Expected: FAIL.

- [ ] **Step 3: Implement the baseline (use a `BTreeMap` for stable ordering)**

```rust
use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::coverage::FileCoverage;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    pub line: u32,
    pub text: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Baseline {
    files: BTreeMap<String, Vec<Gap>>,
}

impl Baseline {
    pub fn gaps(&self, path: &str) -> &[Gap] {
        self.files.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn set_gaps(&mut self, path: &str, mut gaps: Vec<Gap>) {
        gaps.sort_by_key(|g| g.line);
        if gaps.is_empty() {
            self.files.remove(path);
        } else {
            self.files.insert(path.to_string(), gaps);
        }
    }
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.files.keys()
    }
    pub fn from_files(files: &[FileCoverage]) -> Self {
        let mut b = Baseline::default();
        for f in files {
            let gaps: Vec<Gap> = f
                .lines
                .iter()
                .filter(|l| !l.covered)
                .map(|l| Gap { line: l.line, text: l.text.clone() })
                .collect();
            b.set_gaps(&f.path, gaps);
        }
        b
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }
    pub fn from_json(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
    pub fn load(path: &str) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Baseline::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &str) -> Result<()> {
        std::fs::write(path, self.to_json())?;
        Ok(())
    }
}
```

Add `pub mod baseline;` to `coverage/mod.rs`.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::baseline`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/baseline.rs xtask/src/coverage/mod.rs
git commit -m "feat(xtask): accepted-uncovered coverage baseline (load/save/from_files)"
```

---

## Task 5: Classifier (`coverage/classify.rs`)

The heart: given the current `FileCoverage`, the committed `Baseline`, and the `git diff` maps, produce the verdict.

**Files:**
- Create: `xtask/src/coverage/classify.rs`
- Modify: `xtask/src/coverage/mod.rs` (`pub mod classify;` + the verdict types)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces (types in `coverage/mod.rs`):
  - `pub struct FileLines { pub file: String, pub lines: Vec<u32> }`
  - `pub struct CoverageVerdict { pub regressions: Vec<FileLines>, pub new_uncovered: Vec<FileLines>, pub structural: Vec<FileLines>, pub improvements: Vec<FileLines> }` with `pub fn is_clean(&self) -> bool` (no regressions and no new_uncovered).
- `classify::classify(current: &[FileCoverage], baseline: &Baseline, maps: &HashMap<String, LineMap>) -> CoverageVerdict`.

Classification rules (per file, using the current file's executable lines + the baseline gaps mapped HEAD→current via `maps` — files absent from `maps` use `empty_map()` identity):
- For each **baseline gap** (old line `g`): map to current line `c = map(g)`.
  - `c == None` (deleted) → **structural** (heal: drop the gap).
  - `c == Some` and current line `c` is **covered** → **improvement** (heal: drop the gap).
  - `c == Some` and current line `c` is **uncovered** → accepted (still a known gap; keep). Not reported.
- For each **current uncovered executable line** `c` that is **not** the image of an accepted baseline gap:
  - if some baseline gap maps to `c` → it was handled above (accepted) — skip.
  - else look at whether `c` is the image of a baseline **covered** line: invert the map — if any old line `o` with `map(o) == Some(c)` existed and was covered at baseline → **regression**.
  - else (`c` has no baseline preimage, i.e. a newly-added line) → **new_uncovered**.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::baseline::{Baseline, Gap};
    use crate::coverage::diffmap::empty_map;
    use crate::coverage::{FileCoverage, LineCov};
    use std::collections::HashMap;

    fn fc(path: &str, lines: &[(u32, bool)]) -> FileCoverage {
        FileCoverage {
            path: path.into(),
            lines: lines.iter().map(|(l, c)| LineCov { line: *l, covered: *c, text: String::new() }).collect(),
        }
    }

    #[test]
    fn no_diff_identity_classifies_each_bucket() {
        // baseline accepts line 3 as a gap.
        let mut b = Baseline::default();
        b.set_gaps("a.rs", vec![Gap { line: 3, text: String::new() }]);
        // current: line2 covered (was covered → still covered), line3 NOW covered (improvement),
        // line4 uncovered & NEW (new_uncovered), line2... we also mark a previously-covered line uncovered:
        let cur = vec![fc("a.rs", &[(2, false), (3, true), (4, false)])];
        // empty maps → identity; but line 2 was covered at baseline (not a gap) and is now uncovered → regression.
        let maps: HashMap<String, _> = HashMap::new();
        let v = classify(&cur, &b, &maps);
        assert_eq!(v.improvements, vec![FileLines { file: "a.rs".into(), lines: vec![3] }]);
        assert_eq!(v.regressions, vec![FileLines { file: "a.rs".into(), lines: vec![2] }]);
        // line 4: with identity map it has a baseline preimage (old line 4) that was covered? No — baseline
        // only knows gaps (line 3) and the current covered set; treat any non-gap-preimage uncovered line that
        // existed at baseline as regression, brand-new as new_uncovered. Identity can't prove "new", so under
        // an identity (no-diff) map a line is "new" only if it lies beyond the baseline's known line range —
        // see classify() handling. Here line 4 is uncovered and not a gap → regression under identity.
        assert!(v.regressions.iter().any(|r| r.lines.contains(&4)));
        let _ = empty_map();
    }

    #[test]
    fn deleted_gap_is_structural() {
        let mut b = Baseline::default();
        b.set_gaps("a.rs", vec![Gap { line: 2, text: String::new() }]);
        // line 2 deleted in the diff.
        let mut maps = HashMap::new();
        let mut lm = crate::coverage::diffmap::LineMap::default();
        lm.set_for_test(2, None);
        maps.insert("a.rs".to_string(), lm);
        let cur = vec![fc("a.rs", &[(1, true)])];
        let v = classify(&cur, &b, &maps);
        assert_eq!(v.structural, vec![FileLines { file: "a.rs".into(), lines: vec![2] }]);
        assert!(v.is_clean());
    }
}
```

> The "new vs regression under an identity map" subtlety is real: with no diff for a file, every current line has an identity preimage, so a non-gap uncovered line is a `regression` (it was covered before, now isn't) — `new_uncovered` only arises for lines the diff marks as added (no preimage). The classifier must therefore treat **added** current lines (those with no `old` line mapping to them) as the `new_uncovered` candidates, and all others as `regression` candidates. Add a `LineMap::set_for_test` helper under `#[cfg(test)]` and an inverse lookup (added-line set) to support this.

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::classify`
Expected: FAIL.

- [ ] **Step 3: Implement the classifier**

Implement `classify()` per the rules above. Build, per file, from the `LineMap`: (a) `gap_image: Set<u32>` = images of baseline gaps that survived (`map(g) == Some(c)`); (b) `added: Set<u32>` = current lines with no old preimage (the map's new lines that aren't the image of any old line — expose `LineMap::added_lines()` returning the set of new line numbers introduced by `+` hunks). Then:

```rust
use std::collections::{HashMap, HashSet};

use crate::coverage::baseline::Baseline;
use crate::coverage::diffmap::{empty_map, LineMap};
use crate::coverage::{CoverageVerdict, FileCoverage, FileLines};

pub fn classify(
    current: &[FileCoverage],
    baseline: &Baseline,
    maps: &HashMap<String, LineMap>,
) -> CoverageVerdict {
    let mut v = CoverageVerdict::default();
    let default_map = empty_map();

    for f in current {
        let map = maps.get(&f.path).unwrap_or(&default_map);
        let covered_now: HashSet<u32> = f.lines.iter().filter(|l| l.covered).map(|l| l.line).collect();
        let uncovered_now: HashSet<u32> = f.lines.iter().filter(|l| !l.covered).map(|l| l.line).collect();
        let added: HashSet<u32> = map.added_lines();

        // 1. Walk baseline gaps.
        let mut accepted_images: HashSet<u32> = HashSet::new();
        let (mut structural, mut improvements) = (Vec::new(), Vec::new());
        for g in baseline.gaps(&f.path) {
            match map.map(g.line) {
                None => structural.push(g.line),
                Some(c) if covered_now.contains(&c) => improvements.push(c),
                Some(c) => {
                    accepted_images.insert(c);
                } // still an accepted gap
            }
        }
        push_nonempty(&mut v.structural, &f.path, structural);
        push_nonempty(&mut v.improvements, &f.path, improvements);

        // 2. Walk current uncovered lines not covered by an accepted gap image.
        let (mut regr, mut newu) = (Vec::new(), Vec::new());
        for &c in &uncovered_now {
            if accepted_images.contains(&c) {
                continue; // known, unchanged gap
            }
            if added.contains(&c) {
                newu.push(c);
            } else {
                regr.push(c);
            }
        }
        regr.sort();
        newu.sort();
        push_nonempty(&mut v.regressions, &f.path, regr);
        push_nonempty(&mut v.new_uncovered, &f.path, newu);
    }
    v
}

fn push_nonempty(into: &mut Vec<FileLines>, file: &str, mut lines: Vec<u32>) {
    if !lines.is_empty() {
        lines.sort();
        into.push(FileLines { file: file.to_string(), lines });
    }
}
```

Add to `coverage/mod.rs`: the `FileLines`/`CoverageVerdict` types (with `#[derive(Default, PartialEq, Debug)]`), `CoverageVerdict::is_clean`, and to `diffmap.rs`: `LineMap::added_lines() -> HashSet<u32>` (track new line numbers introduced by `+` hunks) and `#[cfg(test)] LineMap::set_for_test`.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::classify`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/classify.rs xtask/src/coverage/mod.rs xtask/src/coverage/diffmap.rs
git commit -m "feat(xtask): line-identity coverage classifier (regression/new/structural/improvement)"
```

---

## Task 6: Host-side CRAP comparison (`coverage/crap.rs`)

**Files:**
- Create: `xtask/src/coverage/crap.rs`
- Modify: `xtask/src/coverage/mod.rs` (`pub mod crap;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces: `pub struct CrapRegression { pub file: String, pub function: String, pub old: f64, pub new: f64 }`, and `crap::compare(new_report: &str, old_manifest: &str) -> anyhow::Result<Vec<CrapRegression>>` — mirrors today's jq: key `(crate|file|function|line)`, flag when `new.crap > old.crap + 0.01` for keys present in both.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
    const NEW_WORSE: &str = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":3.0}]}"#;
    const NEW_SAME: &str = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.005}]}"#;

    #[test]
    fn flags_worse_crap_beyond_epsilon() {
        let r = compare(NEW_WORSE, OLD).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].function, "f");
    }

    #[test]
    fn ignores_sub_epsilon_noise() {
        assert!(compare(NEW_SAME, OLD).unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::crap`
Expected: FAIL.

- [ ] **Step 3: Implement the comparison** (serde over the `{entries:[{crate,file,function,line,crap}]}` shape; build a `HashMap<(String,String,String,i64), f64>` from old, flag new entries exceeding `old + 0.01`). Provide the full impl.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage::crap`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/crap.rs xtask/src/coverage/mod.rs
git commit -m "feat(xtask): host-side CRAP regression comparison"
```

---

## Task 7: Orchestrate — gate, auto-heal, JSON, wire into `check`/`validate`

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (the `run` entry), `xtask/src/result.rs` (envelope `coverage` field), `xtask/src/steps/nix.rs` (after `nix-coverage` builds, post-process its `$out`), `xtask/src/lib.rs` (pass repo root / mode)
- Test: inline `#[cfg(test)]` for the heal logic

**Interfaces:**
- Produces:
  - `CoverageReport` serde struct → the envelope `.coverage` block: `{ regressions, new_uncovered, structural, improvements, crap: { regressions }, healed }` (each `FileLines`-shaped).
  - `coverage::run(out_dir: &str, mode: Mode) -> (StepResult, Option<CoverageReport>)`: reads `<out_dir>/coverage-report.txt` + `<out_dir>/crap-report.json`, runs `git diff` (`git diff --unified=0 HEAD --` for tracked files), loads `.coverage-baseline.json` + `.crap-manifest.json`, classifies, computes CRAP regressions, decides pass/fail (`fail` iff regressions or new_uncovered or CRAP regressions), and in `Mode::Fix` **auto-heals** the baseline (drop healed gaps; never add) and the CRAP manifest, writing them back and setting `healed=true`.

- [ ] **Step 1: Wire `nix.rs` to post-process the coverage `$out`**

After `build_check("nix-coverage", "coverage")` succeeds, the GC-root symlink `.xtask/gcroots/coverage` points at the check `$out`. Call `coverage::run(".xtask/gcroots/coverage", mode)` and merge its `StepResult` + attach the `CoverageReport` to the command result. (Pass `mode` from `coverage()`/`e2e()` callers; `check` → `Mode::Fix`, `validate` → `Mode::Check`.)

- [ ] **Step 2: Implement `coverage::run` + heal**, with a `#[cfg(test)]` test that: given a baseline gap that's now covered, `Mode::Fix` removes it from the saved baseline and sets `healed=true`; a `regression` makes the step fail and is never healed.

- [ ] **Step 3: Add the `coverage` field to the result envelope** (`#[serde(skip_serializing_if = "Option::is_none")]`).

- [ ] **Step 4: Verify the unit/heal tests pass + the crate builds**

Run: `cargo test --manifest-path xtask/Cargo.toml coverage`
Run: `cargo build --manifest-path xtask/Cargo.toml`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/mod.rs xtask/src/result.rs xtask/src/steps/nix.rs xtask/src/lib.rs
git commit -m "feat(xtask): coverage gate + auto-heal + JSON, wired into check/validate"
```

---

## Task 8: Regenerate & commit the initial per-line baseline; end-to-end run

**Files:**
- Create: `.coverage-baseline.json` (committed)
- Delete: `.coverage-manifest.json` (superseded by the per-line baseline; CRAP's `.crap-manifest.json` stays)

- [ ] **Step 1: Produce a full coverage report from Nix**

Run: `nix build --accept-flake-config --out-link .xtask/gcroots/coverage .#checks.x86_64-linux.coverage` (SLOW — the real instrumented run; let it finish). Confirm `.xtask/gcroots/coverage/coverage-report.txt` exists.

- [ ] **Step 2: Generate the baseline from the report**

Add a one-shot `cargo xtask __regen-baseline` (hidden) that parses `<out>/coverage-report.txt` via `Baseline::from_files(parse_text_report(...))` and writes `.coverage-baseline.json`. Run it.

- [ ] **Step 3: Sanity-check + end-to-end gate**

Run: `cargo xtask validate --no-e2e` (uses the cached coverage build). Expected: green — the freshly-generated baseline matches current coverage exactly, so zero regressions/new_uncovered. `jq '.coverage' .xtask/last-result.json` shows empty deltas.

- [ ] **Step 4: Remove the superseded percentage manifest + commit**

```bash
git rm .coverage-manifest.json
git add .coverage-baseline.json
git commit -m "feat(coverage): commit per-line accepted-uncovered baseline; retire percentage manifest"
```

> Note: `scripts/check-coverage`'s legacy `--update`/`--check` percentage paths and `.coverage-manifest.json` references become dead once xtask owns the gate; their removal (and the `coverage-update` package) is part of the B4 cutover, not B2 — leave them for now so the regen path (`--update`) and any CI references keep working until cutover.

---

## Self-review notes (for the implementer)

- **Rich data, no inference:** the check emits the full `cargo llvm-cov report --text` (per-line number + count + source text) and the CRAP report; xtask never guesses which lines are executable or what they contain.
- **Strict ratchet / shrink-only:** `new_uncovered` and `regression` fail; the accepted-uncovered baseline only shrinks via heal. The exclusion escape-hatch is the `// cov:ignore` marker, honored host-side by the Task 2 parser (`cargo-llvm-cov` 0.8.3 on stable has no line-level exclusion of its own).
- **The diff-mapper (Task 3) and classifier (Task 5) are the subtle parts** — they carry the most tests; if a property fails, fix the mapper, not the classifier's rules.
- **Type consistency:** `LineCov`/`FileCoverage` (Task 2), `LineMap`/`parse_unified_diff`/`empty_map`/`added_lines` (Task 3), `Gap`/`Baseline` (Task 4), `FileLines`/`CoverageVerdict`/`classify` (Task 5), `CrapRegression`/`compare` (Task 6), `CoverageReport`/`coverage::run` (Task 7) are the cross-task names.
- **No CI changes; scripts not retired** — that's B3 (postgres-integration collapse) and B4 (cutover).
