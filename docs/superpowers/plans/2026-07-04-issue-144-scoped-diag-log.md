# Plan — Issue #144: app-driven scoped server-diagnostics capture

- **Spec:**
  [`docs/superpowers/specs/2026-07-04-issue-144-scoped-diag-log.md`](../specs/2026-07-04-issue-144-scoped-diag-log.md)
  — the "what/why". This plan is the "how"; it does not restate the spec.
- **Issue:** [#144](https://github.com/jaunder-org/jaunder/issues/144) ·
  Milestone "E2E test suite"
- **For agentic workers:** drive with `jaunder-iterate`; delegate an individual
  task via `jaunder-dispatch` where useful. Tick checkboxes in real time.

---

## Review header (approve this layer)

**Goal.** When `JAUNDER_DIAG_LOG_FILE` is set (e2e only), the server writes a
small JSONL file of WARN+ events + panic records; the e2e zero-panic gate
sources panics from that file ∪ the journal (de-duped by location); CI uploads
it per-combo. Production is untouched when the var is unset.

**Scope — in:** a WARN+ `tracing` JSON layer + a panic hook (independent append
handle) in the `server` crate; the `flake.nix` systemd env + copy-out wiring;
the `xtask` artifact-lift filter; the `e2ePanicGate` union+de-dup rewrite; a new
ADR; `observability.md` + `CONTRIBUTING.md`. **Out:** capture-env-var
consolidation (#227), a configurable diag level, an `analyze-*` tool, the xtask
nix-excerpt companion (all in spec §Out-of-scope).

**Tasks (one line each):**

1. ✅ **DONE** (`ac1bb2d5`) — WARN+ diag JSON layer + injectable synchronous
   writer + `diag_log_file()` reader, wired into `init_tracing_impl` (layer
   only) — real-registry AND-gate test + env-unset + file-exists + open-error
   coverage.
2. ✅ **DONE** (`df87c241`) — `DiagPanicRecord` JSONL formatter **and** the
   panic-hook install (independent `O_APPEND` handle, chains to prev,
   `Option<PathBuf>` so the enablement check keeps `init_tracing_impl`'s CRAP
   flat), wired into `init_tracing_impl` — one commit.
3. ✅ **DONE** (`6c8c4527`) — `flake.nix` server env (`JAUNDER_DIAG_LOG_FILE` in
   `mailCaptureEnv`) + copy-out rename in `e2eRunAndCapture`.
4. ✅ **DONE** (`d8a12e70`) — `xtask` `copy_e2e_diagnostics_between`: add
   `jaunder-diag-` prefix + doc + unit test (positive + negative-rename cases).
5. ✅ **DONE** (`7b709113`) — `e2ePanicGate` rewrite: scoped ∪ journal, de-dup
   by location, scoped precedence, allowlist kept.
6. ✅ **DONE** (draft authored, out of git; numbered/committed at ship) —
   `docs/adr/drafts/app-driven-scoped-server-diagnostics.md` via `jaunder-adr`
   (cross-links 0032/0037).
7. ✅ **DONE** — Docs: `observability.md` (new "look here first" section) +
   `CONTRIBUTING.md` Observability bullet.
8. Full gate: `cargo xtask validate` green (AC proof across all four combos).

**Key risks / decisions:**

- **Filter is an AND-gate** (spec §Filter semantics): the diag layer's threshold
  MUST be a per-layer `.with_filter(LevelFilter::WARN)`, never a second global
  `.with(...)`. Task 1's test builds the _real_ registry shape to lock this.
- **Two clippy/coverage commit-boundary traps** (from cold review, both avoided
  by construction):
  - `expect_used`/`unwrap_used` are **denied** in production (`Cargo.toml`
    workspace lints; expect allowed only in tests via `clippy.toml`). `to_line`
    runs in the hook → use `unwrap_or_default()`, never `.expect`/`.unwrap`.
  - **Producer and its sole consumer must land in the same commit** or the lib
    target sees `dead_code` under `-D warnings` (the repo's known pub-API
    dead-code boundary gotcha). Hence Task 2 keeps `DiagPanicRecord` + the hook
    together.
- **Panic-unwind deadlock-safety** (spec §Writer architecture): the hook uses
  its **own** `O_APPEND` `File`, never the layer's writer/lock, never
  `tracing::error!`. Documented inline in Task 2.
- **De-dup trailing-colon** (spec §Gate): journal token has a trailing `:`, JSON
  `location` does not — one canonical `rstrip(":")` extraction applied to both.
  `location = info.location().to_string()` verbatim so paths match
  byte-for-byte.
- **Copy-out rename is load-bearing** (spec §Wire-up): `_grab` copies flat under
  basename, so rename to `jaunder-diag-${backend}.log` first or the Task-4
  `starts_with("jaunder-diag-")` filter drops it.
- **Coverage gate is line-based** and runs on every `cargo xtask check`. Every
  new branch (env-unset, file-open error, the `prev(info)` chain) has a test
  that executes it (Tasks 1–2). Do NOT weaken tests or reach for the
  baseline-bootstrap escape hatch — these lines are trivially coverable.
- **#153 overlap** (spec §Coordination): Tasks 3 & 5 edit `flake.nix` regions
  #153 also edits. **Re-read the current copy-out block and gate before editing
  — line numbers here are from 2026-07-04 and may drift.**

---

## Global constraints

- **Language:** complete Rust; exact `cargo` commands. Tests are in-file
  `#[cfg(test)]` in `server/src/observability.rs` (crate convention; the module
  already has a `tests` mod with `ENV_LOCK` + `lock_env()`).
- **Crate names (corrected during execution):** the `server/` crate's package is
  **`jaunder`** (run its tests with `cargo nextest run -p jaunder …`, not
  `-p server`). The `xtask/` crate is package **`maik`** and is **outside the
  workspace** (the flake excludes it), so run its tests with
  `cargo nextest run --manifest-path xtask/Cargo.toml …` (not
  `-p xtask`/`-p maik` from the workspace). Task command lines below that say
  `-p server`/`-p xtask` are shorthand — use these real invocations.
- **Workspace lints deny `unwrap_used` / `expect_used` in non-test code** — use
  `unwrap_or_default`/`unwrap_or_else`/`match` in anything the lib target
  compiles. `.unwrap()`/`.expect()` are fine inside `#[cfg(test)]`.
- **Env-touching / panic-hook tests serialize on `ENV_LOCK`** via `lock_env()`.
  Tests run under `cargo nextest` (process-per-test), so global-hook pollution
  across tests isn't a real hazard — but tests that install the global panic
  hook still **save/restore** it (`take_hook`/`set_hook`) so they're also
  correct under plain `cargo test` and self-documenting.
- **Per commit:** run `cargo xtask check` first (fmt + clippy `-D warnings` +
  Nix coverage/tests) so the pre-commit hook passes clean — see
  `jaunder-commit`. **No `Co-Authored-By` trailer.** Do not commit without
  approval per `CLAUDE.md`.
- **No placeholders / TODOs** in committed code.

---

## Task 1 — WARN+ diag JSON layer + injectable synchronous writer

**Files:**

- `server/src/observability.rs` — add `fn diag_log_file() -> Option<PathBuf>`
  (env reader, trim+non-empty, mirroring `otel_exporter_otlp_endpoint` at
  :40-46); a `fn diag_layer<S, W>(make_writer: W) -> impl Layer<S>` generic over
  the sink; wire an `Option` diag layer into `init_tracing_impl`'s registry
  chain gated on `diag_log_file()`.
- Test: in-file `#[cfg(test)]` (same module).

**Interface (sketch):**

```rust
fn diag_log_file() -> Option<std::path::PathBuf> {
    std::env::var("JAUNDER_DIAG_LOG_FILE").ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

// Generic over the writer so tests inject an in-memory sink and prod injects the file.
// HARD REQUIREMENT: the WARN threshold is the layer's OWN per-layer filter
// (`.with_filter`), never a second global `.with(LevelFilter::WARN)` — a global
// clamp would silence INFO to the journal/OTel sinks (spec §Filter semantics).
fn diag_layer<S, W>(make_writer: W) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> fmt::MakeWriter<'a> + 'static,
{
    fmt::layer()
        .json()
        .with_writer(make_writer)
        .with_filter(tracing::level_filters::LevelFilter::WARN)
}
```

In `init_tracing_impl`, after the fmt layer — **synchronous, unbuffered
`Arc<File>` sink** (confirmed a `MakeWriter`; explicitly NOT
`tracing_appender::non_blocking`/`BufWriter`, so `panic=abort` can't drop lines
— spec §Durability):

```rust
// Name the local distinctly from the `diag_layer` fn to avoid confusing shadowing.
let diag_log_layer = diag_log_file().and_then(|path| {
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(diag_layer(std::sync::Arc::new(file))),
        Err(error) => { eprintln!("diag log disabled; could not open {}: {error}", path.display()); None }
    }
});
// ... .with(fmt_layer).with(diag_log_layer).with(otel_layer) ...   // Option<L>: Layer<S>, mirrors otel_layer
```

**TDD steps:**

1. Write the load-bearing AND-gate test — build the **real registry shape**, not
   the layer alone:
   ```rust
   #[test]
   fn diag_layer_captures_warn_and_above_not_info_under_global_info_filter() {
       let _lock = lock_env();
       let diag_buf = Arc::new(Mutex::new(Vec::<u8>::new()));   // in-memory diag sink
       let other_buf = Arc::new(Mutex::new(Vec::<u8>::new()));  // second sink = "journal/otel" stand-in
       let subscriber = tracing_subscriber::registry()
           .with(EnvFilter::new("info"))                        // global filter as in e2e
           .with(fmt::layer().with_ansi(false).with_writer(Shared(other_buf.clone()))) // no per-layer filter
           .with(diag_layer(Shared(diag_buf.clone())));         // WARN per-layer filter
       tracing::subscriber::with_default(subscriber, || {
           tracing::info!("info-line");
           tracing::warn!("warn-line");
           tracing::error!("error-line");
       });
       let diag = String::from_utf8(diag_buf.lock().unwrap().clone()).unwrap();
       let other = String::from_utf8(other_buf.lock().unwrap().clone()).unwrap();
       assert!(!diag.contains("info-line"));
       assert!(diag.contains("warn-line") && diag.contains("error-line"));
       for line in diag.lines() { serde_json::from_str::<serde_json::Value>(line).expect("valid JSONL"); }
       assert!(other.contains("info-line"));   // proves we did NOT globally clamp to WARN
   }
   ```
   `Shared` is a test-module newtype the implementer writes (~15 lines):
   `struct Shared(Arc<Mutex<Vec<u8>>>)` + `impl std::io::Write for Shared`
   (lock, extend, flush) +
   `impl<'a> fmt::MakeWriter<'a> for Shared { type Writer = Shared; fn make_writer(&self) -> Shared { Shared(self.0.clone()) } }`.
   `Arc<Mutex<Vec<u8>>>` is **not** itself a `MakeWriter`, and `TestWriter`
   writes to std{out,err} (uncapturable) — so the newtype is required, not
   optional.
   - **Run (FAIL):** `cargo nextest run -p server diag_layer_captures`
2. Implement `diag_log_file` + `diag_layer` + the `Shared` writer.
   - **Run (PASS):** same command.
3. Add `diag_log_file_is_none_when_env_unset` (remove the var under
   `lock_env()`, assert `None`) — covers the gating branch's false arm.
4. Wire the `Option` layer (only the layer — the hook is Task 2) into
   `init_tracing_impl`. Add two coverage tests, each under `lock_env()`:
   - `init_tracing_impl_creates_diag_file_when_env_set`: point
     `JAUNDER_DIAG_LOG_FILE` at a `TempDir` file, call
     `init_tracing_impl(false)`, assert the file exists (`OpenOptions.create`
     makes it on open, independent of whether `try_init` wins — a prior test's
     global subscriber may already own the slot).
   - `init_tracing_impl_survives_unopenable_diag_path`: point the var at a
     **directory** (`dir.path()`, mirroring `mailer/file.rs:132`), call
     `init_tracing_impl(false)`, assert it returns without panic — **covers the
     `Err`/`eprintln` arm** (line-based coverage would otherwise flag it).
   - **Run (PASS):** `cargo nextest run -p server init_tracing_impl`
5. `cargo xtask check` → **commit**
   (`feat(observability): WARN+ scoped diag JSON layer behind JAUNDER_DIAG_LOG_FILE (#144)`).

---

## Task 2 — `DiagPanicRecord` formatter + panic-hook install (one commit)

Formatter and its sole consumer (the hook) land **together** — splitting them
leaves `DiagPanicRecord`/`from_panic` as `dead_code` in the lib target under
`-D warnings` (the repo's pub-API dead-code boundary gotcha). TDD the pure
formatter first _within_ this commit.

**Files:**

- `server/src/observability.rs` — a private `DiagPanicRecord` serde struct +
  `from_panic` + `panic_payload_str` + `to_line`;
  `fn install_diag_panic_hook(path: PathBuf)`; call it from `init_tracing_impl`
  when `diag_log_file()` is `Some`.
- Tests: in-file.

**Interfaces (sketch):**

```rust
#[derive(serde::Serialize)]
struct DiagPanicRecord<'a> {
    timestamp: &'a str,      // RFC3339 UTC, injected for deterministic tests
    level: &'a str,          // "ERROR"
    kind: &'a str,           // "panic"
    target: &'a str,         // "panic"
    message: String,         // "panicked at <location>: <payload>"  ← literal `panicked at` for the gate
    location: String,        // info.location().to_string() verbatim (byte-identical to journal)
    thread: String,
}

impl<'a> DiagPanicRecord<'a> {
    fn from_panic(info: &std::panic::PanicHookInfo<'_>, thread: &str, timestamp: &'a str) -> Self {
        let location = info.location().map(ToString::to_string).unwrap_or_default();
        let payload = panic_payload_str(info);          // &str / String payload extraction
        DiagPanicRecord {
            timestamp, level: "ERROR", kind: "panic", target: "panic",
            message: format!("panicked at {location}: {payload}"),
            location, thread: thread.to_owned(),
        }
    }
    /// One physical JSONL line. Runs inside the panic hook, so it must never itself
    /// panic: serialization of this fixed struct is infallible, but the workspace
    /// denies `.expect`/`.unwrap` in non-test code — use `unwrap_or_default()`.
    fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// Install a panic hook that appends a scoped JSON panic record to `path`.
///
/// DEADLOCK-SAFETY (load-bearing — do not "simplify"):
/// The hook opens its OWN `File` in append mode and writes directly. It must NOT
/// share the diag *layer's* writer/lock and must NOT route through `tracing::error!`:
/// if a thread panics while holding the subscriber's or a shared writer's lock,
/// re-entering that lock on the panicking thread would DEADLOCK — a captured panic
/// becomes a silent hang, the worst failure for a diagnostics feature. `O_APPEND` on a
/// regular file positions each write() at EOF atomically; we emit the whole record in
/// one `write_all`, so it interleaves with the layer's WARN+ lines at line boundaries
/// without a shared lock. We chain to the previous hook so the default stderr→journald
/// path still fires (journal stays the fallback / catches pre-install panics — spec §Gate).
fn install_diag_panic_hook(path: std::path::PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true); // "…Z"
            let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
            let _ = file.write_all(DiagPanicRecord::from_panic(info, &thread, &ts).to_line().as_bytes());
        }
        prev(info); // keep default behavior (stderr → journald)
    }));
}
```

`chrono` is a `server` dep (`server/Cargo.toml:51`, with `serde`).
`PanicHookInfo` is the current type name (renamed from `PanicInfo` in 1.82).
Errors are swallowed — the hook must never unwind further.

**TDD steps:**

1. Write `diag_panic_record_is_one_json_line_with_panicked_at`: construct
   `DiagPanicRecord` via **struct literal** (fixed `timestamp`, a location, a
   payload **containing a newline** e.g. `"boom\nsecond"`), call `to_line()`;
   assert exactly one physical line (`serde_json::from_str` on the whole trimmed
   string parses), `kind=="panic"`, `level=="ERROR"`, `target=="panic"`,
   `message` contains `panicked at`, `location` equals the injected string.
   - **Run (FAIL):** `cargo nextest run -p server diag_panic_record`
2. Implement `DiagPanicRecord` + `to_line` + `panic_payload_str`.
   - **Run (PASS):** same command.
3. Write `installed_diag_panic_hook_appends_record_and_restores` (covers
   `from_panic`/`panic_payload_str`/`install_diag_panic_hook`/the `prev(info)`
   chain via a real panic):

   ```rust
   #[test]
   fn installed_diag_panic_hook_appends_record_and_restores() {
       let _lock = lock_env();
       let dir = tempfile::TempDir::new().unwrap();
       let path = dir.path().join("jaunder-diag.log");
       let prev = std::panic::take_hook();                 // save
       install_diag_panic_hook(path.clone());
       let _ = std::panic::catch_unwind(|| panic!("boom-under-test"));
       std::panic::set_hook(prev);                          // restore
       let content = std::fs::read_to_string(&path).unwrap();
       let record: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
       assert_eq!(record["kind"], "panic");
       assert!(record["message"].as_str().unwrap().contains("panicked at"));
       assert!(record["message"].as_str().unwrap().contains("boom-under-test"));
   }
   ```

   - **Run (FAIL):** `cargo nextest run -p server installed_diag_panic_hook`

4. Implement `install_diag_panic_hook`; call it from `init_tracing_impl` (once,
   when `diag_log_file()` is `Some`). Update the
   `init_tracing_impl_creates_diag_file_when_env_set` test from Task 1 to
   **save/restore the panic hook** (it now installs one) so no global hook leaks
   under plain `cargo test`.
   - **Run (PASS):** `cargo nextest run -p server installed_diag_panic_hook`;
     then `cargo nextest run -p server observability` to confirm the existing
     `catch_unwind` poison test is unaffected.
5. `cargo xtask check` → **commit**
   (`feat(observability): scoped-diag panic hook + JSONL record, independent append handle (#144)`).

---

## Task 3 — `flake.nix`: server env + copy-out (⚠ re-read region; #153 overlap)

**Files:** `flake.nix`.

**Edits:**

1. **Server env.** Add
   `JAUNDER_DIAG_LOG_FILE = "/var/lib/jaunder/jaunder-diag.log";` to the
   `mailCaptureEnv` attr (~`flake.nix:48-51`). It is spliced into
   `systemd.services.jaunder.environment` for both backends
   (`mailCaptureEnv // { … }`, ~812/945), so the **server** (not the Playwright
   process) receives it — do NOT add it to the Playwright exec env at ~719-720.
   (`mailCaptureEnv` also splices into the base/interactive VM at ~172 —
   harmless.) _(Name nit: `mailCaptureEnv` now also carries a diag var — left
   as-is; #227 will consolidate.)_
2. **Copy-out** in `e2eRunAndCapture`, mirroring the playwright-report rename
   (`flake.nix:752-753`) so the flat-copied basename carries `-${backend}`
   (required by Task 4's filter):
   ```python
   machine.execute("test -s /var/lib/jaunder/jaunder-diag.log && cp /var/lib/jaunder/jaunder-diag.log /tmp/jaunder-diag-${backend}.log")
   _grab("/tmp/jaunder-diag-${backend}.log")
   ```
   Place it in the copy-all block **before** `${e2ePanicGate backend}` — for
   **artifact-safety** (copy-all-before-assert, ADR-0037), _not_ for the gate
   read: the gate reads the in-VM path `/var/lib/jaunder/jaunder-diag.log`
   directly (Task 5), so ordering doesn't affect it.

**Verification:** no Rust test; proven by Task 8's e2e run producing
`.xtask/diagnostics/e2e-*/jaunder-diag-*.log`. **Before editing, re-read
`flake.nix:701-765`** — #153 may have restructured this block (spec
§Coordination).

**Commit** (`test(e2e): capture scoped jaunder-diag.log per combo (#144)`).

---

## Task 4 — `xtask`: lift the diag artifact

**Files:** `xtask/src/steps/nix.rs`.

**Edits:**

1. Add to the `wanted` closure in `copy_e2e_diagnostics_between`
   (~nix.rs:130-136):
   ```rust
   || (name.starts_with("jaunder-diag-") && name.ends_with(".log"))
   ```
2. Update the function's doc comment (nix.rs:121-128) to list the scoped diag
   log.

**TDD steps:**

1. Extend the existing unit test for `copy_e2e_diagnostics_between` (in-file
   `#[cfg(test)]`, ~nix.rs:416-455): write a `jaunder-diag-sqlite.log` into the
   src temp dir, assert it copies to dest and is counted; also assert
   `jaunder-diag.log` (no `-<backend>`) is **not** copied — locks Task 3's
   rename (`"jaunder-diag.log".starts_with("jaunder-diag-")` is `false`).
   - **Run (FAIL):** `cargo nextest run -p xtask copy_e2e_diagnostics`
2. Apply the closure + doc edits.
   - **Run (PASS):** same command.
3. `cargo xtask check` → **commit**
   (`test(xtask): lift jaunder-diag-*.log into diagnostics dir (#144)`).

---

## Task 5 — `e2ePanicGate` rewrite: scoped ∪ journal, de-dup by location (⚠ #153 overlap)

**Files:** `flake.nix` (`e2ePanicGate`, ~684-693).

**Behavior (spec §Gate):** scan **both** the scoped diag file and the journal
for the raw substring `panicked at`; fail (default-deny) if either has an
un-allowlisted hit; de-dup by **location** (`rstrip(":")` applied identically to
both); report the scoped JSON record for a location in the scoped file, else the
journal line.

**Sketch:**

```python
${e2ePanicGate backend}  # rewritten:
diag = machine.execute("cat /var/lib/jaunder/jaunder-diag.log 2>/dev/null")[1]   # in-VM path, may be absent
journal = machine.succeed("cat /tmp/jaunder-journal-${backend}.log")             # captured at 685-689
allowed_panics: list[str] = []  # default-deny; add a proven-benign substring + comment if one appears

def loc_key(line):
    # token after "panicked at ", trailing ':' stripped — canonical across BOTH formats.
    # Assumes the current toolchain's `panicked at <loc>:` form (payload on next line in
    # the journal). Pre-1.73 `panicked at 'msg', <loc>` would break this — pin the assumption.
    return line.split("panicked at ", 1)[1].split()[0].rstrip(":")

reports: dict[str, str] = {}
for l in diag.splitlines():
    if "panicked at" in l and not any(a in l for a in allowed_panics):
        reports[loc_key(l)] = l                      # scoped JSON record (authoritative)
for l in journal.splitlines():
    if "panicked at" in l and not any(a in l for a in allowed_panics):
        reports.setdefault(loc_key(l), l)            # only if scoped didn't have it (pre-init)

assert not reports, "e2e zero-panic gate (${backend}): jaunder.service logged Rust panic(s):\n" + "\n".join(reports.values())
```

Keep the existing journal capture lines (685-689) — the journal remains the
fallback artifact and the pre-init source. **Before editing, re-read the current
gate + its call site** (#153 may have moved surrounding lines).

**Verification:** no unit test possible (VM-only Python) — correctness by
inspection + Task 8. Default-deny means a de-dup miss only double-prints; it
never lets a panic through.

**Commit**
(`test(e2e): zero-panic gate reads scoped diag ∪ journal, de-duped (#144)`).

---

## Task 6 — ADR draft

Use `jaunder-adr` (draft-out-of-git): author a numberless draft in
`docs/adr/drafts/` capturing the four new decisions (app-driven scoped capture;
panic-hook-bypasses-tracing / independent handle; JSONL; gate sources scoped ∪
journal with de-dup + scoped precedence), cross-linking ADR-0032 and ADR-0037.
Numbered at ship by `cargo xtask adr promote` (`jaunder-ship`). **Commit** the
draft (`docs(adr): draft app-driven scoped server-diagnostics capture (#144)`).

---

## Task 7 — Docs

**Files:** `docs/observability.md`, `CONTRIBUTING.md`.

- `docs/observability.md`: new "server-side scoped diagnostic log" section — the
  `jaunder-diag-<backend>.log` location in `.xtask/diagnostics/e2e-*/`, its
  JSONL shape (WARN+ events + `kind:"panic"` records), and a **"look here
  first"** pointer (journal = last resort).
- `CONTRIBUTING.md` (Observability section, ~245): note the scoped diag log as
  the primary server-side failure log; per-combo location; journal demoted to
  fallback.

**Commit** (`docs: document the scoped diag log as look-here-first (#144)`).

---

## Task 8 — Full gate (AC proof)

- **`cargo xtask validate`** (all four `{sqlite,postgres}×{chromium,firefox}`
  combos — green). Confirms the layer/hook compile+run in the VM, the copy-out
  lands, and the rewritten gate passes on a clean run.
- **AC spot-check:** after the run, confirm
  `.xtask/diagnostics/e2e-sqlite-chromium/jaunder-diag-sqlite.log` exists and
  contains only WARN+/panic JSONL (no INFO-request / kernel lines).
- Optional throwaway forced-panic in one combo to eyeball the gate reporting the
  scoped record (revert before shipping) — the AC is a red-run behavior; not
  committed.

Long/cold run → Bash background mode. This is the "green → you may move on" gate
before `jaunder-ship`.

---

## Self-review checklist

- [ ] No `.expect`/`.unwrap` in non-test code (`to_line` uses
      `unwrap_or_default`); clippy `-D warnings` clean.
- [ ] `DiagPanicRecord` and its consumer (the hook) land in one commit — no
      dead-code boundary break (Task 2).
- [ ] Every new branch is executed by a test: env-unset (T1.3), file-open
      error/directory (T1.4), hook + `prev(info)` chain via real panic (T2.3),
      rename negative case (T4.1).
- [ ] The AND-gate filter semantics are locked by a test that builds the real
      registry (T1.1), not the layer in isolation; the "other" sink uses
      `.with_ansi(false)`.
- [ ] The panic hook's deadlock-safety + independent-handle rationale is an
      inline comment (T2), and hook-installing tests save/restore the global
      hook.
- [ ] The copy-out rename (T3) and `starts_with("jaunder-diag-")` filter (T4)
      agree, with a negative test.
- [ ] Tasks 3 & 5 carry the "re-read region — #153 overlap" warning; line
      numbers are dated 2026-07-04.
- [ ] No `Co-Authored-By`; commit only on approval; `cargo xtask check` before
      each commit.
