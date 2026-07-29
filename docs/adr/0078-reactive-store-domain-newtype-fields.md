# ADR-0078: Domain newtypes in reactive-store rows via the `Patch` derive's `#[patch]` escape hatch

- Status: accepted
- Date: 2026-07-28
- Issue: [#587](https://github.com/jaunder-org/jaunder/issues/587)

## Context

[ADR-0061](0061-web-keyed-list-reactive-store.md) made a `reactive_stores` keyed
`Store` the idiom for a list whose rows carry mutable per-row identity or nested
child state. `#[derive(Patch)]` is what makes it pay off: it generates a
field-by-field reconciler that assigns and notifies only the leaves that
actually changed, so a rename updates one row's `<h3>` in place instead of
remounting the list (the #348 bug — an unrelated create/rename/delete reflashing
every row's `MemberChecklist`).

That derive requires **every field** to implement `reactive_stores::PatchField`,
which the crate implements only for a closed enumeration of primitives. There is
no blanket `impl<T: PartialEq>` — it would conflict with the crate's own struct
impls — so a domain newtype cannot be a store-row field by the default path.

The orphan rule then bites. `impl PatchField for AudienceName` is legal only in
the crate defining the trait or the type: not `web` (owns neither), not `client`
([ADR-0069](0069-client-crate-wasm-only-home.md) forbids domain types there and
it owns neither), not a new leaf crate (same). `common` is the sole coherent
home — and a `reactive_stores` dependency there would couple the deliberately
target-agnostic crate, and everything above it (`storage`, `host`, `server`), to
the leptos release train, inverting the target-scoped layering
[ADR-0058](0058-host-crate-layering.md) exists to maintain.

The #475 spec and the `AudienceSummary` doc comment therefore recorded the
resulting erasure as an application of
[ADR-0063](0063-domain-value-newtype-convention.md) §5's external-non-owned-type
carve-out: the row held `audience_id: i64` and `name: String` and converted at
its edges. (ADR-0063's own §5 text is generic — it names `atom_syndication`,
`rss`, `serde_json::Value` — and never mentions reactive stores; the
reactive-store _application_ is what #475 and the doc comment added.)

**That application rested on an incomplete reading of the dependency.** The
derive is declared `#[proc_macro_derive(Patch, attributes(store, patch))]`, and
the `patch` field attribute bypasses the trait entirely.

## Decision

A domain newtype used as a **leaf** field in a `reactive_stores` row is declared
as itself and given the derive's per-field escape hatch:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct AudienceSummary {
    #[patch(|this, new| *this = new)]
    pub audience_id: AudienceId,
    #[patch(|this, new| *this = new)]
    pub name: AudienceName,
}
```

**`common` does not take a `reactive_stores` dependency.** The ADR-0058 layering
stands unamended.

**Why this is safe, not clever.** For a leaf field the derive emits

```rust
if new.name != self.name {
    _ = { let (this, new) = (&mut self.name, new.name); *this = new };
    notify(&new_path);
}
```

against the crate's own primitive impl

```rust
if new != *self { *self = new; notify(path); }
```

— identical comparison, assignment, notification, and `StorePath` (both branches
sit under the same `new_path.push(0)` / `replace_last(idx + 1)` sequence). The
attribute supplies the leaf behavior inline instead of routing it through a
trait the orphan rule will not let us implement. The only bound either path
needs is `PartialEq`, which every ADR-0063 newtype trailer provides. The idiom
is a tested part of the crate's public surface, not an incidental one:
`reactive_stores-0.4.3/src/lib.rs:1136`
(`patching_only_notifies_changed_field_with_custom_patch`) annotates a field
`#[patch(|this, new| *this = new)]` verbatim.

**"Leaf" is load-bearing.** The equivalence holds for a field whose value is a
single compared-and-replaced unit. For a field wrapping a nested `Store` struct,
the `#[patch]` branch notifies the whole field path where `PatchField` would
descend and notify granular subpaths — the equivalence fails and the attribute
is the wrong tool.

**Keyed store keys need no attribute.** `PatchFieldKeyed<K>` bounds
`K: Clone + Debug + Send + Sync + PartialEq + Eq + Hash + 'static`, all of which
`IdNewtype` already carries, and the `Vec<T>` impl's `T: PatchField` is
satisfied by the row struct's own `Patch` derive. So
`#[store(key: AudienceId = |a| a.audience_id)]` works directly. The #475
carve-out had no reason beyond the `PatchField` gap and is retired with it.

**A key field's own `#[patch]` closure is inert.** The key field still needs the
attribute to compile, but inside `patch_field_keyed` rows are matched _by_ that
key, so for any matched pair the comparison is false by construction and the
body is unreachable. Do not treat the e2e (or any gate) as covering it; only
non-key leaf attributes are exercised.

This **supersedes the #475 spec's application** of ADR-0063 §5 to reactive-store
rows, and the carve-out language in the `AudienceSummary` doc comment. It does
not touch ADR-0063 §5 itself, whose general external-type carve-out
(`atom_syndication`, `rss`, `serde_json::Value`) stands. A `reactive_stores` row
is no longer an instance of that carve-out; the newtype is held, and only
genuinely external consumers — the `atom_syndication` boundary, form `value=`
attributes, and view sites where a newtype is not `IntoRender` — read the inner
value out.

**Rejected: emitting `PatchField` from the newtype derives.** `common` would
take an optional, feature-gated `reactive_stores` dependency and
`StrNewtype`/`IdNewtype` would emit the impl behind a `#[cfg(feature = …)]`,
mirroring the sqlx bridge ([ADR-0071](0071-sqlx-string-newtype-bridge.md))
exactly. It is more ergonomic — no per-field attribute, ever. It is rejected on
**reversibility**: the escape hatch forecloses nothing (if the population grows,
add the derive later and delete the attributes as redundant), whereas a leptos
dependency in `common` propagates to every crate above it and is expensive to
unwind. The sqlx precedent does not transfer: `storage` is host-only, so that
feature is never enabled in the wasm graph, while `web` is in **both** the wasm
and host builds — feature unification would put `reactive_graph` in the server
binary's `common`.

Also rejected, as in #587: a `web`-local wrapper type (every field becomes
`Wrapper<T>`), and upstreaming a leaf-impl macro to leptos-rs (timeline outside
our control).

## Consequences

- **`common` stays leptos-free.** ADR-0055/0058's target-scoped layering is
  preserved rather than amended, and `storage`/`host`/`server` take no new
  coupling.
- **Commits us to a per-field annotation** for every future domain newtype leaf
  in a store row. `docs/web-style-guide.md` §10 is updated to show the typed
  shape with the attribute, so verticals copying the template inherit it —
  ADR-0061 makes that template the path later verticals follow, and its example
  teaches `struct Row { id: i64, name: String }` today.
- **Unblocks a deferred decision:** #503 declined to type `AudienceHeader`'s
  `name` prop as `AudienceName` specifically because the carve-out kept the
  store edge primitive. That reasoning no longer applies.
- **Sanctioned escalation.** If the keyed-store population grows enough that the
  attribute is real friction, revisit the rejected derive-trailer. That change
  is purely additive: the `#[patch]` attributes become redundant and are
  deleted.
- **The behavioral guard is the existing e2e**, not a new test.
  `end2end/tests/audiences.spec.ts` asserts in-place rename and no-remount by
  holding element handles across refetches — precisely what a wrong _non-key_
  `#[patch]` closure would break. It must keep passing unmodified; changing it
  to accommodate a code change would destroy its value as evidence. Note the
  inertness caveat above: it does not cover the key field's attribute.
- **Rules out** treating a `reactive_stores` row as an ADR-0063 external-type
  flatten. Store rows hold domain newtypes.
- **Ties us to** the `#[patch]` attribute surviving `reactive_stores` upgrades.
  `Cargo.toml` declares `reactive_stores = { version = "0.4.3" }` — a caret
  range, so a `0.4.x` bump is picked up without review. The attribute is covered
  by an upstream test, which makes silent removal unlikely; a silent _semantic_
  change would surface in the audiences e2e. If that risk is ever judged too
  loose, the response is to pin exactly, not to abandon the idiom. The fallback
  if upstream drops the attribute is the derive-trailer above.
