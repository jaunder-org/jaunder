# ADR-0112: Role-tagged site URLs

- Status: accepted
- Date: 2026-08-11
- Issue: [#875](https://github.com/jaunder-org/jaunder/issues/875)

## Context

`AbsoluteUrl` was one type for every URL the system handles. #688 threaded it
through `WebSubClient::send_publish`, replacing two adjacent `&str` parameters
with two adjacent `&AbsoluteUrl` parameters. That delivered
[ADR-0063](0063-domain-value-newtype-convention.md) §5 adoption — a validated
value stopped being flattened and re-derived across the trait hop — but it
delivered **no transposition safety**, and #688's issue body wrongly claimed it
would. Both parameters had the same type, so

```rust
self.websub.send_publish(feed, hub)   // arguments swapped
```

still compiled and still pinged the wrong endpoint.

A survey found this was not one site but four live hazards, everywhere
same-typed URLs sat adjacent:

| Site                          | Shape                                                |
| ----------------------------- | ---------------------------------------------------- |
| `server/src/websub/mod.rs`    | `send_publish(hub_url, feed_url)` — adjacent params  |
| `common/src/atompub/rsd.rs`   | `render_rsd_document(service_url, homepage_url)`     |
| `common/src/atompub/entry.rs` | `edit_uri`, `edit_media_uri`, `content_src` — fields |
| `common/src/feed/metadata.rs` | `canonical_url`, `self_url`, `hub_url` — fields      |

The parameter cases are the dangerous ones: a struct literal names its fields, a
call does not.

Two shapes were considered and rejected. **Wrapper newtypes**
(`HubUrl(AbsoluteUrl)`) cost one full type per role forever, and
`#[derive(StrNewtype)]`'s trailer assumes a `String` inner, so they need macro
work anyway. **Per-role `String` newtypes** each need their own derive and their
own generated bridge.

## Decision

One generic string newtype carries a zero-sized role marker. Each URL **role**
is a distinct type, so a mis-pass is a compile error.

```rust
pub trait UrlRole {}

pub struct TaggedUrl<T: UrlRole>(String, PhantomData<fn() -> T>);

pub struct Hub;
impl UrlRole for Hub {}
pub type HubUrl = TaggedUrl<Hub>;
```

`AbsoluteUrl` is deleted. There is no neutral tag and no escape-hatch role.

**The tag is a role label, not a validation rule.** All roles share one
`impl<T: UrlRole> FromStr for TaggedUrl<T>`, so `http(s)`-scheme checking and
normalization stay in a single chokepoint. `RootRelativeUrl` therefore stays a
separate type — it is a different _invariant_ on the same carrier, not a
different role — and `FeedPath` stays separate because it canonicalizes through
a `(FeedSurface, FeedFormat)` product.

`PhantomData<fn() -> T>` rather than `PhantomData<T>` keeps `Send`/`Sync` and
the other auto traits unconditional in `T`. The `T: UrlRole` bound sits on the
struct declaration so `split_for_impl()` carries it into every emitted impl.

### Three doors mint a role

1. **Parse** — `"…".parse::<HubUrl>()?`, plus the `TryFrom<String>`, serde
   `Deserialize`, and sqlx `Decode` paths that route through `FromStr`.
2. **Compose** — `compose`, `join`, and `with_query_pairs` are generic in their
   **output** tag. `compose` accepts only a `Base`: composition starts at the
   site root.
3. **Retag** — `retag<U>()`, where one value genuinely plays two roles.

### The output role is stated explicitly at every compose

Leaving the role to inference does **not** deliver transposition safety wherever
both operands are composed. At the RSD site both URLs are freshly composed:

```rust
let service_url:  ServiceDocUrl = compose(&base, ATOMPUB_SERVICE_PATH);
let homepage_url: HomepageUrl   = compose(&base, &format!("/~{username}"));
```

Without those ascriptions a transposed `render_rsd_document(homepage, service)`
would simply infer the opposite role at each `let` and still compile.
`send_publish` was protected only incidentally, because its hub is
config-derived and thus pinned by something other than inference.

The required form is **type ascription, alias-spelled**. Where a value is
consumed inline and cannot be ascribed, a turbofish on the tag is the exception:
`compose::<Permalink>(base_url, &alt_path).to_string()`.

### The alias rule

Every use site that names a **concrete** role spells the **alias** (`HubUrl`),
never `TaggedUrl<Hub>` inline.

Two forms sit outside the rule rather than breaking it, because neither names a
concrete role: a **turbofish mint** (`compose::<Permalink>(…)`), for a value
consumed inline with no binding to ascribe; and a **signature generic over
`UrlRole`**, which serves every role at once — `atompub::entry::rel_link`
renders four roles through one function, and `test_support::parse_url` mints
any. An alias in either position would be wrong, not merely unidiomatic.

This is load-bearing, not stylistic. `site_config_keys!` matches its value slot
as `$value:tt` and validates via `$ty:ident` — `TaggedUrl<Hub>` is four token
trees and will not match, while `HubUrl` is a single ident and matches with no
macro edit. `server_fn_tracing_check` likewise reduces recordable types to a
bare ident.

### Every `retag` call site carries a justification

`retag` exists because some URLs genuinely play two roles. Its doc comment
requires a comment at every call site stating the domain identity it asserts.
There are four, all in AtomPub:

| Site                          | Source → destination          | The fact it asserts                        |
| ----------------------------- | ----------------------------- | ------------------------------------------ |
| `server/src/atompub/media.rs` | `EditUri` → `EntryId`         | the member URL _is_ the entry's `atom:id`  |
| `server/src/atompub/media.rs` | `EditMediaUri` → `ContentSrc` | the content source _is_ the media binary   |
| `server/src/atompub/posts.rs` | `Feed` → `EntryId`            | the collection URL _is_ the feed id        |
| `server/src/atompub/posts.rs` | `Feed` → `Pagination`         | the collection URL _is_ its own first page |

This is policed by review, not by a gate. Four sites is too few to justify one,
and a gate would be a partial guarantee anyway, since `compose` already mints
roles freely.

### Why fifteen roles, when ADR-0063 §1 counts hazards

[ADR-0063](0063-domain-value-newtype-convention.md) §1 admits a newtype on three
axes — invariant, transposition hazard, trust boundary — and warns that
"consistency alone is not sufficient justification." Fifteen roles against four
measured hazards does not clear that bar role-by-role. Three arguments §1 does
not anticipate carry it:

- **§1's cost model assumes one type per value.** It weighs a whole new type —
  declaration, constructor, bridge, trait impls — against the hazard. Under this
  scheme a role costs a zero-sized marker and a type alias and inherits every
  impl. The cost side of the balance is two lines, not a module.
- **Partial classification is unstable.** A neutral tag that "most URLs" carry
  becomes a universal donor: any unclassified value flows into any slot, and the
  guarantee degrades wherever it lingers. Deleting `AbsoluteUrl` outright is
  what makes the remaining distinctions load-bearing.
- **Four is a lower bound.** Each adjacent URL pair added later is a hazard the
  scheme has already prevented.

> **Annotation (2026-08-27).** As of #847, the concrete AtomPub and Syndication
> Feed call sites and metadata named here had moved to `host`; the common
> `TaggedUrl` normalization and role-typing contract remained unchanged. Current
> ownership: [ARCHITECTURE.md](../ARCHITECTURE.md).

## Consequences

- **A sixteenth role costs two lines** — a marker struct and a type alias. That
  is the property the scheme exists to buy.
- **`#[derive(StrNewtype)]` now supports generics.** Ten emitted impls take the
  user's generics via `split_for_impl()`; four that introduce their own
  parameters (serde `Deserialize`'s `'de`, and the sqlx bridge's `DB`, `'q`,
  `'r`) merge the user's generics first. `require_newtype_shape` gained a shape
  parameter so the relaxation is scoped: `IdNewtype` and `NumNewtype` still
  reject generics.
- **`Clone`, `Debug`, `PartialEq`, `Eq`, and `Hash` are hand-written**, not
  derived — `std`'s derives would add `T: Clone`-style bounds on a marker that
  is never stored.
- **`sqlx_newtype_decode_check` resolves type aliases**, cross-crate: the
  aliases are declared in `common/src`, the policed decodes live in
  `storage/src`. Resolution is a single hop; an alias chain would not resolve,
  and none exists.
- **The regression surface is four `compile_fail` doctests**, each with the
  positive companion
  [ADR-0095](0095-doctest-gate-enumerates-the-fence-population.md) requires.
  They are what stops this decision rotting back into #688's residual.
- **No behavioural change.** Every URL produced at runtime is byte-identical.
- **Sibling residuals stay open**: #751 (storage columns), #827 (localStorage),
  #879 (`RootRelativeUrl` in web components). They are the same shape on
  different surfaces and are not addressed here.
