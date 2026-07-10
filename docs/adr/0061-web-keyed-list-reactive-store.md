# ADR-0061: Web keyed lists render via a reactive Store, patch-fed

- Status: proposed
- Date: 2026-07-10
- Issue: [#348](https://github.com/jaunder-org/jaunder/issues/348)

## Context

The account-area lists are driven by a server `Resource` whose resolved `Vec` is
copied into a sticky signal and rendered with
`list.into_iter().map(…).collect()` inside a reactive closure (web-style-guide
§9; the pattern #359's `Invalidator` feeds). That closure re-runs on **every**
refetch, so a create/rename/delete rebuilds **every** row from scratch —
remounting each row's nested stateful children. On the audiences screen this
remounts every `MemberChecklist`, which resets to its initial `None` and flashes
"Loading members…" on rows the mutation never touched (#348).

The Leptos primitive for this is a keyed `<For>`: unchanged keys keep their DOM
and reactive subtree, so only added/removed/changed rows are touched. But a
keyed `<For>` alone is a trap for lists whose **rows mutate in place** (a rename
keeps the same `audience_id`): `<For>` leaves an unchanged-key row's child
closure un-run, so a row built from a captured `AudienceSummary` snapshot would
show a **stale name**. Making the row's fields individually reactive — so a
rename updates the name without rebuilding the row — is exactly what
`reactive_stores::Store` provides, and it is the idiomatic Leptos answer for a
keyed list of editable items.

This is the first `<For>` and the first `Store` in the `web` crate. The web
verticals are being converted to canonical co-located Leptos one at a time
(ADR-0056), so the audiences conversion is the **reference** every later
vertical copies — the pattern chosen here is load-bearing beyond this one
screen.

## Decision

A `web` list whose rows carry **per-row identity that can mutate** or **nested
component state to preserve** across a refetch is rendered from a
`reactive_stores::Store` (a new direct dependency of `web`):

- The row type derives `Store` + `Patch`; a container struct holds the
  collection as a keyed field (`#[store(key: <Id> = |row| row.<id>)]`). The
  container is named distinctly from any co-existing invalidator scope (e.g.
  audiences: the `AudienceList` invalidator from #359 stays; the store container
  is a separate `AudienceListData`).
- The server `Resource` still fetches; #359's `Invalidator` still triggers the
  refetch. The resource → state-machine → patch-on-success plumbing is packaged
  as **`Invalidator::patched(fetch, patch) -> Signal<ListState>`** (a
  `web::reactive` peer of `resource`/`action`): it patches the store only on
  `Some(Ok(vec))` — **never** on `None` (first load) or `Some(Err)`, so
  last-good rows are retained — and returns the list's load `ListState`. The
  `patch` step is passed as a **closure** so the caller's concrete keyed field
  runs its **in-place, keyed** `patch`: the keyed `patch` is an _inherent_
  method on the store field, whereas a generic `field.patch(vec)` would resolve
  to the `Patch` _trait_'s **unkeyed, positional** patch and lose row identity.
  This is load-bearing: keyed `patch` notifies only the subfields whose value
  changed (a rename notifies just that row's `name`), so unchanged rows are
  never remounted; a plain `write`/`set` (or the unkeyed patch) notifies every
  keyed child and remounts the whole list (reactive_stores `keyed.rs` test
  `patching_keyed_field_only_notifies_changed_keys`).
- The list is iterated with a keyed
  `<For each=move || store.<field>() key=|r| r.key()>`, **mounted
  unconditionally** (not inside a reactive loading/error branch that could tear
  it down); load/empty/error is a **sibling** node driven by a small state
  signal. Each row reads its mutable fields as reactive subfields
  (`row.name()`), so a rename updates in place with no remount; fields bound to
  editable inputs stay uncontrolled (initial value only) so a background refetch
  cannot clobber in-progress edits.

`patch`-on-resolve provides the sticky-retention behavior (web-style-guide §9)
for such a list: it retains prior rows and never blanks to a _loading_
placeholder, subsuming a separate sticky signal for the keyed collection. (A
hard fetch _error_ still surfaces — a refetch failure shows the error alongside
the retained rows; a first-load failure shows it alone.)

**Store is not used for flat lists.** A read-only or stateless list — no per-row
identity that mutates, no nested state to keep (e.g. the audiences screen's
subscriber roster and the member-checklist items) — stays plain `map`/`collect`
rendering. Store there is ceremony for no reactive benefit, and the reference
conversion is a better guide when it shows Store used _where it is the right
tool_, not applied indiscriminately.

## Consequences

- `web` gains a direct `reactive_stores` dependency and its first
  `Store`/`<For>`. The `<Foo>StoreFields` trait the macro generates must be
  `use`d wherever a row's subfields are accessed across a module boundary.
- The keyed-list idiom (Store + `Patch` + keyed `<For>`, patch-not-write,
  flat-lists-stay-plain, wired via `Invalidator::patched` + `ListState`) becomes
  a web-style-guide section and the template later verticals follow — they get
  the two-line wiring, not a hand-rolled effect.
- Loading / empty / error is no longer carried by the list value itself (the
  store holds only rows): `ListState` — a reusable `web::reactive` enum returned
  by `Invalidator::patched` — distinguishes first-load, empty, and error in a
  sibling node next to the `<For>`.
- #372 (a sticky-retention helper) remains scoped to genuinely flat resources
  (subscriber roster, `home.rs`); it does not cover — and is not needed for — a
  keyed Store list.
- Ties us to `reactive_stores`' keyed-field semantics; the `patch`-not-`write`
  rule is a correctness constraint a reviewer must know, so it is stated in the
  style guide and guarded by the e2e no-remount assertions.
