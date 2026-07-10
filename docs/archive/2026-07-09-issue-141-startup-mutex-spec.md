# Spec — #141: refuse to start on a live `runtime.json` (start-up mutex + stale detection)

- Issue: jaunder-org/jaunder#141
- Milestone: Emacs blogging front-end (#4)
- Status: draft (awaiting approval)
- Date: 2026-07-09

## Problem

`jaunder serve` writes `runtime.json` (`{ip, port}`, ADR-0035) and — as of #140
— removes it signal-robustly on a normal stop. But nothing stops a **second**
`serve` from starting on the **same data dir** while a first instance is
running: they would contend on the same SQLite file / storage path. The runtime
file records no owner, so it cannot answer "is an instance already running
here?".

This is the read-side complement to #140's write-side removal: graceful-remove
(write)

- stale detection (read) together make `runtime.json` a reliable "an instance is
  running here" signal.

## Goal

On `serve` startup, if `runtime.json` exists and records a **live writer
process** (its pid is alive _and_ has the recorded start-time), **refuse to
start** with a clear error. If the recorded process is **gone** (dead pid, or
pid recycled to a different process — start-time differs), or the file is
otherwise unusable, treat it as **stale**: warn and overwrite, then proceed.

## Design decisions (resolved in the interview)

1. **pid + start-time fields (not flock).** Record enough to identify the exact
   writer _process_: `runtime.json` now carries
   `{ "ip", "port", "pid", "start_time" }`, where `pid = std::process::id()` and
   `start_time` is that process's start-time (jiffies since boot,
   `/proc/self/stat` field 22). `start_time` is read **early** (before the DB
   opens — decision 3) and threaded into the write, so `write_atomic` / `write`
   / `for_serve` gain a `start_time: u64` parameter (a signature change; pid
   stays internal via `std::process::id()`). It is **not** best-effort: if we
   cannot read our own start-time at startup, that is a **hard fail**
   (decision 5) — a runtime that can't use `/proc` can't enforce the mutex at
   all, so we refuse rather than serve with a silently-broken guard. A
   flock-based lock was considered and rejected: it would largely obviate #140's
   graceful-removal work, is advisory-only, and has NFS caveats. The residual
   TOCTOU window (two servers racing between the early read-check and the
   post-bind write) is accepted and documented; the true contended resource (the
   SQLite file) and the TCP bind provide backstops.

2. **Liveness via `/proc/<pid>/stat` start-time, compared to the recorded
   start-time.** The holder is a _live writer_ iff `/proc/<pid>/stat` exists
   **and** its start-time (field 22) equals the `start_time` recorded in
   `runtime.json`. This pins the **exact process** that wrote the file — pid +
   start-time is the canonical Linux way to identify a process instance across
   pid reuse. We deliberately do **not** use `comm` or `/proc/<pid>/exe` for
   identity: both identify the **binary**, not the **process instance**, so
   neither can tell our (dead) writer apart from a _different_ live process that
   reused its pid — including another `jaunder`, which is exactly the pid-reuse
   case the mutex must get right. (`comm` is also a truncated, mutable basename;
   `exe` also needs same-user read permission.) A process's start-time, by
   contrast, pins the specific instance and never false-matches a recycled pid.
   It depends on neither the binary nor its name, is not truncated, and
   `/proc/<pid>/stat` is world-readable. Dependency-free (`std::fs`),
   Linux-native (jaunder targets Linux/NixOS), consistent with #140's
   no-runtime-dependency stance.

   **`/proc/<pid>/stat` parsing (pure, testable).** A pure
   `parse_stat_start_time(&str) -> Option<u64>` does the field extraction: field
   2 (`comm`) is wrapped in parens and may contain spaces and `)`, so parse from
   the **last** `)`, then **`split_whitespace`** the remainder (which coalesces
   the leading space after `)`); start-time is index **19** of that split (the
   20th field after `comm`). Returns `None` on any malformed input (no `)`, too
   few fields, non-numeric). Unit-tested directly with a real line and malformed
   lines.

   **Read semantics — path-injectable so every arm is a real test (no
   `cov:ignore`).** The read is split so its error arms are exercised by planted
   files rather than marked.
   `read_start_time_at(path: &Path) -> io::Result<Option<u64>>`:
   - `Ok(Some(t))` — read and parsed (planted valid-format file);
   - `Ok(None)` — path **`NotFound`**: for `/proc/<pid>/stat` this is the
     _dead-pid_ signal (stale); this arm is the ordinary "pid gone" case, not a
     failure;
   - `Err(_)` — a non-`NotFound` I/O error (e.g. a path that is a directory)
     **or** a read-but-**unparseable** file (`parse_stat_start_time` → `None`):
     the read mechanism is unusable → the caller **hard-fails**.

   Because the path is a parameter, unit tests plant a temp file to drive
   **all** arms — valid stat text → `Ok(Some)`; absent path → `Ok(None)`;
   garbage text → `Err` (unparseable); a directory path → `Err` (I/O). **No
   `cov:ignore` is needed.** The `/proc` binding is a thin wrapper,
   `read_proc_start_time(pid) = read_start_time_at(Path::new(&format!("/proc/{pid}/stat")))`,
   covered by the self-pid (`Ok(Some)`) and `u32::MAX` (`Ok(None)`) vectors.

   **Predicate structure — coverage-clean.** `holder_is_live` decides in a
   **single** match, propagating the hard-fail:

   ```rust
   /// `recorded` is the start_time stored in runtime.json.
   /// Err propagates a hard fail (unusable /proc); Ok(false) = dead/mismatch.
   fn holder_is_live(pid: u32, recorded: u64) -> io::Result<bool> {
       Ok(match read_proc_start_time(pid)? {  // Err (unusable /proc) -> hard fail
           Some(actual) => actual == recorded,
           None => false,                     // NotFound => pid dead
       })
   }
   ```

   The `Some(actual) => actual == recorded` line is covered by both the matching
   vector (self pid + self's real start-time → `true`) and the mismatch vector
   (self pid + a wrong start-time → `false`); the `None => false` line by the
   dead-pid vector (`u32::MAX`); the `?` error-return is a value branch on a
   line the `Ok` vectors already cover. **No `cov:ignore` anywhere in this
   module** — the hard-fail arms live in `read_start_time_at`, which the
   planted-file tests above cover directly.

3. **Check early — before opening the DB.** The read-check runs first thing in
   `prepare_server`, **before** `open_existing_database` / auto-init / worker
   start, so a refusal never opens a DB or touches a data dir another instance
   owns, and avoids SQLite lock contention. The `pid`+`addr` **write** still
   happens after `bind` (as #140), because the address isn't known until then.

4. **Path resolution shared.** The runtime-file path (explicit `--runtime-file`
   / `JAUNDER_RUNTIME_FILE` override, else `<storage_path>/runtime.json`) is
   resolved once by a shared helper used by both the early check and the late
   write (today it lives inside `RuntimeFileGuard::for_serve`; extract it so the
   check can reuse it).

5. **Refusal, stale, and hard-fail behavior.** Four outcomes, two of which stop
   startup:
   - **Live holder** → `prepare_server` returns an error; `cmd_serve` propagates
     it and the process exits **non-zero** with a message naming the holding pid
     and the data dir (e.g.
     `another jaunder instance is already running on this data dir (pid 1234); refusing to start`).
     No PII (ADR-0011).
   - **Hard fail (unusable `/proc`)** → we cannot read our own start-time at
     startup, a holder's `/proc/<pid>/stat` gives a non-`NotFound` I/O error, or
     a `stat` we _did_ read is unparseable. `prepare_server` returns an error
     and `serve` exits non-zero — we do **not** silently proceed with a broken
     mutex. (Distinct from a `NotFound`, which is the normal dead-pid signal
     below.)
   - **Stale** (dead pid = `/proc/<pid>/stat` `NotFound`; start-time mismatch;
     missing/`null` `pid`/`start_time`; unparseable **runtime.json**) → one
     `WARN` that a stale runtime file was found and is being overwritten, then
     proceed normally (the post-bind write overwrites it). Note: unparseable
     _runtime.json_ is stale (our own file, non-authoritative if corrupt);
     unparseable _/proc stat_ is a hard fail (system corruption) — different
     sources, different severities.
   - **No file** → proceed silently (the common fresh-start case).

   **Where the log/decision lives (coverage-relevant).** The check **function**
   returns `io::Result<StartupCheck>` — `Err` is the hard fail — and owns the
   stale `WARN`:

   ```rust
   enum StartupCheck { Refuse { pid: u32 }, Stale, Proceed }
   // check_startup_mutex(path) -> io::Result<StartupCheck>
   //   no file                              -> Ok(Proceed)
   //   live writer (pid + start_time match) -> Ok(Refuse { pid })
   //   dead / start-time mismatch / pid-or-start_time-less / unparseable-json
   //                                        -> tracing::warn!(...) ; Ok(Stale)
   //   unusable /proc (holder_is_live? => Err)
   //                                        -> Err  (hard fail)
   ```

   Emitting the `WARN` **inside** the function means the AC3/AC4 unit tests
   (which call it) both assert the returned `Ok(Stale)` **and** execute the
   `WARN` line — so it is covered without a heavier `prepare_server`-level stale
   test. `prepare_server` only **maps** the outcome: `Err` → propagate (hard
   fail); `Ok(Refuse { pid })` → construct and return the `anyhow` mutex error
   (covered by AC6's test); `Ok(Stale)` / `Ok(Proceed)` → continue. Separately,
   `prepare_server` establishes **our own** start-time early via a
   path-injectable `require_start_time_at(path) -> anyhow::Result<u64>` =
   `read_start_time_at(path)?.ok_or_else(|| anyhow!("cannot read own start-time"))?`
   — hard-failing on both `Err` and `Ok(None)`. Its own tests plant paths (valid
   → `Ok`; absent → the `None → Err` line; garbage → `Err`), so **that line is
   covered too** — no `cov:ignore`. `prepare_server` calls
   `require_start_time_at("/proc/self/stat")` and threads the value into the
   post-bind write.

   **Signature-change fan-out.** Adding `start_time: u64` to `write_atomic` /
   `write` / `for_serve` touches every caller — the production `for_serve` in
   `prepare_server`, **the `write` caller in the #140 signal test in
   `commands.rs`**, and the `runtime_file.rs` unit-test call sites (~7 total).
   The plan must update all of them (don't miss the `commands.rs` signal-test
   caller).

6. **Testability.** Fully host-testable — **no `cov:ignore`** — using the
   current process as the "live writer" and planted temp files for the read
   arms:
   - `parse_stat_start_time(&str) -> Option<u64>` (pure): a real `stat` line →
     `Some`; malformed lines (no `)`, too few fields, non-numeric field 22) →
     `None`.
   - `read_start_time_at(path) -> io::Result<Option<u64>>`
     (**path-injectable**): planted valid-format file → `Ok(Some)`; absent path
     → `Ok(None)`; garbage file → `Err` (unparseable); a directory path → `Err`
     (I/O). All four arms are real tests.
   - `read_proc_start_time(pid)` (thin `/proc` wrapper): `Ok(Some)` for
     `std::process::id()`; `Ok(None)` for **`u32::MAX`** — above `pid_max` on
     any Linux system, so `/proc/<u32::MAX>/stat` never exists (`NotFound`).
     `u32::MAX` is the deterministic dead-pid vector; **not** a spawn-then-reap
     pid (racy).
   - `require_start_time_at(path)`: planted valid → `Ok(v)`; absent → `Err` (the
     `None → Err` line); garbage → `Err`. Covers the own-start-time hard-fail
     line.
   - `holder_is_live(pid, recorded)`: `(self_pid, our real start-time)` →
     `Ok(true)`; `(self_pid, wrong start-time)` → `Ok(false)` (pid-reuse case);
     `(u32::MAX, _)` → `Ok(false)`.
   - The check function returns `io::Result<StartupCheck>` — planted-file tests
     cover: `{pid: self, start_time: <our real start-time>}` → `Ok(Refuse)`;
     `{pid: self,   start_time: <wrong>}` → `Ok(Stale)`; `{pid: u32::MAX, …}` →
     `Ok(Stale)`; a `{ip,port}`-only (pre-#141) file → `Ok(Stale)`; corrupt JSON
     → `Ok(Stale)`; absent → `Ok(Proceed)`. (A test reads its own start-time via
     `read_proc_start_time(std::process::id())` to build the matching vector.)
     The refuse-path wiring through `prepare_server` (early return before DB
     open) gets one test: a planted self-pid + real-start-time runtime file
     makes `prepare_server` return the mutex error without initializing a
     database. The hard-fail _behavior_ through the real `/proc` binding can't
     be forced in a test, but the arms it would hit are the `read_start_time_at`
     / `require_start_time_at` arms already covered by planted files.

7. **Documentation.** Amend ADR-0035: contents are now
   `{ip, port, pid, start_time}`; document the start-up mutex + stale-detection
   semantics (pid + start-time identity) and mark that deferred follow-on (#141)
   delivered. Also update the `runtime_file.rs` **module doc comment**
   (currently "`{ip, port}`" + "Follow-ons add a `pid`") to match the new
   contents.

## Out of scope (explicitly deferred)

- **`admin_token` + `jaunder shut-down` channel** — #142.
- **An atomic OS lock (flock/`O_EXCL`)** — deliberately not chosen (decision 1).
- **Cross-host / networked-filesystem correctness** — the mutex is
  local-data-dir, Linux `/proc`; NFS data dirs are out of scope.
- **Non-Linux liveness** — `/proc`-based; jaunder targets Linux/NixOS.
- **Binary-identity checks (`comm`, `/proc/<pid>/exe`)** — deliberately not
  used: they identify the _binary_, not the writer _process_, so a recycled pid
  running another `jaunder` would be mistaken for our still-live writer (a false
  refusal). pid + start-time identifies the specific writer process instead.

## Acceptance criteria (observable)

- **AC1 — `pid` + `start_time` are written.** After `serve` binds,
  `runtime.json` contains `{ip, port, pid, start_time}` where
  `pid == std::process::id()` and `start_time` equals that process's
  `/proc/self/stat` field 22. _(unit test on the write, passing a known
  `start_time`.)_
- **AC1b — unusable stat is a hard fail, not silent degrade.** A read that
  succeeds but is unparseable, or a non-`NotFound` I/O error, yields `Err` (→
  `serve` refuses to start), distinct from a `NotFound` (dead pid → stale).
  _(directly testable via the path-injectable read:
  `read_start_time_at(garbage_file)` → `Err`; `read_start_time_at(a_directory)`
  → `Err`; `read_start_time_at(missing)` → `Ok(None)`; and
  `require_start_time_at(missing)` → `Err` for the own-start-time hard-fail
  line. No `cov:ignore`.)_
- **AC2 — live writer (pid + start-time match) is refused.** With a
  `runtime.json` recording the current pid **and** its real start-time, the
  check returns _Refuse_ naming that pid. _(unit test planting
  `std::process::id()` + `read_proc_start_time` of self.)_
- **AC3 — dead pid is stale → overwrite.** With a `runtime.json` recording
  `pid = u32::MAX` (deterministically absent — above `pid_max`), the check
  returns _Stale_ (warns + proceeds). _(unit test; `u32::MAX`, not a reaped
  pid.)_
- **AC3b — start-time mismatch (pid reuse) is stale.** With `runtime.json`
  recording the current pid but a **wrong** start-time, the check returns
  _Stale_ — a recycled pid is not treated as a live holder. _(unit test.)_
- **AC4 — legacy / pid-less file is stale.** A `{ip,port}`-only file (no `pid` /
  `start_time`) is treated as _Stale_, not a hard refusal. _(unit test.)_
- **AC5 — no file → proceed.** Absent `runtime.json` yields _Proceed_. _(unit
  test.)_
- **AC6 — refusal is early, before the DB opens.** With a live holder present,
  `prepare_server` returns the mutex error **before** `open_existing_database` /
  auto-init, so the DB is **not** created. _(test: planted self-pid file + a
  non-existent DB path → `prepare_server` errs with the mutex message and the DB
  file is never created.)_ The non-zero **process** exit is trivial `?`
  propagation from `cmd_serve` and is not separately asserted in the unit suite
  (entering `cmd_serve` past `prepare_server` would hit the serve loop); the
  error-return above is the observable.
- **AC7 — read helpers + liveness predicate correct and tested.**
  `parse_stat_start_time` → `Some` for a real `stat` line, `None` for malformed;
  `read_start_time_at` → `Ok(Some)` / `Ok(None)` / `Err` for planted valid /
  missing / garbage(+directory) paths; `read_proc_start_time` → `Ok(Some)` for
  `std::process::id()`, `Ok(None)` for `u32::MAX`; `holder_is_live` → `Ok(true)`
  only for `(self pid, self's real start-time)`, `Ok(false)` for a wrong
  start-time or `u32::MAX`. _(unit tests.)_
- **AC8 — no new runtime dependency.** Liveness uses `/proc` via `std::fs`;
  nothing is added to `[dependencies]`. _(Cargo.toml diff; `cargo deny`
  passes.)_
- **AC9 — existing behavior preserved.** #140's graceful-removal behavior is
  unchanged and the `runtime_file` / signal tests still pass **after the
  mechanical `start_time` call-site update** (they can't compile unchanged —
  decision 5 fan-out); the elisp harness (unique per-run `--runtime-file`) is
  unaffected. _(tests green post-update.)_
- **AC10 — gate green.** `cargo xtask validate --no-e2e` passes (static +
  clippy + coverage) with **no `cov:ignore`/`crap:allow` added** for this
  change, and no unapproved lint suppressions.
- **AC11 — ADR-0035 updated** to `{ip,port,pid,start_time}` + mutex/stale
  semantics (pid + start-time identity), #141 marked delivered. _(diff to
  `docs/adr/0035-elisp-live-integration-harness.md`.)_
