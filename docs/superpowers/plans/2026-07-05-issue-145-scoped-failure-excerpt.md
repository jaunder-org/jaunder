# Scoped nix-build failure-excerpt (#145) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating a task to a subagent via **jaunder-dispatch**
> when useful). Steps use checkbox (`- [ ]`) syntax.

**Spec:**
[`docs/superpowers/specs/2026-07-05-issue-145-scoped-failure-excerpt.md`](../specs/2026-07-05-issue-145-scoped-failure-excerpt.md)
— the "what/why". This plan is the "how".

**Goal:** On a failed Nix check, write a scoped `failure-excerpt.log` (nix's
`error:` block) alongside the full `build.log`, and name it first in the failure
detail.

**Architecture:** All in `xtask/src/steps/nix.rs`. A pure `failure_excerpt`
carves nix's `error:`→EOF block (which already de-interleaves the failing
builder's tail); a thin `write_failure_excerpt` writes it next to `build.log`;
`failure_detail` names it first; `build_check` calls it on failure and adds
`--log-lines 50`.

**Tech Stack:** Rust (xtask), `nix build -L`.

## Global Constraints

- **Carve rule:** excerpt = from the first line where
  `line.starts_with("error:")` (column 0 — `-L` builder lines are prefixed
  `<name>> `) through EOF.
- **Fallback (no `error:` line):** the last **50** lines of `build.log`,
  prefixed with the marker
  ``=== no `error:` block in build log; last 50 lines: ===``. (Distinct
  mechanism from `--log-lines 50`, which sizes nix's in-block builder tail.)
- **`--log-lines 50`** added to `build_check`'s `nix build` args.
- **Excerpt path:** `<parent of build.log>/failure-excerpt.log` (i.e.
  `.xtask/diagnostics/<check>/failure-excerpt.log`). `.xtask/` is gitignored; CI
  already uploads `.xtask/diagnostics/` (`validate-diagnostics`,
  `if: always()`).
- **`build.log` retained** and still named as the fallback artifact. No
  CI/`flake.nix` change.
- **Commits:** run `cargo xtask check` clean first (**jaunder-commit**). **No
  `Co-Authored-By` trailer.**

---

## Review header

**Scope — in:** `xtask/src/steps/nix.rs` (excerpt carve + write +
`failure_detail` + `build_check` wiring + `--log-lines 50`) and a
`CONTRIBUTING.md` note. **Out:** drv-prefix parsing, `rescue_diagnostics`, `-L`
streaming, CI upload, #144. **Separable concerns:** none.

**Tasks:**

1. Pure `failure_excerpt` carve + fallback (unit-tested) — AC4.
2. `write_failure_excerpt` + `failure_detail` `Option` param +
   `--log-lines 50` + `build_check` wiring — AC1, AC2, AC3, AC5, AC7.
3. CONTRIBUTING "read `failure-excerpt.log` first" note — AC6.

**Execution note:** Tasks 1 and 2 landed in one commit — a pure helper used only
by `#[cfg(test)]` code (Task 1 alone) is dead code to the non-test build and
trips `xtask-clippy`'s `-D warnings`, so it must be wired into production
(Task 2) in the same commit.

**Key risks/decisions:** the carve is grounded in a captured real `nix build -L`
failure (spec) — nix de-interleaves the failing builder's tail into its `error:`
block, so no fragile prefix parsing. The pure `failure_excerpt` carries the
algorithmic risk (AC4); AC1/AC2/AC5 are thin I/O + string wiring, covered by a
temp-dir `write_failure_excerpt` test + the updated `failure_detail` test +
inspection.

---

### Task 1: Pure `failure_excerpt` carve + fallback

**Files:**

- Modify: `xtask/src/steps/nix.rs` (add `failure_excerpt` + a const; in-file
  tests)

**Interfaces:**

- Produces: `fn failure_excerpt(build_log: &str) -> String` (private);
  `const EXCERPT_FALLBACK_LINES: usize = 50`.

- [x] **Step 1: Write the failing tests** (in the existing
      `#[cfg(test)] mod tests`; add `failure_excerpt` to the `use super::{…}`
      import)

```rust
const SAMPLE_LOG: &str = "\
fail-probe> build-output-line-1
other-drv> interleaved noise from a parallel derivation
fail-probe> build-output-line-2
fail-probe> FATAL_ERROR_MARKER
error: Cannot build '/nix/store/xxx-fail-probe-0.1.0.drv'.
       Reason: builder failed with exit code 3.
       Last 3 log lines:
       > build-output-line-58
       > build-output-line-59
       > FATAL_ERROR_MARKER
       For full logs, run:
         nix log /nix/store/xxx-fail-probe-0.1.0.drv
";

#[test]
fn failure_excerpt_carves_error_block_dropping_interleaved_head() {
    let e = failure_excerpt(SAMPLE_LOG);
    assert!(e.starts_with("error: Cannot build"), "starts at the error block: {e:?}");
    assert!(e.contains("Last 3 log lines"));
    assert!(e.contains("FATAL_ERROR_MARKER"));
    // The interleaved -L head (prefixed builder lines) is excluded.
    assert!(!e.contains("interleaved noise"));
    assert!(!e.contains("fail-probe> build-output-line-1"));
}

#[test]
fn failure_excerpt_falls_back_to_tail_when_no_error_block() {
    // 60 numbered lines, no column-0 `error:` line.
    let log: String = (1..=60).map(|i| format!("plain-line-{i}\n")).collect();
    let e = failure_excerpt(&log);
    assert!(e.contains("no `error:` block"), "marker present: {e:?}");
    assert!(e.contains("plain-line-60")); // last line kept
    assert!(e.contains("plain-line-11")); // first kept line (last 50 of 60)
    assert!(!e.contains("plain-line-10")); // trimmed head
}

#[test]
fn failure_excerpt_ignores_error_prefixed_by_a_builder() {
    // A builder printing its own `error:` streams as `<name>> error:` — NOT column 0,
    // so it must not be treated as nix's error block.
    let log = "drv> error: cargo test failed\ndrv> more\nerror: builder for 'x' failed\n";
    let e = failure_excerpt(log);
    assert!(e.starts_with("error: builder for 'x' failed"));
    assert!(!e.contains("drv> error"));
}
```

- [x] **Step 2: Run, verify FAIL**

Run:
`cargo nextest run --manifest-path xtask/Cargo.toml steps::nix::tests::failure_excerpt`
Expected: FAIL — `failure_excerpt` undefined.

- [x] **Step 3: Implement**

```rust
/// Lines of `build.log` kept in the fallback excerpt when the log carries no nix
/// `error:` block (an unusual failure). Distinct from the `--log-lines 50` that sizes
/// nix's in-block builder tail on the normal path.
const EXCERPT_FALLBACK_LINES: usize = 50;

/// Carve a scoped failure excerpt from a captured `nix build -L` log. Nix's own error
/// summary is a self-contained block at column 0 (`error: …` through EOF) that names the
/// failing derivation and includes its *de-interleaved* `Last N log lines` tail — exactly
/// the scoped content we want, no drv/prefix parsing needed (builder output streams
/// prefixed `<name>> …`, so it never matches at column 0). If there is no such block,
/// fall back to the log's last [`EXCERPT_FALLBACK_LINES`] lines behind a marker so the
/// excerpt is never empty.
fn failure_excerpt(build_log: &str) -> String {
    let lines: Vec<&str> = build_log.lines().collect();
    if let Some(i) = lines.iter().position(|l| l.starts_with("error:")) {
        return lines[i..].join("\n");
    }
    let start = lines.len().saturating_sub(EXCERPT_FALLBACK_LINES);
    // Interpolate the const so the marker text can't silently desync from the slice.
    let mut out = format!("=== no `error:` block in build log; last {EXCERPT_FALLBACK_LINES} lines: ===\n");
    out.push_str(&lines[start..].join("\n"));
    out
}
```

- [x] **Step 4: Run, verify PASS**

Run:
`cargo nextest run --manifest-path xtask/Cargo.toml steps::nix::tests::failure_excerpt`
Expected: PASS (3 tests).

- [x] **Step 5: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "feat(xtask): failure_excerpt — carve nix's error block from a build log (#145)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 2: Wire it into `build_check` + `failure_detail`

**Files:**

- Modify: `xtask/src/steps/nix.rs` (`write_failure_excerpt`; `failure_detail`
  signature; `build_check` args + failure arm; update the existing
  `failure_detail` test; add a `write_failure_excerpt` test)

**Interfaces:**

- Consumes: `failure_excerpt` (Task 1); `build_check`'s `log_path`;
  `failure_detail`.
- Produces:
  - `fn write_failure_excerpt(log_path: &str) -> Option<String>` — writes
    `<parent>/failure-excerpt.log`, returns its path (or `None` if the log is
    unreadable). **Intentionally simplifies the spec's `(check, log_path)`
    signature** — the excerpt is a sibling of `build.log`, so the dir is derived
    from `log_path`'s parent; `check` is redundant.
  - `failure_detail(installable: &str, status: &ExitStatus, excerpt_path: Option<&str>, log_path: &str) -> String`.
- Import: extend the test module's `use super::{…}` to
  `use super::{failure_detail, failure_excerpt, write_failure_excerpt, MultiWriter};`.

- [x] **Step 1: Write/adjust the failing tests**

Update the existing `failure_detail_names_installable_status_and_log_path` to
the new signature (both branches), and add a temp-dir `write_failure_excerpt`
test:

```rust
#[test]
fn failure_detail_names_excerpt_first_then_full_log() {
    let status = std::process::Command::new("false").status().unwrap();
    let with = failure_detail(
        ".#checks.x86_64-linux.e2e",
        &status,
        Some(".xtask/diagnostics/e2e/failure-excerpt.log"),
        ".xtask/diagnostics/e2e/build.log",
    );
    assert!(with.contains("failure-excerpt.log"));
    assert!(with.contains("full build log: .xtask/diagnostics/e2e/build.log"));
    // Excerpt named before the full log.
    assert!(with.find("failure-excerpt.log").unwrap() < with.find("full build log").unwrap());

    let without = failure_detail(".#checks.x86_64-linux.e2e", &status, None, ".xtask/diagnostics/e2e/build.log");
    assert!(without.contains("full build log: .xtask/diagnostics/e2e/build.log"));
    assert!(!without.contains("failure-excerpt.log"));
}

#[test]
fn write_failure_excerpt_writes_sibling_carved_file() {
    let dir = std::env::temp_dir().join(format!("xtask-excerpt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("build.log");
    std::fs::write(&log, SAMPLE_LOG).unwrap(); // reuse Task 1's module-level SAMPLE_LOG const
    let path = write_failure_excerpt(log.to_str().unwrap()).unwrap();
    assert!(path.ends_with("failure-excerpt.log"));
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.starts_with("error: Cannot build"));
    assert!(!body.contains("interleaved noise"));
    std::fs::remove_dir_all(&dir).ok();
}
```

(If `SAMPLE_LOG` scoping is awkward, hoist it to a module-level `const` in the
test module.) Delete the old
`failure_detail_names_installable_status_and_log_path` (superseded by the new
one).

- [x] **Step 2: Run, verify FAIL**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml steps::nix` Expected:
FAIL — `write_failure_excerpt` undefined; `failure_detail` arity mismatch.

- [x] **Step 3: Implement**

```rust
use std::path::Path;

/// On a failed check, write the scoped [`failure_excerpt`] of the captured build log to
/// `failure-excerpt.log` beside it. Best-effort: `None` if the log is unreadable (then
/// only the full log is named).
fn write_failure_excerpt(log_path: &str) -> Option<String> {
    let log = std::fs::read_to_string(log_path).ok()?;
    let dir = Path::new(log_path).parent().unwrap_or(Path::new("."));
    let excerpt_path = dir.join("failure-excerpt.log");
    std::fs::write(&excerpt_path, format!("{}\n", failure_excerpt(&log))).ok()?;
    Some(excerpt_path.to_string_lossy().into_owned())
}
```

`failure_detail` (replace the body):

```rust
fn failure_detail(
    installable: &str,
    status: &std::process::ExitStatus,
    excerpt_path: Option<&str>,
    log_path: &str,
) -> String {
    match excerpt_path {
        Some(e) => format!(
            "nix build {installable} exited with {status}; scoped excerpt (read first): {e}; full build log: {log_path}"
        ),
        None => format!("nix build {installable} exited with {status}; full build log: {log_path}"),
    }
}
```

`build_check`: add `"--log-lines", "50"` to the `.args([…])` list (e.g. right
after `"-L"`), and rewrite the failure arm:

```rust
Ok(s) => {
    rescue_diagnostics(check);
    let excerpt = write_failure_excerpt(&log_path);
    StepResult::fail(step_name).detail(failure_detail(&installable, &s, excerpt.as_deref(), &log_path))
}
```

- [x] **Step 4: Run, verify PASS**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml steps::nix` Expected:
PASS (Task 1 + the two updated/new tests + existing MultiWriter tests).

- [x] **Step 5: Verify AC3 + AC5 by inspection**

`rg -n 'log-lines' xtask/src/steps/nix.rs` shows `--log-lines 50` in
`build_check`'s args (AC3); the `build.log` `File::create` + `MultiWriter` drain
is unchanged and still named in `failure_detail` (AC5).

- [x] **Step 6: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "feat(xtask): write scoped failure-excerpt.log on a failed nix check (#145)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 3: Documentation

**Files:**

- Modify: `CONTRIBUTING.md` (near the #144 "Failure logs — look here first"
  note, ~line 274)

**Interfaces:**

- Consumes: the shipped excerpt behavior.

- [x] **Step 1: Add the note**

Add a short bullet (matching the #144 note's style): on a failed
`cargo xtask check`/`validate` Nix check, **read
`.xtask/diagnostics/<check>/failure-excerpt.log` first** — it is nix's scoped
`error:` block (the failing builder's de-interleaved tail), not the interleaved
`-L` firehose; the full `build.log` beside it is the fallback. Note it is
CI-uploaded (`validate-diagnostics`).

- [x] **Step 2: Verify docs formatting**

Run: `cargo xtask check` — Expected: PASS (prettier covers Markdown).

- [x] **Step 3: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs(xtask): read failure-excerpt.log first on a red nix check (#145)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

## Self-review

- **Spec coverage:** AC1 → T2 (`write_failure_excerpt` + wiring; temp-dir test);
  AC2 → T2 (`failure_detail` `Option` + test); AC3 → T2/S5 (inspection); AC4 →
  T1 (pure tests); AC5 → T2/S5 (build.log unchanged); AC6 → T3; AC7 → T2
  (updated test) + `cargo xtask check` at each commit. All mapped.
- **Placeholders:** none — real Rust + exact `cargo nextest` commands.
- **Type consistency:** `failure_excerpt(&str)->String` (T1) consumed by
  `write_failure_excerpt(&str)->Option<String>` (T2);
  `failure_detail(installable, status, Option<&str>, log_path)` matches the
  single caller in `build_check` and the rewritten test; `excerpt.as_deref()`
  yields the `Option<&str>`.
