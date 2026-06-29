# Elisp Live Integration Harness (#137) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A self-booting ERT harness that runs the Emacs client against a real Jaunder server, enabled by two small server affordances, run hermetically as a `nixosTest` check.

**Architecture:** The server gains a `runtime.json` writer (so a `--bind :0` subprocess is discoverable) and a `jaunder app-password-create` CLI (so credentials can be minted out of process). An elisp macro provisions a DB + user + app-password, boots `jaunder serve` as a subprocess, waits on `runtime.json` then `GET /`, runs the test body, and tears everything down. A committed smoke test exercises the unauthenticated service document and an authenticated collection GET. A `nixosTest` VM check (Emacs + the binary, no systemd) runs the suite in `cargo xtask validate`; the same suite runs host-side for dev iteration.

**Tech Stack:** Rust (clap, axum, sqlx, serde_json, tempfile), Emacs Lisp (ERT, url.el, auth-source, json), Nix (`pkgs.testers.nixosTest`), xtask.

## Global Constraints

- **No `Co-Authored-By` trailers** in any commit (overrides global default).
- **Verify gate is git-enforced**: the pre-commit hook runs `cargo xtask check --no-test` + `validate --no-e2e --allow-dirty`; `validate` refuses a dirty tree. Per-task Rust gate while iterating: `cargo xtask check --no-test` (clippy + fmt) plus the task's own `cargo nextest run` selection. Final gate: full `cargo xtask validate` (includes the new `nixosTest`).
- **Elisp**: Emacs 27.1+, `lexical-binding: t`, built-in libraries only (`url`, `auth-source`, `json`); formatting enforced by `jaunder-fmt-check` (run `-f jaunder-fmt-fix` before committing).
- **Coverage gate** is line-based; every new Rust line needs a covering test. Page-like/serve-loop regions are the exception (already baselined).
- **Issue hygiene** (`jaunder-issues`): every issue gets `--type`, a topic label, milestone, native deps; add to Project #1.
- All paths are relative to the worktree root `/home/mdorman/src/jaunder/.claude/worktrees/issue-137-elisp-test-harness`.

---

### Task 1: File the three follow-on issues

The spec defers three separable concerns built *on* `runtime.json`. File them first so they can be scheduled independently; each is `blocked_by #137` (needs the file to exist), milestone 4, type Task, label `test-infra` for the first two and `tooling` for the admin one.

**Files:** none (GitHub only).

- [x] **Step 1: Create the signal-robust-removal issue**

```bash
gh issue create --repo jaunder-org/jaunder --type Task --label tooling --milestone "Emacs blogging front-end" \
  --title "serve: signal-robust removal of runtime.json (graceful shutdown hook)" \
  --body $'Give `jaunder serve` a graceful-shutdown hook (`axum::serve(...).with_graceful_shutdown` on SIGTERM/SIGINT) so the `runtime.json` written in #137 is reliably removed on a normal stop, not only when the serve loop returns on its own (today a signal kills the process without unwinding, so the `RuntimeFileGuard::drop` never runs and the file is left behind).\n\nFoundation: #137 (ADR-0035). Pairs with the start-up mutex follow-on.'
```

- [x] **Step 2: Create the start-up mutex issue**

```bash
gh issue create --repo jaunder-org/jaunder --type Task --label tooling --milestone "Emacs blogging front-end" \
  --title "serve: refuse to start on a live runtime.json (start-up mutex + stale detection)" \
  --body $'On `serve` startup, if `runtime.json` exists and records a *live* `pid`, refuse to start (another instance is running on this data dir); if the pid is dead, treat the file as stale, warn, and overwrite. Requires adding a `pid` field to `runtime.json` (#137 writes only `{ip,port}`). Pairs with signal-robust removal: graceful-remove (write side) + stale detection (read side) make the file a reliable "is an instance running" signal.\n\nFoundation: #137 (ADR-0035).'
```

- [x] **Step 3: Create the admin-control-channel issue**

```bash
gh issue create --repo jaunder-org/jaunder --type Task --label tooling --milestone "Emacs blogging front-end" \
  --title "serve: local admin control channel (admin_token in runtime.json + jaunder shut-down)" \
  --body $'Add a random `admin_token` field to `runtime.json` (0600) and a `jaunder shut-down` command (and any further local admin operations) authenticated by it, so e.g. systemd can stop the server cleanly. Security: the token grants local privileged control; it is gated by data-dir file permissions, the same trust boundary as the DB file.\n\nFoundation: #137 (ADR-0035).'
```

- [x] **Step 4: Record the issue numbers, set `blocked_by #137`, add to Project #1**

For each new issue number `N` (replace `137_ID` with #137\'s REST id from `gh api repos/jaunder-org/jaunder/issues/137 --jq .id`):

```bash
gh api --method POST repos/jaunder-org/jaunder/issues/N/dependencies/blocked_by -F issue_id=137_ID
gh project item-add 1 --owner jaunder-org --url https://github.com/jaunder-org/jaunder/issues/N
```

No commit (no repo changes).

---

### Task 2: Server — `runtime.json` writer wired into `serve`

**Files:**
- Create: `server/src/runtime_file.rs`
- Modify: `server/src/lib.rs` (add `pub mod runtime_file;`)
- Modify: `server/src/cli.rs` (the `Serve` variant — add `runtime_file`)
- Modify: `server/src/commands.rs` (`PreparedServer`, `prepare_server`, `cmd_serve`)
- Modify: `server/src/main.rs` (the `Commands::Serve` arm)
- Test: `server/src/runtime_file.rs` (unit tests), `server/tests/misc/commands.rs` (the existing `prepare_server` test)

**Interfaces:**
- Produces: `jaunder::runtime_file::RuntimeFileGuard` with `pub fn write(path: PathBuf, addr: std::net::SocketAddr) -> RuntimeFileGuard` (best-effort; removes the file on `Drop`). `PreparedServer` gains a `runtime_guard: RuntimeFileGuard` field. `prepare_server` and `cmd_serve` gain a trailing `runtime_file: Option<PathBuf>` parameter.

- [x] **Step 1: Write the failing unit tests for the writer/guard**

Create `server/src/runtime_file.rs` with only the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use tempfile::TempDir;

    fn addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 34567)
    }

    #[test]
    fn writes_ip_and_port_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let _guard = RuntimeFileGuard::write(path.clone(), addr());
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["ip"], "127.0.0.1");
        assert_eq!(v["port"], 34567);
    }

    #[test]
    fn removes_file_on_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let guard = RuntimeFileGuard::write(path.clone(), addr());
        assert!(path.exists());
        drop(guard);
        assert!(!path.exists());
    }
}
```

- [x] **Step 2: Run the tests; expect a compile failure** — `cargo nextest run -p jaunder runtime_file` → fails (`RuntimeFileGuard` undefined). First add `pub mod runtime_file;` to `server/src/lib.rs` next to `pub mod commands;`.

- [x] **Step 3: Implement the writer/guard** (prepend above the test module in `server/src/runtime_file.rs`):

```rust
//! The `serve` runtime-info file — a small JSON file recording the bound address
//! so an ephemeral (`--bind …:0`) server is discoverable by an out-of-process
//! caller (the elisp test harness). See ADR-0035.
//!
//! Contents are intentionally minimal for now: `{ "ip": <ip>, "port": <port> }`.
//! Follow-ons add a `pid` (start-up mutex) and an `admin_token` (admin channel).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Serializes `{ "ip", "port" }` and writes it to `path` atomically
/// (write a sibling `.tmp`, then rename — atomic on the same filesystem).
fn write_atomic(path: &Path, addr: SocketAddr) -> std::io::Result<()> {
    let body = serde_json::json!({
        "ip": addr.ip().to_string(),
        "port": addr.port(),
    })
    .to_string();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// RAII guard: writes the runtime file on construction, removes it on `Drop`.
///
/// Removal is best-effort and only runs on a normal unwind (a `SIGKILL` skips
/// `Drop`); making removal signal-robust is a deferred follow-on.
pub struct RuntimeFileGuard {
    path: Option<PathBuf>,
}

impl RuntimeFileGuard {
    /// Best-effort: on write failure, logs and returns an inert guard so a
    /// runtime-file problem never stops the server from serving.
    #[must_use]
    pub fn write(path: PathBuf, addr: SocketAddr) -> Self {
        match write_atomic(&path, addr) {
            Ok(()) => Self { path: Some(path) },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to write runtime file");
                Self { path: None }
            }
        }
    }
}

impl Drop for RuntimeFileGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}
```

- [x] **Step 4: Run the tests; expect PASS** — `cargo nextest run -p jaunder runtime_file`.

- [x] **Step 5: Add the `--runtime-file` flag to the `Serve` variant** in `server/src/cli.rs` (inside the `Serve { … }` variant, after `environment`):

```rust
        /// Path to write the runtime-info JSON file (default
        /// `<storage-path>/runtime.json`). Records the bound `ip`/`port`.
        #[arg(long, env = "JAUNDER_RUNTIME_FILE")]
        runtime_file: Option<std::path::PathBuf>,
```

- [x] **Step 6: Thread the path through `commands.rs`.** In `server/src/commands.rs`:

Add the field to `PreparedServer`:

```rust
    /// Removes the runtime-info file on drop (see ADR-0035).
    runtime_guard: crate::runtime_file::RuntimeFileGuard,
```

Change `prepare_server`'s signature to add a trailing param and write the file after `bind`:

```rust
pub async fn prepare_server(
    storage: &StorageArgs,
    bind: SocketAddr,
    prod: bool,
    runtime_file: Option<std::path::PathBuf>,
) -> anyhow::Result<PreparedServer> {
    // … unchanged body through:
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let runtime_path =
        runtime_file.unwrap_or_else(|| storage.storage_path.join("runtime.json"));
    let runtime_guard = crate::runtime_file::RuntimeFileGuard::write(runtime_path, addr);

    Ok(PreparedServer {
        listener,
        router,
        backup_scheduler,
        feed_scheduler,
        runtime_guard,
    })
}
```

Change `cmd_serve` to accept and keep the guard alive across the serve loop:

```rust
pub async fn cmd_serve(
    storage: &StorageArgs,
    bind: SocketAddr,
    prod: bool,
    runtime_file: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let PreparedServer {
        listener,
        router,
        backup_scheduler,
        feed_scheduler,
        runtime_guard,
    } = prepare_server(storage, bind, prod, runtime_file).await?;

    tracing::info!(bind = %bind, prod, "starting HTTP server");
    let _backup_scheduler = backup_scheduler;
    let _feed_scheduler = feed_scheduler;
    // Kept alive until the serve loop returns; removes runtime.json on drop.
    let _runtime_guard = runtime_guard;
    axum::serve(listener, router).await?;
    Ok(())
}
```

- [x] **Step 7: Update the `Commands::Serve` dispatch** in `server/src/main.rs`:

```rust
        Commands::Serve { storage, bind, environment, runtime_file } => {
            jaunder::commands::cmd_serve(&storage, bind, environment.is_prod(), runtime_file).await?;
        }
```

- [x] **Step 8: Fix the existing `prepare_server` test + assert the file.** Open `server/tests/misc/commands.rs`, find the test that calls `prepare_server(...)` (around the `prepared.listener.local_addr()` use). Update the call to pass a tempdir path and assert lifecycle. Pass `Some(rt_path.clone())` as the new 4th arg, where `rt_path` is `<the test's TempDir>/runtime.json`, and after the existing assertions add:

```rust
    assert!(rt_path.exists(), "prepare_server should write the runtime file");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rt_path).unwrap()).unwrap();
    assert_eq!(v["port"], prepared.listener.local_addr().unwrap().port());
    drop(prepared);
    assert!(!rt_path.exists(), "dropping PreparedServer should remove the runtime file");
```

Update any other `prepare_server(` call sites (e.g. in `server/src/main.rs` tests) to pass `None`.

- [x] **Step 9: Run the suite** — `cargo nextest run -p jaunder runtime_file` and the commands test (`cargo nextest run -p jaunder -E 'test(prepare_server)'`); then `cargo xtask check --no-test`.

- [x] **Step 10: Commit**

```bash
git add server/src/runtime_file.rs server/src/lib.rs server/src/cli.rs server/src/commands.rs server/src/main.rs server/tests/misc/commands.rs
git commit -m "feat(server): write a runtime.json with the bound ip/port on serve (#137)"
```

---

### Task 3: Server — `jaunder app-password-create`

**Files:**
- Modify: `server/src/cli.rs` (new `AppPasswordCreate` variant)
- Modify: `server/src/commands.rs` (`app_password_create` inner fn + `cmd_app_password_create` wrapper + a unit test)
- Modify: `server/src/main.rs` (dispatch arm)

**Interfaces:**
- Consumes: `state.users.get_user_by_username(&Username) -> sqlx::Result<Option<UserRecord>>` (use `record.id`), `state.sessions.create_session(user_id: i64, label: &str) -> sqlx::Result<String>`.
- Produces: `pub async fn app_password_create(state: &storage::AppState, username: &Username, label: &str) -> anyhow::Result<String>` (returns the raw token); `pub async fn cmd_app_password_create(storage: &StorageArgs, username: &Username, label: &str) -> anyhow::Result<()>` (prints the token).

- [x] **Step 1: Write the failing unit test** (add to the `#[cfg(test)] mod tests` in `server/src/commands.rs`):

```rust
    #[tokio::test]
    async fn app_password_create_mints_token_for_existing_user() {
        let state = db_test_harness::Backend::Sqlite.setup().await.unwrap();
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        state.users.create_user(&username, &password, None, false).await.unwrap();

        let token = app_password_create(&state, &username, "ert").await.unwrap();
        assert!(!token.is_empty());
    }

    #[tokio::test]
    async fn app_password_create_errors_for_unknown_user() {
        let state = db_test_harness::Backend::Sqlite.setup().await.unwrap();
        let username: Username = "ghost".parse().unwrap();
        assert!(app_password_create(&state, &username, "ert").await.is_err());
    }
```

If `db_test_harness` is not already a dev-dependency of `server`, add `db-test-harness.workspace = true` under `[dev-dependencies]` in `server/Cargo.toml` (it is the shared both-backend harness, ADR-0033). Confirm `Username`/`Password` are imported in the test module (the file already uses them elsewhere; add `use common::username::Username; use common::password::Password;` to the test module if needed).

- [x] **Step 2: Run; expect failure** — `cargo nextest run -p jaunder app_password_create` → fails (`app_password_create` undefined).

- [x] **Step 3: Implement the inner fn + wrapper** (add to `server/src/commands.rs`, near `cmd_user_create`):

```rust
/// Mints an app password (a labelled session token) for an existing user and
/// returns the raw token. The only out-of-process minter (see ADR-0035).
///
/// # Errors
/// Returns an error if the user does not exist or the session cannot be created.
pub async fn app_password_create(
    state: &storage::AppState,
    username: &Username,
    label: &str,
) -> anyhow::Result<String> {
    let user = state
        .users
        .get_user_by_username(username)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("no such user '{username}'"))?;
    let token = state
        .sessions
        .create_session(user.id, label)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(token)
}

/// CLI wrapper: opens the database, mints the token, prints it to stdout.
///
/// # Errors
/// Returns an error if the database cannot be opened or minting fails.
pub async fn cmd_app_password_create(
    storage: &StorageArgs,
    username: &Username,
    label: &str,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;
    let token = app_password_create(&state, username, label).await?;
    println!("{token}");
    Ok(())
}
```

If `UserRecord`'s id field is not named `id`, adjust `user.id` accordingly (confirm against `storage/src/users.rs`). If `storage::AppState` is not re-exported, use the path `open_existing_database` returns (`std::sync::Arc<storage::AppState>` derefs to `&storage::AppState`, so `&state` works).

- [x] **Step 4: Run; expect PASS** — `cargo nextest run -p jaunder app_password_create`.

- [x] **Step 5: Add the CLI variant** to the `Commands` enum in `server/src/cli.rs`:

```rust
    /// Mint an app password (session token) for a user and print it.
    AppPasswordCreate {
        #[command(flatten)]
        storage: StorageArgs,
        #[arg(long)]
        username: String,
        #[arg(long, default_value = "app-password")]
        label: String,
    },
```

- [x] **Step 6: Add the dispatch arm** in `server/src/main.rs` (mirrors the `UserCreate` arm's `Username` parse):

```rust
        Commands::AppPasswordCreate { storage, username, label } => {
            let username = username.parse::<Username>().map_err(|e| anyhow::anyhow!("{e}"))?;
            jaunder::commands::cmd_app_password_create(&storage, &username, &label).await?;
        }
```

- [x] **Step 7: Gate** — `cargo nextest run -p jaunder app_password_create` and `cargo xtask check --no-test`.

- [x] **Step 8: Commit**

```bash
git add server/src/cli.rs server/src/commands.rs server/src/main.rs server/Cargo.toml
git commit -m "feat(server): add jaunder app-password-create to mint a session token (#137)"
```

---

### Task 4: Elisp harness + committed smoke test

Build the harness, prove it works with a *throwaway* pipecleaner (boot + `GET /`, **not committed**), then write the committed smoke test and remove the pipecleaner. Runs host-side; needs the binary built from Tasks 2–3.

**Files:**
- Create: `elisp/test/jaunder-integration-helper.el`
- Create: `elisp/test/jaunder-smoke-integration.el`
- Create: `elisp/scripts/run-integration-tests.el`
- Modify: `elisp/README.md`

**Interfaces:**
- Produces: macro `jaunder-test--with-live-server` (binds `jaunder-base-url`, `jaunder-username`, `jaunder-test-app-password`, and `auth-sources` for its body); the runner `run-integration-tests.el` (globs `test/*-integration.el`).

- [x] **Step 1: Build the server binary** — `cargo build -p jaunder` (so `target/debug/jaunder` carries Tasks 2–3).

- [x] **Step 2: Write the harness helper** — create `elisp/test/jaunder-integration-helper.el`:

```elisp
;;; jaunder-integration-helper.el --- live-server harness for jaunder ERT -*- lexical-binding: t; -*-
;;; Commentary:
;; Boots a real jaunder server in a tempdir, provisions a user + app password
;; (before serving, so there is no concurrent sqlite writer), and runs a body
;; against it.  See ADR-0035.  Not loaded by the pure suite (run-tests.el globs
;; -test.el only).
;;; Code:
(require 'jaunder)
(require 'json)
(require 'url)
(require 'auth-source)
(require 'subr-x)

(defvar jaunder-test-app-password nil
  "Raw app-password token, bound inside `jaunder-test--with-live-server'.")

(defun jaunder-test--binary ()
  "Locate the jaunder binary or signal an error (never silently skip)."
  (or (getenv "JAUNDER_TEST_BINARY")
      (executable-find "jaunder")
      (error "jaunder-test: set JAUNDER_TEST_BINARY or put `jaunder' on PATH")))

(defun jaunder-test--run-cli (bin &rest args)
  "Run BIN with ARGS synchronously; return stdout; error (with output) on failure."
  (with-temp-buffer
    (let ((code (apply #'call-process bin nil t nil args)))
      (unless (eq code 0)
        (error "jaunder-test: %s %S exited %s: %s" bin args code (buffer-string)))
      (buffer-string))))

(defun jaunder-test--read-runtime-file (path)
  "Return (IP . PORT) from runtime file PATH, or nil if absent/unparseable."
  (when (and (file-exists-p path)
             (> (file-attribute-size (file-attributes path)) 0))
    (ignore-errors
      (let* ((json-object-type 'alist)
             (data (json-read-file path)))
        (cons (alist-get 'ip data) (alist-get 'port data))))))

(defun jaunder-test--wait (predicate what)
  "Poll PREDICATE up to 100×0.1s; return its value or error WHAT timed out."
  (or (catch 'done
        (dotimes (_ 100)
          (let ((v (funcall predicate)))
            (when v (throw 'done v)))
          (sleep-for 0.1))
        nil)
      (error "jaunder-test: timed out waiting for %s" what)))

(defun jaunder-test--http-reachable-p (url)
  "Return non-nil if a GET of URL yields any HTTP response."
  (ignore-errors
    (let ((buf (url-retrieve-synchronously url t t 5)))
      (when buf (kill-buffer buf) t))))

(defmacro jaunder-test--with-live-server (&rest body)
  "Boot a jaunder server in a tempdir, provision creds, then run BODY.
Bound in BODY: `jaunder-base-url', `jaunder-username',
`jaunder-test-app-password', and `auth-sources' (a temp netrc with the token)."
  (declare (indent 0) (debug t))
  `(let* ((bin (jaunder-test--binary))
          (tmp (make-temp-file "jaunder-it-" t))
          (storage (expand-file-name "data" tmp))
          (db (concat "sqlite:" (expand-file-name "jaunder.db" tmp)))
          (rf (expand-file-name "runtime.json" tmp))
          (stderr (generate-new-buffer " *jaunder-server*"))
          (proc nil))
     (unwind-protect
         (progn
           (make-directory storage t)
           ;; Provision before serving — no concurrent sqlite writer.
           (jaunder-test--run-cli bin "init" "--db" db "--storage-path" storage)
           (jaunder-test--run-cli bin "user-create" "--db" db "--storage-path" storage
                                  "--username" "alice" "--password" "password123")
           (let ((token (string-trim
                         (jaunder-test--run-cli bin "app-password-create"
                                                "--db" db "--storage-path" storage
                                                "--username" "alice" "--label" "ert"))))
             (setq proc (make-process
                         :name "jaunder-server" :buffer stderr :noquery t
                         :command (list bin "serve"
                                        "--bind" "127.0.0.1:0"
                                        "--db" db "--storage-path" storage
                                        "--runtime-file" rf
                                        "--environment" "dev")))
             (let* ((addr (jaunder-test--wait
                           (lambda () (jaunder-test--read-runtime-file rf)) "runtime.json"))
                    (jaunder-base-url (format "http://%s:%s" (car addr) (cdr addr)))
                    (jaunder-username "alice")
                    (jaunder-test-app-password token)
                    (authinfo (expand-file-name "authinfo" tmp))
                    (auth-source-do-cache nil)
                    (auth-sources (list authinfo)))
               (jaunder-test--wait
                (lambda () (jaunder-test--http-reachable-p (concat jaunder-base-url "/")))
                "server readiness")
               (with-temp-file authinfo
                 (insert (format "machine %s login %s password %s\n"
                                 (car addr) jaunder-username jaunder-test-app-password)))
               ,@body)))
       (when (process-live-p proc) (delete-process proc))
       (when (buffer-live-p stderr) (kill-buffer stderr))
       (delete-directory tmp t))))

(provide 'jaunder-integration-helper)
;;; jaunder-integration-helper.el ends here
```

- [x] **Step 3: Write the runner** — create `elisp/scripts/run-integration-tests.el`:

```elisp
;;; run-integration-tests.el --- live-server ERT runner for jaunder -*- lexical-binding: t; -*-
;;; Commentary:
;; Loads jaunder + the integration helper and every test/*-integration.el, then
;; runs ERT in batch.  Needs a built jaunder binary via JAUNDER_TEST_BINARY or
;; PATH (ADR-0035).  Parallel to run-tests.el, which globs -test.el and so
;; excludes these server-backed tests from the fast pure suite.
;;; Code:
(require 'ert)
(let* ((this (file-name-directory
              (or load-file-name buffer-file-name default-directory)))
       (root (file-name-directory (directory-file-name this)))
       (test-dir (expand-file-name "test" root)))
  (add-to-list 'load-path root)
  (add-to-list 'load-path test-dir)
  (require 'jaunder)
  (require 'jaunder-integration-helper)
  (dolist (f (directory-files test-dir t "-integration\\.el\\'"))
    (load f nil t)))
(ert-run-tests-batch-and-exit)
;;; run-integration-tests.el ends here
```

- [x] **Step 4: Pipecleaner (throwaway, NOT committed).** Create `elisp/test/jaunder-pipecleaner-integration.el` with a single test that boots and reaches the server:

```elisp
;;; jaunder-pipecleaner-integration.el --- THROWAWAY -*- lexical-binding: t; -*-
;;; Code:
(require 'ert)
(require 'jaunder-integration-helper)
(ert-deftest jaunder-pipecleaner-boots ()
  (jaunder-test--with-live-server
    (should (jaunder-test--http-reachable-p (concat jaunder-base-url "/")))))
;;; jaunder-pipecleaner-integration.el ends here
```

- [x] **Step 5: Run the pipecleaner host-side; expect PASS**

```bash
JAUNDER_TEST_BINARY=target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el
```
Expected: `Ran 1 tests, 1 results as expected`. If it hangs on readiness, inspect — the harness surfaces server stderr via the `*jaunder-server*` buffer on error paths; add a temporary `(message "%s" (with-current-buffer stderr (buffer-string)))` while debugging.

- [x] **Step 6: Delete the pipecleaner** — `rm elisp/test/jaunder-pipecleaner-integration.el`.

- [x] **Step 7: Write the committed smoke test** — create `elisp/test/jaunder-smoke-integration.el`:

```elisp
;;; jaunder-smoke-integration.el --- live-server smoke tests -*- lexical-binding: t; -*-
;;; Commentary:
;; End-to-end smoke over real HTTP: proves boot + provisioning + auth.  Uses
;; url.el directly (the client HTTP layer is Unit C, #74).  See ADR-0035.
;;; Code:
(require 'ert)
(require 'jaunder)
(require 'jaunder-integration-helper)

(defun jaunder-smoke--get (url &optional extra-headers)
  "GET URL with optional EXTRA-HEADERS; return (STATUS . BODY)."
  (let ((url-request-method "GET")
        (url-request-extra-headers extra-headers)
        (buf (url-retrieve-synchronously url t t 10)))
    (unwind-protect
        (with-current-buffer buf
          (goto-char (point-min))
          (let ((status (and (re-search-forward "^HTTP/[0-9.]+ \\([0-9]+\\)" nil t)
                             (string-to-number (match-string 1))))
                (body (progn (goto-char (point-min))
                             (when (re-search-forward "\r?\n\r?\n" nil t)
                               (buffer-substring-no-properties (point) (point-max))))))
            (cons status body)))
      (when (buffer-live-p buf) (kill-buffer buf)))))

(ert-deftest jaunder-smoke-service-document-advertises-capability ()
  "The unauthenticated service document advertises the j:extension capability."
  (jaunder-test--with-live-server
    (let ((resp (jaunder-smoke--get
                 (jaunder--build-url jaunder-base-url "atompub" "service"))))
      (should (eq (car resp) 200))
      (should (string-match-p "j:extension" (cdr resp)))
      (should (string-match-p "format-media-type" (cdr resp)))
      (should (string-match-p "slug" (cdr resp))))))

(ert-deftest jaunder-smoke-authenticated-collection ()
  "An app-password Basic request returns the user's (empty) posts collection."
  (jaunder-test--with-live-server
    (let ((resp (jaunder-smoke--get
                 (jaunder--build-url jaunder-base-url "atompub" jaunder-username "posts")
                 (list (jaunder--basic-auth-header
                        jaunder-username jaunder-test-app-password)))))
      (should (eq (car resp) 200)))))

(provide 'jaunder-smoke-integration)
;;; jaunder-smoke-integration.el ends here
```

- [x] **Step 8: Run the committed smoke host-side; expect PASS**

```bash
JAUNDER_TEST_BINARY=target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el
```
Expected: `Ran 2 tests, 2 results as expected`.

- [x] **Step 9: Format + pure-gate** — `emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix` then `cargo xtask check --no-test` (runs `elisp-fmt` + the pure `ert`; confirm the pure suite is unchanged and the new files are excluded from it).

- [x] **Step 10: Document** — in `elisp/README.md`, add a short "Integration tests" section noting: the live suite is `elisp/scripts/run-integration-tests.el`; run it host-side with `JAUNDER_TEST_BINARY=target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el` after `cargo build -p jaunder`; it boots a real server per test.

- [x] **Step 11: Commit**

```bash
git add elisp/test/jaunder-integration-helper.el elisp/test/jaunder-smoke-integration.el elisp/scripts/run-integration-tests.el elisp/README.md
git commit -m "test(elisp): live-server integration harness + smoke tests (#137)"
```

---

### Task 5: Nix `nixosTest` check + xtask wiring

Make the smoke run hermetically in the gate. The harness self-boots, so the VM only supplies Emacs + the binary (no `services.jaunder`, no Playwright).

**Files:**
- Modify: `flake.nix` (new `elisp-integration` check + add to `checks`)
- Modify: `xtask/src/steps/nix.rs` (new `elisp_integration` invocation)
- Modify: `xtask/src/lib.rs` (call it in the e2e tier of `validate`)

- [x] **Step 1: Add the check to `flake.nix`.** Inside the `pkgs.lib.optionalAttrs pkgs.stdenv.isLinux { … }` block (next to `e2e-sqlite`), add:

```nix
        elisp-integration = pkgs.testers.nixosTest {
          name = "elisp-integration";
          nodes.machine = _: {
            virtualisation.memorySize = 2048;
            environment.systemPackages = [
              emacsForCi
              jaunderBin
            ];
          };
          testScript = ''
            machine.start()
            machine.wait_for_unit("multi-user.target")
            machine.succeed(
                "JAUNDER_TEST_BINARY=${jaunderBin}/bin/jaunder "
                + "emacs --batch -Q -l ${emacsSrc}/scripts/run-integration-tests.el"
            )
          '';
        };
```

(`jaunderBin`, `emacsForCi`, and `emacsSrc` are already let-bound in `flake.nix`.)

- [x] **Step 2: Build the check directly to verify it** —

```bash
nix build -L .#checks.x86_64-linux.elisp-integration
```
Expected: the VM boots, the suite prints `Ran 2 tests, 2 results as expected`, the build succeeds.

- [x] **Step 3: Add the xtask invocation** in `xtask/src/steps/nix.rs` (mirror `e2e`):

```rust
/// Runs the hermetic elisp live-integration `nixosTest` check.
pub fn elisp_integration(result: &mut CommandResult) {
    result.push(build_check("nix-elisp-integration", "elisp-integration"));
}
```

- [x] **Step 4: Call it in the validate e2e tier.** In `xtask/src/lib.rs`, find where `validate` invokes `nix::e2e(...)` (the branch that runs only when e2e is enabled, i.e. not `--no-e2e`). Add, adjacent to that call:

```rust
            nix::elisp_integration(&mut result);
```
Keep it gated the same way as `nix::e2e` so it runs in full `cargo xtask validate` and CI's e2e surface, and is skipped by `validate --no-e2e` / the pre-push hook.

- [x] **Step 5: Gate the xtask change** — `cargo xtask check --no-test` (clippy + fmt for the xtask crate).

- [x] **Step 6: Full validate** — `cargo xtask validate`. Expected: all static checks, coverage, e2e, and the new `nix-elisp-integration` step pass (`xtask-done: … ok=true`). Confirm via the sidecar: `jq '.steps[] | select(.name=="nix-elisp-integration")' .xtask/last-result.json`.

- [x] **Step 7: Commit**

```bash
git add flake.nix xtask/src/steps/nix.rs xtask/src/lib.rs
git commit -m "ci(xtask): run the elisp live-integration nixosTest in validate (#137)"
```

---

## Self-Review

**Spec coverage:**
- `app-password-create` → Task 3. ✓
- `serve` runtime-info file (`{ip,port}`, data dir, atomic, `--runtime-file`/env override, best-effort) → Task 2. ✓
- Harness owns lifecycle (boot, provision, readiness via runtime-file + `GET /`, teardown) → Task 4 (`jaunder-test--with-live-server`). ✓ (Provisioning runs before `serve` to avoid a concurrent sqlite writer — an improvement consistent with the spec's "provisions a user + app password.")
- Binary via `JAUNDER_TEST_BINARY`, errors loudly if missing → Task 4 `jaunder-test--binary`. ✓
- Separate `run-integration-tests.el`, pure suite stays serverless → Task 4 (globs `-integration.el`; `run-tests.el` globs `-test.el`). ✓
- `nixosTest` VM check, no systemd/Playwright, wired into `validate` not `check --no-test` → Task 5. ✓
- Committed smoke: unauth service-doc capability + authed empty-collection → Task 4. ✓
- Throwaway pipecleaner, not committed → Task 4 Steps 4–6. ✓
- ADR-0035 → already written.
- Three follow-ons (signal-robust removal, mutex, admin channel) filed → Task 1. ✓
- Edge cases: binary-missing (errors), boot-failure (runtime.json never appears → bounded wait times out, stderr captured), teardown-on-failure (`unwind-protect`) → Task 4 harness. ✓ Auth negative path is exercised implicitly; not separately committed (optional, omitted to keep the smoke minimal — acceptable per spec "smoke-adjacent, cheap").

**Placeholder scan:** No TBD/TODO. The one read-then-adapt point (Task 2 Step 8, the existing `prepare_server` test) gives exact assertions to add; the `UserRecord.id` field name is flagged to confirm (Task 3 Step 3).

**Type consistency:** `RuntimeFileGuard::write(PathBuf, SocketAddr)`, `prepare_server(…, Option<PathBuf>)`, `PreparedServer.runtime_guard`, `app_password_create(&AppState, &Username, &str) -> anyhow::Result<String>`, `get_user_by_username`/`create_session` signatures, and the macro-bound elisp vars (`jaunder-base-url`, `jaunder-username`, `jaunder-test-app-password`) are used consistently across tasks.
