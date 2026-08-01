# Spec — #251: rename the mount-readiness marker off "hydration"

**Issue:** jaunder-org/jaunder#251 · **Milestone:** Web: canonical Leptos CSR
convergence · **Label:** `tooling` · **Branch:**
`worktree-issue-251-mount-ready-marker`

## Problem

After the CSR re-architecture (ADR-0040/0041 — `mount_to_body`, no hydration),
the e2e readiness signal is still called _hydration_ at every level: the DOM
attribute `body[data-hydrated]`, the helper `waitForHydration`, the module
`hydration.ts`, the OTel action `wait.hydration`, the fixture plumbing
(`hydratedMs`, `__jaunderRecordHydration`), and the prose that describes them.
**There is no hydration on CSR.** The name describes a rendering strategy the
app abandoned, so every reader has to learn that "hydration" here means "mount".

#228 already renamed the trace field `commit_to_hydration → commit_to_mount` and
#224 renamed the timeout scalers `hydrationHeavy* → slowBrowser*`, deliberately
deferring the marker itself to this issue. The result is a tree that contradicts
itself: `fixtures.ts` computes a variable called `hydratedMs` and emits it as
`navigation.commit_to_mount_ms`.

### The issue text is out of date

Filed 2026-07-04, it names call sites that have since moved. Corrected
inventory:

- `posts.spec.ts` is **no longer** a reader of the marker (it retains only an
  unrelated "post-hydration Effect" comment).
- `docs/observability.md:319` is now `:456`.
- The `hydratedMs` / `__jaunderRecordHydration` / `wait.hydration` OTel surface
  in `fixtures.ts` did not exist in its current form and is not mentioned at
  all.
- `end2end/CLAUDE.md` (untracked) is a reader, and is **already** stale from
  #224.

## Decisions

| #      | Decision                                                                                                                                                                                                                                                                                                                                                                                            |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | The stem is **`mount`/`mounted`**: `data-mounted`, `waitForMount`, `mount.ts`, `wait.mount`, `mountedMs`, `__jaunderRecordMount`. Chosen to match #228's `commit_to_mount`, not to invent a third vocabulary.                                                                                                                                                                                       |
| **D2** | The OTel action renames to `wait.mount`. No code couples to it — `xtask/src/traces/analyze.rs::action_hotspot_rows` reads action names generically out of `e2e.action_top_json`, and trace files are never committed.                                                                                                                                                                               |
| **D3** | Dated findings sections are **renamed in place, with a trailing note recording the rename** — the convention #224 used at `docs/observability.md:306`/`:308` (new name inline, old name deleted, "renamed accordingly in #224" appended). #228 renamed `commit_to_hydration` in that same bullet with no note at all, so a note is the more generous end of the precedent, not a departure from it. |
| **D4** | Scope is **every use in tracked `csr/`, `end2end/`, and `xtask/src/steps/build_csr.rs` that calls the CSR mount "hydration"** — the marker chain, the fixture OTel internals, and the prose comments. Real-SSR-hydration prose elsewhere stays.                                                                                                                                                     |
| **D5** | `theme.spec.ts`'s test title renames to `…after CSR mount`, and `docs/coverage/server-fns-evidence.json` is **regenerated** so the committed evidence matches. The old title appears there **4×** (`:153`, `:305`, `:849`, `:1037`).                                                                                                                                                                |
| **D6** | The TS side collapses its four attribute-name literals into one exported `MOUNTED_ATTR` (+ `MOUNTED_SELECTOR`) in `mount.ts`. Rust keeps its own literal — it crosses the wasm boundary — with a comment naming the TS constant as counterpart.                                                                                                                                                     |
| **D7** | `end2end/CLAUDE.md` is **untracked and stays untracked**: updated in place in the main checkout, never added to this branch. No follow-up issue filed. Because it is invisible to the branch it is a **ship-checklist item, not an acceptance criterion** (see below).                                                                                                                              |
| **D8** | **No ADR.** This renames into vocabulary ADR-0040 already established; #228 set no ADR for `commit_to_mount`.                                                                                                                                                                                                                                                                                       |
| **D9** | **AC1 (zero residue) is absolute and outranks D4's "prose stays".** The two comments that exist solely to _explain_ the misnomer — `csr/src/lib.rs:11` ("CSR has no hydration, but the same marker…") and `fixtures.ts:453` ("CSR has no hydration; `data-hydrated` marks mount-ready") — become unnecessary once the name is honest, and are rewritten without the word rather than retained.      |

## Out of scope — must not be touched

These use "hydration"/"hydrate" _correctly_ and renaming them would introduce
error:

- **Real SSR-hydration history and contrast:** ADR-0002, 0011, 0012, 0032, 0039,
  0040, 0041, 0051, 0056, 0072; `docs/web-style-guide.md`;
  `docs/issue-177-csr-spike-findings.md`; everything under `docs/archive/`.
- **Assertions that hydration is absent:** `web/src/registration/component.rs`
  and `web/src/password_reset/component.rs` ("no SSR-hydration race").
- **The unrelated data-loading sense:** `storage/src/posts.rs` ("Tags are
  hydrated"), `server/tests/storage/mod.rs`.
- **`xtask/src/server_fn_coverage/testdata/**`\*\* — synthetic fixtures
  recording a past run; they are inputs, not a mirror of current test titles.
- **The rest of `xtask/`** beyond `steps/build_csr.rs`, which D4 admits
  explicitly.
- **`Cargo.lock`** (`hydration_context` is an upstream leptos crate).

## Acceptance criteria

Each is checkable by a reviewer without reading the diff.

- **AC1 — zero residue.** Over tracked files only:
  `git ls-files csr end2end xtask/src/steps/build_csr.rs | xargs rg -i hydrat`
  returns **no matches**. (Scoped to `git ls-files` because the untracked
  `end2end/CLAUDE.md` is not gitignored and would otherwise fail this
  spuriously.)
- **AC2 — the marker moved.** `csr/src/lib.rs` sets `data-mounted`;
  `end2end/tests/mount.ts` exists and `end2end/tests/hydration.ts` does not; the
  exported helper is `waitForMount`.
- **AC3 — one TS definition.** Two greps, because one cannot decide this:
  1. `rg -F '"data-mounted"' end2end/` matches **exactly once**, in `mount.ts`
     (that is `MOUNTED_ATTR`'s definition; zero matches is a fail).
  2. `rg -F '[data-mounted]"' end2end/` returns **no matches** — no residual
     hardcoded selector literal such as `"body[data-mounted]"` survives.

  A single unquoted `rg -F 'data-mounted' end2end/` cannot serve: the rename
  deliberately puts the attribute name into prose comments in `layout-shift.ts`,
  `authed-cls.spec.ts`, `timeline-cls.spec.ts` and `auth.spec.ts`, so "matches
  only `mount.ts`" is unsatisfiable by construction. Both greps above are immune
  to those comments, which write the name in backticks, not double quotes.
  `fixtures.ts` reaches the name only through `MOUNTED_ATTR` /
  `MOUNTED_SELECTOR`.

- **AC4 — the span renamed.** The `withTimedAction` call in `mount.ts` passes
  `"wait.mount"`.
- **AC5 — docs renamed in place.** In `docs/observability.md`:
  1. `rg -cF 'wait.hydration'` reports **exactly 1** — D3's rename note is the
     only surviving mention, and the `#155 … (findings, 2026-07-02)` bullet
     itself reads `wait.mount`.
  2. `rg -F 'renamed from \`wait.hydration\` in #251'` matches **once**.
  3. `rg -F 'data-hydrated'` returns **no matches**; the warmup paragraph
     (currently `:456`) says `body[data-mounted]`.

  Note the deliberate asymmetry: D3's note necessarily _quotes_ the old span
  name, so demanding zero `wait.hydration` matches would forbid the very
  sentence D3 requires. The attribute has no such note, so its count is zero.

- **AC6 — title and evidence agree.** `theme.spec.ts`'s test title ends
  `after CSR mount`, and `docs/coverage/server-fns-evidence.json` contains that
  title with **zero** occurrences of the old one (it currently has 4).
- **AC7 — the two languages still agree.** `cargo xtask validate` is green,
  including all four `{sqlite,postgres}×{chromium,firefox}` e2e combos. This is
  the real proof: if the Rust `inline_js` attribute and the TS selector
  disagree, every e2e test times out.

### Ship checklist (not an acceptance criterion)

- **`end2end/CLAUDE.md`, main checkout only.** Update it to document
  `waitForMount` / `body[data-mounted]`, and clear its pre-existing #224 debt
  (`hydrationHeavy*` → `slowBrowser*`). Deliberately excluded from the ACs: it
  is untracked, so it is invisible to the branch, the PR diff, and CI, and no
  automated check can decide it. Per D7 nothing tracks it and nothing will catch
  its regression.

## Blast radius

**Rust (2 files):** `csr/src/lib.rs` — the `inline_js` `setAttribute` and the
comment above `mark_ready`; `xtask/src/steps/build_csr.rs:5` — one doc-comment
clause.

**TypeScript (13 files):** `hydration.ts`→`mount.ts` (rename + `MOUNTED_ATTR`,
`MOUNTED_SELECTOR`, `waitForMount`, `wait.mount`, the `HydrationRecorder`
types); `fixtures.ts` (the three literal reads, `hydratedMs`,
`__jaunderRecordHydration`, `__jaunderHydrationNotified`, `notifyHydration`,
`hydrationObserver`, header comment, `:453` comment); `helpers.ts` (import,
re-export, call, header comment); `layout-shift.ts`; `password_reset.spec.ts`;
`feeds.spec.ts`; `auth.spec.ts`; `atompub.spec.ts`; `media.spec.ts`;
`posts.spec.ts`; `timeline-cls.spec.ts`; `authed-cls.spec.ts`; `theme.spec.ts`
(comment **and** test title).

**Docs (1 tracked + 1 generated + 1 untracked):** `docs/observability.md`;
`docs/coverage/server-fns-evidence.json` (regenerated, needs a fresh
`cargo xtask e2e sqlite chromium` capture first); `end2end/CLAUDE.md` (main
checkout only, ship checklist).

**Risk:** low and loud. The only cross-language coupling is the attribute
string, and a mismatch fails every e2e test rather than degrading silently.
