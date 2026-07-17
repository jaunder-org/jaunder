# Backup Vertical Co-location Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Co-locate backup's reactive UI (`BackupSettingsPage` + `BackupBanner`)
with its already-conformant server half under `web/src/backup/`, remove the
`pages/backup.rs` counterpart, and gate only by cargo `feature` — no
`target_arch`.

**Architecture:** A **behavior-preserving relocation**, not new behavior. Move
the two `#[component]`s out of the `#[cfg(target_arch="wasm32")]`-gated `pages`
module into a new unconditional `web/src/backup/ui.rs` (mirroring the converged
`web/src/audiences/mod.rs`), repointing imports (`crate::pages::Topbar` →
`crate::ui::Topbar`; server-fn calls to their co-located siblings) and the
`/admin/backups` route. The `#[server]` fns and `web/src/backup/server.rs` are
untouched.

**Tech Stack:** Rust, Leptos (`#[component]` / `#[server]`), `cargo xtask` gate,
Playwright e2e (`end2end/`).

## Spec

`docs/superpowers/specs/2026-07-17-issue-316-backup-vertical-colocate.md` — read
it for the what/why and the AC list (AC1–AC10). This plan is the how; it does
not restate the spec.

## Global Constraints

- **This is a relocation with no behavior change.** No new tests are authored.
  The behavior pins are the existing `end2end/tests/backup.spec.ts` (three
  `/admin/backups` tests) and the existing `web/src/backup/server.rs` unit test
  — they must keep passing **unchanged**. "Write the failing test" does not
  apply; each task's verification is compile + existing tests + the gate.
- **No `target_arch` gate** may be introduced on any relocated component (AC4).
  The new `ui` module is unconditional, exactly like `web/src/audiences/mod.rs`.
- **No fake-value host stub** (AC7, ADR-0055 surviving principle).
- **Preserve the `// cov:ignore-start` / `// cov:ignore-stop` markers** wrapping
  `backup_settings_form` when it moves — `#[component]` fns are coverage-exempt,
  but that plain `fn` relies on the markers.
- **Server half is out of scope** (AC8): do not touch the four `#[server]` fns
  or `web/src/backup/server.rs`.
- **Per-commit gate:** `cargo xtask check` (fmt + clippy + Nix coverage/tests)
  via the pre-commit hook — run it first so it passes clean
  (**jaunder-commit**). Because default `cargo check` skips `feature="server"`
  web code, also run the host `--all-features --all-targets` compile named in
  each task. **No `Co-Authored-By` trailer.**

---

## Review header (one line per task)

- **Task 1** — File the follow-up issue (relocate `current_user_is_operator` →
  `auth`, deferred behind #315). _Separable concern; AC9._
- **Task 2** — Relocate `BackupSettingsPage` + `backup_settings_form` into
  `web/src/backup/ui.rs`; delete `pages/backup.rs`; repoint route + mod decls.
  _AC1–AC5, AC8._
- **Task 3** — Relocate `BackupBanner` into `web/src/backup/ui.rs`; fix up
  `AppShell` import, the `pages` re-export, and the narrowed `pages/ui.rs`
  import. _AC6._
- **Task 4** — Full `cargo xtask validate` + AC sweep. _AC10, closing
  verification._

**Scope (in):** the two backup `#[component]`s, their `pages/` wiring, one
follow-up issue. **Scope (out):** `current_user_is_operator` relocation, the
app-shell sidebar chrome, the `#[server]` bodies, `common::backup`, the `server`
backup runner, `host::metrics`.

**Key risks/decisions:**

- The `#[server]` fns stay put and stay `crate::backup::*`; `pages/ui.rs` still
  imports `current_user_is_operator` from `crate::backup` after Task 3 (only
  `backup_warning_visible` leaves its import list).
- `AppShell` (`pages/mod.rs`) currently sees `BackupBanner` via the
  `pub use ui::{…}` glob; after Task 3 it must import it from `crate::backup`
  and it is dropped from that glob.
- Confirmed no external `crate::pages::BackupBanner` / `crate::pages::backup::*`
  consumers exist outside `pages/mod.rs` (a `render/mod.rs` mention is a doc
  comment only).

---

### Task 1: File the follow-up issue (separable concern)

**Files:** none (GitHub tracker only).

**Interfaces:**

- Produces: an open issue number referenced by AC9.

- [x] **Step 1: Create the issue via jaunder-issues.** Use the
      **jaunder-issues** skill to file into `jaunder-org/jaunder`:
  - Title:
    `web(backup): relocate current_user_is_operator from web::backup to the auth vertical`
  - Body (in essence): `current_user_is_operator` currently lives in
    `web/src/backup/mod.rs` but is a generic operator check, consumed only by
    the app-shell sidebar (`web/src/pages/ui.rs` `authed_sidebar`), not
    backup-domain logic. Its correct home is the `auth` vertical. Deferred out
    of #316 because the `auth` vertical is under concurrent work (#315).
    Acceptance: `current_user_is_operator` defined in the `auth` vertical, its
    caller in `pages/ui.rs` repointed, `web::backup` no longer exports it, gate
    green.
  - Labels/type/milestone/project: match #316 (`tooling`, Task, milestone "Web:
    canonical Leptos CSR convergence", project #1) per jaunder-issues
    conventions.
  - Mark it **blocked by #315** using a native GitHub issue dependency (not a
    body note).

- [x] **Step 2: Record the number** — filed **#510** (Task, `tooling`, milestone
      "Web: canonical Leptos CSR convergence", Backlog project #1, blocked-by
      #315). No commit (tracker-only task).

---

### Task 2: Relocate `BackupSettingsPage` into `web/src/backup/ui.rs`

**Files:**

- Create: `web/src/backup/ui.rs` (the settings-page UI, moved from
  `pages/backup.rs`).
- Modify: `web/src/backup/mod.rs` (declare `mod ui;` + re-export).
- Modify: `web/src/pages/mod.rs` (drop `pub mod backup`; repoint the route
  import).
- Delete: `web/src/pages/backup.rs`.

**Interfaces:**

- Consumes: the existing
  `crate::backup::{get_backup_settings, UpdateBackupSettings}` server fns/args
  (unchanged); `crate::ui::Topbar`; `crate::forms::{Field, ValidatedInput}`;
  `common::backup::{BackupConfig, BackupMode, BackupSchedule, RetentionCount}`.
- Produces: `crate::backup::BackupSettingsPage`
  (`#[component] pub fn BackupSettingsPage() -> impl IntoView`).

- [x] **Step 1: Create `web/src/backup/ui.rs`** with the contents of
      `pages/backup.rs` (lines 1–130) moved verbatim —
      `#[component] pub fn BackupSettingsPage()` and the `// cov:ignore-start` …
      `backup_settings_form(…)` … `// cov:ignore-stop` block — with the import
      header changed to (the only edits vs. the original):

  ```rust
  use crate::backup::{get_backup_settings, UpdateBackupSettings};
  use crate::error::WebError;
  use crate::forms::{Field, ValidatedInput};
  use crate::ui::Topbar; // was: crate::pages::Topbar (AC5)
  use common::backup::{BackupConfig, BackupMode, BackupSchedule, RetentionCount};
  use leptos::prelude::*;
  use strum::VariantArray;
  ```

  The `<Topbar … />` call site and every other line stay identical. Keep the
  `// cov:ignore-start/stop` markers around `backup_settings_form`.

- [x] **Step 2: Wire the submodule in `web/src/backup/mod.rs`.** Add (module is
      unconditional — no `target_arch`, no `feature` gate, matching audiences):

  ```rust
  mod ui;
  pub use ui::BackupSettingsPage;
  ```

  Place the `mod ui;` with the other module decls (near the existing
  `#[cfg(feature = "server")] pub(crate) mod server;`) and the `pub use` at an
  appropriate top-level spot.

- [x] **Step 3: Delete `web/src/pages/backup.rs`** and remove its module
      declaration — delete `pub mod backup;` at `web/src/pages/mod.rs:2`.

- [x] **Step 4: Repoint the route import** in `web/src/pages/mod.rs:30`:

  ```rust
  use crate::backup::BackupSettingsPage; // was: crate::pages::backup::BackupSettingsPage
  ```

  The route table entry (`pages/mod.rs:132–135`, `view=BackupSettingsPage`, path
  `/admin/backups`) is unchanged (AC3).

- [x] **Step 5: Compile (host, all features).**
      `cargo check -p web     --all-features --all-targets` PASS;
      `cargo build -p web` PASS. The e2e pin is run once at Task 4's full
      `validate` (avoids running the heavy browser suite three times for a
      verbatim relocation).

- [x] **Step 6: Gate + commit.** `cargo xtask check` clean (coverage 19406
      lines, 0 CRAP over threshold); committed as the Task 2 refactor commit.
      Then:

  ```bash
  git add web/src/backup/ui.rs web/src/backup/mod.rs web/src/pages/mod.rs
  git rm web/src/pages/backup.rs
  git commit -m "refactor(web): co-locate BackupSettingsPage in web::backup::ui (#316)"
  ```

  (Verify `git status` — the git-add hook may auto-stage; confirm
  `pages/backup.rs` is staged as a deletion.)

---

### Task 3: Relocate `BackupBanner` into `web/src/backup/ui.rs`

**Files:**

- Modify: `web/src/backup/ui.rs` (add `BackupBanner`).
- Modify: `web/src/backup/mod.rs` (extend the re-export).
- Modify: `web/src/pages/ui.rs` (remove `BackupBanner` def; narrow the backup
  import).
- Modify: `web/src/pages/mod.rs` (drop `BackupBanner` from the `ui` re-export;
  import it from `crate::backup` for `AppShell`).

**Interfaces:**

- Consumes: the sibling `crate::backup::backup_warning_visible` server fn;
  `crate::server_resource`.
- Produces: `crate::backup::BackupBanner`
  (`#[component] pub fn BackupBanner() -> impl IntoView`).

- [x] **Step 1: Move `BackupBanner` into `web/src/backup/ui.rs`.** Cut the
      `#[component] pub fn BackupBanner()` definition from `web/src/pages/ui.rs`
      (lines 156–181) and paste it into `web/src/backup/ui.rs`. The body calls
      **bare** `backup_warning_visible()`, so add the import that made it
      resolve at its old home — insert at the top of `web/src/backup/ui.rs`:

  ```rust
  use super::backup_warning_visible;
  ```

  `crate::server_resource` is already an absolute path, and
  `Suspense`/`Suspend`/`view!` come from the existing `use leptos::prelude::*;`
  already in `ui.rs` — no further imports.

- [x] **Step 2: Extend the re-export** in `web/src/backup/mod.rs`:

  ```rust
  pub use ui::{BackupBanner, BackupSettingsPage};
  ```

- [x] **Step 3: Clean up `web/src/pages/ui.rs`.** After removing the
      `BackupBanner` definition, narrow the backup import at `pages/ui.rs:5` so
      it drops the now-unused `backup_warning_visible` (web denies warnings):

  ```rust
  use crate::backup::current_user_is_operator; // was: {backup_warning_visible, current_user_is_operator}
  ```

  (`current_user_is_operator` stays — the sidebar still consumes it.)

- [x] **Step 4: Fix `AppShell`'s wiring in `web/src/pages/mod.rs`.** Drop
      `BackupBanner` from the `pub use ui::{…}` glob (lines 17–20) and import it
      from `crate::backup` so the `<BackupBanner />` at line 64 resolves:

  ```rust
  pub use ui::{
      Avatar, Chip, Dot, Icon, Icons, InlineComposer, PostCard, PostDisplay, Sidebar, Topbar,
  }; // BackupBanner removed
  use crate::backup::BackupBanner;
  ```

- [x] **Step 5: Compile (host, all features).**
      `cargo check -p web     --all-features --all-targets` PASS;
      `cargo build -p web` PASS (no dead-import / unresolved-name warnings). The
      e2e pin runs once at Task 4's full `validate`.

- [x] **Step 6: Gate + commit.** `cargo xtask check` clean (coverage 19419
      lines, 0 CRAP over threshold); committed as the Task 3 refactor commit.
      Then:

  ```bash
  git add web/src/backup/ui.rs web/src/backup/mod.rs web/src/pages/ui.rs web/src/pages/mod.rs
  git commit -m "refactor(web): co-locate BackupBanner in web::backup::ui (#316)"
  ```

---

### Task 4: Full validate + acceptance sweep

**Files:** none (verification only).

**Interfaces:** none.

- [x] **Step 1: Run the full local gate.** `cargo xtask validate --no-e2e` green
      (coverage clean — 19419 lines, 0 failures, 0 CRAP over threshold) +
      `cargo xtask e2e sqlite chromium` green (all three `backup.spec.ts` tests,
      `[26–28/91]`, passed). Decomposed per ADR-0034 (coverage gate + one e2e
      combo foreground; the full `{sqlite,postgres}×{chromium,firefox}` matrix
      is CI's `e2e-gate` at merge) to avoid background-reaping under concurrent
      worktree load.

  Run: `devtool run -- cargo xtask validate` (foreground; long — coverage + all
  four `{sqlite,postgres}×{chromium,firefox}` e2e combos). Expected: PASS
  (AC10). Read `.xtask/last-result.json` `steps[]` on any non-`ok`.

- [x] **Step 2: Objective AC sweep** — all green (grep/inspection over
      `git diff wt-base-issue-316..HEAD`; the only `target_arch` hit under
      `web/src/backup/` is an explanatory comment, not a `#[cfg]`):
  - AC2: `web/src/pages/backup.rs` gone; no `pub mod backup` in `pages/mod.rs`.
  - AC4: `rg 'target_arch' web/src/backup/` returns nothing new.
  - AC1/AC6: `crate::backup::BackupSettingsPage` and
    `crate::backup::BackupBanner` both resolve; no
    `BackupBanner`/`BackupSettingsPage` definition remains under `pages/`.
  - AC5: `web/src/backup/ui.rs` imports `Topbar` from `crate::ui`.
  - AC8: `git diff` shows no change to the four `#[server]` fns or
    `backup/server.rs`.
  - AC9: the Task 1 follow-up issue exists (record its number).
  - AC7: diff introduces no fake-value host stub.

- [x] **Step 3: No commit.** This is verify-only; the branch is ready for
      **jaunder-ship**.

---

## Self-review

- **Spec coverage:** AC1/AC5 → Task 2 Steps 1–2; AC2 → Task 2 Steps 3–4; AC3 →
  Task 2 Step 4 + Step 5 e2e; AC4/AC7 → Global Constraints + Task 4 Step 2; AC6
  → Task 3; AC8 → Global Constraints (untouched) + Task 4 Step 2; AC9 → Task 1;
  AC10 → Task 4 Step 1. All ten ACs map to a task.
- **No placeholders:** every import edit is written in full; moved bodies are
  verbatim from named source lines; commands are exact with expected results.
- **Type consistency:** `BackupSettingsPage` / `BackupBanner` names and the
  `crate::backup::*` re-export paths are used identically across Tasks 2–4.
