# ADR-0063: Domain-value newtypes — when to introduce one, and the standard trailer

- Status: accepted
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

A value that is genuinely **polymorphic** is modeled as an **enum**, not a
string newtype. Wrapping a union in a single `String`-newtype hides the very
distinction the type should expose.

**URLs are an exception to this rule's cost model.** Site URLs are not typed one
newtype per role; they are `TaggedUrl<T>`, a single generic carrying a
zero-sized role marker, so a role costs a marker struct and a type alias and
inherits every trait impl. The balance this rule strikes — weigh a whole new
type against a demonstrated hazard — does not apply when the type side of it is
two lines. Do **not** cite "consistency alone is not sufficient justification"
to argue a URL role out of existence, and do not add a bare `AbsoluteUrl`-style
catch-all back. See
[the role-tagged site URLs ADR](0112-role-tagged-site-urls.md) (#875).

`ViewerIdentity` (ADR-0020) is this rule applied, and #6 is what it cost to
apply it late. A viewer's `subscriber_ref` is a stringified `user_id` for a
local account and an opaque external reference for a remote one; carried as one
`String` field, the two were indistinguishable, and the SQL that decides "is
this viewer the author" recovered the distinction by asking whether the string
parsed as an integer. It now splits at the type: `Local { user_id: UserId }`
versus `Remote { channel_id, subscriber_ref: String }`. The remaining `String`
is genuinely free-form and genuinely remote — nothing left to disambiguate.

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
- `PartialOrd` and `Ord`, delegating to the inner value (#761). A string newtype
  _is_ a string, so lexicographic order on the wrapped `String` is the only sane
  ordering — and it is the one `Borrow<str>` already obliges, since `Borrow`'s
  contract requires `Ord`/`Eq`/`Hash` to agree with the borrowed form. Emitting
  it removes the trap: a type whose members lack `Ord` invites a hand-written
  `impl` that can get that agreement wrong.
- Derive `Clone, Debug, PartialEq, Eq, Hash`. `PartialEq`/`Eq` are **required**,
  not optional, because `Ord: Eq`; a type that genuinely cannot have them takes
  `#[str_newtype(no_ord)]` (below). `Hash` remains the one per-type call — a
  hash has a cost story (it is not free to compute and invites the type into hot
  paths) that a total order does not.

**The trailer grants capability, not intent.** Omitting a derive was never
enforcement — any author can add `Hash` in one line — so "this type is never a
map/set key" was documentation, not a guarantee, and is not a reason to withhold
ordering. Before #761 a dozen newtypes carried exactly that rationale; ordering
makes every one of them a `BTreeMap` key, which would have left "never a key"
true for `HashMap` alone. Say what a trait buys, not what a type is "for".

**`#[str_newtype(no_ord)]`** suppresses only the ordering half, for a newtype
that deliberately has no `PartialEq`/`Eq`. Its one user is the bearer-token
`RawToken`, whose security value is the type distinction rather than value
comparison. It is valid with `infallible` and rejected with `secret` (already
unordered). It exists on `str_newtype` alone: every `NumNewtype`/`IdNewtype`
orders meaningfully, so neither macro carries the flag.

**`Deref<Target = str>` is the one place we accept "deref polymorphism."** It
mirrors `String: Deref<str>` and `PathBuf: Deref<Path>` — the standard-library
idiom for a smart-string — and is sanctioned **only** for `str`-backed newtypes,
nowhere else.

**Secret-bearing exception.** A newtype wrapping a true secret (e.g. `Password`)
— one that must never be rendered or transmitted at all — selected with
`#[str_newtype(secret)]`, exposes a deliberately **tight** surface: a
**redacting `Debug`**, explicit borrowed access via **`AsRef<str>` only**, and
construction via `TryFrom<String>`. It **omits** `Display`, the serde bridge,
`Deref<str>` **and** `Borrow<str>`, `From<Self> for String`,
`PartialEq<str>`/`<&str>`, and `PartialOrd`/`Ord`. So a secret cannot render,
(de)serialize, implicitly coerce to `&str` (via `Deref`, which would also reopen
owned extraction through `str::to_owned`/`to_string`), hand out an owned
plaintext `String`, be value-compared in non-constant time, or be sorted — you
never sort secrets, and a comparison operator is the wrong affordance to hand
out by default. The result is readable-for-hashing but un-leakable — it
satisfies ADR-0011's no-secrets-in-telemetry rule **by construction** rather
than by discipline.

**One honest limit on that.** The absence of ordering on a secret is a
**convention, not a structural invariant**. A derive macro cannot see the user's
`#[derive]` list, so the macro emits nothing for a secret rather than forbidding
anything: `#[derive(PartialOrd, Ord, StrNewtype)] #[str_newtype(secret)]` still
compiles. Every _other_ omission above is structural, because the macro is the
only source of those impls. Closing this would take a bespoke `xtask` gate,
which is disproportionate to a hazard nobody has hit — but the asymmetry is
recorded rather than implied, so nobody reads the list above as airtight.

**Inbound-secret variant.** A secret is sometimes _submitted by a client_ — it
must cross the `#[server]` boundary client→server — while still never being
_rendered_ or _returned_. `#[str_newtype(secret, serde)]` re-opens **only** the
validating serde bridge on the secret surface (redacting `Debug`, `AsRef<str>`,
`TryFrom<String>`), keeping every other restriction: no `Display`, `Deref`,
`Borrow`, owned-`String`, `PartialEq`, or ordering. Serde encodes/decodes
_operations_, not a _direction_ (a `#[server]` payload needs both `Serialize`
and `Deserialize` regardless of which way it flows), so "inbound only" cannot be
a property of the type's traits — it is enforced **structurally**: the inbound
type is a **distinct** newtype, paired with a plain-`secret` domain type it
converts into, and an `xtask` gate pins the inbound type to `#[server]`
**parameter** positions so it can never be a return type or DTO field. Placing
the domain type in a server-only crate (never built for wasm) makes "never
client-side" a compile fact. `ProfferedInviteCode` (#400), paired with the
domain `InviteCode`, is the first user.

**Inbound non-secret representations do not automatically become public twins.**
Amended by `docs/adr/0084-media-filename-encoded-canonical.md` (#720, later
refined by #1149). The `Proffered` prefix still names the inbound-_secret_
profile above, but the filename case proved that representation seams can be
narrower than a cross-crate domain type. axum percent-decodes path parameters,
so routes carrying a filename temporarily hold a decoded spelling while
`Filename` holds the canonical encoded one — and because encoding is not
idempotent, one `FromStr` cannot serve both without double-encoding a value that
is already canonical.

That decoded spelling is an **extractor-private intermediate**, not a public
`common` type. `common::media` owns the validating decoded-segment conversion to
`Filename`: check the safe-leaf oracle on the decoded text, percent-encode once
with the media segment encode set, enforce the encoded byte budget, and return
the canonical `Filename`. Server extractors may wrap the raw route segment
privately to select that door, but handler, domain, storage, DTO, and web
surfaces retain only `Filename` or validated address structs containing it. The
decoded intermediate takes no `Display`, `Serialize`, `Deref`, `AsRef`, sqlx
bridge, or other string-newtype trailer because it is not a durable domain
value.

**Bearer-token profile.** A distinct case is a value that is _transmitted by
design_ yet must not be logged — the session `RawToken` (#458), which the server
mints and delivers into `Set-Cookie`, the app-password response, `Bearer`
headers, and reset/verify URLs. This is **not** a secret in the sense above (it
is _meant_ to leave the server), so it takes the **full ergonomic trailer**
(`Display`, `Deref<str>`, `AsRef`, `PartialEq<str>`, serde) — interpolating or
binding it costs nothing — with a **single deviation**: a hand-written redacting
`Debug` (so it is never `#[derive(Debug)]`'d), because the real hazard is an
accidental `{:?}` in a log or span (ADR-0011), not a deliberate render. The
security value is the redacting `Debug` plus the **type distinction** —
`RawToken` cannot be confused with, or converted to, its stored `TokenHash` —
not the surface tightness a true secret needs. Corollary: do not reach for the
tight `secret` surface merely because a value is credential-shaped; weigh
whether it is ever _meant_ to be transmitted. `Password` (never transmitted) is
a `secret`; `RawToken` (always transmitted) is a bearer token.

**Numeric IDs** take the same idea with a numeric trailer: `struct UserId(i64)`
deriving `Clone, Copy, Debug, PartialEq, Eq, Hash` — plus emitted
`PartialOrd`/`Ord` on the inner `i64` (#761), so an id sorts deterministically
and keys a `BTreeMap` — plus `From<i64>` / `Into<i64>`, `Display`, `FromStr`,
and a **transparent-i64 serde bridge**. The serde bridge keeps the wire form a
bare integer, so a DTO field can adopt the type without changing any serialized
shape; deserialize is an infallible wrap (an id has no value invariant, only the
transposition guarantee). `FromStr` delegates to `i64`'s parse and wraps — the
inverse of `Display`, for the few sites that carry an id as a _string_ (a Leptos
route param, whose `ParamsMap` yields `String`; a `subscriber_ref`), so
`"42".parse::<UserId>()` works. It is **not** a validating chokepoint like a
string newtype's `FromStr` (an id has no invariant beyond "is an integer"); no
other `str` traits are provided. The trailer also carries a **transparent-i64
sqlx bridge** (ADR-0071), so an id is a first-class DB column type; its `Decode`
is an infallible wrap for the same reason its `FromStr` does not validate.

**Numeric values** are the third case: a scalar integer config value with a
**bound** — a retention count (`>= 1`), a feed's minimum item/day window
(`>= 1`), a byte-size limit, a page size (`1..=N`). Unlike an id, a numeric
value _does_ have a rejecting invariant, so it takes a numeric trailer with a
**validating** twist: `struct RetentionCount(usize)` deriving
`Clone, Copy, Debug, PartialEq, Eq` — with `PartialOrd`/`Ord` emitted on the
inner integer (#761), since comparing a bounded numeric is exactly what it is
for — plus a `FromStr` that trims, parses the inner integer, then enforces the
declared `min`/`max` bound (the single chokepoint); a `value()` accessor and a
`From<Self>` for the inner integer (the idiomatic extraction, mirroring the ID
trailer's `From<Self> for i64`); `Display`; a **compile-checked `Default`**; and
a **validating** transparent-integer serde bridge (deserialize re-runs the
bound, so an out-of-range value is rejected on the wire, exactly as a string
newtype's serde rejects a malformed string) — and, on the same principle, a
**validating sqlx bridge** (ADR-0071) whose `Decode` re-runs the bound at the
column. The inner integer type (`u32`/`usize`/`i64`/…) and the bounds are
per-type, so — unlike the ID trailer, which is fixed — the numeric-value trailer
is **parameterized** (see §3). The range case (`min` + `max`) may additionally
opt into **`clamp`**, which emits `MIN`/`MAX` associated consts and an
infallible `const fn clamped(inner) -> Self` that coerces its argument into
range. `clamped` is a **validated** door — it cannot yield an out-of-range
value, so it does not weaken the invariant — and it is **opt-in**, so non-range
or non-clamping numeric newtypes don't silently gain coercion. Its use case is a
public bound that should **coerce** an out-of-range request rather than reject
it on the wire (the AtomPub `?limit=` page size). First users: `RetentionCount`
(#455), `FeedMinItems`/`FeedMinDays` (#535); `PageSize` (#537, the first `clamp`
adopter).

**Min-only saturating doors** are the same idea where only a lower bound exists,
and are **hand-written** rather than generated: `clamp` requires both bounds,
and a type with a meaningful floor but no principled ceiling should not invent
one just to unlock the generated `clamped`. `RowLimit::at_most(n)` (#696)
saturates a value below its `min` up to it, which makes it a **validated** door
in the same sense — it cannot yield an out-of-range value, so it does not weaken
the invariant. The constraint that keeps it honest: it is for values **derived
internally** (a literal cap, a scan batch size), not for user input, which still
goes through `FromStr`, the serde bridge, or `clamped`. Saturating a caller's
`0` to `1` is fine; silently accepting a client's is not.

**An unsigned `inner` is not a substitute for a declared bound.** A `NumNewtype`
whose value crosses the `sqlx` boundary should declare its `min` rather than
lean on `u32` to express "non-negative", because sqlx implements no Postgres
`Encode` for unsigned types: the value is widened to `i64` at every bind, so the
primitive's range is discarded exactly where the database could act on it, while
a declared bound is re-run by `FromStr`, the serde bridge, and the sqlx
`Decode`. #696 moved `PageOffset` from `inner = u32` (bound implied) to
`inner = i64, min = 0` (bound declared) for this reason, and gave `RowLimit`
`min = 1` from the start. **The trap to name explicitly:** changing `inner` to
`i64` _without_ declaring the `min` makes the bind gate green while deleting the
only guarantee the type carried — the change looks like a widening and is
actually a removal.

**String truncating door.** The string analog of `clamped` is a **hand-written**
`truncated(&str) -> Self` on a length-bounded `str`-newtype: it trims and
truncates to the cap, yielding an infallible **validated** door that cannot
exceed the length bound (so it does not weaken that half of the invariant) for
values **derived internally** rather than submitted — the way
`RenderedHtml::from_trusted` is a trusted rebuild door. Unlike `clamped` it is
per-type (not a macro flag) and not `const` (trim/char-boundary aren't const),
and it is a **trust** door for the non-length half of the invariant: a
`truncated` on a non-empty-plus-cap type guarantees only the cap, not
non-emptiness, so its callers must supply non-empty input (pinned by a
`debug_assert!`). Reach for it only when a value is minted from a known-valid
internal source that should be coerced-to-fit rather than rejected. First user:
`PostSummary` (#545), whose derived fallback summary label is built from a
post's body line, title, or slug.

### 3. The trailer is generated, not hand-written

The trailer is mechanical and identical across types, so it lives in a
`#[derive(StrNewtype)]` (and `#[derive(IdNewtype)]` / `#[derive(NumNewtype)]`)
proc-macro in the **`macros` crate** (ADR-0062) — its second tenant. For a
**string** newtype the derive generates everything except `FromStr` **and the
std `#[derive]`s** (`Clone`/`Debug`/`PartialEq`/`Eq`/`Hash`/`Copy`). `FromStr`
stays hand-written because the validation/normalization rule is the one
genuinely per-type part. (A **numeric** `IdNewtype` has no such rule, so it
_generates_ its `FromStr` too — a non-validating delegate to `i64`'s parse, per
§2.) A **numeric value** (`NumNewtype`) _also_ generates its `FromStr`, but a
**validating** one: because a numeric bound is declarative, the rule is not
per-type prose but attributes —
`#[num_newtype(inner = u32, min = 1, default = 20)]` (optional `max`, `error`) —
from which the derive emits the bound-checking `FromStr`, a `value()` accessor
and `From<Self>` for the inner integer, `Display`, a compile-checked `Default`,
a self-contained error type (no `thiserror` in emitted code), and the validating
serde bridge — plus, under an opt-in `clamp` flag (which requires both `min` and
`max`), `MIN`/`MAX` consts and an infallible `const fn clamped` (§2). So a
numeric-value newtype is a struct, a derive, and one attribute line — no
hand-written `FromStr` at all. The remaining std derives stay in the user's
`#[derive(...)]` list so per-type variation is expressed idiomatically (Slug
omits `Hash`, a secret omits `Debug` so the generated redacting one applies).

**Ordering is the exception, and was moved deliberately (#761).** It used to sit
in that list — "Tag adds `Ord`" was the worked example — and that is precisely
what made it a trap: nothing steered an author toward the derive, so the reflex
on hitting a type without `Ord` was to hand-write one. All three macros now emit
`PartialOrd`/`Ord` from a single shared helper, since the code is identical for
a `String` and an integer inner. Because a derive macro cannot append to the
user's `#[derive(...)]` list — rustc does not even show it that attribute — the
impls are emitted blocks, which makes the convention **self-enforcing wherever
the macro actually emits**: for a non-secret, non-`no_ord` newtype, both a
leftover `Ord` in a derive list and a hand-written `impl Ord` are compile
errors. No gate is needed, and none was added.

The qualifier is load-bearing, not throat-clearing. Where the macro emits
_nothing_ — a `secret`, or a type that took `no_ord` — there is nothing to
collide with, so a hand-added `Ord` compiles there and the rule is convention
only (§2's honest limit). That is also why the "no `Ord` in a newtype's derive
list" sweep is scoped to non-secrets rather than stated absolutely.

The serde bridge is emitted as **direct `Serialize`/`Deserialize` impls**, not a
`#[serde(try_from/into)]` attribute (serialize borrows instead of cloning into a
`String`; deserialize routes through `FromStr` so invalid input is rejected on
the wire). No inherent `as_str()` is generated — the `str` traits replace it. A
new domain newtype is then a struct, a derive, and a `FromStr` — not 40 lines of
boilerplate that drift apart over time.

For a value with **no rule that can fail** — one that only normalizes, or wraps
verbatim, and for which no input is invalid — `#[str_newtype(infallible)]`
supplies the trailer's `From<String>`-when-infallible half (§2): the author
hand-writes `From<String>` instead of `FromStr`, the derive omits
`TryFrom<String>` (which would collide with it via the std blanket
`impl<T, U: Into<T>> TryFrom<U>`) and routes `Deserialize` through that
`From<String>`.

**Choose it on the invariant, not on the signature** —
[ADR-0101](0101-infallible-kind-is-invariant-first.md) decides this and
supersedes the wording this paragraph used to carry ("for a value whose
invariant never rejects"). The reviewer's question is _is there a string this
type should refuse?_, not _does the constructor reject?_ — the latter is a
property of the code already written, and reading it as evidence about the value
is what mislabelled both of this section's original first users.

The diagnostic: **if a type declared `infallible` needs a downstream gate to
reject some of its values, it was mis-declared.** The gate is the invariant,
displaced. The first-users list is **gone rather than updated**: `PostTitle` was
corrected in #830 and `PostBody` in #811, so neither is an infallible newtype
any more, and no production type takes the flag today (ADR-0101 decision 2).

**2026-08-24 correction.** The final present-state sentence above became stale
when `SubscriberRef` adopted the flag during #750. The #857 audit then applied
ADR-0101's question, found that blank strings must be refused, made
`SubscriberRef` validating, and removed the separate zero-user macro mode. The
Decision text remains historical; current architecture follows
`docs/adr/0151-subscriber-reference-invariant.md`.

### 4. Boundary rule

Parse into the newtype at the **outermost** boundary — `#[server]` argument and
return types, CLI argument types, storage record fields and trait signatures —
and hold the newtype inward. Because the trailer gives `Deref<Target = str>`, a
storage method can still take `&str` internally and a caller holding a
`Username` passes `&username` unchanged; the type is not a tax on the read path.
This is the shape #14 needs: storage returns the newtype, the DTO field _is_ the
newtype, and no `parse().expect()` appears at the web boundary.

### 5. Use an existing newtype everywhere its value appears

Once a domain newtype exists, **every** field, argument, return, DTO, **and
serialization/DTO surface** that carries that value is typed as the newtype.
Flattening it to a primitive requires **express owner approval**, recorded in
the issue/spec — it is not a discretionary per-site call, and "it is only a
serialization / DTO / feed-native surface" is not itself a reason.

This is distinct from §1. §1's "consistency alone is not sufficient
justification" governs whether to **introduce a new** type; it must **not** be
cited to leave an **existing** newtype's value as a primitive. Adoption of an
existing type is mandatory, not consistency-optional. The §4 boundary
enumeration (`#[server]`, CLI, storage) is likewise **non-exhaustive**: internal
serialization/DTO surfaces hold the newtype too. A field whose value is sourced
from a newtype-typed record field _is_ that type, and must be declared as it —
not re-derived as a bare `String` with a `.to_string()` / `String::from` /
`.map(String::from)` at the assignment.

**Carve-outs — external types and wire decoders.** Handing the inner value to a
type we do not own (e.g. `atom_syndication`, the `rss` crate,
`serde_json::Value`) is a sanctioned flatten; read the value out via
`Deref`/`AsRef`/`Display`/`Serialize` at that boundary. The newtype must still
be held on every surface _we_ define up to that point.

**Wire-decoding test doubles.** Amended by
[#688](https://github.com/jaunder-org/jaunder/issues/688). The same carve-out
covers **test doubles that decode the wire** — an axum `Form`/`Json` extractor
in a spawned test server, a capture-file parser. Decode into the primitive and
validate explicitly in the test body: a validating field turns a malformed send
into a transport-layer rejection the test never observes, instead of a readable
assertion diff. Instances: `HubForm` (`server/src/websub/http.rs`) and `Resp`
(`server/tests/web/web_auth.rs`), whose production counterpart
`web/src/auth/api.rs LoginResponse` _is_ typed.

This does **not** extend to **in-process doubles**, which receive already-typed
values with no serialization hop and so take the newtype like any other surface
we define (`CapturedPing`, `server/tests/helpers/websub_capturing.rs`). The
distinction is whether the double is observing bytes or receiving values.

**2026-08-27 adoption — `IdempotencyKey`.** Issue #1086 adopts `IdempotencyKey`
as a non-secret string-backed domain value. It prevents a retry key from being
transposed with another string while it crosses AtomPub, post creation,
duplicate lookup, and persistence. Its `FromStr` trims outer whitespace, rejects
an empty result, and preserves every other string; the standard trailer supplies
the validating serde and SQLx bridges.

AtomPub retains one compatibility seam before that outermost parse:
`HeaderValue::to_str` may reject a header value (including non-ASCII UTF-8 bytes
and invalid UTF-8), and missing, rejected, or blank-after-trimming values mean
no key rather than a `400`. Only readable, non-blank header text becomes an
owned `IdempotencyKey`; every Jaunder-defined surface thereafter remains typed,
with borrowed keys for orchestration and lookup and owned keys for creation
input and persistence. This type-only adoption needs no schema migration. The
frozen #697 historical artifacts remain unchanged.

> **Annotation (2026-08-27).** As of #847, the `Password` example had moved to
> `host`; the dual-target `ProfferedPassword` validation path and common-owned
> `RenderedHtml` remained. The convention and their validation/trust invariants
> remained unchanged. Current ownership: [ARCHITECTURE.md](../ARCHITECTURE.md).

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
- **A second proc-macro.** `macros` gains `StrNewtype`/`IdNewtype` (and later
  `NumNewtype` for bounded numeric values, #535). Per ADR-0062 the crate is
  build-time only with no runtime footprint; the derive must stay a pure code
  generator (it emits trait impls, nothing observable at runtime).
- **Incremental, never big-bang.** Threading a type through storage and the DTOs
  is a large mechanical diff. Each value class is its own reviewable change
  (`preparatory refactor` → behavior unchanged), never one sweeping commit.
- **What this rules out.** Bare primitives for values with an invariant, a
  transposition hazard, or a trust boundary; string-newtypes for genuinely
  polymorphic values (use an enum); and "consistency-only" newtypes for
  free-form local strings. When a value is borderline, the crossing-a-boundary
  test in §1 decides.
- **Pervasiveness closes a loophole (§5).** An existing newtype cannot be left
  as a primitive on a "generic serialization surface" without express approval —
  §1's consistency caveat governs _introducing_ a type, not _adopting_ one.
  `FeedItem.title`/`content_html` (#470), flattened to `String` under exactly
  that reasoning during the #402/#398 sweeps, are the first correction.
- **Ordering moved from the derive list into the trailer (#761), and that is a
  reversal.** §3 previously made "the std derives stay in the user's list" the
  mechanism for per-type variation, with `Ord` among them. Two consequences
  follow. `PartialEq`/`Eq` are now **required** on every non-`no_ord` newtype,
  because `Ord: Eq` — a constraint the old rule did not impose. And the ~12
  types that documented an omitted `Hash` as "never a map/set key" had their
  rationale falsified, since ordering makes them `BTreeMap` keys; those comments
  were corrected rather than left to mislead the next author. The prompt was
  #711, where the absence of `Ord` on three members produced a hand-written
  `impl` over their string views — the exact reflex the trailer exists to
  prevent, in the one place the trailer did not reach.
