# ADR-0063: Domain-value newtypes — when to introduce one, and the standard trailer

- Status: proposed
- Date: 2026-07-11
- Issue: [#17](https://github.com/jaunder-org/jaunder/issues/17)

## Context

The code conventions already say to "use Rust's type system to make invalid
states impossible" and to "parse data into infallible types at boundaries."
`common` follows this for a handful of values — `Username`, `Slug`, `Tag`,
`Password`, `PostFormat` — each a validated newtype whose `FromStr` is the
single chokepoint and whose `#[serde(try_from = "String", into = "String")]`
bridge carries the same validation onto the wire. `Slug` (ADR-0025) is the
exemplar.

But there is no written rule for **when** a value earns a newtype, so the
decision is re-litigated per issue and the coverage is patchy. A cluster of open
type-safety work all circles the same missing policy:

- **#17** — bare `i64` IDs (`user_id`, `post_id`, `tag_id`, …), session
  `RawToken` vs `TokenHash`, and the media `ContentHash`/`Sha256` are all
  primitive-typed, so `tag_post(post_id, …)` accepts a `user_id` and compiles,
  and nothing stops a raw token being logged where its hash was meant.
- **#350** — `AudienceName`'s trim/non-empty rule is duplicated inline in two
  server functions instead of living in a type.
- **#14 / #91** — threading the _existing_ newtypes (and typed timestamps) out
  through web DTOs and the `#[server]` boundary.

Two facts make the current state actively self-defeating:

1. **The existing newtypes are ergonomically thin.** They expose only
   `as_str()`, `Display`, `FromStr`, `TryFrom<String>`, and
   `From<Self> for String`. They implement none of `AsRef<str>`, `Borrow<str>`,
   `Deref<Target =    str>`, or `PartialEq<str>`. So every consumer that wants a
   `&str` writes `.as_str()` (≈140 production sites), a `HashMap<Username, _>`
   can't be probed with a `&str` key without allocating, and code tends to drop
   back to `String` at the first friction point.

2. **That thinness is exactly why #14 is blocked.** Its own note: "web response
   DTOs are built from storage records that return `String`, so typing a DTO
   field forces `parse().expect()` at the web boundary — clippy denies
   `expect_used`." A newtype that behaved like a `&str` and flowed unbroken from
   storage outward would dissolve that blocker. The thin type discourages the
   very propagation the convention is supposed to encourage.

Absent a policy, each new type also re-imports the same 40 lines of trait
boilerplate, and "should this be a newtype at all?" gets answered by taste.

## Decision

Two rules: a **criterion** for introducing a domain-value newtype, and a
**standard trailer** every such newtype implements.

### 1. When a value earns a newtype

Introduce a newtype for a domain value when **at least one** of these holds:

- **Invariant** — it has a constraint a bare primitive can't express (format,
  normalization, length bound). The newtype's fallible constructor is the one
  place that constraint is enforced; interior code is then invalid-state-free.
  _(Username, Slug, Tag, Email, FeedUrl, AudienceName.)_
- **Transposition hazard** — another value of the same primitive type is a
  plausible mis-pass at a call site. The type turns the mix-up into a compile
  error. _(the `i64` IDs; `RawToken` vs `TokenHash`; a raw `body` vs its
  `rendered_html`.)_
- **Trust / safety boundary** — the value carries a semantic guarantee that must
  not be forged. _(`RenderedHtml` is safe to emit unescaped; a raw user string
  is not.)_

Do **not** introduce one for a genuinely free-form, locally-scoped string with
no invariant, no same-typed sibling to be confused with, and no trust semantics
— a log message, a one-off internal label. **Consistency alone is not sufficient
justification.** Bias toward a type for values that cross a module or crate
boundary; toward a primitive for values that live and die in one function.

A value that is genuinely **polymorphic** — `ViewerIdentity::Channel`'s
`subscriber_ref`, which is a stringified `user_id` in one arm and an external
reference in another (ADR-0020) — is modeled as an **enum**, not a string
newtype. Wrapping a union in a single `String`-newtype hides the very
distinction the type should expose.

### 2. The standard trailer

Every **string-backed** domain newtype exposes exactly this surface — no less
(so consumers never pay a conversion tax) and no more (so the type stays a
value, not a `String` in disguise):

- `FromStr` — the single validating/normalizing chokepoint. Fallible when the
  value has an invariant; the constructor normalizes (e.g. lowercasing) so the
  stored form is canonical.
- `#[serde(try_from = "String", into = "String")]` — routes (de)serialization
  through that same `FromStr`, so the type serializes as a plain string and
  rejects invalid input on the wire.
- `TryFrom<String>` (or `From<String>` when infallible) and
  `From<Self> for String` — owned conversion both directions.
- `AsRef<str>`, `Borrow<str>`, and `Deref<Target = str>` — so the newtype _is_ a
  `&str` at use sites: `&x` coerces to `&str`, every `str` method is callable
  directly, and `HashSet<X>` / `HashMap<X, _>` can be probed with a `&str` key
  with no allocation. This is what retires the `.as_str()` tax.
- `Display` — user-facing rendering.
- `PartialEq<str>` and `PartialEq<&str>` — compare against a string literal
  without unwrapping.
- Derive `Clone, Debug, PartialEq, Eq, Hash` (add `Ord` when the type is used as
  a sort/map key).

**`Deref<Target = str>` is the one place we accept "deref polymorphism."** It
mirrors `String: Deref<str>` and `PathBuf: Deref<Path>` — the standard-library
idiom for a smart-string — and is sanctioned **only** for `str`-backed newtypes,
nowhere else.

**Secret-bearing exception.** A newtype wrapping a secret (`RawToken`,
`Password`), selected with `#[str_newtype(secret)]`, exposes a deliberately
**tight** surface: a **redacting `Debug`**, explicit borrowed access via
**`AsRef<str>` only**, and construction via `TryFrom<String>`. It **omits**
`Display`, the serde bridge, `Deref<str>` **and** `Borrow<str>`,
`From<Self> for String`, and `PartialEq<str>`/`<&str>`. So a secret cannot
render, (de)serialize, implicitly coerce to `&str` (via `Deref`, which would
also reopen owned extraction through `str::to_owned`/`to_string`), hand out an
owned plaintext `String`, or be value-compared in non-constant time. The result
is readable-for-hashing but un-leakable — it satisfies ADR-0011's
no-secrets-in-telemetry rule **by construction** rather than by discipline.

**Inbound-secret variant.** A secret is sometimes _submitted by a client_ — it
must cross the `#[server]` boundary client→server — while still never being
_rendered_ or _returned_. `#[str_newtype(secret, serde)]` re-opens **only** the
validating serde bridge on the secret surface (redacting `Debug`, `AsRef<str>`,
`TryFrom<String>`), keeping every other restriction: no `Display`, `Deref`,
`Borrow`, owned-`String`, or `PartialEq`. Serde encodes/decodes _operations_,
not a _direction_ (a `#[server]` payload needs both `Serialize` and
`Deserialize` regardless of which way it flows), so "inbound only" cannot be a
property of the type's traits — it is enforced **structurally**: the inbound
type is a **distinct** newtype, paired with a plain-`secret` domain type it
converts into, and an `xtask` gate pins the inbound type to `#[server]`
**parameter** positions so it can never be a return type or DTO field. Placing
the domain type in a server-only crate (never built for wasm) makes "never
client-side" a compile fact. `ProfferedInviteCode` (#400), paired with the
domain `InviteCode`, is the first user.

**Outbound-secret variant.** The mirror case is a secret the server _mints_ and
_deliberately delivers_ to a client exactly once — the session `RawToken` (#458)
returned by `create_app_password` for the user to copy into an AtomPub client.
This also uses `#[str_newtype(secret, serde)]`, but it is **not** paired with
the inbound gate, and deliberately so: the value is server-generated (never
client-submitted, so there is no untrusted-parameter position to pin), the
redacting `Debug` keeps it un-leakable in telemetry (ADR-0011), and the
load-bearing invariant is the **type distinction** — `RawToken` cannot be
confused with, or converted to, its stored `TokenHash`. The
`ProfferedInviteCode` `xtask` gate is type-specific (it names that identifier),
so it neither fires on nor governs `RawToken`; a single sanctioned return site
(the app-password response) is the whole outbound surface.

**Numeric IDs** take the same idea with a numeric trailer: `struct UserId(i64)`
deriving `Clone, Copy, Debug, PartialEq, Eq, Hash`, plus `From<i64>` /
`Into<i64>`, `Display`, and a **transparent-i64 serde bridge** — no `str`
traits. The serde bridge keeps the wire form a bare integer, so a DTO field can
adopt the type without changing any serialized shape; deserialize is an
infallible wrap (an id has no value invariant, only the transposition
guarantee).

### 3. The trailer is generated, not hand-written

The trailer is mechanical and identical across types, so it lives in a
`#[derive(StrNewtype)]` (and `#[derive(IdNewtype)]`) proc-macro in the
**`macros` crate** (ADR-0062) — its second tenant. The derive generates
everything except `FromStr` **and the std `#[derive]`s**
(`Clone`/`Debug`/`PartialEq`/`Eq`/`Hash`/`Ord`/`Copy`). `FromStr` stays
hand-written because the validation/normalization rule is the one genuinely
per-type part; the std derives stay in the user's `#[derive(...)]` list so
per-type variation is expressed idiomatically (Slug omits `Hash`, Tag adds
`Ord`, a secret omits `Debug` so the generated redacting one applies). The serde
bridge is emitted as **direct `Serialize`/`Deserialize` impls**, not a
`#[serde(try_from/into)]` attribute (serialize borrows instead of cloning into a
`String`; deserialize routes through `FromStr` so invalid input is rejected on
the wire). No inherent `as_str()` is generated — the `str` traits replace it. A
new domain newtype is then a struct, a derive, and a `FromStr` — not 40 lines of
boilerplate that drift apart over time.

For a value whose invariant never rejects (only normalizes, or wraps verbatim),
`#[str_newtype(infallible)]` supplies the trailer's
`From<String>`-when-infallible half (§2): the author hand-writes `From<String>`
instead of `FromStr`, the derive omits `TryFrom<String>` (which would collide
with it via the std blanket `impl<T, U: Into<T>> TryFrom<U>`) and routes
`Deserialize` through that `From<String>`. First users: `PostBody`/`PostTitle`
(#402).

### 4. Boundary rule

Parse into the newtype at the **outermost** boundary — `#[server]` argument and
return types, CLI argument types, storage record fields and trait signatures —
and hold the newtype inward. Because the trailer gives `Deref<Target = str>`, a
storage method can still take `&str` internally and a caller holding a
`Username` passes `&username` unchanged; the type is not a tax on the read path.
This is the shape #14 needs: storage returns the newtype, the DTO field _is_ the
newtype, and no `parse().expect()` appears at the web boundary.

## Consequences

- **One decision surface.** "Does this value deserve a type, and what shape does
  it have?" is answered here. The six currently-unfiled gaps (Email,
  RenderedHtml, FeedUrl, InviteCode, DisplayName, PostBody/PostTitle) and the
  open #14 / #17 / #91 / #350 all cite this ADR instead of re-deriving the rule.
- **Sequencing is load-bearing.** The trailer (as the derive, or hand-rolled)
  must land on the _existing_ newtypes **first**. Only then can storage records
  and traits return them without forcing `parse().expect()` at consumers — the
  concrete unblock for #14. New value classes follow; #17's ID sweep and its
  token/hash split are independent tracks.
- **A second proc-macro.** `macros` gains `StrNewtype`/`IdNewtype`. Per ADR-0062
  the crate is build-time only with no runtime footprint; the derive must stay a
  pure code generator (it emits trait impls, nothing observable at runtime).
- **Incremental, never big-bang.** Threading a type through storage and the DTOs
  is a large mechanical diff. Each value class is its own reviewable change
  (`preparatory refactor` → behavior unchanged), never one sweeping commit.
- **What this rules out.** Bare primitives for values with an invariant, a
  transposition hazard, or a trust boundary; string-newtypes for genuinely
  polymorphic values (use an enum); and "consistency-only" newtypes for
  free-form local strings. When a value is borderline, the crossing-a-boundary
  test in §1 decides.
