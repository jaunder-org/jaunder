# devtool run — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `devtool run -- <argv>` subcommand that runs exactly one program with no shell, parking stdout/stderr to files and returning a structured JSON result, exiting with the child's exit code.

**Architecture:** A new `tools/devtool/src/run.rs` module with a clap `Run` leaf command wired into `main.rs`. A pure `validate_argv` guard, an output store under `.xtask/run/`, an `exec_capture` that redirects the child's streams straight to files (so raw bytes never transit the runner's stdout), and a serde `RunResult` printed as JSON. The runner propagates the child's exit code via `std::process::exit`, mirroring the existing `pg::run_command`.

**Tech Stack:** Rust 2021, clap 4 (derive), serde / serde_json, std only (no new crates). Tests are `#[cfg(test)]` unit/integration tests in `run.rs` driving real coreutils (`true`, `false`, `printf`, `ls`).

## Global Constraints

- **No new dependencies.** `tools/devtool/Cargo.toml` already has clap, serde, serde_json, anyhow, tempfile, signal-hook. Use std for everything else.
- **No shell.** The child is always `Command::new(argv[0]).args(&argv[1..])` — never `sh -c`.
- **`tools/` is its own cargo workspace** (`tools/Cargo.toml`, members `coverage`, `devtool`). Build/test/lint it with `--manifest-path tools/Cargo.toml -p devtool`.
- **No dead code.** `tools/` builds under `-D warnings`; a new item unused by `main` or a test fails clippy. Each task must leave every new item consumed (this is why features are added vertically, not as unused scaffolding).
- **Commit style:** Conventional Commits, scope `devtool`, reference `(#158)`. **No `Co-Authored-By` trailers.**
- **`.xtask/` is gitignored** (`/.xtask/` in `.gitignore`); output files are never committed.
- **`CLAUDE.md` is intentionally untracked** in this repo (committing it busts the coverage cache). Durable guidance goes in the tracked **`CONTRIBUTING.md`**, not `CLAUDE.md`.

---

### Task 1: Minimal `devtool run` (happy path)

Run one program, capture to `.xtask/run/<id>.{out,err}`, print the JSON result, exit with the child's code. No validation refusal, no `--timeout`, no pruning yet.

**Files:**
- Create: `tools/devtool/src/run.rs`
- Modify: `tools/devtool/src/main.rs` (add `mod run`, `Run` variant, dispatch)

**Interfaces:**
- Produces:
  - `pub fn execute(argv: &[String], cwd: &Path, ) -> Result<(RunResult, i32), RunError>` — runs the command, returns the result plus the process exit code; no printing, no `exit` (so it's testable).
  - `pub fn run(argv: &[String], cwd: Option<PathBuf>) -> !` — calls `execute`, prints JSON, `exit`s.
  - `pub struct RunResult { command, exit_code: Option<i32>, ok: bool, signal: Option<String>, duration_ms: u128, stdout: Stream, stderr: Stream }` (all `pub`, `#[derive(Serialize)]`).
  - `pub struct Stream { path: String, bytes: u64, lines: u64 }`.
  - `pub struct RunError { message: String, kind: &'static str }`.

- [ ] **Step 1: Write the failing tests**

Create `tools/devtool/src/run.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        tempfile::Builder::new()
            .prefix("devtool-run-test.")
            .tempdir()
            .unwrap()
            .keep()
    }

    #[test]
    fn true_succeeds_with_empty_streams() {
        let cwd = tmp();
        let (r, code) = execute(&["true".into()], &cwd).unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert!(r.ok);
        assert_eq!(code, 0);
        assert_eq!(r.stdout.bytes, 0);
        assert_eq!(r.stderr.bytes, 0);
        assert!(std::path::Path::new(&r.stdout.path).exists());
    }

    #[test]
    fn false_fails_with_exit_one() {
        let cwd = tmp();
        let (r, code) = execute(&["false".into()], &cwd).unwrap();
        assert_eq!(r.exit_code, Some(1));
        assert!(!r.ok);
        assert_eq!(code, 1);
    }

    #[test]
    fn stdout_bytes_and_lines_counted() {
        let cwd = tmp();
        let (r, _) = execute(&["printf".into(), "a\nb\n".into()], &cwd).unwrap();
        assert_eq!(r.stdout.bytes, 4);
        assert_eq!(r.stdout.lines, 2);
        assert_eq!(r.stderr.bytes, 0);
    }

    #[test]
    fn stderr_captured_separately() {
        let cwd = tmp();
        let (r, _) = execute(&["ls".into(), "/no/such/path/xyzzy".into()], &cwd).unwrap();
        assert!(!r.ok);
        assert!(r.stderr.bytes > 0);
        assert_eq!(r.stdout.bytes, 0);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (no `execute` yet)**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::`
Expected: FAIL — `cannot find function `execute` in this scope`.

- [ ] **Step 3: Implement the module above the test block**

Prepend to `tools/devtool/src/run.rs`:

```rust
//! `devtool run` — run exactly one program with no shell, capturing stdout and
//! stderr to files under `.xtask/run/` and returning a structured JSON result.
//! The runner exits with the child's exit code, so callers get an honest
//! pass/fail signal without shell scaffolding (`; echo $?`, `2>&1 | tail`).

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Serialize;

const RUN_DIR: &str = ".xtask/run";

#[derive(Serialize)]
pub struct Stream {
    pub path: String,
    pub bytes: u64,
    pub lines: u64,
}

#[derive(Serialize)]
pub struct RunResult {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub ok: bool,
    pub signal: Option<String>,
    pub duration_ms: u128,
    pub stdout: Stream,
    pub stderr: Stream,
}

/// A runner-level failure (not a child failure). Serialized as `{error, kind}`.
pub struct RunError {
    pub message: String,
    pub kind: &'static str, // "usage" | "shell_refused" | "spawn"
}

fn run_dir(cwd: &Path) -> PathBuf {
    cwd.join(RUN_DIR)
}

fn alloc_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{millis}-{}", std::process::id())
}

/// Count `\n` bytes (wc -l semantics) by streaming, so a huge file never lands in
/// memory all at once.
fn count_lines(path: &Path) -> u64 {
    let Ok(f) = File::open(path) else {
        return 0;
    };
    let mut reader = BufReader::new(f);
    let mut buf = [0u8; 64 * 1024];
    let mut count = 0u64;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => count += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64,
            Err(_) => break,
        }
    }
    count
}

fn stream_of(path: &Path) -> Stream {
    let bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Stream {
        path: path.display().to_string(),
        bytes,
        lines: count_lines(path),
    }
}

struct Capture {
    exit_code: Option<i32>,
    signal: Option<i32>,
    duration_ms: u128,
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        2 => "SIGINT".into(),
        6 => "SIGABRT".into(),
        9 => "SIGKILL".into(),
        15 => "SIGTERM".into(),
        n => format!("SIG{n}"),
    }
}

#[cfg(not(unix))]
fn signal_name(sig: i32) -> String {
    format!("SIG{sig}")
}

fn interpret_status(status: &std::process::ExitStatus) -> (Option<i32>, Option<i32>) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return (None, Some(sig));
        }
    }
    (status.code(), None)
}

fn exec_capture(
    argv: &[String],
    cwd: &Path,
    out_path: &Path,
    err_path: &Path,
) -> Result<Capture, RunError> {
    let mk = |p: &Path| -> Result<File, RunError> {
        File::create(p).map_err(|e| RunError {
            message: format!("creating {}: {e}", p.display()),
            kind: "spawn",
        })
    };
    let out_file = mk(out_path)?;
    let err_file = mk(err_path)?;

    let start = Instant::now();
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|e| RunError {
            message: format!("spawning {argv:?}: {e}"),
            kind: "spawn",
        })?
        .wait()
        .map_err(|e| RunError {
            message: format!("waiting on {argv:?}: {e}"),
            kind: "spawn",
        })?;
    let duration_ms = start.elapsed().as_millis();

    let (exit_code, signal) = interpret_status(&status);
    Ok(Capture {
        exit_code,
        signal,
        duration_ms,
    })
}

pub fn execute(argv: &[String], cwd: &Path) -> Result<(RunResult, i32), RunError> {
    let dir = run_dir(cwd);
    fs::create_dir_all(&dir).map_err(|e| RunError {
        message: format!("creating {}: {e}", dir.display()),
        kind: "spawn",
    })?;
    let id = alloc_id();
    let out_path = dir.join(format!("{id}.out"));
    let err_path = dir.join(format!("{id}.err"));

    let cap = exec_capture(argv, cwd, &out_path, &err_path)?;

    let process_code = match (cap.exit_code, cap.signal) {
        (Some(code), _) => code,
        (None, Some(sig)) => 128 + sig,
        (None, None) => 1,
    };
    let result = RunResult {
        command: argv.to_vec(),
        exit_code: cap.exit_code,
        ok: cap.exit_code == Some(0),
        signal: cap.signal.map(signal_name),
        duration_ms: cap.duration_ms,
        stdout: stream_of(&out_path),
        stderr: stream_of(&err_path),
    };
    Ok((result, process_code))
}

pub fn run(argv: &[String], cwd: Option<PathBuf>) -> ! {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match execute(argv, &cwd) {
        Ok((result, code)) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(code);
        }
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({ "error": e.message, "kind": e.kind })
            );
            std::process::exit(64);
        }
    }
}
```

Then wire `main.rs` — add `mod run;`, a `Run` variant, and dispatch:

```rust
// near the other `mod` lines
mod run;

// add to `enum Command`
    /// Run one program (no shell), capturing output to .xtask/run/ and
    /// returning a structured JSON result; exits with the child's exit code.
    Run(RunArgs),

// new args struct, beside the other derives
#[derive(clap::Args)]
struct RunArgs {
    /// Working directory for the command (defaults to the current directory).
    #[arg(long)]
    cwd: Option<std::path::PathBuf>,
    /// The program and its arguments, after `--`.
    #[arg(trailing_var_arg = true, required = true)]
    cmd: Vec<String>,
}

// add to the `match cli.command` in `main`
        Command::Run(args) => run::run(&args.cmd, args.cwd),
```

(The `Run` arm calls a `-> !` function, so it type-checks inside `fn main() -> anyhow::Result<()>` without an `Ok(())`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::`
Expected: PASS (4 tests).

- [ ] **Step 5: Lint + format, then smoke-test the binary**

Run: `cargo clippy --manifest-path tools/Cargo.toml -p devtool --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt --manifest-path tools/Cargo.toml -p devtool`
Run: `cargo run --manifest-path tools/Cargo.toml -p devtool -- run -- printf 'hi\n'`
Expected: JSON with `"exit_code": 0, "ok": true`, `stdout.bytes: 3, stdout.lines: 1`, and the process exit code 0.

- [ ] **Step 6: Commit**

```bash
git add tools/devtool/src/run.rs tools/devtool/src/main.rs
git commit -m "feat(devtool): add run subcommand — single-command runner (#158)"
```

---

### Task 2: Refuse shell re-entry

Add `validate_argv`, called first in `execute`, so the runner cannot be used to re-open a shell. This is what makes `Bash(devtool run *)` narrower than `bash *`.

**Files:**
- Modify: `tools/devtool/src/run.rs`

**Interfaces:**
- Produces: `pub fn validate_argv(argv: &[String]) -> Result<(), RunError>`.
- Consumes: `RunError` (Task 1).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `run.rs`:

```rust
    #[test]
    fn accepts_normal_programs() {
        for ok in [
            vec!["cargo".to_string(), "build".into()],
            vec!["git".into(), "status".into()],
            vec!["gh".into(), "pr".into(), "view".into()],
            vec!["rg".into(), "foo".into()],
            vec!["nix".into(), "build".into()],
            vec!["emacs".into(), "--batch".into()],
            vec!["prettier".into(), "--check".into()],
            vec!["/usr/bin/printf".into(), "x".into()],
        ] {
            assert!(validate_argv(&ok).is_ok(), "{ok:?} should be accepted");
        }
    }

    #[test]
    fn refuses_shells_and_nix_develop() {
        for bad in [
            vec!["bash".to_string(), "-c".into(), "echo hi".into()],
            vec!["sh".into()],
            vec!["zsh".into()],
            vec!["fish".into()],
            vec!["dash".into()],
            vec!["ash".into()],
            vec!["eval".into()],
            vec!["/bin/bash".into(), "-c".into(), "x".into()],
            vec!["nix".into(), "develop".into(), "-c".into(), "cargo".into()],
        ] {
            let err = validate_argv(&bad).unwrap_err();
            assert_eq!(err.kind, "shell_refused", "{bad:?}");
        }
    }

    #[test]
    fn refuses_empty_argv() {
        let err = validate_argv(&[]).unwrap_err();
        assert_eq!(err.kind, "usage");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::tests::refuses`
Expected: FAIL — `cannot find function `validate_argv``.

- [ ] **Step 3: Implement `validate_argv` and call it**

Add to `run.rs` (above `execute`):

```rust
const REFUSED_SHELLS: &[&str] = &["bash", "sh", "zsh", "fish", "dash", "ash", "eval"];

/// Refuse argv that re-opens a shell or the `nix develop` wrapper. Everything
/// else runs. `env VAR=x cmd` is *not* refused (a legitimate per-command env
/// idiom); `xargs` is not refused (neutered here by the `/dev/null` stdin).
pub fn validate_argv(argv: &[String]) -> Result<(), RunError> {
    let first = argv.first().ok_or_else(|| RunError {
        message: "no command given after `--`".into(),
        kind: "usage",
    })?;
    let base = Path::new(first)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(first);
    if REFUSED_SHELLS.contains(&base) {
        return Err(RunError {
            message: format!("refusing to run a shell (`{base}`); invoke the program directly"),
            kind: "shell_refused",
        });
    }
    if base == "nix" && argv.get(1).map(String::as_str) == Some("develop") {
        return Err(RunError {
            message: "refusing `nix develop`; the toolchain is already on PATH via direnv".into(),
            kind: "shell_refused",
        });
    }
    Ok(())
}
```

In `execute`, make the first line:

```rust
pub fn execute(argv: &[String], cwd: &Path) -> Result<(RunResult, i32), RunError> {
    validate_argv(argv)?;
    let dir = run_dir(cwd);
    // ... unchanged ...
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::`
Expected: PASS (all tests).

- [ ] **Step 5: Lint + format**

Run: `cargo clippy --manifest-path tools/Cargo.toml -p devtool --all-targets -- -D warnings`
Run: `cargo fmt --manifest-path tools/Cargo.toml -p devtool`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add tools/devtool/src/run.rs
git commit -m "feat(devtool): refuse shell re-entry in run (#158)"
```

---

### Task 3: Prune output history to the newest 50

Stop `.xtask/run/` from growing unboundedly.

**Files:**
- Modify: `tools/devtool/src/run.rs`

**Interfaces:**
- Produces: `fn prune(dir: &Path)` (private) + `const RUN_HISTORY_LIMIT: usize = 50`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn prune_keeps_newest_limit() {
        let dir = tmp();
        // create RUN_HISTORY_LIMIT + 10 files with increasing mtimes
        for i in 0..(RUN_HISTORY_LIMIT + 10) {
            let p = dir.join(format!("{i:04}.out"));
            std::fs::write(&p, b"x").unwrap();
            // bump mtime monotonically so ordering is deterministic
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(i as u64);
            filetime_set(&p, t);
        }
        prune(&dir);
        let remaining = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(remaining, RUN_HISTORY_LIMIT);
        // the oldest (0000) must be gone, the newest must remain
        assert!(!dir.join("0000.out").exists());
        assert!(dir.join(format!("{:04}.out", RUN_HISTORY_LIMIT + 9)).exists());
    }

    // Set mtime without adding a dependency: use std on unix via utimensat through
    // File's set_modified (stable since Rust 1.75).
    fn filetime_set(p: &std::path::Path, t: std::time::SystemTime) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(t).unwrap();
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::tests::prune`
Expected: FAIL — `cannot find function `prune`` / `RUN_HISTORY_LIMIT`.

- [ ] **Step 3: Implement `prune` and call it after capture**

Add the constant next to `RUN_DIR`:

```rust
const RUN_HISTORY_LIMIT: usize = 50;
```

Add the function (below `alloc_id`):

```rust
/// Keep only the newest `RUN_HISTORY_LIMIT` files in `dir`; best-effort, so a
/// race or a stat error never fails an otherwise-successful run.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if files.len() <= RUN_HISTORY_LIMIT {
        return;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    for (_, path) in files.into_iter().skip(RUN_HISTORY_LIMIT) {
        let _ = fs::remove_file(path);
    }
}
```

In `execute`, call `prune` after `exec_capture` (the just-written files are newest, so they are never pruned):

```rust
    let cap = exec_capture(argv, cwd, &out_path, &err_path)?;
    prune(&dir);
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::`
Expected: PASS.

- [ ] **Step 5: Lint + format**

Run: `cargo clippy --manifest-path tools/Cargo.toml -p devtool --all-targets -- -D warnings`
Run: `cargo fmt --manifest-path tools/Cargo.toml -p devtool`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add tools/devtool/src/run.rs
git commit -m "feat(devtool): prune run output history to newest 50 (#158)"
```

---

### Task 4: `--timeout` (kill runaway commands)

Add an optional timeout that kills the child and reports `timed_out` + exit 124.

**Files:**
- Modify: `tools/devtool/src/run.rs`, `tools/devtool/src/main.rs`

**Interfaces:**
- Changes: `execute(argv, cwd, timeout: Option<u64>)`, `run(argv, cwd, timeout)`, `exec_capture(..., timeout)`. `RunResult` gains `timed_out: Option<bool>` (`#[serde(skip_serializing_if = "Option::is_none")]`). `Capture` gains `timed_out: bool`.
- Consumes: everything from Tasks 1–3. **Note:** existing call sites (`execute(&["true".into()], &cwd)`) must gain a `None` timeout argument — update the Task 1/2/3 tests accordingly in Step 3.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn timeout_kills_and_reports() {
        let cwd = tmp();
        let (r, code) = execute(&["sleep".into(), "10".into()], &cwd, Some(1)).unwrap();
        assert_eq!(r.timed_out, Some(true));
        assert!(!r.ok);
        assert_eq!(code, 124);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::tests::timeout`
Expected: FAIL — `execute` takes 2 args, not 3 (compile error).

- [ ] **Step 3: Thread the timeout through**

In `run.rs`:

Add `use std::time::Duration;` to the time import line:
```rust
use std::time::{Duration, Instant};
```

Add `timed_out` to `RunResult` (after `duration_ms`):
```rust
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
```

Add `timed_out` to `Capture`:
```rust
struct Capture {
    exit_code: Option<i32>,
    signal: Option<i32>,
    timed_out: bool,
    duration_ms: u128,
}
```

Add the poll-based waiter:
```rust
/// Wait for the child, killing it if `secs` elapses. Returns (status, timed_out).
fn wait_with_timeout(
    child: &mut std::process::Child,
    secs: u64,
) -> Result<(std::process::ExitStatus, bool), RunError> {
    let wait_err = |e: std::io::Error| RunError {
        message: format!("waiting on child: {e}"),
        kind: "spawn",
    };
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(status) = child.try_wait().map_err(wait_err)? {
            return Ok((status, false));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait().map_err(wait_err)?;
            return Ok((status, true));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
```

Rewrite `exec_capture` to take `timeout` and branch:
```rust
fn exec_capture(
    argv: &[String],
    cwd: &Path,
    timeout: Option<u64>,
    out_path: &Path,
    err_path: &Path,
) -> Result<Capture, RunError> {
    let mk = |p: &Path| -> Result<File, RunError> {
        File::create(p).map_err(|e| RunError {
            message: format!("creating {}: {e}", p.display()),
            kind: "spawn",
        })
    };
    let out_file = mk(out_path)?;
    let err_file = mk(err_path)?;

    let start = Instant::now();
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|e| RunError {
            message: format!("spawning {argv:?}: {e}"),
            kind: "spawn",
        })?;

    let (status, timed_out) = match timeout {
        None => (
            child.wait().map_err(|e| RunError {
                message: format!("waiting on {argv:?}: {e}"),
                kind: "spawn",
            })?,
            false,
        ),
        Some(secs) => wait_with_timeout(&mut child, secs)?,
    };
    let duration_ms = start.elapsed().as_millis();

    let (exit_code, signal) = interpret_status(&status);
    Ok(Capture {
        exit_code,
        signal,
        timed_out,
        duration_ms,
    })
}
```

Update `execute` signature, the `exec_capture` call, the `process_code` (124 on timeout), and `RunResult` construction:
```rust
pub fn execute(argv: &[String], cwd: &Path, timeout: Option<u64>) -> Result<(RunResult, i32), RunError> {
    validate_argv(argv)?;
    let dir = run_dir(cwd);
    fs::create_dir_all(&dir).map_err(|e| RunError {
        message: format!("creating {}: {e}", dir.display()),
        kind: "spawn",
    })?;
    let id = alloc_id();
    let out_path = dir.join(format!("{id}.out"));
    let err_path = dir.join(format!("{id}.err"));

    let cap = exec_capture(argv, cwd, timeout, &out_path, &err_path)?;
    prune(&dir);

    let process_code = if cap.timed_out {
        124
    } else {
        match (cap.exit_code, cap.signal) {
            (Some(code), _) => code,
            (None, Some(sig)) => 128 + sig,
            (None, None) => 1,
        }
    };
    let result = RunResult {
        command: argv.to_vec(),
        exit_code: cap.exit_code,
        ok: cap.exit_code == Some(0),
        signal: cap.signal.map(signal_name),
        duration_ms: cap.duration_ms,
        timed_out: if cap.timed_out { Some(true) } else { None },
        stdout: stream_of(&out_path),
        stderr: stream_of(&err_path),
    };
    Ok((result, process_code))
}
```

Update `run`:
```rust
pub fn run(argv: &[String], cwd: Option<PathBuf>, timeout: Option<u64>) -> ! {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match execute(argv, &cwd, timeout) {
        Ok((result, code)) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(code);
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "error": e.message, "kind": e.kind }));
            std::process::exit(64);
        }
    }
}
```

Update the existing tests from Tasks 1–3 to pass `None` (e.g. `execute(&["true".into()], &cwd, None)`).

In `main.rs`, add the flag and pass it through:
```rust
#[derive(clap::Args)]
struct RunArgs {
    /// Working directory for the command (defaults to the current directory).
    #[arg(long)]
    cwd: Option<std::path::PathBuf>,
    /// Kill the command after this many seconds (default: no limit).
    #[arg(long)]
    timeout: Option<u64>,
    /// The program and its arguments, after `--`.
    #[arg(trailing_var_arg = true, required = true)]
    cmd: Vec<String>,
}
```
```rust
        Command::Run(args) => run::run(&args.cmd, args.cwd, args.timeout),
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool run::`
Expected: PASS (all, including `timeout_kills_and_reports`).

- [ ] **Step 5: Lint + format**

Run: `cargo clippy --manifest-path tools/Cargo.toml -p devtool --all-targets -- -D warnings`
Run: `cargo fmt --manifest-path tools/Cargo.toml -p devtool`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add tools/devtool/src/run.rs tools/devtool/src/main.rs
git commit -m "feat(devtool): add --timeout to run (#158)"
```

---

### Task 5: Expose `devtool` in the default devShell

So `devtool run -- …` is on PATH interactively (direnv loads the default devShell). `devtoolBin` already builds (flake.nix:336) and is already on PATH inside the coverage sandbox; this just adds it to the interactive shell.

**Files:**
- Modify: `flake.nix`

**Interfaces:**
- Consumes: the `Run` command (Tasks 1–4).

**Why no git-sha build stamp (deliberate change from the spec):** stamping
`devtoolBin` with `self.shortRev` would couple the whole-repo revision into the
`devtool` derivation. The coverage check depends on `devtoolBin`
(`flake.nix:950`), so its inputs — and therefore the cachix-warmed coverage
cache — would bust on *every commit*. That regresses the warm cache the entire
gate relies on, which is not worth a staleness nicety. `devtool --version` shows
the crate version (clap default); staleness while developing the runner is
handled operationally (use `cargo run -p devtool -- run -- …` for live edits, and
`direnv reload` to refresh the devShell copy). If per-content staleness detection
is later wanted, derive the id from `devtoolSrc`'s store-path hash (which changes
only when `tools/` changes — i.e. exactly when `devtoolBin` already rebuilds), not
from the repo revision.

- [ ] **Step 1: Confirm clap reports a version**

clap's derive already wires `--version` from `CARGO_PKG_VERSION` once a `version`
is set. Add it to the existing `#[command(...)]` in `main.rs`:
```rust
#[derive(Parser)]
#[command(name = "devtool", about = "Jaunder in-sandbox dev tooling", version)]
struct Cli {
```
Run: `cargo run --manifest-path tools/Cargo.toml -p devtool -- --version`
Expected: prints `devtool 0.1.0`.

- [ ] **Step 2: Add `devtoolBin` to the default devShell**

In `flake.nix`, the default shell is `default = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ devOnly; });` (~line 1087), where `devOnly` (~line 1059) holds interactive-only tools `cargo xtask validate` never needs. Add `devtoolBin` to `devOnly` so the lean CI `ci` shell is unaffected:
```nix
            devOnly = [
              # ... existing interactive-only tools ...
              devtoolBin
            ];
```

- [ ] **Step 3: Verify the default devShell exposes it**

Run: `nix develop --command devtool --version`
Expected: prints `devtool 0.1.0` (proves it resolved from the devShell PATH, not a stray binary).

- [ ] **Step 4: Commit**

```bash
git add flake.nix tools/devtool/src/main.rs
git commit -m "feat(devtool): expose devtool run in the default devShell (#158)"
```

---

### Task 6: ADR-0028 supplement

Record the architectural decision with the code. **Usage guidance and the
allowlist entry are deferred to post-merge** (see Follow-ups): documenting "use
`devtool run`" before it exists on `main` would be wrong, and the CLAUDE.md-vs-
CONTRIBUTING.md placement is an open decision for the maintainer to make once the
tool has landed.

**Files:**
- Modify: `docs/adr/0028-*.md` (find the exact filename)
- Modify: `docs/README.md` only if its ADR table tracks a per-row supplement note

- [ ] **Step 1: Find ADR-0028**

Run: `ls docs/adr/ | rg '^0028'`
Note the exact filename for the next step.

- [ ] **Step 2: Append the supplement**

Append to that file:

```markdown
## Supplement (#158): `devtool run`

`devtool` gains a `run` subcommand: a no-shell single-command runner that exits
with the child's code and parks output under `.xtask/run/`, returning a JSON
result. It is the gate-execution surface for agents and humans — it makes
pass/fail honest and keeps raw output out of the conversation. It refuses shell
re-entry, so `Bash(devtool run *)` is a narrower grant than `bash *`. devtool is
exposed in the default devShell (direnv) and remains the in-sandbox tool of
record per this ADR's boundary.
```

If `docs/README.md`'s ADR table carries per-row supplement notes, add one for
ADR-0028; otherwise leave the table unchanged (the supplement lives in the ADR
file).

- [ ] **Step 3: Verify the gate passes via the new tool (live dogfood)**

Run: `cargo run --manifest-path tools/Cargo.toml -p devtool -- run -- cargo xtask check --no-test`
Expected: JSON `"ok": true` and the process exits 0 — a live end-to-end proof of
the tool gating a real command. (Use `cargo run` here rather than a bare
`devtool` since the devShell copy isn't rebuilt until `direnv reload`.)

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0028-*.md docs/README.md
git commit -m "docs(devtool): ADR-0028 supplement for run subcommand (#158)"
```

---

## Final gate

- [ ] Run the full local gate from the worktree: `cargo xtask validate --no-e2e`
  (the runner change is host-only tooling; e2e is unaffected, but run the
  `--no-e2e` gate to confirm the workspace builds, lints, and the coverage gate
  is green). Expected: exit 0.
- [ ] Confirm `devtool run` tests executed: `cargo test --manifest-path tools/Cargo.toml -p devtool run::` → all pass.

## Post-merge follow-ups (do once #158 is on `main`)

These deliberately wait until the tool exists on `main`:

- **Usage guidance + routing guardrail.** Write the "gate commands via
  `devtool run`; `ctx_execute` for data; raw Bash for atomic mutations" note.
  **Open decision (maintainer's call):** `CLAUDE.md` vs `CONTRIBUTING.md`. The
  maintainer leans `CLAUDE.md` — this is agent-local-context hygiene, and
  `CLAUDE.md` is the agent-local channel — even though `CONTRIBUTING.md` is the
  shared canonical guide. `CLAUDE.md` is untracked here, so this lands as a local
  edit regardless.
- **Local allowlist entry.** Add `Bash(devtool run *)` to
  `.claude/settings.local.json` once the binary is on PATH (after the devShell
  rebuild / `direnv reload`).

## Possible follow-ups (file as issues if confirmed)

- **CI wiring for devtool tests.** Confirm whether `cargo xtask check`/`validate`
  exercises `tools/` tests. If not, the `run` tests run only locally — file a
  Task to add `tools/` to the gate.
- **Adopt `devtool run` in the jaunder skills** (local, untracked): point the
  dispatch/gate skills at `devtool run` so subagents use it by default.
