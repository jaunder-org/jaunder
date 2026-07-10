# Plan — #348: Keyed audience list via a reactive Store

**Spec:** `docs/superpowers/specs/2026-07-10-issue-348-keyed-audience-store.md`
**ADR draft:** `docs/adr/0061-web-keyed-list-reactive-store.md` (already
written; promoted at ship) **Branch/worktree:**
`worktree-issue-348-keyed-audience-list`

Separable concerns: none new (the sticky-helper follow-up is already filed as
#372; the web-style-guide update is in-scope here, AC8). No issue-filing task.

Commit grouping: Tasks 1–2 land as one commit
(`web(audiences): render the audience list from a keyed reactive Store (#348)`);
Task 3 the e2e (folded or its own commit); Task 4 the style-guide; ADR promoted
in its own commit at ship.

## Task 1 — Compile spike: de-risk the Store/Patch/For API on both targets

Before touching the real rendering, prove the API compiles. The dep, derives,
and container struct added here are **kept** (Task 2 builds on them); only the
scratch component is thrown away.

- Add `reactive_stores` as a direct dep of `web` (pin to 0.4.3, already in
  `Cargo.lock`).
- `#[derive(Store, Patch)]` on `AudienceSummary`; define
  `#[derive(Store, Patch, Default)] struct AudienceListData { #[store(key: i64 = |a| a.audience_id)] audiences: Vec<AudienceSummary> }`.
- In a **throwaway scratch** `#[component]`, write the keyed
  `<For each=move || store.audiences() key=|r| r.key() let:row>` reading
  `{move || row.name().get()}`, importing the generated
  `AudienceListDataStoreFields` / `AudienceSummaryStoreFields`.

**Verify:** host clippy **and** wasm clippy both pass with the derives + scratch
`<For>` in place; resolve any `Patch` bound / trait-import surprises here.
**Done when** both target checks are green. Then delete only the scratch
component (keep the dep, derives, and `AudienceListData`).

_Spec: Risks (API compile spike)._

## Task 2 — Convert `AudiencesPage` rendering to the keyed Store (one compile unit)

The three steps below share one compile checkpoint — the crate is intentionally
non-compiling **between** them (removing the old signal in (a) orphans the old
closure until (b); the new `<For>` in (b) needs the new row signature from (c)).
Do not treat (a)/(b)/(c) as independently-shippable; the single compile gate is
at the end.

> **Mid-execution deviation (per user steer):** the resource → state → patch
> plumbing in (a), and the `ListState` enum, were factored out of
> `AudiencesPage` into reusable `web::reactive` primitives —
> `Invalidator::patched(fetch, patch) -> Signal<ListState>` (a peer of
> `resource`/`action`) and a shared `ListState`. So (a) collapses to
> `let store = Store::new(AudienceListData::default());` +
> `let state = list.patched(list_my_audiences, move |rows| store.audiences().patch(rows));`,
> and the reference conversion ships the two-line wiring, not a hand-rolled
> effect. The subscriber roster was likewise simplified to a one-line
> `Signal::derive` (it is a constant-source resource — no sticky machinery
> needed). Reflected in the ADR, spec, and style-guide §10.

- **(a) Data wiring.** Replace
  `audiences: RwSignal<Option<Result<Vec, String>>>` with a
  `Store::new(AudienceListData::default())` and a `RwSignal<ListState>`.
  `ListState` is a plain **derive-only** enum, defined component-local, no
  hand-written methods (matching is inline in the view), so it stays within the
  `#[component]` coverage exemption (ADR-0050):
  `enum ListState { Loading, Ready, Error(String) }`. Keep
  `list.resource(list_my_audiences)` + the `AudienceList` invalidator untouched.
  One guarded `Effect` (the only store writer): `None` → no-op (stays
  `Loading`); `Some(Ok(vec))` → `store.audiences().patch(vec)` then
  `state.set(Ready)`; `Some(Err(e))` → `state.set(Error(e.to_string()))`, store
  untouched.
- **(b) Rendering.** Remove the old `{move || match audiences.get() {…}}`
  map/collect closure. Render the list **unconditionally**:
  `<ul class="j-audience-list"><For each=move || store.audiences() key=|r| r.key() let:row><AudienceRow row=row subscribers=… /></For></ul>`.
  A **sibling** reactive node renders status from `state` + store-emptiness:
  `Loading` → `<p class="j-loading">"Loading…"`; `Error(e)` →
  `<p class="error">{e}`; `Ready` → `"No audiences yet."` only when the store is
  empty, else nothing.
- **(c) Row components.** `AudienceRow(row: <keyed field>, subscribers)`:
  `let audience_id = row.key();`
  `<h3 class="j-audience-name">{move || row.name().get()}</h3>`; pass owned
  `audience_id` + initial `name = row.name().get_untracked()` to
  `AudienceHeader`, and owned `audience_id` + `subscribers` to
  `MemberChecklist`. `AudienceHeader(audience_id, name)`: rename
  `<input value=name>` stays uncontrolled **exactly as today** — the only change
  is `name` now arrives as an untracked snapshot, not from a tracked signal;
  actions still fire the `AudienceList` invalidator (unchanged from #359).
  `MemberChecklist` unchanged (owned `audience_id` + `subscribers`).

**Verify (single checkpoint):** host clippy **and** wasm clippy pass; the
guarded `Effect` matches the spec's three arms (patch only on `Ok`, never
`write`/`set`); only `<h3>` reads reactively; `AudienceHeader`/`MemberChecklist`
take owned values. **Done when** the crate compiles on both targets and the
wiring reads as specced.

_Spec: AC1, AC2, AC3, AC5, AC6, AC7._

## Task 3 — Extend the audiences e2e for the no-remount guarantee

In `end2end/tests/audiences.spec.ts`, add (keeping all #359 assertions):

- After **create** (with ≥2 loaded checklists): a stable handle on an untouched
  row stays `isConnected` (primary); corroboratively no "Loading members…"
  appears.
- **Rename** A → new name: grab a handle on A's own
  `<ul class="j-audience-members">` (or an `<li>`) **before** the rename; assert
  `isConnected` after and A's `<h3>` shows the new name; assert an unrelated
  row's handle also stays `isConnected`.
- After **delete**: an unrelated row's pre-grabbed handle stays `isConnected`,
  no reflash.

**Verify:** `npx playwright test audiences` passes locally (or via the gate's
nix-e2e); old assertions still pass. **Done when** the new assertions pass.

_Spec: AC1, AC2, AC3, AC4, AC7._

## Task 4 — web-style-guide keyed-list section

Add a section to `docs/web-style-guide.md`: keyed lists with per-row
identity/nested state use `reactive_stores::Store` + `Patch` + keyed `<For>`,
fed by `.patch()` (state the **patch-not-write** rule and why), mounted
unconditionally with a sibling status node; flat/read-only lists stay plain
`map`/`collect`. Cross-reference the ADR and #359.

**Verify:** prettier-clean; the patch-not-write rule and flat-list exception are
stated. **Done when** the section is written and formatted.

_Spec: AC8._

## Task 5 — Gate green + clean history + structural checks

- **AC6 positive check:** confirm `<For>` / `.patch()` / the store appear
  **only** for the audience list — the subscriber roster and the
  member-checklist `<li>` list still use plain `.map(...).collect()` (grep
  `mod.rs`).
- `cargo xtask validate` green (run via `devtool run --`).
- Fold spike/review churn into the logical commits (no scratch/churn commits);
  prettier any edited Markdown before staging.
- ADR promotion (`cargo xtask adr promote`) happens at ship (jaunder-ship), its
  own commit.

**Verify:** validate exits 0; the AC6 grep confirms flat lists untouched. **Done
when** the gate is green on the final tree.
