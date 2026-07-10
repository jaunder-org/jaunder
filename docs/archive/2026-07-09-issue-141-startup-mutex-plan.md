# #141 — `serve` Start-up Mutex + Stale Detection Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** `jaunder serve` refuses to start when `runtime.json` records a live
writer process (pid + start-time), warns-and-overwrites a stale one, and
hard-fails if `/proc` is unusable.

**Architecture:** Extend `runtime.json` to `{ip, port, pid, start_time}`. A
path-injectable `read_start_time_at` (over a pure `parse_stat_start_time`) reads
`/proc/<pid>/stat` field 22; `NotFound` = dead pid (stale), any other read/parse
error = hard fail. `prepare_server` reads our own start-time early (hard-fail if
it can't) and, before opening the DB, runs `check_startup_mutex` on the existing
file: a live-writer match → refuse; else stale/proceed.

**Tech Stack:** Rust, `std::fs` (`/proc`), `serde_json`, `axum` — **no new
dependency**.

**Spec:** `docs/superpowers/specs/2026-07-09-issue-141-startup-mutex.md` — this
plan is "how"; the spec is "what/why". Task N ↔ acceptance criteria noted
inline.

## Global Constraints

_Every task's requirements implicitly include these._

- **No new dependency** — `/proc` via `std::fs`, pid via `std::process::id()`,
  existing `serde_json`; nothing added to `[dependencies]` or
  `[dev-dependencies]` (spec AC8).
- **No `.unwrap()`/`.expect()` in production** — clippy
  `unwrap_used`/`expect_used` denied outside `#[cfg(test)]`. Every
  `Result`/`Option` handled explicitly.
- **No `cov:ignore` / `crap:allow` added** — the path-injectable reads make
  every arm a real test (spec decision 2/6); if the gate reports an uncovered
  line, add a test, do not mark it. (spec AC10)
- **Dead-code rule** — clippy `-D warnings` flags a `pub(crate) fn` whose only
  caller is `#[cfg(test)]` as unused. Each new fn lands with a real (non-test)
  caller in the same commit — hence the task split below.
- **Commits:** Conventional Commits; run `cargo xtask check` clean before
  committing (**jaunder-commit**). **No `Co-Authored-By` trailer.**
- **Out of scope:** #142 admin channel, flock, NFS/cross-host, non-Linux (the
  `/proc` reads compile everywhere but only function on Linux; off-Linux,
  startup hard-fails — jaunder targets Linux/NixOS, so this is acceptable and
  un-`cfg`-gated).

## Task list (reviewer summary)

1. **Record identity + own-start hard-fail (write side).** `runtime.json` →
   `{ip, port, pid, start_time}`; `parse_stat_start_time` / `read_start_time_at`
   / `require_start_time_at`; `prepare_server` reads own start-time early
   (hard-fail) and threads it into the write; update all `write`/`for_serve`
   call sites. (AC1, AC1b, AC7-part, AC8.)
2. **Start-up mutex check (read side / refuse).** `read_proc_start_time`,
   `holder_is_live`, `StartupCheck`, `check_startup_mutex`,
   `resolve_runtime_path`; wire the early check into `prepare_server`. (AC2,
   AC3, AC3b, AC4, AC5, AC6, AC7-rest.)
3. **Docs — ADR-0035 + module doc comment.** (AC11, plus AC9/AC10 verified
   across the branch.)

**Key risks/decisions:**

- Identity is **pid + start-time**, not `comm`/`exe` (spec decision 2):
  binary-name/path can't tell our writer from a recycled pid running another
  `jaunder`.
- `NotFound` = dead (soft, stale); any other `/proc` read/parse failure = **hard
  fail** (spec decision 5). The split is
  `read_to_string(...).kind() == NotFound`.
- Path-injectable reads ⇒ the hard-fail arms are real tests ⇒ **no
  `cov:ignore`**.
- `/proc/<pid>/stat` field 22 = **index 19** after splitting the post-last-`)`
  remainder with **`split_whitespace`** (coalesces the leading space).
- `start_time: u64` param fans out to **9** call sites (7 in `runtime_file.rs`
  tests + `for_serve`@`commands.rs:450` + the #140 signal-test
  `write`@`commands.rs:770`) — don't miss the signal test.

---

### Task 1: Record identity + own-start hard-fail (write side)

**Files:**

- Modify: `server/src/runtime_file.rs` — `write_atomic` (`:14-23`), `write`
  (`:50`), `for_serve` (`:64`), module doc (`:1-6`); add
  `parse_stat_start_time`, `read_start_time_at`, `require_start_time_at`; tests.
- Modify: `server/src/commands.rs` — `prepare_server` (`:377-459`, read own
  start-time early + pass to `for_serve` at `:450`); update the signal-test
  `write` caller (`:770`).

**Interfaces:**

- Produces:
  - `pub(crate) fn parse_stat_start_time(stat: &str) -> Option<u64>`
  - `pub(crate) fn read_start_time_at(path: &std::path::Path) -> std::io::Result<Option<u64>>`
  - `pub(crate) fn require_start_time_at(path: &std::path::Path) -> anyhow::Result<u64>`
  - `write_atomic(path, addr, start_time: u64)`,
    `RuntimeFileGuard::write(path, addr, start_time: u64)`,
    `RuntimeFileGuard::for_serve(override, storage_path, addr, start_time: u64)`
    — all gain the trailing `start_time` param.
- Consumes: `std::process::id()`.

- [x] **Step 1: Write the failing tests** — append to `runtime_file.rs`
      `mod tests`:

```rust
#[test]
fn parse_stat_start_time_reads_field_22() {
    // pid (comm may contain spaces and ')') state ppid ... field22=starttime
    let line = "1234 (jaunder blog) S 1 1234 1234 0 -1 4194560 100 0 0 0 \
                1 2 0 0 20 0 1 0 987654 12345 0 ...";
    assert_eq!(parse_stat_start_time(line), Some(987654));
}

#[test]
fn parse_stat_start_time_rejects_malformed() {
    assert_eq!(parse_stat_start_time(""), None);
    assert_eq!(parse_stat_start_time("no parens here"), None);
    assert_eq!(parse_stat_start_time("1 (x) S 1"), None); // too few fields
    assert_eq!(parse_stat_start_time("1 (x) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 0 1 0 notnum"), None);
}

#[test]
fn read_start_time_at_arms() {
    let dir = TempDir::new().unwrap();
    // valid
    let ok = dir.path().join("stat");
    std::fs::write(&ok, "1 (x) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 0 1 0 555 0").unwrap();
    assert_eq!(read_start_time_at(&ok).unwrap(), Some(555));
    // absent -> Ok(None)
    assert_eq!(read_start_time_at(&dir.path().join("nope")).unwrap(), None);
    // garbage -> Err
    let bad = dir.path().join("bad");
    std::fs::write(&bad, "garbage").unwrap();
    assert!(read_start_time_at(&bad).is_err());
    // a directory -> Err (I/O)
    assert!(read_start_time_at(dir.path()).is_err());
}

#[test]
fn require_start_time_at_arms() {
    let dir = TempDir::new().unwrap();
    let ok = dir.path().join("stat");
    std::fs::write(&ok, "1 (x) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 0 1 0 777 0").unwrap();
    assert_eq!(require_start_time_at(&ok).unwrap(), 777);
    assert!(require_start_time_at(&dir.path().join("nope")).is_err()); // None -> Err
    // our own real stat parses:
    assert!(require_start_time_at(std::path::Path::new("/proc/self/stat")).is_ok());
}

#[test]
fn writes_pid_and_start_time_json() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("runtime.json");
    let _guard = RuntimeFileGuard::write(path.clone(), addr(), 4242);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["ip"], "127.0.0.1");
    assert_eq!(v["port"], 34567);
    assert_eq!(v["pid"], std::process::id());
    assert_eq!(v["start_time"], 4242);
}
```

Also update every existing `write`/`for_serve` call site to pass a `start_time`
arg (e.g. `0`) — they won't compile otherwise. In `runtime_file.rs` tests:
`writes_ip_and_port_json`, `removes_file_on_drop`,
`write_failure_yields_inert_guard` (`write`);
`for_serve_defaults_into_storage_dir`, `for_serve_honors_override`
(`for_serve`); and **`path_is_some_for_active_guard_and_none_for_inert`, which
has two `write` calls** (`:170`, `:172`). That's 7 test sites; plus
`for_serve`@`commands.rs:450` and the signal-test `write`@`commands.rs:770` =
**9 total**.

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p jaunder --lib runtime_file` Expected: FAIL to compile
— `parse_stat_start_time` / `read_start_time_at` / `require_start_time_at`
undefined; `write`/`for_serve` arity mismatch.

- [x] **Step 3: Implement**

In `runtime_file.rs`:

```rust
/// Field 22 (start-time, jiffies since boot) of a `/proc/<pid>/stat` line, or
/// `None` if malformed. Field 2 (comm) is paren-wrapped and may contain spaces
/// and `)`, so parse from the LAST `)`; after it, `split_whitespace` coalesces
/// the leading space and start-time is index 19 (the 20th field after comm).
pub(crate) fn parse_stat_start_time(stat: &str) -> Option<u64> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19)?.parse().ok()
}

/// `Ok(Some)` when `path` reads and parses; `Ok(None)` when it does not exist
/// (`NotFound` — a dead pid for `/proc/<pid>/stat`); `Err` on any other I/O error
/// or an unparseable file (the `/proc` mechanism is unusable → caller hard-fails).
pub(crate) fn read_start_time_at(path: &Path) -> std::io::Result<Option<u64>> {
    match std::fs::read_to_string(path) {
        Ok(s) => parse_stat_start_time(&s).map(Some).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unparseable /proc stat")
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads a required start-time (our own): a missing file or read error is a hard
/// fail — a runtime that can't read `/proc` can't enforce the mutex.
pub(crate) fn require_start_time_at(path: &Path) -> anyhow::Result<u64> {
    read_start_time_at(path)?
        .ok_or_else(|| anyhow::anyhow!("cannot read own start-time from {}", path.display()))
}
```

Add `start_time: u64` to `write_atomic` (emit
`"pid": std::process::id(), "start_time": start_time`), `write`, and `for_serve`
(thread it through). Update the module doc (`:1-6`) to state contents are
`{ip, port, pid, start_time}` and that pid+start-time is the start-up-mutex
identity (#141).

In `commands.rs` `prepare_server`, **before** `open_existing_database` (`:383`):

```rust
    // Establish our own start-time up front: if /proc is unusable we cannot enforce
    // the start-up mutex, so refuse rather than serve with a broken guard (#141).
    let start_time =
        crate::runtime_file::require_start_time_at(std::path::Path::new("/proc/self/stat"))?;
```

Thread `start_time` into the `for_serve` call (`:450`). Update the signal-test
`write` caller in `commands.rs` (`:770`) to pass a `start_time` (e.g. `0`).

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p jaunder --lib runtime_file` Expected: PASS (new +
updated). Run:
`cargo nextest run -p jaunder --lib -E 'test(drains_and_removes_runtime_file)'`
Expected: PASS (the #140 signal tests still pass with the updated `write` call).

- [x] **Step 5: Commit** (run `cargo xtask check` first — **jaunder-commit**)

```bash
git add server/src/runtime_file.rs server/src/commands.rs
git commit -m "feat(serve): record pid + start-time in runtime.json; hard-fail on unusable /proc"
```

---

### Task 2: Start-up mutex check (read side / refuse)

**Files:**

- Modify: `server/src/runtime_file.rs` — add `read_proc_start_time`,
  `holder_is_live`, `StartupCheck`, `check_startup_mutex`,
  `resolve_runtime_path`; factor `for_serve`'s path resolution through
  `resolve_runtime_path`; tests.
- Modify: `server/src/commands.rs` — `prepare_server`: resolve the runtime path
  and run the early check; map the outcome; a `#[cfg(test)]` refuse test.

**Interfaces:**

- Consumes: `read_start_time_at`, `require_start_time_at` (Task 1).
- Produces:
  - `pub(crate) fn read_proc_start_time(pid: u32) -> std::io::Result<Option<u64>>`
  - `pub(crate) fn resolve_runtime_path(override_path: Option<&Path>, storage_path: &Path) -> PathBuf`
  - `pub(crate) enum StartupCheck { Refuse { pid: u32 }, Stale, Proceed }`
  - `pub(crate) fn check_startup_mutex(path: &Path) -> std::io::Result<StartupCheck>`

- [x] **Step 1: Write the failing tests** — append to `runtime_file.rs`
      `mod tests`:

```rust
fn own_start_time() -> u64 {
    require_start_time_at(std::path::Path::new("/proc/self/stat")).unwrap()
}

fn write_runtime(path: &std::path::Path, json: serde_json::Value) {
    std::fs::write(path, json.to_string()).unwrap();
}

#[test]
fn read_proc_start_time_self_and_dead() {
    assert!(read_proc_start_time(std::process::id()).unwrap().is_some());
    assert_eq!(read_proc_start_time(u32::MAX).unwrap(), None); // above pid_max => NotFound
}

#[test]
fn holder_is_live_matrix() {
    let me = std::process::id();
    assert!(holder_is_live(me, own_start_time()).unwrap());        // exact writer
    assert!(!holder_is_live(me, own_start_time() + 1).unwrap());   // pid reuse (start-time differs)
    assert!(!holder_is_live(u32::MAX, 0).unwrap());                // dead pid
}

#[test]
fn check_startup_mutex_outcomes() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("runtime.json");
    let me = std::process::id();

    // absent -> Proceed
    assert!(matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Proceed));

    // live writer (our pid + real start-time) -> Refuse { pid: me }
    write_runtime(&p, serde_json::json!({"ip":"127.0.0.1","port":1,"pid":me,"start_time":own_start_time()}));
    assert!(matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Refuse { pid } if pid == me));

    // our pid + wrong start-time (reuse) -> Stale
    write_runtime(&p, serde_json::json!({"ip":"127.0.0.1","port":1,"pid":me,"start_time":own_start_time()+1}));
    assert!(matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Stale));

    // dead pid -> Stale
    write_runtime(&p, serde_json::json!({"ip":"127.0.0.1","port":1,"pid":u32::MAX,"start_time":0}));
    assert!(matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Stale));

    // legacy {ip,port}-only -> Stale
    write_runtime(&p, serde_json::json!({"ip":"127.0.0.1","port":1}));
    assert!(matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Stale));

    // corrupt JSON -> Stale
    std::fs::write(&p, "not json").unwrap();
    assert!(matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Stale));
}
```

And in `commands.rs` `mod tests`, a refuse-wiring test (AC6) — models the
existing `prepare_server_auto_initializes_in_dev_mode` (`:729`) for building
`StorageArgs`:

```rust
#[tokio::test]
async fn prepare_server_refuses_on_live_holder_before_db_open() {
    let base = TempDir::new().unwrap();
    let storage = /* StorageArgs with storage_path = base, a sqlite db path under base */;
    // Plant a live-writer runtime.json at the resolved path (storage_path/runtime.json).
    let rt = base.path().join("runtime.json");
    let start = crate::runtime_file::require_start_time_at(
        std::path::Path::new("/proc/self/stat")).unwrap();
    std::fs::write(&rt, serde_json::json!({
        "ip":"127.0.0.1","port":1,"pid":std::process::id(),"start_time":start
    }).to_string()).unwrap();

    let bind = "127.0.0.1:0".parse().unwrap();
    let err = prepare_server(&storage, bind, false, None).await.unwrap_err();
    assert!(err.to_string().contains("already running"));
    // Refused before open_existing_database/auto-init: no DB file created.
    assert!(!<db path>.exists());
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run:
`cargo nextest run -p jaunder --lib -E 'test(check_startup_mutex_outcomes) | test(holder_is_live_matrix) | test(read_proc_start_time_self_and_dead) | test(refuses_on_live_holder)'`
Expected: FAIL to compile — the new items are undefined.

- [x] **Step 3: Implement**

In `runtime_file.rs`:

```rust
pub(crate) fn read_proc_start_time(pid: u32) -> std::io::Result<Option<u64>> {
    read_start_time_at(Path::new(&format!("/proc/{pid}/stat")))
}

/// `Ok(true)` iff pid is live AND its start-time matches `recorded` (the exact
/// writer). `Ok(false)` = dead pid or start-time mismatch. `Err` = unusable /proc.
pub(crate) fn holder_is_live(pid: u32, recorded: u64) -> std::io::Result<bool> {
    Ok(match read_proc_start_time(pid)? {
        Some(actual) => actual == recorded,
        None => false,
    })
}

pub(crate) enum StartupCheck { Refuse { pid: u32 }, Stale, Proceed }

/// Reads the runtime file at `path` and decides whether a live writer holds it.
/// `Err` propagates a hard fail (unusable /proc). Corrupt/legacy/missing-field
/// runtime.json is Stale (our own non-authoritative file). Emits the stale WARN.
pub(crate) fn check_startup_mutex(path: &Path) -> std::io::Result<StartupCheck> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(StartupCheck::Proceed); // no file (or unreadable-as-absent) -> fresh start
    };
    let stale = |reason: &str| {
        tracing::warn!(path = %path.display(), reason, "stale runtime file; overwriting");
        StartupCheck::Stale
    };
    let (Some(pid), Some(recorded)) = serde_json::from_str::<serde_json::Value>(&contents)
        .ok()
        .map(|v| (v["pid"].as_u64().and_then(|p| u32::try_from(p).ok()), v["start_time"].as_u64()))
        .unwrap_or((None, None))
    else {
        return Ok(stale("missing pid/start_time or corrupt json"));
    };
    Ok(if holder_is_live(pid, recorded)? {
        StartupCheck::Refuse { pid }
    } else {
        stale("recorded process is gone")
    })
}
```

Extract
`resolve_runtime_path(override_path: Option<&Path>, storage_path: &Path) -> PathBuf`
(the `override.unwrap_or_else(|| storage_path.join("runtime.json"))` currently
inside `for_serve`) and call it from `for_serve`.

In `commands.rs` `prepare_server`, right after the `start_time` read (Task 1),
before `open_existing_database`:

```rust
    let runtime_path = crate::runtime_file::resolve_runtime_path(
        runtime_file.as_deref(), &storage.storage_path);
    match crate::runtime_file::check_startup_mutex(&runtime_path)? {   // Err -> hard fail
        crate::runtime_file::StartupCheck::Refuse { pid } => {
            anyhow::bail!(
                "another jaunder instance is already running on this data dir (pid {pid}); \
                 refusing to start"
            );
        }
        crate::runtime_file::StartupCheck::Stale | crate::runtime_file::StartupCheck::Proceed => {}
    }
```

(`for_serve` at `:450` already receives `runtime_file`; leave it — or pass
`Some(runtime_path)` to avoid re-resolving. Either is fine; keep one resolution
path.)

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p jaunder --lib runtime_file` Expected: PASS (all
`check_startup_mutex`/`holder_is_live`/`read_proc_start_time` vectors). Run:
`cargo nextest run -p jaunder --lib -E 'test(refuses_on_live_holder)'` Expected:
PASS. Run: `cargo nextest run -p jaunder` Expected: PASS (incl. existing
`prepare_server_*`, spec AC9 after the mechanical update).

- [x] **Step 5: Verify the gate** (AC10 — no markers added)

Run: `cargo xtask check` Expected: green — clippy clean (no unwrap/expect in
prod), coverage passes with **no new `cov:ignore`** (every read arm is a real
test).

- [x] **Step 6: Commit**

```bash
git add server/src/runtime_file.rs server/src/commands.rs
git commit -m "feat(serve): refuse start-up when a live writer holds runtime.json (#141)"
```

---

### Task 3: Docs — ADR-0035 lifecycle note

**Files:**

- Modify: `docs/adr/0035-elisp-live-integration-harness.md` (contents +
  follow-on list).

**Interfaces:** none.

- [x] **Step 1: Update ADR-0035**

Update the runtime.json **contents** to `{ip, port, pid, start_time}` and the
**lifecycle** note to describe the start-up mutex + stale-detection (pid +
start-time identity; `NotFound` = dead → stale; unusable `/proc` = hard fail),
and mark the start-up-mutex deferred follow-on **done (#141)**. Keep the
canonical `# ADR-0035:` heading and `- Status: accepted` line **unchanged**. No
new ADR (spec decision 7). (The `runtime_file.rs` module-doc update already
landed in Task 1.)

- [x] **Step 2: Verify the ADR + prose gates**

Run `prettier -w docs/adr/0035-elisp-live-integration-harness.md`, then: Run:
`cargo xtask check --no-test` Expected: green — `adr-format`,
`adr-readme-parity`, `prettier` pass.

- [x] **Step 3: Commit**

```bash
git add docs/adr/0035-elisp-live-integration-harness.md
git commit -m "docs(adr): ADR-0035 runtime.json gains pid+start_time start-up mutex (#141)"
```

---

## Final verification (after all tasks)

- [x] `cargo xtask validate --no-e2e` — static + clippy + coverage over the
      whole change (spec AC10). #141 has no e2e/web surface, so `--no-e2e` is
      the local gate; the ship step runs full `validate`.
