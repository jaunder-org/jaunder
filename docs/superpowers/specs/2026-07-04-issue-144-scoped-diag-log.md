# Spec — Issue #144: app-driven scoped server-diagnostics capture

- Issue: [#144](https://github.com/jaunder-org/jaunder/issues/144)
- Milestone: E2E test suite
- Status: draft (awaiting approval)
- Date: 2026-07-04

## Problem

Diagnosing an e2e failure means grovelling the full systemd journal inside the
VM — kernel boot spam plus every INFO request line bury the signal (the panic or
error). The zero-panic gate (ADR-0032) already greps the whole journal for
`panicked at`. Agents and humans need a small, **scoped** artifact containing
just the server's errors + panics, not the firehose. (Surfaced 2026-06-28
diagnosing #138/#129 — `rg` over the journal drowned in kernel boot noise.)

## Goal

The app emits a purpose-built, **scoped** diagnostic file (WARN+ and panics
only) that the e2e zero-panic gate consumes and CI uploads, demoting the full
journal to last-resort. Extends the existing `otel-traces.jsonl` +
`analyze-otel-traces` "scoped structured artifact" model to failure logs.

## Design (app-driven capture)

Add an e2e-only `JAUNDER_DIAG_LOG_FILE` env var, mirroring the existing
`JAUNDER_MAIL_CAPTURE_FILE` / `JAUNDER_WEBSUB_CAPTURE_FILE` capture-to-file
idiom (env-gated; production leaves it unset → zero behavior change). When set,
two independent feeders append to that file:

1. **A WARN+ `tracing` layer** — a `fmt().json()` layer carrying its **own**
   per-layer filter, `.with_filter(LevelFilter::WARN)` (a `Filtered` layer),
   added to the registry in `server/src/observability.rs` alongside the existing
   `env_filter → slow_span → fmt → otel` chain. The threshold is **hardcoded
   WARN+** — the file's contract is "WARN, ERROR, and panics, nothing lower"; a
   tunable filter env var is an explicit future follow-up.

   **"WARN+" means warning-and-above, not above-warning.** In `tracing` the
   severity order is `TRACE < DEBUG < INFO < WARN < ERROR`, and
   `LevelFilter::WARN` enables WARN **and everything more severe** — so the
   scoped file captures WARN events, ERROR events, and the panic records; INFO
   and below are excluded. It is _not_ ERROR-only.

   **Filter semantics (corrected — this is an AND-gate, not independence).** The
   existing `env_filter` is a _global_ filter (a bare `EnvFilter` added via
   `.with()`), so an event it disables reaches **no** layer, including a
   per-layer-`Filtered` one. The diag layer therefore captures
   `WARN+ ∩ global-filter`, not "WARN+ regardless of the global filter." This
   meets the contract in e2e **because** the global filter is `info` there
   (`RUST_LOG=info`, `flake.nix` ~813/946), which admits WARN/ERROR; the scoped
   file is exactly WARN+ whenever the global filter admits WARN+. Two hard
   requirements fall out:
   - The WARN threshold **must** be the layer's own `.with_filter(...)`
     (`Filtered`) — **never** a second global `.with(LevelFilter::WARN)`, which
     would clamp the whole registry to WARN+ and silence INFO to the
     journal/OTel sinks (a real regression).
   - A regression test must build the **actual** registry shape (global `info` +
     the diag WARN layer + a second capture sink) and assert INFO is dropped
     from the diag sink while still reaching the other sink — not merely a
     thread-local diag-layer-in-isolation test, which never exercises the
     global+per-layer interaction.

2. **A panic hook** — installed at startup **only when `JAUNDER_DIAG_LOG_FILE`
   is set**, chaining to (not replacing) the previous hook so the default stderr
   → journald path still fires and the full journal remains a fallback. The hook
   appends a JSON panic record to the scoped file. Because
   `std::panic::set_hook` is process-global and `init_tracing` is documented to
   run **twice in one process** in tests, any in-process test that installs the
   real hook **must** save and restore it (`take_hook` → install →
   `set_hook(prev)` via a drop guard) under the existing `ENV_LOCK`, or a stale
   hook fires on a later test and writes to a since-deleted temp path. In the
   e2e VM `init_tracing` runs once (`serve`), so double-install is a test-only
   concern; even if it did double-record, the gate's location de-dup collapses
   it.

### Format: JSONL

The file is JSON-lines — one record per line — so we can build tooling around it
(cf. `analyze-otel-traces` over `otel-traces.jsonl`). The tracing layer emits
`fmt().json()` events; the panic hook emits a record of this shape:

```json
{
  "timestamp": "2026-07-04T12:00:00Z",
  "level": "ERROR",
  "kind": "panic",
  "target": "panic",
  "message": "panicked at server/src/foo.rs:42:5: <payload>",
  "location": "server/src/foo.rs:42:5",
  "thread": "main"
}
```

- **`kind":"panic"`** discriminates panic records from tracing events (which
  carry no `kind`), so downstream tooling selects panics without
  string-sniffing. This also makes the scoped side **more precise** than the
  journal grep: a WARN/ERROR _message_ that merely contains "panicked at" is not
  a `kind:"panic"` record, so it won't be miscounted (the journal grep would,
  and does today).
- **`message` contains the literal `panicked at <location>: <payload>`** so the
  gate's `panicked at` substring match works whether it scans raw bytes or a
  parsed field — keeping ADR-0032's detection string stable and format-agnostic.
- **`level":"ERROR"`, `timestamp`, `target`** reuse the tracing JSON event
  _vocabulary_. Note the schemas are **not** identical: `fmt().json()` nests the
  message under `fields.message`, whereas the panic record puts `message` at top
  level. That is intentional and harmless — the gate scans the raw line for
  `panicked at` and selects on `kind`, never a unified message path — but the
  file is not a single flat schema; don't claim a uniform reader.
- **The record is built with `serde_json::to_string`** (a serde struct → one
  line), never hand-formatted — panic payloads can contain newlines (multi-line
  asserts, `{:#?}`), and serde escapes them so the record stays one physical
  JSONL line.
- **`location` is `info.location().to_string()`** verbatim (not reconstructed by
  hand) so it is byte-identical to the path the default hook prints to the
  journal — load-bearing for the gate's cross-source de-dup (see gate section).
- **No backtrace field** — one line per panic keeps the file small and scoped;
  `RUST_BACKTRACE` output stays in the journal fallback we still write to.

### Writer architecture — independent append handles (panic-unwind safe)

The WARN+ layer and the panic hook write to the **same file via separate
handles**, never a shared lock:

- The layer owns its own file writer.
- The panic hook opens its **own** `File` in `O_APPEND` mode.

This is the crux of the design and **must be documented inline at the hook
site**: if the two shared one `Mutex<File>`, a thread panicking while it (or the
tracing machinery) holds that mutex would deadlock when the hook re-acquired it
on the panicking thread — turning a captured panic into a silent hang, the worst
outcome for a diagnostics feature. This is exactly why the panic hook writes
**directly** and does **not** route through `tracing::error!` — going through
the subscriber during a panic is the deadlock path being avoided.

**Atomicity (corrected — no `PIPE_BUF` claim).** `PIPE_BUF` governs pipes/FIFOs,
not regular files. On a regular file `O_APPEND` makes each `write()` syscall
atomically positioned at EOF, but a logical line emitted as **two** `write()`s
(a short write, or a payload larger than the kernel returns in one go) can be
interleaved by the other appender between syscalls. `fmt().json()` and the hook
each emit a line via one `write_all`, which is _usually_ one syscall but loops
on short writes — so torn/interleaved JSONL is rare but possible. Therefore
**the gate detects panics by raw-substring scan for `panicked at`** (the
`message` field is engineered to carry that literal exactly so byte-scanning
works) and only best-effort `json.loads` a line for the pretty report, skipping
unparseable lines — it never parses the whole file as strict JSONL (a single
torn line must not turn the gate into a Python traceback). The residual
interleave risk is accepted.

**Durability (must be synchronous).** Both sinks — the layer's writer and the
hook's handle — are a **synchronous, unbuffered** `File` (wrapped as `Arc<File>`
/ `Mutex<File>` since `fmt().with_writer` needs a `MakeWriter`; a bare `File` is
not one). Explicitly **not** `tracing_appender::non_blocking` and **not** a
`BufWriter`: a `panic = "abort"` or hard exit would drop buffered WARN/panic
lines — the exact data the feature exists to keep. This mirrors the mailer idiom
(`mailer/file.rs` opens a plain `File` and `write_all`s per record; the bytes
hit the page cache in the syscall and survive process death).

## Wire-up (Nix / e2e harness)

- **Per-VM env:** set `JAUNDER_DIAG_LOG_FILE=/var/lib/jaunder/jaunder-diag.log`
  on the `jaunder` systemd service environment for **both** backends in
  `flake.nix` (the file is per-VM, so the in-VM path is constant; each e2e combo
  runs its own VM → per-combo file automatically, coordinating with #129's
  `{backend}×{browser}` matrix).
- **Copy-out (rename first — load-bearing):** `_grab` copies a file flat under
  its **source basename**, so a literal
  `_grab("/var/lib/jaunder/jaunder-diag.log")` would land `jaunder-diag.log` (no
  `-${backend}`) in `$out` and the host lift filter
  (`starts_with("jaunder-diag-")`, below) would **silently skip it**. So, like
  the OTel/report/tarball artifacts, rename into `/tmp` first:
  `machine.execute("cp /var/lib/jaunder/jaunder-diag.log /tmp/jaunder-diag-${backend}.log")`
  then `_grab("/tmp/jaunder-diag-${backend}.log")` — existence-guarded, within
  the existing "copy-all-before-assert" flow (ADR-0037).
- **Host lift:** add the `jaunder-diag-${backend}.log` prefix to the artifact
  filter that `xtask/src/steps/nix.rs` lifts from the kept outPath into
  `.xtask/diagnostics/<check>/` (uploaded by CI's per-combo e2e artifact /
  `validate-diagnostics`). Reusing `copy_e2e_diagnostics_between` covers both
  the success path and the `--keep-failed` `rescue_diagnostics` path. The
  `${backend}`- only name is safe because each combo is its own nix check → its
  own `$out` and `.xtask/diagnostics/<check>/` (same property the existing
  `jaunder-journal-${backend}.log` relies on); it would only collide if
  diagnostics were ever flattened across combos into one dir.

## Gate change (ADR-0032) — union with de-dup, scoped file authoritative

The zero-panic gate (`e2ePanicGate` in `flake.nix`) currently greps the full
journal for `panicked at`. It changes to source panics from the **union** of the
scoped diag file **and** the journal:

- **Union, not replace:** scan both for the raw substring `panicked at`; fail
  (default-deny) if it appears in **either**. Raw-substring scan (not strict
  JSONL parsing of the scoped file) so a rare torn line can't crash the gate.
  This preserves ADR-0032's guarantee with no completeness regression — a panic
  that fires _before_ the hook is installed (very early startup) reaches only
  the journal and is still caught.
- **De-dup key = panic location** (`path:line:col`, the token after
  `panicked at `), extracted from both sources with **one canonical extraction**
  applied identically to each: take the whitespace token after `panicked at `
  and **`rstrip(":")`**. This is load-bearing — the default Rust hook prints
  `…panicked at server/src/foo.rs:42:5:` (trailing colon, payload on the next
  line), while the scoped record's `location` field has **no** trailing colon; a
  naive compare would differ by that `:` and never merge, double-reporting every
  hooked panic. Paths already match byte-for-byte because both derive from the
  same `info.location()` (see record section). The extraction assumes the
  current toolchain's `panicked at <loc>:` format; a comment must pin that
  assumption (the pre-1.73 `panicked at 'msg', <loc>` form would break token
  position). There is **no unit test for this Python** (it runs only in the VM),
  so it must be correct by inspection.
- The same panic normally appears in both (hook → scoped file, chained default
  hook → journald), so the gate must merge, not double-report. For a
  default-deny gate, a de-dup _miss_ only double-prints the report; it never
  lets a panic through — so this is cosmetic, not a correctness hole, but worth
  getting right.
- **Report precedence:** for each unique location, the failure message prints
  the scoped file's JSON record (clean, single-line, payload inline); a location
  found **only** in the journal is reported from the journal line.
- **Allowlist preserved:** the existing `allowed_panics` substring allowlist
  still applies against the report line (default-deny; add a proven-benign
  substring + comment to bypass).
- Full journal remains captured as the fallback artifact.

## ADR

Record a **new ADR** (draft-out-of-git via `jaunder-adr`; numberless draft in
`docs/adr/drafts/`, numbered at ship by `cargo xtask adr promote`),
cross-linking ADR-0032 (zero-panic gate) and ADR-0037 (e2e diagnostics capture)
as the ancestors it extends. It captures the genuinely new decisions, which sit
awkwardly grafted onto 0037's "harness copies whatever the VM produced" story:

- **App-driven scoped capture** — the app emits its own purpose-built
  WARN+/panic stream, vs 0037's harness-copies model.
- **Why the panic hook bypasses `tracing` and uses an independent handle**
  (deadlock-safe unwind).
- **Why JSONL** (tool-able, extends the `otel-traces.jsonl` model).
- **The gate now sources panics from scoped ∪ journal** with de-dup and
  scoped-file precedence (a change to ADR-0032's mechanism).

## Docs

- `docs/observability.md`: add a "server-side scoped diagnostic log" section —
  where `jaunder-diag-<backend>.log` lands in the diagnostics dirs, its JSONL
  shape, and a "look here first" pointer (journal = last resort).
- `CONTRIBUTING.md` (Observability section): note the scoped diag log as the
  primary server-side failure log; per-combo location; journal demoted to
  fallback.

## Testability (coverage is a hard gate)

`init_tracing` installs a **global** subscriber and the panic hook is
process-global, so push the real logic behind pure/constructible seams and keep
the global-install glue thin:

- **Pure panic-record formatter** — `diag_panic_record(...) -> String` (serde
  struct → JSON line via `serde_json::to_string`) as a pure function. Unit tests
  synthesize panic info and assert the JSON shape, the literal `panicked at`
  substring, the `location` field equals `info.location().to_string()`, and a
  newline-bearing payload stays one physical line.
- **WARN+ filter over the real registry shape** — the regression test from the
  design section: build a registry with the **global `info` filter + the diag
  WARN layer + a second capture sink**, emit INFO/WARN/ERROR (via
  `tracing::subscriber::with_default(...)`, thread-local, not the process-global
  registry), and assert INFO is dropped from the diag sink while still reaching
  the second sink, and each diag line is valid JSON. This exercises the
  global+per-layer AND-gate, not the layer in isolation.
- **Thin glue, coverage-honest** — factor so the only glue line is the single
  `std::panic::set_hook(...)` call plus the env-var read; the decision logic and
  record construction live in the pure seams above. Cover the glue with a test
  that, under `ENV_LOCK`, `take_hook()`s the current hook, points
  `JAUNDER_DIAG_LOG_FILE` at a `TempDir`, installs, triggers a panic inside
  `catch_unwind`, asserts a record landed, and **restores the previous hook**
  (drop guard) so no stale global hook leaks into later tests (precedent: the
  `catch_unwind` test already in `observability.rs`). This satisfies the
  coverage gate on the new lines without polluting the shared test process.

## Acceptance criteria

- On an e2e failure, a small `jaunder-diag-<backend>.log` containing only WARN+
  and panics is captured and uploaded — no kernel/INFO-request noise.
- The zero-panic gate sources its panic detection from the scoped file (union
  with the journal, de-duped by location, scoped record wins the report).
- Full journal remains available as fallback.
- Production behavior is unchanged when `JAUNDER_DIAG_LOG_FILE` is unset.

## Coordination with #153 (dedupe Playwright config)

#153 is in flight in a sibling worktree and edits **the same e2e-plumbing
regions** this spec touches — no design conflict (its "no new env vars" ethos is
about the _Playwright invocation_; `JAUNDER_DIAG_LOG_FILE` is a _server-app_
capture var, a different surface), but a rebase collision:

- `flake.nix` `e2eRunAndCapture` copy-out block (~740–768): #153 repoints the
  json-report flat-copy and moves it into the `tar -C /tmp/e2e test-results`
  bundle; #144 adds a `_grab` for the diag log in the same block.
- `xtask/src/steps/nix.rs` artifact predicate: #153 relies on the
  `playwright-report-` prefix; #144 adds a `jaunder-diag-` prefix to the same
  filter.

Both sets of edits are additive and semantically independent. **Preferred order:
#153 lands first** (it _restructures_ the copy-out/artifact flow, so #144's diag
copy-out should follow whatever structure #153 establishes rather than build on
a block about to change). If #144 lands first, #153 absorbs one extra `_grab`
line — also fine. Whoever lands second rebases around a handful of lines; the
plan's e2e-wiring task should re-read the current copy-out block rather than
assume today's line numbers.

## Out of scope / follow-ups

- **Consolidating the capture-to-file env vars** (`JAUNDER_MAIL_CAPTURE_FILE`,
  `JAUNDER_WEBSUB_CAPTURE_FILE`, `JAUNDER_DIAG_LOG_FILE`) into a single
  output-dir contract — filed as **#227**. #144 deliberately mirrors the
  existing single-`_FILE` idiom to stay in scope; the consolidation pays off
  once there are three streams and is tracked separately.
- A configurable diag-log level env var (`JAUNDER_DIAG_LOG_FILTER`) — deferred
  until a concrete need; the contract is hardcoded WARN+ for now.
- A dedicated `analyze-*` tool over the diag JSONL — the file is JSONL so this
  is possible later; not built here.
- The companion scoped `nix build` failure-excerpt for xtask (filed separately
  per the issue).
