# Rename `web` feature `ssr` → `server` — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Rename the `web` crate's cargo feature `ssr` → `server` (it now
compiles the server-side data-API build, not an SSR page render), with no
behavior change.

**Architecture:** Two commits. Commit 1 is the **atomic** code rename — the
feature definition (`web/Cargo.toml`), its one activation site
(`server/Cargo.toml`), and all 20 `web/src` `cfg` sites must change together,
because a half-rename either references a non-existent feature (hard cargo
error) or silently drops `ssr`-gated code. Commit 2 updates the live contributor
docs. There are no new unit tests: this is a rename refactor, and the existing
suite compiling + passing under the renamed feature (via `cargo xtask check`) is
the regression net that proves no site was missed and no behavior changed (spec
§"Acceptance criteria" #8).

**Tech Stack:** Rust, cargo features, Leptos.

**Spec:**
`docs/superpowers/specs/2026-07-04-issue-197-rename-web-feature-ssr-server.md` —
read it; this plan is the "how," the spec is the "what/why."

## Global Constraints

- **Never touch leptos's own feature:** `leptos/ssr`, `leptos_meta/ssr`,
  `leptos_router/ssr` (values in `web/Cargo.toml`) and the two
  `leptos = { … features = ["ssr"] }` deps in `server/Cargo.toml` stay verbatim.
  Only the literal `feature = "ssr"` (web's cfg gates), the `ssr = [` feature
  key, and the **web dependency's** `features = ["ssr"]` change.
- **No behavior change** — rename only. Do not touch `csr` / `hydrate` /
  `default`.
- **Commit trailer:** no `Co-Authored-By`. Pre-commit hook runs full
  `cargo xtask check`; run it first so it passes clean (**jaunder-commit**).

---

## Review header

**Scope (in):** `web/Cargo.toml` feature key, `server/Cargo.toml` web-dep
feature, 20 `web/src/**` cfg/cfg_attr sites, `CONTRIBUTING.md`,
`docs/web-style-guide.md` §8, `docs/adr/0041-public-projector-and-csr-client.md`
note. **Scope (out):** leptos's `ssr` feature; `csr`/`hydrate`/`default`;
`docs/archive/**`, ADR-0013, ADR-0017 (historical records left as-is).

- **Task 1** — atomic code rename (22 files: 2 Cargo.toml + 20 web/src), gated
  by `cargo xtask check` green and zero `feature = "ssr"` in `web/src`.
- **Task 2** — doc updates (CONTRIBUTING.md, web-style-guide §8, ADR-0041).

**Key risks/decisions:** the code rename cannot be split into compiling
sub-commits (feature-key rename and cfg-site rename must be one commit) — so
Task 1 is one deliberately larger-but-mechanical task, verified in bulk by the
gate, not per-file. No separable concerns surfaced → no issue-filing first task.

---

## Task 1: Atomic code rename (`ssr` → `server`)

**Files:**

- Modify: `web/Cargo.toml:68` — feature key `ssr = [` → `server = [` (the
  12-entry value list, including `leptos/ssr` / `leptos_meta/ssr` /
  `leptos_router/ssr`, stays byte-identical — only the key changes).
- Modify: `server/Cargo.toml` — the **`web`** dependency line
  (`web = { path = "../web", default-features = false, features = ["ssr"] }`):
  `features = ["ssr"]` → `features = ["server"]`. Leave the two `leptos` deps
  (`leptos = { workspace = true, features = ["ssr"] }`) untouched.
- Modify: all 20 files under `web/src/` that contain the literal
  `feature = "ssr"` — every `#[cfg(feature = "ssr")]` and
  `#[cfg_attr(feature = "ssr", …)]`: `web/src/error.rs`, `web/src/lib.rs`,
  `web/src/viewer.rs`, `web/src/feed_events.rs`, `web/src/backup/mod.rs`,
  `web/src/posts/mod.rs`, `web/src/posts/listing.rs`, `web/src/posts/server.rs`,
  `web/src/auth/mod.rs`, `web/src/site/mod.rs`, `web/src/audiences/mod.rs`,
  `web/src/subscriptions/mod.rs`, `web/src/password_reset/mod.rs`,
  `web/src/media/mod.rs`, `web/src/email/mod.rs`, `web/src/profile/mod.rs`,
  `web/src/tags/mod.rs`, `web/src/sessions/mod.rs`, `web/src/invites/mod.rs`,
  `web/src/pages/invites.rs`.
- Test: none added — the existing suite run by `cargo xtask check` is the net.

**Interfaces:**

- Consumes: nothing.
- Produces: the cargo feature `web/server` (was `web/ssr`), activated by the
  `server` crate. Task 2 (docs) references the new name but has no code
  dependency.

**Method (deterministic, structured edits — not `sed -i`):**

- [x] **Step 1: Rename the feature key** in `web/Cargo.toml` — change the single
      line `ssr = [` (line 68) to `server = [`. Leave lines 69–81 (the value
      list) unchanged. Confirm `default = []` and `csr = ["leptos/csr"]` are
      untouched.

- [x] **Step 2: Rename the web-dep activation** in `server/Cargo.toml` — on the
      `web = { … }` dependency line only, change `features = ["ssr"]` →
      `features = ["server"]`. Do **not** alter the `leptos` dependency lines.

- [x] **Step 3: Rename all cfg sites** — for each of the 20 files above, use the
      Edit tool with `replace_all: true`, matching `feature = "ssr"` →
      `feature = "server"`. This literal cannot match `leptos/ssr` (different
      string), so cfg and cfg_attr forms are both covered and leptos is safe.
      One Edit call per file.

- [x] **Step 4: Verify the rename is complete and the old name is gone**

      Run: `rg 'feature = "ssr"' web/src`
      Expected: **no matches** (exit 1 / empty).

      Run: `rg 'ssr = \[|features = \["ssr"\]' web/Cargo.toml`
      Expected: **no matches** in `web/Cargo.toml`.

      Run: `rg 'leptos.*"ssr"|/ssr' web/Cargo.toml server/Cargo.toml`
      Expected: **still matches** — leptos's own feature untouched (the three
      `leptos*/ssr` values in `web/Cargo.toml`, the two `leptos … ["ssr"]` deps in
      `server/Cargo.toml`).

- [x] **Step 5: Run the gate** — proves the workspace + `server` crate + `web`
      unit tests compile and pass under the renamed feature (spec AC #8).

      Run: `cargo xtask check`
      Expected: PASS (`ok:true`, `xtask-done: … ok=true`). A missed cfg site would
      show as dead/undefined-symbol or feature-resolution errors here.

- [x] **Step 6: Commit** (run the gate first per **jaunder-commit**; the
      pre-commit hook re-runs `cargo xtask check`)

      ```bash
      git add web/Cargo.toml server/Cargo.toml web/src
      git commit -m "refactor(web): rename cargo feature ssr -> server

      After #180 removed the reactive SSR render, the web feature no longer
      means isomorphic SSR — it compiles the server-side data-API build with no
      page render. Rename ssr -> server for honesty. Mechanical, no behavior
      change. leptos's own ssr feature is untouched.

      Closes #197"
      ```

---

## Task 2: Update live contributor docs

**Files:**

- Modify: `CONTRIBUTING.md:604` —
  `Use #[cfg(feature = "ssr")] for server-only imports and logic` →
  `#[cfg(feature = "server")]`.
- Modify: `docs/web-style-guide.md` §8 — the two code-block lines (174, 176)
  `#[cfg(feature = "ssr")]` → `#[cfg(feature = "server")]`, and the prose on
  line 181 `No per-import #[cfg(feature = "ssr")] annotations…` →
  `feature = "server"`.
- Modify: `docs/adr/0041-public-projector-and-csr-client.md:96-98` — the bullet
  currently reads "The `web` feature still named `ssr` (it now means …); a
  cosmetic rename to `server` is a deferred follow-up." Rewrite to reflect
  completion, e.g.: "The `web` feature is named `server` (it compiles the
  server-side data-API build, no page render); renamed from `ssr` in #197." Keep
  it factual and one bullet.
- Test: none — docs have no build impact.

**Interfaces:**

- Consumes: the `web/server` feature name from Task 1.
- Produces: nothing downstream.

- [x] **Step 1: Edit the three docs** as specified above (Edit tool).

- [x] **Step 2: Verify no stale live references remain**

      Run: `rg 'feature = "ssr"' CONTRIBUTING.md docs/web-style-guide.md docs/adr/0041*`
      Expected: **no matches**.

      Run: `rg 'still named' docs/adr/0041*`
      Expected: **no matches** (the false statement is gone).

      Confirm ADR-0013 / ADR-0017 / `docs/archive/**` are untouched:
      Run: `git diff wt-base-issue-197..HEAD --name-only -- docs/adr/0013* docs/adr/0017* docs/archive`
      Expected: **empty**.

- [x] **Step 3: Commit**

      ```bash
      git add CONTRIBUTING.md docs/web-style-guide.md docs/adr/0041-public-projector-and-csr-client.md
      git commit -m "docs: update web feature guidance for ssr -> server rename

      CONTRIBUTING.md and web-style-guide §8 now show #[cfg(feature = \"server\")];
      ADR-0041's note corrected to state the feature is named server (renamed in
      #197). ADR-0013/0017 left as historical records.

      Refs #197"
      ```

---

## Self-Review

- **Spec coverage:** AC1 → T1 S1; AC2 → T1 S2 + S4; AC3 → T1 S3 + S4; AC4 → T2
  S1+S2; AC5 → T2 S1+S2; AC6 (leptos untouched) → T1 S4 guard; AC7
  (archive/ADR-0013/0017 unchanged) → T2 S2 diff check; AC8 (gate) → T1 S5. All
  covered.
- **Placeholder scan:** none — every step has an exact file, edit, command, and
  expected result.
- **Type consistency:** the only produced name is the feature `server`,
  referenced consistently across both tasks.
