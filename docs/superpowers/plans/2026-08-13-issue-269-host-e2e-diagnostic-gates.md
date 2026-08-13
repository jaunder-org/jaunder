# Host E2E Zero-Panic Gate — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cargo xtask e2e-local` reject server panics through the same
single verifier used by every NixOS-VM e2e check.

**Architecture:** A deep `test_support::panic_gate` module owns byte scanning,
allowlisting, location deduplication, diagnostic-path resolution, and reporting
behind one `verify_no_panics(capture_dir, server_log)` interface. The
`test-support verify-no-panics` CLI is the process seam used by both Nix and
`xtask`; the host driver adds a streaming stderr tee so its server log can fill
the VM journal role without losing live output.

**Tech Stack:** Rust 2024, `test-support`, `xtask`, `std::process`, NixOS test
Python embedded in `flake.nix`, `cargo nextest`, Playwright.

**Spec:**
[`2026-08-13-issue-269-host-e2e-diagnostic-gates.md`](../specs/2026-08-13-issue-269-host-e2e-diagnostic-gates.md)

## Review header

**Scope — in:** one byte-oriented panic verifier and CLI; VM migration from
inline Python; host stderr streaming, teardown/drain, verifier invocation, and
independent failure aggregation; architecture documentation; targeted host/VM
smokes and the full local gate.

**Scope — out:** treating WARN+ diagnostics as failures; host OTel (#802);
persistent host artifacts; Playwright/browser/retry changes; capture filename
changes; runtime panic exemptions; a new ADR.

**Tasks:**

1. Build and test the shared `test-support` panic verifier and CLI.
2. Replace the VM's inline parser with the shared verifier and smoke one VM
   combo.
3. Mirror host stderr, enforce the shared verifier after Playwright, document
   the shared flow, and run the complete gate.

**Key risks/decisions:**

- Scan bytes, not JSON or UTF-8 text: a torn or invalid record containing the
  raw panic marker must still fail.
- Accept the capture directory, then resolve `Stream::Diag` in Rust; never
  restate `diag.log` in Nix or `xtask`.
- The host mirror thread starts immediately after spawn, writes each chunk to
  live stderr and the per-run file, and is joined only after stopping/reaping
  the child so pipe EOF is guaranteed.
- A Playwright failure is stored, not returned immediately. Server shutdown,
  mirror drain, and panic verification still execute; both final failures become
  distinct `CommandResult` steps.
- No separable concern needs filing: host OTel already exists as #802.

## Global Constraints

- Preserve ADR-0032's raw union, location deduplication, scoped-record
  preference, and default-deny policy.
- `ALLOWED_PANICS` remains source-controlled and empty; add no flag or env
  override.
- Preserve ADR-0057: `host::capture::Stream::Diag` remains the only diagnostic
  filename definition.
- The required server-log path must fail loudly when missing or unreadable; a
  missing diagnostic stream is empty input.
- Host server stderr stays live and byte-identical while capture remains
  per-run/temporary; do not buffer the whole log in memory.
- Keep server kill/reap cleanup on every exit path.
- Invoke repository tools through `devtool run --`; use exact commands below.
- Before every commit, follow `jaunder-commit`: stage the intended tree, run
  `devtool run -- cargo xtask check`, restage any formatter changes, then
  commit.
- No lint suppression and no `Co-Authored-By` trailer.

---

### Task 1: Shared byte-oriented panic verifier and CLI

**Files:**

- Create: `test-support/src/panic_gate.rs`
- Modify: `test-support/src/lib.rs` — export the panic-gate module.
- Modify: `test-support/src/main.rs` — add and dispatch `verify-no-panics`.
- Modify: `test-support/tests/cli.rs` — process-seam smoke tests.
- Include in first commit:
  `docs/superpowers/specs/2026-08-13-issue-269-host-e2e-diagnostic-gates.md` and
  `docs/superpowers/plans/2026-08-13-issue-269-host-e2e-diagnostic-gates.md`.

**Interfaces:**

- Consumes: `host::capture::Stream::Diag.filename()` and two filesystem paths.
- Produces:

```rust
pub mod panic_gate;

pub fn verify_no_panics(
    capture_dir: &std::path::Path,
    server_log: &std::path::Path,
) -> anyhow::Result<()>;
```

- Produces CLI:
  `test-support verify-no-panics --capture-dir <DIR> --server-log <FILE>`. Exit
  0 means no panic; non-zero stderr names either the input error or every
  detected panic record.
- Keeps private: `const ALLOWED_PANICS: &[&[u8]] = &[]`, byte-line collection,
  location-key extraction, deduplication, and rendering.

- [x] **Step 1: Add failing module tests for every verifier branch**

Add `pub mod panic_gate;` in `lib.rs`, create `panic_gate.rs`, and place the
following tests under `#[cfg(test)]`. They intentionally call the
not-yet-defined interface, so the first run fails to compile.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("write fixture");
    }

    fn verify(diag: Option<&[u8]>, server: &[u8]) -> anyhow::Result<()> {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("capture");
        std::fs::create_dir_all(&capture).expect("capture dir");
        if let Some(bytes) = diag {
            write(&capture.join(host::capture::Stream::Diag.filename()), bytes);
        }
        let server_log = dir.path().join("server.log");
        write(&server_log, server);
        verify_no_panics(&capture, &server_log)
    }

    #[test]
    fn clean_non_json_and_invalid_utf8_without_marker_pass() {
        verify(Some(b"not json\n\xff warning\n"), b"ordinary stderr\n")
            .expect("non-panic bytes are clean");
    }

    #[test]
    fn absent_optional_diag_passes() {
        verify(None, b"ordinary stderr\n").expect("missing diag is empty");
    }

    #[test]
    fn server_only_raw_panic_fails() {
        let error = verify(None, b"thread panicked at src/server.rs:7:9:\nboom\n")
            .expect_err("server panic must fail")
            .to_string();
        assert!(error.contains("src/server.rs:7:9"), "{error}");
    }

    #[test]
    fn diag_only_torn_invalid_utf8_panic_fails() {
        let error = verify(
            Some(b"\xff{torn panicked at src/diag.rs:4:2: boom\n"),
            b"clean\n",
        )
        .expect_err("raw marker must fail without JSON or UTF-8")
        .to_string();
        assert!(error.contains("src/diag.rs:4:2"), "{error}");
    }

    #[test]
    fn marker_without_location_still_fails() {
        let error = verify(Some(b"torn panicked at\n"), b"clean\n")
            .expect_err("marker-only line must fail")
            .to_string();
        assert!(error.contains("torn panicked at"), "{error}");
    }

    #[test]
    fn same_location_is_reported_once_with_diag_preferred() {
        let error = verify(
            Some(b"scoped panicked at src/shared.rs:12:5: scoped payload\n"),
            b"journal panicked at src/shared.rs:12:5:\nlegacy payload\n",
        )
        .expect_err("duplicate panic must fail")
        .to_string();
        assert_eq!(error.matches("src/shared.rs:12:5").count(), 1, "{error}");
        assert!(error.contains("scoped payload"), "{error}");
        assert!(!error.contains("legacy payload"), "{error}");
    }

    #[test]
    fn distinct_locations_are_all_reported() {
        let error = verify(
            Some(b"panicked at src/a.rs:1:2: a\n"),
            b"panicked at src/b.rs:3:4: b\n",
        )
        .expect_err("both panics must fail")
        .to_string();
        assert!(error.contains("src/a.rs:1:2"), "{error}");
        assert!(error.contains("src/b.rs:3:4"), "{error}");
    }

    #[test]
    fn unreadable_present_diag_is_infrastructure_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("capture");
        let diag = capture.join(host::capture::Stream::Diag.filename());
        std::fs::create_dir_all(&diag).expect("directory at diagnostic file path");
        let server = dir.path().join("server.log");
        write(&server, b"clean\n");

        let error = verify_no_panics(&capture, &server)
            .expect_err("present diagnostic stream must be readable")
            .to_string();
        assert!(error.contains("diagnostic log"), "{error}");
        assert!(error.contains(host::capture::Stream::Diag.filename()), "{error}");
    }

    #[test]
    fn required_server_log_read_failure_is_infrastructure_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("capture");
        std::fs::create_dir_all(&capture).expect("capture dir");
        let missing = dir.path().join("missing-server.log");
        let error = verify_no_panics(&capture, &missing)
            .expect_err("required server log must be readable")
            .to_string();
        assert!(error.contains("server log"), "{error}");
        assert!(error.contains("missing-server.log"), "{error}");
    }
}
```

- [x] **Step 2: Run the module tests and verify the interface is absent**

Run: `devtool run -- cargo nextest run -p test-support panic_gate`

Expected: **FAIL** — `verify_no_panics` is not defined.

- [x] **Step 3: Implement the deep verifier module**

Implement the public interface exactly as declared. Internally:

1. Resolve the optional diagnostic path with
   `capture_dir.join(host::capture::Stream::Diag.filename())`.
2. Read the required server log or return contextual `anyhow` failure naming
   `server log` and the path. Treat `ErrorKind::NotFound` only for the
   diagnostic path as empty input; propagate every other diagnostic read error.
3. Iterate raw byte lines. A line matches when it contains the byte substring
   `b"panicked at"` and no entry in the private empty `ALLOWED_PANICS` matches.
   Do not call `from_utf8` to decide whether a line matches.
4. Derive the deduplication key from the non-whitespace token immediately after
   `b"panicked at "`, stripping trailing `b':'`. When no location token exists,
   use the entire marker-bearing line as a fallback key so it cannot be dropped.
   Scan diagnostic lines first and insert server lines only when the key is
   absent.
5. If reports remain, return one `anyhow` error headed
   `e2e zero-panic gate: server logged Rust panic(s):` and render offending
   bytes with `String::from_utf8_lossy` only at the human-reporting edge.

The tests pin all observable branches; keep the byte-search and map helpers
private rather than expanding the interface.

- [x] **Step 4: Run the verifier tests**

Run: `devtool run -- cargo nextest run -p test-support panic_gate`

Expected: **PASS** — all nine verifier cases pass.

- [x] **Step 5: Add failing CLI seam tests**

Extend `test-support/tests/cli.rs` with complete subprocess tests:

```rust
#[test]
fn verify_no_panics_cli_accepts_clean_capture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture = dir.path().join("capture");
    std::fs::create_dir_all(&capture).expect("capture dir");
    let server = dir.path().join("server.log");
    std::fs::write(&server, b"clean stderr\n").expect("server log");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["verify-no-panics", "--capture-dir"])
        .arg(&capture)
        .arg("--server-log")
        .arg(&server)
        .output()
        .expect("spawn verifier");

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn verify_no_panics_cli_reports_panic_and_exits_nonzero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture = dir.path().join("capture");
    std::fs::create_dir_all(&capture).expect("capture dir");
    let server = dir.path().join("server.log");
    std::fs::write(&server, b"panicked at src/cli.rs:8:3: boom\n")
        .expect("server log");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["verify-no-panics", "--capture-dir"])
        .arg(&capture)
        .arg("--server-log")
        .arg(&server)
        .output()
        .expect("spawn verifier");

    assert!(!out.status.success(), "panic must fail CLI");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("src/cli.rs:8:3"), "{stderr}");
}
```

Run: `devtool run -- cargo nextest run -p test-support verify_no_panics_cli`

Expected: **FAIL** — clap rejects the unknown `verify-no-panics` subcommand.

- [x] **Step 6: Wire the CLI adapter**

Add to `Commands`:

```rust
/// Fail if the scoped diagnostic stream or required server log records a Rust panic.
VerifyNoPanics {
    /// E2E capture directory; the diagnostic filename is resolved by host::capture.
    #[arg(long)]
    capture_dir: std::path::PathBuf,
    /// Required VM-journal or host-stderr capture.
    #[arg(long)]
    server_log: std::path::PathBuf,
},
```

Dispatch directly to
`test_support::panic_gate::verify_no_panics(&capture_dir, &server_log)`. Do not
add an allowlist option or read `JAUNDER_CAPTURE_DIR` in this command; explicit
paths make the process interface identical for VM and host.

- [x] **Step 7: Run the complete crate suite**

Run: `devtool run -- cargo nextest run -p test-support`

Expected: **PASS** — module, dispatch, and subprocess suites pass.

- [x] **Step 8: Stage, gate, and commit Task 1**

Tick Task 1 in this plan and stage only its files, including the approved spec
and plan. Then run: `devtool run -- cargo xtask check`

Expected: **PASS**. If formatting changed files, restage the formatter output
and run the same gate again so the staged tree—not a predecessor—is certified.
Then commit:

```bash
git commit -m "test(e2e): share zero-panic verifier"
```

Expected: pre-commit repeats the full check successfully; commit contains no
trailer.

---

### Task 2: Migrate every VM e2e check to the shared verifier

**Files:**

- Modify: `flake.nix:556-597` — retain journal capture, replace inline Python
  parsing/allowlist with the CLI call.
- Modify:
  `docs/superpowers/plans/2026-08-13-issue-269-host-e2e-diagnostic-gates.md` —
  tick Task 2 steps as executed.

**Interfaces:**

- Consumes:
  `test-support verify-no-panics --capture-dir <DIR> --server-log <FILE>` from
  Task 1 and the existing `testSupportBin` guest package.
- Produces: one `e2ePanicGate backend` Nix helper still interpolated by the
  shared `e2eRunAndCapture`, therefore structurally covering every
  `{sqlite,postgres}×{chromium,firefox}` combination.

- [ ] **Step 1: Replace the inline VM parser**

Keep journal materialization and `copy_from_vm` unchanged. Replace the `diag`,
`allowed_panics`, `panic_location`, `collect`, and `reports` Python block with:

```nix
          machine.succeed(
              "test-support verify-no-panics"
              + " --capture-dir /var/lib/jaunder/capture"
              + " --server-log /tmp/jaunder-journal-${backend}.log"
          )
```

Update the adjacent comment to name the shared Rust verifier. Do not spell
`diag.log`; the CLI resolves `Stream::Diag`. Keep `${e2ePanicGate backend}`
after all diagnostic copies and before the existing `pw_status` assertion.

- [ ] **Step 2: Run static checks before the VM smoke**

Run: `devtool run -- cargo xtask check --no-test`

Expected: **PASS** — Rust, Nix formatting/evaluation, clippy, and repository
static invariants accept the migrated helper.

- [ ] **Step 3: Exercise the changed VM process seam**

Run: `devtool run -- cargo xtask e2e sqlite chromium`

Expected: **PASS**. The `e2e` result contains a successful Nix e2e step; this
proves the VM guest finds `test-support`, clap accepts both paths, the required
journal exists, and the clean suite passes the shared verifier. Because all four
checks interpolate the same `e2eRunAndCapture`/`e2ePanicGate` helper, this one
real combo plus structural sharing covers the invocation wiring before the full
four-combo gate in Task 3.

- [ ] **Step 4: Stage, gate, and commit Task 2**

Tick Task 2 and stage `flake.nix` plus the plan. Then run:
`devtool run -- cargo xtask check`

Expected: **PASS**. If formatting changed files, restage them and rerun the
gate. Then commit:

```bash
git commit -m "test(e2e): use shared panic gate in VM"
```

Expected: pre-commit passes; commit contains no trailer.

---

### Task 3: Enforce the shared verifier in `e2e-local`

**Files:**

- Modify: `xtask/src/steps/e2e_local.rs` — stderr tee, explicit stop/drain,
  delayed Playwright result, CLI invocation, independent step aggregation, unit
  tests.
- Modify: `docs/ARCHITECTURE.md:1202-1215,1791-1798,1837-1853` — state that the
  host and VM use one verifier and identify the host server-log adapter.
- Modify:
  `docs/superpowers/plans/2026-08-13-issue-269-host-e2e-diagnostic-gates.md` —
  tick Task 3 steps as executed.

**Interfaces:**

- Consumes:
  `test-support verify-no-panics --capture-dir <DIR> --server-log <FILE>` and
  the already-built `target/debug/test-support` path.
- Produces private host-driver interfaces:

```rust
fn mirror_server_stderr(
    reader: impl std::io::Read,
    terminal: impl std::io::Write,
    capture: impl std::io::Write,
) -> std::io::Result<()>;

struct ServerChild {
    child: Option<std::process::Child>,
    stderr_mirror: Option<std::thread::JoinHandle<std::io::Result<()>>>,
}

impl ServerChild {
    fn stop(&mut self) -> anyhow::Result<()>;
}

fn record_post_playwright_results(
    result: &mut CommandResult,
    playwright_ok: bool,
    panic_gate_result: Result<(), String>,
);
```

`ServerChild::drop` calls `stop` best-effort. `stop` is idempotent: if the child
is still running, kill it; always wait/reap it; then join the mirror. The
recording helper always pushes both `e2e-local-playwright` and
`e2e-local-panic-gate` steps.

- [ ] **Step 1: Add failing unit tests for the stderr and aggregation seams**

Add these tests to the existing `e2e_local.rs` test module, updating the
existing `ServerChild` construction only after the new type is implemented:

```rust
#[test]
fn stderr_mirror_copies_every_byte_to_both_sinks() {
    let input = b"first\n\xff panicked at src/x.rs:1:2: boom\n";
    let mut terminal = Vec::new();
    let mut capture = Vec::new();

    mirror_server_stderr(&input[..], &mut terminal, &mut capture)
        .expect("mirror succeeds");

    assert_eq!(terminal, input);
    assert_eq!(capture, input);
}

#[test]
fn playwright_and_panic_failures_are_both_recorded() {
    let mut result = CommandResult::new("e2e-local");

    record_post_playwright_results(
        &mut result,
        false,
        Err("shared verifier rejected a panic".to_owned()),
    );

    assert!(!result.ok);
    let playwright = result
        .steps
        .iter()
        .find(|step| step.name == "e2e-local-playwright")
        .expect("playwright step");
    let panic_gate = result
        .steps
        .iter()
        .find(|step| step.name == "e2e-local-panic-gate")
        .expect("panic step");
    assert!(!playwright.ok);
    assert!(!panic_gate.ok);
    assert_eq!(
        panic_gate.detail.as_deref(),
        Some("shared verifier rejected a panic")
    );
}

#[test]
fn clean_post_playwright_results_are_both_successful() {
    let mut result = CommandResult::new("e2e-local");
    record_post_playwright_results(&mut result, true, Ok(()));
    assert!(result.ok);
    assert_eq!(
        result
            .steps
            .iter()
            .filter(|step| {
                step.name == "e2e-local-playwright"
                    || step.name == "e2e-local-panic-gate"
            })
            .filter(|step| step.ok)
            .count(),
        2
    );
}
```

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml e2e_local`

Expected: **FAIL** — the mirror and aggregation interfaces are undefined.

- [ ] **Step 2: Implement streaming stderr ownership and teardown**

Import `File`, `Read`, `Write`, `Stdio`, and `JoinHandle`. Implement
`mirror_server_stderr` with one fixed stack buffer: each successful read writes
the exact slice to terminal first, flushes the terminal for live visibility,
then writes the slice to capture; EOF flushes both writers. No `read_to_end`,
`String`, or whole-log allocation belongs in this path.

Before spawning the server, create `<tempdir>/server-stderr.log`. Spawn
`jaunder serve` with `stderr(Stdio::piped())`; immediately take the pipe and
start the mirror thread with `std::io::stderr()` and the capture file. Construct
the expanded `ServerChild` with the child and join handle.

Implement idempotent `stop`: `try_wait`; kill only a still-running child; wait
to reap; join the mirror after child exit. Convert a mirror-thread panic or I/O
failure into contextual `anyhow` failure. `Drop` calls `stop` and ignores its
result, preserving cleanup on every pre-Playwright early return. Update
`server_child_kills_on_drop` for the new constructor and retain its `/proc`
reaping assertion.

- [ ] **Step 3: Preserve Playwright status, drain, verify, and record both
      results**

Replace the Playwright failure early return with a stored `playwright_ok` bool.
After Playwright exits:

1. Call `server.stop()` before verification. If stop/drain fails, push a failed
   `e2e-local-server-log` step but continue to invoke the verifier.
2. Change back to the repo root or invoke the already absolute
   `target/debug/test-support` path through `cmd!` with
   `verify-no-panics --capture-dir {capture} --server-log {server_stderr}`.
3. Convert the command outcome to `Result<(), String>` with stable detail
   `shared zero-panic verifier failed`; its own stderr already prints exact
   panic records or input errors live.
4. Call `record_post_playwright_results` once, ensuring it pushes both steps for
   every combination of outcomes.

Do not expose the parser in `xtask`, do not return before the shared verifier,
and do not persist the temp directory.

- [ ] **Step 4: Run the host-driver unit tests**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml e2e_local`

Expected: **PASS** — byte-exact teeing, dual-failure aggregation, clean
aggregation, runtime parsing, and kill/reap tests all pass.

- [ ] **Step 5: Update the architecture view**

Update `docs/ARCHITECTURE.md` without restating implementation trivia:

- Scoped diagnostics: one `test_support::panic_gate` verifier owns the raw-byte
  union/deduplication policy and resolves `Stream::Diag`.
- E2e suite: VM passes its materialized `jaunder.service` journal; `e2e-local`
  streams server stderr live to a per-run file and passes that file after
  stopping/draining the child.
- Both surfaces execute verification after Playwright has produced a status, so
  a Playwright failure cannot mask a server panic.

Run: `devtool run -- prettier -w docs/ARCHITECTURE.md`

Expected: **PASS**; only intended paragraphs reflow.

- [ ] **Step 6: Smoke the real host loop**

Run: `devtool run -- cargo xtask e2e-local example.spec.ts`

Expected: **PASS** with successful `e2e-local-playwright` and
`e2e-local-panic-gate` steps. During the run, server stderr remains visible; on
completion the server is reaped and the command exits without retaining the
per-run directory.

- [ ] **Step 7: Run the full local gate**

Run: `devtool run -- cargo xtask validate`

Expected: **PASS** — verify-only static checks, coverage, and all four
`{sqlite,postgres}×{chromium,firefox}` e2e combinations, each using the shared
verifier.

- [ ] **Step 8: Stage, gate, and commit Task 3**

Tick Task 3 and stage `xtask/src/steps/e2e_local.rs`, `docs/ARCHITECTURE.md`,
and the completed plan. Then run: `devtool run -- cargo xtask check`

Expected: **PASS**. If formatting changed files, restage them and rerun the
gate. The broader Step 7 `validate` remains the AC10 behavioral proof; this
post-staging check certifies the exact tree being committed. Then commit:

```bash
git commit -m "test(e2e): gate host loop on server panics"
```

Expected: pre-commit's `cargo xtask check` passes; commit contains no trailer.
