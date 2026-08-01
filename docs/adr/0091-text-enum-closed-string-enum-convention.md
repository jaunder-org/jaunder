# ADR-0091: `#[text_enum]` owns the closed-string-enum convention

- Status: accepted
- Date: 2026-07-31
- Issue: [#746](https://github.com/jaunder-org/jaunder/issues/746)

Amends ADR-0075 (which stays `accepted`): its Decision's per-enum derive list is
replaced by this attribute, and its Consequences' "explicitly NOT a return to a
bespoke proc-macro" is reversed on the point of form. ADR-0075's amendment
markers point here.

## Context

ADR-0075 replaced the bespoke `StrEnum` derive with plain `strum`, correctly:
`StrEnum` was ~300 lines duplicating a crate already in the tree (ADR-0075:12).
But it left each closed string enum assembling its own ceremony from six sites —
a strum derive list, `#[strum(serialize_all)]`, a
`#[strum(parse_err_ty, parse_err_fn)]` pair, a `parse_error!` invocation below
the type, a serde proxy attribute plus `impl_string_serde_proxy!`, and, if
stored, `impl_text_column_enum!`. Nothing at the type said what it was.

Meanwhile the sqlx bridge existed in **three** hand-maintained copies — the
newtype derives' shared codegen, `common/src/db_enum.rs` for enums, and
`RenderedHtml`'s own impls in `common/src/render.rs` — and the enum copy had
already drifted from the newtype one (no `size_hint`, no
`#[automatically_derived]`, a different `where` bound).

By contrast a **newtype** declares everything in one derive, with storage
expressed as an attribute option. The asymmetry was historical, not principled.

ADR-0075 anticipated a step like this. Its Decision closes: _"Reconsider a small
shared helper only if the completed migration shows the pattern repeated and
grating"_ (ADR-0075:84-85). That clause has already been exercised once — #607
introduced `parse_error!` and `impl_string_serde_proxy!` under it, asserting in
their module doc that they were "**not** a return to `StrEnum`". This is the
second application of the same clause, not a defiance of the ADR.

## Decision

**A closed string enum is declared with one attribute, `#[text_enum(…)]`.**

```rust
#[text_enum(
    sqlx,
    error = InvalidPostFormat,
    message = "post format must be \"markdown\", \"org\", or \"html\"",
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum PostFormat { … }
```

It **injects** `strum`'s `AsRefStr`/`Display`/`EnumString`/`IntoStaticStr` and
the `#[strum(parse_err_ty, parse_err_fn)]` pair, and **generates** the named
parse error, its parse fn, `Serialize`/`Deserialize`, and — with `sqlx` — the
storage bridge. The author keeps the non-uniform derives (`VariantArray`,
`EnumMessage`, `Default`, …) and `serialize_all`.

**An attribute, not a derive**, for one reason: a derive cannot add attributes
to its item. The convention needs `#[strum(parse_err_ty = …, parse_err_fn = …)]`
to name a fn the macro generates; only an attribute can write that line rather
than making the author guess a generated ident.

**It must be the item's first attribute.** An attribute macro receives only the
attributes written _below_ it. A uniform derive written above is invisible to
the duplicate-suppression and collides (`E0119`/`E0592`). Established by
executing a proc-macro reproduction, not by reasoning: a derive above does
**not** suppress injection — expansion is additive — it duplicates it.

**One bridge implementation.** `macros::sqlx_bridge::bridge` takes a
`BridgeSpec` with independent `Type`/`Encode`/`Decode` inner types, and is the
only bridge codegen in the repo. Four callers use it: the three newtype derives,
`#[derive(SqlxBridge)]` (bridge and nothing else, for a type like `RenderedHtml`
whose construction is deliberately not the derive's business), and
`#[text_enum(sqlx)]`.

### Why this is not a return to `StrEnum`

The line is **engine vs. periphery**, and the overlap must be stated rather than
denied.

`StrEnum` generated the wire-token mapping (`as_str`), `Display`, `FromStr`, and
`TryFrom<&str>` (ADR-0074:46-47; ADR-0075:30-38) — the engine, and the reason it
ran to ~300 lines. `#[text_enum]` generates **none** of those, nor
`VariantArray` nor `EnumMessage`. `strum` does all of it; the attribute writes
the derives the author would have written by hand.

`StrEnum` **also** generated the named error struct and an opt-in serde bridge
(ADR-0074:47, 52-56), and `#[text_enum]` generates both. That overlap is real.
The claim is not that nothing returns — it is that the **duplicated engine, the
whole reason `StrEnum` was deleted, stays deleted**.

## Consequences

- **`common/src/db_enum.rs` and `common/src/strum_enum.rs` are deleted**, along
  with `impl_text_column_enum!`, `impl_string_serde_proxy!`, and `parse_error!`.
  The precedent those helpers set — that ADR-0075's reconsider-with-data clause
  permits a shared helper — is carried forward _here_, since the module
  recording it is gone.
- **`impl_string_serde_proxy!`'s public impls go with it**:
  `From<Enum> for String` and `TryFrom<String> for Enum` no longer exist for any
  of these types. A whole-worktree search found no caller. `FromStr` and
  `Display` remain the doors.
- **Eight enums adopt it**, including three (`Channel`, `SubscriptionStatus`,
  `TargetKind`) that gain `Serialize`/`Deserialize` they do not need. That is
  the price of one convention rather than two — a `no_serde` option carried by
  three of eight would be a second convention. **The cost is real**: those
  tokens are a storage encoding, and the absence of serde was a compile-time
  barrier before they could become a wire contract. If that barrier is wanted
  back, the fix is `no_serde`, not an exemption.
- **`strum` becomes a named dependency** of any adopting crate: injected derives
  are emitted as `::strum::…`, so an unqualified import will not do.
- **The generated error deliberately avoids `thiserror`**, hand-writing
  `Display` + `Error` as `NumNewtype`'s error already does, so adopting a crate
  costs no dependency beyond `strum`. Its unit-struct shape is load-bearing:
  `host`'s `validation_from!` registers these by name and constructs them as
  bare unit expressions.
- **`BackupMode` gains a named parse error.** It had none, so `FromStr` returned
  strum's generic `Matching variant not found`; `site_config` JSON is
  operator-editable, so it now names the valid tokens. It also stops declaring
  its token twice — ADR-0075's accepted `serialize_all`/`rename_all` duplication
  is retired for every adopter.
- **`RenderedHtml`'s decode is now generated.** Its rationale — why a sanitizing
  decode was rejected, when to revisit that, and why neither `sanitize` nor
  `from_trusted` is the door — stays on the type, and `SqlxBridge`'s doc carries
  a standing warning that its `Decode` is an inbound door re-establishing no
  invariant.
- **What this does not do:** it does not police anything. The
  `sqlx-newtype-decode` gate reads only the `i64` family and never inspects TEXT
  decodes; extending it is a storage-layer audit tracked as #759 (after #716).
  `PostTitle`'s double decode allocation is #758.
