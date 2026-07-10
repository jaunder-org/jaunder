# Spec — #348: Keyed audience list via a reactive Store

**Issue:** [#348](https://github.com/jaunder-org/jaunder/issues/348) **Status:**
proposed **Decision record:** `docs/adr/0061-web-keyed-list-reactive-store.md`
**Depends on:** #359 (the `Invalidator` primitive, merged) — still drives the
refetch.

## Problem

`AudiencesPage` renders the audience list with
`list.into_iter().map(…).collect()` inside the reactive
`{move || match audiences.get() {…}}` closure. Any list-level refetch (create /
rename / delete `notify`s the `AudienceList` invalidator → the sticky
`audiences` signal changes) re-runs the whole closure, rebuilding every
`AudienceRow` and **remounting every `MemberChecklist`** — which resets to
`member_ids = None` and flashes "Loading members…" on rows the mutation never
touched.

## Approach

Render the audience list from a `reactive_stores::Store` with a keyed Vec field,
fed by `.patch()`, iterated with a keyed `<For>` that is **unconditionally
mounted**. Per the ADR, `patch` reconciles by `audience_id` and notifies only
changed subfields, so unchanged rows are never remounted and a rename updates
the row's name in place. #359's `Invalidator` is unchanged — it still triggers
the refetch; an `Effect` patches the store on resolve. The flat lists
(subscriber roster, member-checklist items) stay plain rendering.

Concretes (settling the details the review flagged):

- **Store container — distinct name.** `#359` already owns `AudienceList` (the
  invalidator scope). The store container is a **separate** type,
  `AudienceListData { #[store(key: i64 = |a| a.audience_id)] audiences: Vec<AudienceSummary> }`
  (`#[derive(Store, Patch, Default)]`); `AudienceSummary` gains
  `#[derive(Store, Patch)]`. The two coexist: `AudienceList` (invalidator)
  triggers the refetch, `AudienceListData` (store) holds the reactive rows.

- **Wiring via `Invalidator::patched` (a factored primitive).** The resource →
  guarded-effect → patch-on-success plumbing is factored into a reusable
  `web::reactive` primitive,
  `Invalidator::patched(fetch, patch) -> Signal<ListState>` — a peer of
  `resource`/`action`. `AudiencesPage` wires the list in two lines:
  `let store = Store::new(AudienceListData::default());` /
  `let state = list.patched(list_my_audiences, move |rows| store.audiences().patch(rows));`.
  The primitive patches only on `Some(Ok(vec))` (never on `None`/`Err` —
  last-good rows retained). The patch is passed as a **closure** so the keyed
  field's _inherent_ in-place `patch` runs; a generic bound would resolve to the
  `Patch` trait's unkeyed, positional patch and remount the list.

- **Load state is a sibling, not a wrapper.** `ListState`
  (`Loading | Empty | Loaded | Error(String)`, a reusable `web::reactive` enum
  returned by `patched`) drives a **sibling** node; the `<For>`
  (`<ul><For each=move || store.audiences() key=|r| r.key() let:row>…</For></ul>`)
  is mounted **unconditionally**, so it is never inside a branch that could tear
  it down — only keyed reconciliation touches rows. The sibling renders
  `Loading` → "Loading…", `Empty` → "No audiences yet.", `Error(e)` → the error,
  `Loaded` → nothing. (`Empty` is set from the row count when patched, so an
  empty store never reads as "still loading".)

- **Row components — minimal reactive surface.** `<For>`'s child receives the
  keyed field `row` (value `AudienceSummary`). `AudienceRow` derives
  `audience_id = row.key()` and renders the name reactively:
  `<h3>{move || row.name().get()}</h3>` — this is what updates a rename in
  place. `AudienceHeader` and `MemberChecklist` keep **owned** props
  (`audience_id: i64`, and for the header an initial `name` snapshot via
  `row.name().get_untracked()`): the header's rename `<input>` stays
  **uncontrolled** (initial value only), so a background refetch that patches
  the store while the user is mid-edit cannot clobber their in-progress text.
  `audience_id` is stable, so the hidden form fields need no reactivity.

## Acceptance criteria

Observable unless marked _(structural)_. For the "no remount" criteria, the
**primary** observable is a stable element handle staying `isConnected` across
the mutation; a "Loading members…" count is **secondary/corroborating** (it is a
sampled negative and can race a remount-then-resolve).

1. **Create — no reflash.** With ≥2 audiences whose member checklists have
   loaded, creating a new audience: the new row appears, and a stable element
   handle on an untouched row stays `isConnected` (primary); corroboratively, no
   already-loaded row shows "Loading members…".

2. **Rename — updates in place, no reflash of A or others.** Renaming audience
   A: A's `<h3>` updates to the new name; a stable element handle grabbed on
   **A's own** member checklist (`<ul class="j-audience-members">` or one of its
   `<li>`) _before_ the rename stays `isConnected` after (proving A itself was
   not remounted); and an unrelated audience B's row handle also stays
   `isConnected`. The rename `<input>` is uncontrolled, so it is not overwritten
   by unrelated refetches.

3. **Delete — only that row goes.** Deleting audience A removes A's row; an
   unrelated audience B's row handle (grabbed before) stays `isConnected` and
   its checklist does not reflash.

4. **Membership toggle unchanged.** The #359 guarantees still hold: an
   add/remove re-fetches only that audience's members (one
   `list_audience_members` round-trip) and never re-fetches the audience list
   (`list_my_audiences` count unmoved). (Covered by the existing e2e
   assertions.)

5. **Loading / empty / error distinct.** Before the first resolve the screen
   shows "Loading…"; a freshly registered author (zero audiences) shows "No
   audiences yet."; a failed list fetch shows the error (alongside any last-good
   rows on a _refetch_ failure — the store retains them). Loading and empty are
   e2e-observable; the **error branch is preserved from the current code and not
   newly e2e-tested** _(structural)_ — there is no fault injection for
   `list_my_audiences` in the harness.

6. **Store used only for the audience list** _(structural)._ The audience list
   is backed by `AudienceListData`'s Vec field keyed on `audience_id`, and
   refetched data is applied with `.patch()` — never `.write()`/`.set()`. The
   subscriber roster and the member-checklist `<li>` list remain plain
   `map`/`collect` rendering.

7. **Refetch still success-gated** _(structural)._ Create/rename/delete refetch
   the list only on success (via the #359 `Invalidator::action`); a failed
   create does not refetch (and so does not patch). (Covered by the existing
   failed-dup-create assertion.)

8. **Decision recorded** _(structural)._ The ADR ships (promoted at merge), and
   the web-style-guide gains a keyed-list section stating the Store +
   keyed-`<For>` + `patch`-not-`write` idiom and the "flat lists stay plain"
   rule.

## Non-goals

- No change to any `#[server]` function, storage API, or the wire shape of
  `AudienceSummary` beyond the added `Store`/`Patch` derives.
- No Store adoption for other verticals or the flat lists (the ADR sets the
  rule; applying it elsewhere is future per-vertical work).
- #372's flat-resource sticky helper is out of scope (the keyed Store subsumes
  sticky for this list; #372 covers the flat cases).

## Risks / to verify early

- **API compile spike — the first implementation task.** Confirm on **both** web
  targets (host + wasm): `AudienceSummary` deriving `Store` + `Patch` alongside
  its existing serde/`Clone`/`PartialEq`/`Eq` derives compiles; the
  `#[store(key: i64 = |a| a.audience_id)]` keyed field; the keyed
  `<For each=move || store.audiences() key=|r| r.key() let:row>` with
  `row.name()` subfield reads type-checks; and the generated
  `AudienceListDataStoreFields` / `AudienceSummaryStoreFields` traits are
  imported where needed. `Patch`'s field bounds (String, i64) are the specific
  unknown.
- **Coverage.** The store _wiring_ — the patch `Effect` and the `<For>` — is
  client-only `#[component]` code (coverage-exempt, exercised by e2e), as with
  #359. The `Store`/`Patch` **derives** on `AudienceSummary`/`AudienceListData`
  are module-scope but macro-generated, so their expansion is attributed to the
  derive macros, not measured against our lines.
