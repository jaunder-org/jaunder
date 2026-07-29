# Plan — Issue #587: retire the reactive-store primitive carve-outs

Spec:
[`docs/superpowers/specs/2026-07-28-issue-587-patchfield-newtypes.md`](../specs/2026-07-28-issue-587-patchfield-newtypes.md)
ADR draft: `docs/adr/drafts/reactive-store-domain-newtype-fields.md` (already
written; gitignored until `promote` at ship)

## Review header

**Goal.** Type `AudienceSummary`'s two fields and the `AudienceListData` store
key as the domain newtypes they hold, using the `Patch` derive's `#[patch]`
escape hatch, and update the style-guide template so later verticals inherit the
typed shape. `common` gains no dependency.

**Scope — in:** `web/src/audiences/{api,component}.rs`,
`web/src/posts/component.rs`, `docs/web-style-guide.md` §10, one ADR
cross-reference decision. **Scope — out:** any `common` dependency/feature
change; a `PatchField` derive-trailer in `macros`; #675 (media serve-URL); #417.
No new e2e — criterion 12 requires the existing audiences spec to pass
**unmodified**.

**Tasks.**

1. Type the store row + key, drop the carve-out conversions, rewrite the doc
   comment.
2. Update style-guide §10's template; decide the ADR-0063/0061 cross-reference.
3. Verify the behavioral guard: audiences e2e unmodified + the full local gate.

**Key risks / decisions.**

- The `audience_id` `#[patch]` closure is **behaviorally inert** (rows are
  matched by that key inside `patch_field_keyed`, so its comparison is false by
  construction). It is compile-required only. No gate covers it — do not let a
  green e2e imply it.
- `AudienceName` is not `IntoRender`. Two render sites legitimately keep
  `.to_string()`; per spec criterion 5 these are correct, not leftovers to
  remove.
- Decode of `list_my_audiences` becomes validating (spec Risks). No reachable
  path writes an invalid name; recorded, not mitigated.
- The ADR is referenced by its **`docs/adr/drafts/<slug>.md` path**, never by a
  number. `cargo xtask adr promote` rewrites path-form references repo-wide at
  ship (`xtask/src/adr.rs`, Pass C), so citing the draft path is the designed
  workflow.

**For agentic workers.** Execute with **`jaunder-iterate`**; delegate an
individual task via **`jaunder-dispatch`** if useful. Tick checkboxes in real
time.

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Run `cargo xtask check` before each commit (the pre-commit hook runs the full
  gate; running it first keeps the commit clean) — see **`jaunder-commit`**.
  `check` auto-fixes formatting, so re-check `git status --porcelain` after it
  goes green.
- **`cargo check -p web` never compiles the audiences component** — it is
  wasm-only (ADR-0070), so a plain host check proves little here. Run the
  standalone wasm-clippy command below while iterating: it is a **fast subset**
  of what `cargo xtask check` already runs (`static_checks::specs()` includes
  `wasm-clippy` with a byte-identical arg vector; `check` calls it at
  `xtask/src/lib.rs:292`, `validate` at `:324`). It is not filling a gap in the
  gate — it just fails in seconds instead of minutes. Criterion 11 is subsumed
  by criterion 10.
- Do not edit `end2end/tests/audiences.spec.ts`. It is the evidence.
- Serialize edit → gate → commit; no edits while a gated commit is running (the
  Nix build reads the working tree mid-commit).

**Commands used below**

```bash
# host build of web (server-gated code)
cargo check -p web --features server --all-targets
# fast subset of the gate's wasm-clippy step — the only thing that compiles
# wasm-only component files (also run inside `check`/`validate`)
cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- \
  -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
# full local gate, no e2e
cargo xtask validate --no-e2e
# the audiences behavioral guard (host runner, ~3 min)
cargo xtask e2e-local audiences
```

Run each via `devtool run -- <cmd>` (worktree-aware, honest exit, parked
output).

## Current state

A verified prototype of Task 1's _code_ already sits uncommitted in the worktree
(`web/src/audiences/api.rs`, `web/src/audiences/component.rs`,
`web/src/posts/component.rs`); both the host check and wasm-clippy pass on it.
The `AudienceSummary` doc comment still carries the **old carve-out text**,
which contradicts the fields directly below it — Task 1 is not done until that
is rewritten. No Rust test constructs `AudienceSummary`, so no
`common::test_support::parse_*` sweep is needed.

---

## Task 1 — Type the store row and retire the carve-out

Spec criteria: 1, 2, 3, 4, 5, 6.

**Files**

- `web/src/audiences/api.rs` — `AudienceSummary` struct + doc comment;
  `list_my_audiences`.
- `web/src/audiences/component.rs` — `AudienceListData` key attribute;
  `AudienceRow`; `AudienceHeader` signature.
- `web/src/posts/component.rs` — `audience_checkbox`; the `common::ids` import.

**Interfaces**

```rust
// web/src/audiences/api.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct AudienceSummary {
    #[patch(|this, new| *this = new)]
    pub audience_id: AudienceId,
    #[patch(|this, new| *this = new)]
    pub name: AudienceName,
}

// web/src/audiences/component.rs
#[store(key: AudienceId = |a| a.audience_id)]
audiences: Vec<super::api::AudienceSummary>,

fn AudienceHeader(audience_id: AudienceId, name: AudienceName) -> impl IntoView
```

**Steps**

The code edits below are **already present and compiling** in the working tree
(see Current state). Confirm each rather than re-deriving it; the only
outstanding work is the doc comment.

- [x] `api.rs`: fields typed `AudienceId` / `AudienceName`, each with the
      attribute.
- [x] `api.rs`: `list_my_audiences` maps
      `audience_id: a.audience_id, name: a.name` — no `i64::from`, no `.into()`,
      and the carve-out comment on `name` is gone.
- [x] `component.rs`: key attribute → `AudienceId`.
- [x] `component.rs`: `AudienceRow` reads `row.audience_id().get_untracked()`
      directly (the `AudienceId::from` re-wrap **and its explanatory comment**
      are gone).
- [x] `component.rs`: `<h3>` renders `row.name().get().to_string()`.
- [x] `component.rs`: `AudienceHeader`'s `name` param typed `AudienceName`;
      `ValidatedField::<AudienceName>::prefilled(&name)` compiles via `Deref`.
- [x] `posts/component.rs`: `let id = audience.audience_id;`; label renders
      `audience.name.to_string()`; `AudienceId` dropped from the `common::ids`
      import.
- [x] `posts/component.rs`: `audience_checkbox` keeps its **by-value**
      `AudienceSummary` parameter. _(Corrected after the standards review: an
      earlier revision switched it to `&AudienceSummary`, on the reasoning that
      `needless_pass_by_value` fires once the body stops consuming the struct.
      That was true only because the label read the name with `.to_string()` — a
      clone. Using `String::from(audience.name)` instead moves the name out of
      the owned row, so the struct **is** consumed, the lint does not fire, the
      clone is gone, and the signature never had to change. Verified clean
      against the wasm-clippy command.)_
- [x] `api.rs`: replace the `AudienceSummary` doc comment — **the one thing
      left**. Delete the carve-out narration (it currently contradicts the
      fields two lines below it). State instead: this is a keyed-store row; a
      domain-newtype **leaf** field carries `#[patch(|this, new| *this = new)]`
      because `reactive_stores::PatchField` has no blanket impl and the orphan
      rule bars implementing it here; cite the path
      `docs/adr/drafts/reactive-store-domain-newtype-fields.md`. Add one line
      noting the `audience_id` attribute is compile-required but inert (rows are
      matched by that key, so its comparison never fires).

**Verify**

- [x] `cargo check -p web --features server --all-targets` → PASS. _(Compiles
      the `#[server]` half only; it carries no `-D warnings`, so it proves
      compilation, not lint-cleanliness.)_
- [x] wasm-clippy command → PASS. _(The lint-enforcing one, and the only thing
      that compiles `component.rs` at all.)_ — discharged by
      `cargo xtask check --no-test`, which runs the identical step.
- [x] `rg -n 'carve-out|stays a bare' web/src/` → no hits. _(Deliberately
      **not** matching `PatchField`: the replacement comment names the trait to
      explain why the attribute exists. Matching it would fail correct work.)_
- [x] `rg -n 'drafts/reactive-store-domain-newtype-fields' web/src/audiences/api.rs`
      → one hit, proving criterion 4's citation landed in the path form
      `promote` rewrites.
- [x] `cargo xtask check` → green; then `git status --porcelain` for fmt fixups.

**Commit** —
`refactor(web): type the audience store row as AudienceId/AudienceName (#587)`

---

## Task 2 — Update the style-guide template and settle the ADR cross-reference

Spec criteria: 7, 9. (Criterion 8 — the ADR draft — is already satisfied.)

**Files**

- `docs/web-style-guide.md` §10 (example at ~lines 296–299, plus surrounding
  prose).
- Possibly `docs/adr/0063-domain-value-newtype-convention.md` and/or
  `docs/adr/0061-web-keyed-list-reactive-store.md`.

**Steps**

- [x] §10 example: update **all three** parts — the row struct (currently
      `struct Row { id: i64, name: String }`, line 297), the key attribute
      (currently `#[store(key: i64 = |r| r.id)]`, line 299), **and** the
      `#[patch(|this, new|     *this = new)]` attribute on the newtype leaf
      field, without which the example would not compile. Keep it a **generic
      template** (`Row`/`Rows` with placeholder newtype names), not the real
      `AudienceSummary`.
- [x] §10 prose: add a short bullet — a domain-newtype **leaf** field needs
      `#[patch(|this, new| *this = new)]`; the **key** type does not; cite the
      draft ADR path. Mention that a newtype is not `IntoRender`, so view sites
      stringify (the existing idiom, `web/src/taglist/component.rs:36`).
- [x] **Decide** the cross-reference and record the outcome in the commit
      message. **Decided: pointer added to ADR-0061 only; ADR-0063 left
      untouched.** ADR-0061 owns the keyed-store idiom, so a reader asking "what
      may a row hold?" lands there — it gets a header `Note:` in the same style
      as ADR-0058's `#334` clarification and ADR-0069's `re-scoped by ADR-0070`.
      ADR-0063 §5's text is generic (it names `atom_syndication`, `rss`,
      `serde_json::Value`) and never mentions reactive stores, so there is
      nothing there to correct — and adding a reactive-store exception note
      would actively mislead, implying store rows are special when the whole
      point of this change is that they are **not**.
- [x] If ADR text is edited, reference the draft by **path**, never a number.

**Verify**

- [x] `rg -n 'id: i64|key: i64' docs/web-style-guide.md` → no hits.
- [x] `rg -n '#\[patch\(' docs/web-style-guide.md` → at least one hit. _(The
      struct and key checks above cannot detect a missing `#[patch]`, which is
      the half of criterion 7 that makes the template actually compile.)_
- [x] `prettier -w docs/web-style-guide.md` (and any ADR touched) before staging
      — the pre-commit prettier otherwise restages prose under you.
- [x] `cargo xtask check` → green (`--no-test`; the full gate runs in the commit
      hook).

**Commit** —
`docs(web): teach the typed keyed-store row shape in style-guide §10 (#587)`

---

## Task 3 — Verify the behavioral guard

Spec criteria: 10, 11, 12. No commit unless a fix is needed.

**Steps**

- [x] `git diff wt-base-issue-587 -- end2end/` → **empty**. Note the single-ref
      form: it compares the **working tree** against the base, so an uncommitted
      or staged-but-uncommitted edit is caught. `..HEAD` would compare commits
      only and let a dirty edit through — and this file is the evidence for
      criterion 12, so this is the one check that must be airtight. If it shows
      anything, stop and reassess.
- [x] `cargo xtask e2e-local audiences` → PASS. The load-bearing assertions: the
      renamed row's `<h3>` shows the new name, and both rows' checklist `<ul>`
      element handles remain `isConnected` across create / rename / delete.
      **7/7 passed in 20.6s.** (One test deliberately induces a 500 to assert
      the error node; the logged `tower_http` 500 is that, not a failure.)
- [x] `cargo xtask validate --no-e2e` → green. _(This already includes
      wasm-clippy, so it discharges criteria 10 and 11 together.)_

**On failure.** A rename that no longer updates in place, or a detached
checklist handle, means the `name` `#[patch]` closure is wrong — not that the
test needs adjusting. Fix the closure.

---

## Self-review

- Every spec criterion maps to a task: 1–6 → Task 1; 7, 9 → Task 2; 8 → already
  done (ADR draft); 10–12 → Task 3. Criterion 11 is a fast subset of 10,
  discharged by either.
- Task 1 is 8 edits over 3 files in one commit. Splitting is genuinely blocked —
  the type change is compiler-coupled across every call site, so no intermediate
  state builds. Seven of the eight are already prototyped and marked `[x]`,
  leaving the doc comment as the sole outstanding edit.
- No task smuggles work the spec didn't authorize; #675 and the derive-trailer
  are explicitly out of scope and appear nowhere below the header.
- Each task is independently verifiable by a named command with an expected
  outcome.
- The one judgement call (Task 2's cross-reference decision) is scoped, has a
  stated default, and requires its reasoning in the commit message rather than
  being left implicit.
