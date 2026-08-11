# Spec — issue #875: role-tagged site URLs

- Issue: [#875](https://github.com/jaunder-org/jaunder/issues/875)
- Date: 2026-08-10
- Branch: `worktree-issue-875-hub-feed-url`

## Problem

`AbsoluteUrl` is one type used for every URL the system handles. #688 threaded
it through `WebSubClient::send_publish`, replacing two adjacent `&str`
parameters with two adjacent `&AbsoluteUrl` parameters. That satisfied ADR-0063
§5 (adopt an existing newtype) but delivered no transposition safety: both
parameters have the same type, so

```rust
self.websub.send_publish(feed, hub)   // arguments swapped
```

still compiles and still pings the wrong endpoint.

This is not confined to `send_publish`. A survey of the tree found **four** live
transposition hazards where same-typed URLs sit adjacent:

| Site                                  | Shape                                                                  |
| ------------------------------------- | ---------------------------------------------------------------------- |
| `server/src/websub/mod.rs:19-20`      | `send_publish(hub_url, feed_url)` — two adjacent params                |
| `common/src/atompub/rsd.rs:24`        | `render_rsd_document(service_url, homepage_url)` — two adjacent params |
| `common/src/atompub/entry.rs:416-420` | `edit_uri`, `edit_media_uri`, `content_src` — three adjacent fields    |
| `common/src/feed/metadata.rs:16-18`   | `canonical_url`, `self_url`, `hub_url` — three adjacent fields         |

The parameter cases are the dangerous ones: a struct literal names its fields, a
call does not.

## Decision

Replace `AbsoluteUrl` with a phantom-tagged generic. Each URL **role** becomes a
distinct type, so a mis-pass is a compile error.

```rust
pub trait UrlRole {}

pub struct TaggedUrl<T: UrlRole>(String, PhantomData<fn() -> T>);

pub struct Hub;
impl UrlRole for Hub {}
pub type HubUrl = TaggedUrl<Hub>;
```

The tag carries **only a role label**, not a validation rule. All roles share
the existing `AbsoluteUrl` validator via one
`impl<T: UrlRole> FromStr for TaggedUrl<T>`, so `http(s)`-scheme checking and
normalization stay in a single chokepoint.

The `T: UrlRole` bound sits on the **struct declaration**, not only on the
impls. The derive's `TryFrom<String>`, `Deserialize`, and sqlx `Decode` all
route through `FromStr`, so an unbounded `<T>` would emit impls that fail to
compile. Declaring the bound once lets `split_for_impl()` carry it everywhere.

`PhantomData<fn() -> T>` (rather than `PhantomData<T>`) keeps `Send`/`Sync` and
the other auto traits unconditional in `T`.

`AbsoluteUrl` is **deleted**. There is no neutral `Unclassified` tag and no
escape hatch role — every current use site is classified in this change.
`common/src/absolute_url.rs` is renamed to `common/src/tagged_url.rs`;
`InvalidAbsoluteUrl` is renamed to `InvalidUrl`.

### Why not a wrapper newtype

`HubUrl(AbsoluteUrl)` was considered and rejected. `#[derive(StrNewtype)]`'s
emitted trailer assumes a `String` inner (`Deref<Target = str>`,
`TryFrom<String>`, `FromStr`-routed serde and sqlx), so a
newtype-wrapping-newtype needs macro work anyway — and it costs one full type
per role forever, where the generic costs one zero-sized marker plus a type
alias.

### Why not per-role `String` newtypes

Each new role would need its own `#[derive(StrNewtype)]` and its own generated
bridge. The generic writes the bridge once.

### Relationship to ADR-0063 §1

ADR-0063 §1 admits a newtype on three axes — invariant, transposition hazard,
trust boundary — and says "consistency alone is not sufficient justification."
This spec mints fifteen roles against four measured transposition hazards, so
most roles do not individually clear that bar today. That is deliberate, and
rests on an argument §1 does not anticipate:

- **§1's cost model assumes one type per value.** It weighs a new _type_ — a
  declaration, a constructor, a bridge, a set of trait impls — against the
  hazard. Under the tagged scheme a role costs a zero-sized marker and a type
  alias, and inherits every impl. The cost side of §1's balance is roughly two
  lines, not a module.
- **Partial classification is unstable.** A neutral tag that "most URLs" carry
  becomes a universal donor: any unclassified value flows into any slot, and the
  guarantee degrades wherever it lingers. Deleting `AbsoluteUrl` outright is
  what makes the remaining distinctions load-bearing.
- **The hazard count is a lower bound.** Four hazards exist _today_; each new
  adjacent URL pair added later is a hazard the scheme has already prevented.

The ADR draft records this argument, and ADR-0063 gains a cross-reference so a
future reader does not apply §1's per-type cost model to a role.

## Roles

Fifteen roles, covering every current `AbsoluteUrl` site.

| Role tag         | Alias               | Representative sites                                                                                                                                       |
| ---------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Base`           | `BaseUrl`           | `common/src/site.rs:17`, `storage/src/site_config.rs:209`, `server/src/atompub/mod.rs:121`, `web/src/site/api.rs:25`, `web/src/site/component.rs:67,94-95` |
| `Hub`            | `HubUrl`            | `server/src/websub/mod.rs:19`, `common/src/feed/mod.rs:37`, `storage/src/site_config.rs:166`, `common/src/feed/metadata.rs:18`                             |
| `Feed`           | `FeedUrl`           | `server/src/websub/mod.rs:20`, `common/src/feed/metadata.rs:17`, `common/src/atompub/entry.rs:346`                                                         |
| `CollectionHref` | `CollectionHrefUrl` | `common/src/atompub/service.rs:20`                                                                                                                         |
| `Canonical`      | `CanonicalUrl`      | `common/src/feed/metadata.rs:16`                                                                                                                           |
| `Permalink`      | `PermalinkUrl`      | `common/src/feed/metadata.rs:26`, `server/src/atompub/mapping.rs:169`                                                                                      |
| `MediaOrigin`    | `MediaSourceUrl`    | `storage/src/media.rs:39`, `storage/src/helpers.rs:324`                                                                                                    |
| `Pagination`     | `PaginationUrl`     | `common/src/atompub/entry.rs:348,350,352`                                                                                                                  |
| `EntryId`        | `EntryIdUrl`        | `common/src/atompub/entry.rs:340,410`                                                                                                                      |
| `EditUri`        | `EditUriUrl`        | `common/src/atompub/entry.rs:416`, `server/src/atompub/posts.rs:395`                                                                                       |
| `EditMediaUri`   | `EditMediaUriUrl`   | `common/src/atompub/entry.rs:418`                                                                                                                          |
| `ContentSrc`     | `ContentSrcUrl`     | `common/src/atompub/entry.rs:420`                                                                                                                          |
| `ServiceDoc`     | `ServiceDocUrl`     | `server/src/atompub/rsd.rs:36`                                                                                                                             |
| `Homepage`       | `HomepageUrl`       | `server/src/atompub/rsd.rs:38`                                                                                                                             |
| `MailConfirm`    | `MailConfirmUrl`    | `web/src/email/api.rs:45`, `web/src/invites/api.rs:71`, `web/src/password_reset/api.rs:62`, `server/src/commands.rs:302`                                   |

Two naming notes:

- The tag is `MediaOrigin`, not `MediaSource`, because `common/src/media.rs:608`
  already declares `pub enum MediaSource` in the same crate. Both the sqlx
  decode gate and `server_fn_tracing_check` reduce types to a bare ident, so two
  `common` types sharing a name is precisely the ambiguity they cannot see
  through.
- `CollectionHref` is its own role rather than `Feed`, because
  `server/src/atompub/service.rs:48,54` builds it twice — once for the posts
  collection and once for the media collection. Typing it `FeedUrl` would assert
  a media collection is a syndication feed.

`server/src/atompub/posts.rs:395` (the `Location` response header) takes
`EditUri`, and `server/src/atompub/mapping.rs:169` (the `alt` link, immediately
stringified) takes `Permalink`.

`RootRelativeUrl` and `FeedPath` stay as they are. `RootRelativeUrl` is a
different invariant on the same carrier, not a different role; `FeedPath`
canonicalizes through a `(FeedSurface, FeedFormat)` product and has inherent API
tied to that structure.

## Minting a role

Three doors, and only three.

### 1. Parse

`"…".parse::<HubUrl>()?`, plus the `TryFrom<String>`, serde `Deserialize`, and
sqlx `Decode` paths that route through `FromStr`.

`common/src/test_support/mod.rs:49`'s `parse_absolute_url` is part of this door.
It becomes `pub fn parse_url<T: UrlRole>(s: &str) -> TaggedUrl<T>`. Its call
sites in `common/src/atompub/{entry,service,rsd}.rs` mint eight different roles
and each must name the role it wants.

### 2. Compose

`compose`, `join`, and `with_query_pairs` are generic in their **output** tag:

```rust
pub fn compose<U: UrlRole>(base: &TaggedUrl<Base>, path: &str) -> TaggedUrl<U>;

impl<T: UrlRole> TaggedUrl<T> {
    pub fn join<U: UrlRole>(&self, path: &str) -> Result<TaggedUrl<U>, InvalidUrl>;
    pub fn with_query_pairs<U: UrlRole>(&self, pairs: &[(&str, &str)]) -> TaggedUrl<U>;
}
```

`compose` accepts only a `Base`, which is the honest constraint: composition
starts at the site root.

**The output role must be stated explicitly at every mint.** Leaving it to
inference does not deliver transposition safety wherever both operands are
composed — at `rsd.rs:36,38` a swapped call to `render_rsd_document` would
simply infer the opposite role at each `let` and still compile. `send_publish`
is protected only incidentally, because its hub is config-derived.

The required form is **type ascription, alias-spelled**:

```rust
let service_url:  ServiceDocUrl = compose(&base, &service_path);
let homepage_url: HomepageUrl   = compose(&base, &format!("/~{username}"));
```

Sites that consume a composed URL inline and cannot be ascribed use a turbofish
on the tag:

```rust
compose::<Permalink>(base_url, &alt_path).to_string()
```

Ascription is preferred because it spells the **alias**, and the alias spelling
is load-bearing (see _The alias rule_ below). The turbofish, which spells the
tag, is the exception rather than the norm.

There are 18 production `compose` call sites, one `with_query_pairs`, and the
`join` sites; each gains an ascription or a turbofish.

### 3. Retag

`fn retag<U: UrlRole>(self) -> TaggedUrl<U>`, for the case where one value
genuinely plays two roles. Exactly four call sites, each asserting a deliberate
domain identity:

| Site                              | Source → destination          | The fact it asserts                        |
| --------------------------------- | ----------------------------- | ------------------------------------------ |
| `server/src/atompub/media.rs:58`  | `EditUri` → `EntryId`         | the member URL _is_ the entry's `atom:id`  |
| `server/src/atompub/media.rs:62`  | `EditMediaUri` → `ContentSrc` | the content source _is_ the media binary   |
| `server/src/atompub/posts.rs:191` | `Feed` → `EntryId`            | the collection URL _is_ the feed id        |
| `server/src/atompub/posts.rs:195` | `Feed` → `Pagination`         | the collection URL _is_ its own first page |

`retag` is policed by doc comment and ADR rule, not by a gate: its doc comment
requires a justifying comment at every call site. A gate would be
disproportionate at four sites.

## The alias rule

**Every use site spells the alias (`HubUrl`), never `TaggedUrl<Hub>` inline.**
The tag name appears only in the type's own module and in turbofish exceptions.

This is a hard convention, not a style preference. Two gates depend on it:

- `site_config_keys!` (`common/src/config_key.rs:85`) matches its value slot as
  `$value:tt` and validates via `$ty:ident`. `TaggedUrl<Hub>` is four token
  trees and will not match; `HubUrl` is a single ident and matches with zero
  macro edits. The rows at `:165` and `:169` use aliases.
- `server_fn_tracing_check` reduces recordable types to a bare ident.

## Macro work

`#[derive(StrNewtype)]` must learn generics. It currently never reads
`input.generics`, so it would emit `impl Display for TaggedUrl` and fail to
compile.

- **Plain `split_for_impl()`** suffices for the 8 `default_trailer` impls and
  the 2 `ord_impls` — none of them introduce generics of their own.
- **Merged generics** are required for 4 impls, not 3: the serde `Deserialize`
  impl (`str_newtype.rs:212`) already introduces `'de`, exactly as the three
  `sqlx_bridge` impls introduce `DB`, `'q`, `'r`. For each, clone the user's
  generics, push the extra params, then split — `impl_generics` cannot be
  dropped in directly.
- **Relax the shape check** to permit a trailing `PhantomData<_>` field
  alongside the single data field. `require_newtype_shape`
  (`macros/src/lib.rs:486`) is shared with `IdNewtype` and `NumNewtype`, so the
  relaxation must be **scoped to `StrNewtype`** — either a parameter or a
  sibling function — and its doc comment (`:480-485`), which states the generics
  rejection exists to avoid a confusing "missing generics" error, must be
  corrected.
- `ord_impls` is a shared `pub(crate)` helper (`macros/src/lib.rs:549`) used by
  all three newtype macros; a signature change there has that blast radius.
- `sqlx_bridge`'s `BridgeSpec` carries only `name: &syn::Ident` and serves four
  call sites including the standalone `SqlxBridge` derive. It needs a generics
  field, and the other three call sites must keep compiling with empty generics.

Nothing in the macro constructs `Self` — `TryFrom`, `Deserialize`, and `Decode`
all route through the hand-written `FromStr` — so no construction sites need a
`PhantomData` argument threaded in.

`Clone`, `Debug`, `PartialEq`, `Eq`, and `Hash` are **hand-written** in
`common/src/tagged_url.rs` rather than derived, because `std`'s derives naively
add `T: Clone`-style bounds on a marker that is never stored.

Keeping the derive (rather than hand-writing all thirteen impls) is what keeps
`sqlx_newtype_decode_check` honest — see below.

## Gate work

- **`sqlx_newtype_decode_check`** builds its approve-set by scanning for
  `#[derive(StrNewtype)]` and keys it on the bare struct ident, resolving leaf
  types by ident without following type aliases. `TaggedUrl` enters the set
  legitimately once the derive applies, but the **aliases** do not. Teach the
  gate to resolve `type X = TaggedUrl<Tag>;` declarations. Note this must be
  **cross-crate**: the aliases are declared in `common/src`, the policed decodes
  are in `storage/src` (`site_config.rs:166`, `media.rs:39`, `helpers.rs:324`).
  `common/src` is already in `DECLARATION_ROOTS`, so the gate parses it — but it
  is `Root::DeclarationsOnly` and `visit_item_type` currently collects only
  tuple aliases, so the new alias pass must be added alongside that without
  widening its existing over-bite.
- **`sqlx_newtype_bind_check`** is a line-based text scan with no type names in
  it. It is indifferent to this change; no work.
- **`server_fn_tracing_check`**
  (`xtask/src/steps/server_fn_tracing_check.rs:81`) holds the literal
  `("AbsoluteUrl", "operator-configured site base URL")`. Update it to
  `BaseUrl`.

## Recording the decision

A new ADR draft at `docs/adr/drafts/role-tagged-site-urls.md` records the
scheme: the `UrlRole` marker trait, the three minting doors, the explicit-role
rule at compose, the alias rule, and the `retag` justification rule. It also
carries the §1 cost-model argument above.

ADR-0063 gains a short amendment noting that URL roles are handled by that
scheme, so a future reader does not apply §1's per-type cost model to a role.

## Acceptance criteria

Each is observable — a reviewer can check it by reading a diff or running a
command.

1. No identifier `AbsoluteUrl` remains anywhere in the tree.
   `rg '\bAbsoluteUrl\b'` returns matches only under `docs/adr/` (historical ADR
   text that records what was decided at the time). Comments in
   `end2end/tests/admin-site.spec.ts:36,43` and
   `common/src/root_relative_url.rs:1` are updated, not carved out.
2. `common/src/absolute_url.rs` is renamed to `common/src/tagged_url.rs`;
   `InvalidAbsoluteUrl` is renamed to `InvalidUrl`.
3. `TaggedUrl<T: UrlRole>` exists with a `UrlRole` marker trait, and all fifteen
   roles have a marker struct and a type alias.
4. Swapping the two arguments at any `send_publish` call site fails to compile,
   demonstrated by a `compile_fail` doctest.
5. Swapping the two arguments of `render_rsd_document` fails to compile,
   demonstrated by a `compile_fail` doctest.
6. Both new `compile_fail` fences satisfy ADR-0095's fence-population gate.
7. `#[derive(StrNewtype)]` applies successfully to a generic struct with a
   trailing `PhantomData` field; a test in `macros/tests/` covers it.
   `IdNewtype` and `NumNewtype` still reject generics, with a test covering that
   too.
8. `compose`, `join`, and `with_query_pairs` are generic in their output tag,
   and every call site states its role — by alias ascription, or by turbofish
   where the value is consumed inline. `rg 'compose\(' ` shows no site relying
   on inference.
9. `retag` exists, has exactly four call sites, and each carries a comment
   stating the domain identity it asserts.
10. No use site outside `common/src/tagged_url.rs` spells `TaggedUrl<` inline,
    except turbofish mints.
11. `sqlx_newtype_decode_check` resolves type aliases across crates: a storage
    decode into `Option<HubUrl>` passes the gate, and a decode into an
    unapproved type still fails it (a negative test covers the second half).
12. `server_fn_tracing_check` names `BaseUrl`, and the gate passes.
13. The config-key rows at `common/src/config_key.rs:165,169` use the alias
    spelling and the registry compiles unchanged.
14. `web/src/site/component.rs:67,94-95` (`Field::<…>` and the `ValidatedInput`
    generic component) use `BaseUrl` and the Leptos `view!` macro compiles.
15. `common/src/test_support/mod.rs`'s helper is `parse_url<T: UrlRole>`, and
    every call site's role is pinned — by turbofish, or by the field/parameter
    type it is passed to. **Amended during implementation:** the original
    wording demanded a turbofish at every call site. In a struct literal or
    argument position the expected type already pins the role and the compiler
    _checks_ it, so the safety property is identical — unlike `compose`, where
    inference would _choose_ a role freely. Turbofishing the ~40 fixture sites
    would also require importing the tag `Feed` into
    `common/src/atompub/entry.rs`'s tests, where it collides with
    `atom_syndication::Feed`.
16. `docs/adr/drafts/role-tagged-site-urls.md` exists and follows the draft
    format (`# ADR-DRAFT:` heading, single-token status line). ADR-0063 carries
    the cross-reference amendment.
17. `cargo xtask validate` is green.

## Out of scope

- `RootRelativeUrl` and `FeedPath` are untouched.
- Sibling issues #751 (storage columns), #827 (localStorage), #879
  (`RootRelativeUrl` in web components) are unaffected and stay open.
- No behavioural change. Every URL produced at runtime is byte-identical to what
  the current code produces; this is a typing change only.
