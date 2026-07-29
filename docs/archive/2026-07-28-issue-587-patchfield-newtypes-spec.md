# Issue #587 — retire the reactive-store primitive carve-outs

- Issue: [#587](https://github.com/jaunder-org/jaunder/issues/587)
- Milestone: 13 — Domain-value type safety (newtypes)
- Date: 2026-07-28

## Problem

`web/src/audiences/` holds the workspace's only `reactive_stores` surface, and
it is the last place in milestone 13 where validated domain values are erased to
primitives:

| Site                                                            | Today    | Should be      |
| --------------------------------------------------------------- | -------- | -------------- |
| `web/src/audiences/api.rs` — `AudienceSummary.audience_id`      | `i64`    | `AudienceId`   |
| `web/src/audiences/api.rs` — `AudienceSummary.name`             | `String` | `AudienceName` |
| `web/src/audiences/component.rs` — `AudienceListData` store key | `i64`    | `AudienceId`   |

Both fields carry doc comments explaining the erasure, and `audience_id` was
deferred as an explicit carve-out by #475. The stated cause: `#[derive(Patch)]`
requires every field to implement `reactive_stores::PatchField`, which the crate
implements only for a closed set of primitives with no blanket impl. `web` owns
neither the trait nor the newtypes, so the orphan rule forbids writing the impl
there; `common` is the only coherent home, and taking a `reactive_stores`
dependency there would couple the target-agnostic crate to the leptos release
train (ADR-0055/0058).

The carve-out has downstream reach beyond the two fields: #503 explicitly
deferred typing `AudienceHeader`'s `name` prop as `AudienceName`, reasoning that
it "would push a fallible parse to the reactive-store edge the carve-out exists
to keep primitive — strictly worse." That reasoning is voided by this change, so
the prop is in scope here.

## Investigation result — the premise was incomplete

`reactive_stores_macro` declares
`#[proc_macro_derive(Patch, attributes(store, patch))]`. The `patch` field
attribute is an escape hatch: given `#[patch(|this, new| …)]`, the derive emits
a direct compare/assign/notify for that field instead of dispatching through
`PatchField`. For a **leaf** field it is the same comparison, assignment,
notification, and `StorePath` as the crate's own primitive impls; the only bound
is `PartialEq`, which every newtype trailer already provides. Upstream ships a
regression test using the idiom verbatim —
`reactive_stores-0.4.3/src/lib.rs:1136`
`patching_only_notifies_changed_field_with_custom_patch`, whose
`CustomTodos.user` field is annotated `#[patch(|this, new| *this = new)]`.

Independently, the keyed-store **key** type never needed `PatchField` at all.
`PatchFieldKeyed<K>` bounds
`K: Clone + Debug + Send + Sync + PartialEq + Eq + Hash + 'static` — all of
which `AudienceId` derives — and the `Vec<T>: PatchFieldKeyed<K>` impl requires
`T: PatchField`, where `T` is `AudienceSummary`, which gets it from its own
`#[derive(Patch)]`.

So the dependency change #587 proposed is not required. Both carve-outs are
retirable with a per-field attribute and no change to `common`.

**This inverts the issue's own Acceptance section**, which asked for
"feature-gated `PatchField` support in `common`." Issue #587's body has been
annotated to point at this spec and the ADR draft, so a later conformance review
does not score the original direction as undelivered.

## Scope confirmation

`PatchField` is demanded by exactly one thing — the `Patch` derive, the sole
generator of `PatchField::patch_field` calls. The set of possibly-affected
fields is therefore closed and enumerable, and the enumeration is reproducible:

- `rg 'derive\(.*Patch'` workspace-wide → **2 structs**, both in
  `web/src/audiences/` (plus two hits in `docs/web-style-guide.md`, which are
  the template criterion 7 targets).
- `rg 'reactive_stores'` across all source → **2 files** (plus one doc-link in
  `client/src/reactive.rs`).
- `rg 'PatchField|patch_field'` → the audiences doc comment; **no other call
  site**.

Three fields. Nothing else in the workspace can be blocked by this problem —
this follows from the enumeration above, not from an absence of documented
carve-outs.

Separately, an audit of primitive-typed fields against the existing newtype set
surfaced one genuine _unrelated_ erasure — the media serve-URL chain — now filed
as **#675**. That audit is background, not evidence for the scope claim above;
it is recorded so the finding isn't lost. #675 is not blocked by `PatchField`
(neither struct derives `Patch`) and is out of scope here.

## Decision

Take the escape hatch. Keep `common` free of `reactive_stores`.

The deciding argument is **reversibility**, not scarcity of call sites. If the
keyed-store population grows enough that the per-field attribute becomes real
friction, the derive-trailer can be added later and the attributes simply
deleted. The converse does not hold: once `common` depends on `reactive_graph`,
every crate above it (`storage`, `host`, `server`) inherits the leptos version
coupling, and unwinding that is far more expensive than adding a derive.

This reverses the reactive-store carve-out as applied by the #475 spec and the
`api.rs` doc comment, so it is recorded as an ADR (draft:
`docs/adr/0078-reactive-store-domain-newtype-fields.md`).

## Acceptance criteria

Each is stated so a conformance review can tell delivered from not.

1. `AudienceSummary.audience_id` is declared `AudienceId` and
   `AudienceSummary.name` is declared `AudienceName`; neither field is
   `i64`/`String`.
2. `AudienceListData`'s store key attribute reads
   `#[store(key: AudienceId = |a| a.audience_id)]`.
3. Each of the two fields carries `#[patch(|this, new| *this = new)]`.
4. The `AudienceSummary` doc comment no longer describes a `PatchField`
   carve-out; it states the `#[patch]` idiom and cites the ADR by its **draft
   path**, `docs/adr/0078-reactive-store-domain-newtype-fields.md`. The
   `drafts/<slug>` segment is load-bearing: `cargo xtask adr promote`
   (`xtask/src/adr.rs`, Pass C) substring-rewrites that stem repo-wide to the
   assigned `NNNN-<slug>` at ship. A bare filename with no `drafts/` segment
   would not be rewritten and would rot into a dangling reference.
5. The edge conversions the carve-out existed to justify are gone: no
   `i64::from(a.audience_id)` or `a.name.into()` in `list_my_audiences`; no
   `AudienceId::from(...)` re-wrap in `AudienceRow`
   (`web/src/audiences/component.rs`) or `audience_checkbox`
   (`web/src/posts/component.rs`). Read-outs that **remain and are correct** —
   the ADR-0063 §5 sanctioned external-boundary flatten — are the
   `value=i64::from(...)` form attributes and the string read-out at the two
   render sites (`audiences/component.rs` `<h3>`, `posts/component.rs` checkbox
   label), since `AudienceName` is not `IntoRender`; this matches the existing
   idiom at `web/src/taglist/component.rs:36`. Prefer the **cheapest** read-out
   the ownership allows: `.to_string()` where the row is borrowed,
   `String::from(...)` (a move via the derived `From<Self> for String`) where it
   is owned — never clone a value that could be moved.
6. `AudienceHeader`'s `name` parameter is typed `AudienceName` (not `String`),
   retiring the deferral recorded in the #503 spec.
7. `docs/web-style-guide.md` §10's example teaches the typed shape: **both** the
   row struct (currently `struct Row { id: i64, name: String }`, line ~297) and
   the key attribute (currently `#[store(key: i64 = |r| r.id)]`, line ~299) are
   updated, and the `#[patch]` attribute appears on the newtype fields. The
   example stays a generic template (`Row`/`Rows`), not the real
   `AudienceSummary` — so it uses placeholder newtype names, and prose states
   that a domain newtype field needs the attribute while the key does not.
8. A numberless ADR draft exists in `docs/adr/drafts/` recording the decision,
   its rejected alternatives, and the `PatchField`-equivalence argument.
9. ADR-0063 §5 and/or ADR-0061 carry a pointer to the new draft, **or** the spec
   records why no pointer is needed. (ADR-0063's §5 text is generic and never
   names reactive stores, so the reactive-store application lives in #475 and
   the `api.rs` comment — this criterion is about deciding deliberately, not
   about editing by default.)
10. `cargo xtask validate --no-e2e` is clean.
11. `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations`
    is clean. The audiences component is wasm-only (ADR-0070), so this is the
    only invocation that compiles it at all — `cargo check -p web` does not.
    Note this is a **fast subset** of criterion 10, not additional coverage: the
    same step runs inside `cargo xtask check`/`validate` via
    `static_checks::specs()`. Listed separately because it is the check to run
    while iterating.
12. The audiences e2e passes **unmodified**: `cargo xtask e2e-local audiences`
    (host-runnable), whose "CRUD + membership toggle re-fetch without list
    remount or flash" test pins the patch-in-place semantics — rename updates
    the `<h3>` while both rows' checklist `<ul>` element handles stay
    `isConnected`. No new e2e is written; changing this test to accommodate the
    code change would destroy its value as evidence.

## Out of scope

- Any change to `common`'s dependencies, features, or `Cargo.toml`.
- A `PatchField` derive-trailer in `macros` (recorded in the ADR as the
  sanctioned escalation if the keyed-store population grows).
- #675's media serve-URL chain.
- #417 (request-aggregate types) — a separate, exploratory milestone-13 issue.

## Risks

- **The `name` closure is guarded; the `audience_id` closure is not.** Inside
  `patch_field_keyed`, rows are matched _by_ the key, so for any matched pair
  `new.audience_id == self.audience_id` holds by construction and that closure
  body is unreachable. The attribute is required for compilation but
  behaviorally inert, and no gate — including the e2e — can catch a wrong
  closure there. Only the `name` attribute is exercised, and criterion 12 is
  what guards it. Stated so a reviewer does not over-trust the e2e.
- **Decode of `list_my_audiences` becomes validating.** `AudienceSummary`
  derives `Deserialize`; with `name: AudienceName` the response decode routes
  through `AudienceName::from_str`, so a stored row whose name fails the
  non-empty-after-trim rule now fails the **whole list** decode instead of
  rendering. The wire format is unchanged (the serde bridge is transparent), and
  `create_audience`/`rename_audience` already validate on the way in, so no
  reachable path writes such a row — but this is a genuinely new failure mode
  and is recorded rather than discovered later.
- **Template drift.** Without criterion 7, the next vertical copying style-guide
  §10 reintroduces the primitive shape. This is why the doc change is an
  acceptance criterion rather than a nicety.
