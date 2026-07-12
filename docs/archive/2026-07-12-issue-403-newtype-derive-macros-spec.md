# Spec — issue #403: `StrNewtype` / `IdNewtype` derive macros in the `macros` crate

- Issue: [#403](https://github.com/jaunder-org/jaunder/issues/403)
- Milestone: #13 Domain-value type safety (newtypes)
- ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md) (this issue
  implements §3 and amends the secret exception + ID trailer — see "ADR
  amendment" below); [ADR-0062](../../adr/0062-macros-crate-proc-macro-home.md)
  (build-time-only constraint)
- Date: 2026-07-12

## Goal

Add two derive macros to the `macros` crate — `#[derive(StrNewtype)]` for
`struct X(String)` and `#[derive(IdNewtype)]` for `struct X(i64)` — that
generate the mechanical ADR-0063 trailer so a new domain newtype is a struct,
its std `#[derive]`s, and a hand-written `FromStr`, with **zero** hand-written
trailer boilerplate. This is foundation issue #1 of the milestone; #404-family
(see "Separable concerns") retrofits the existing newtypes onto it.

## Scope

**In:** the two derives, their generated trait surface, unit tests (fixtures +
`compile_fail` doctests), the `syn`/`quote`/`proc-macro2` dependencies (+
`serde` and `serde_json` **dev**-dependencies for the fixtures), and the
ADR-0063 amendment.

### Dependencies

- **Regular:** `syn`, `quote`, `proc-macro2` (workspace deps). `syn`'s
  **default** features suffice — parsing a single-field-tuple `DeriveInput` plus
  the `attributes(str_newtype)` helper needs `derive`+`parsing`, not `full`.
- **Dev:** `serde` (with `derive` not needed) and `serde_json`, used only by the
  fixture tests/doctests. Dev-deps are not linked into dependents, so ADR-0062's
  build-time-only, no-runtime-footprint guarantee holds.
- Generated impls need no local temporaries, so macro hygiene risk is nil; they
  carry `#[automatically_derived]`.

**Out:** converting any existing newtype (`Username`/`Slug`/`Tag`/`Password`) or
any storage/DTO/boundary threading. Those are downstream verticals. #403 proves
the derives against **fixture** newtypes only; the four real types stay
untouched.

## Design decisions (resolved in the design interview)

1. **The macro owns only the non-`#[derive]`-able trailer.** Std traits
   (`Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `PartialOrd`, `Ord`, `Copy`)
   stay in the user's `#[derive(...)]` list, so the non-uniformity of the real
   types is expressed idiomatically (Slug omits `Hash`, Tag adds `Ord`, Password
   derives only `Clone`) with no macro-side opt-in/opt-out attribute. The one
   exception is the secret redacting `Debug` (decision 3).

2. **The macro owns the serde bridge**, emitted as **direct `impl Serialize` /
   `impl Deserialize` blocks** (not by emitting a `#[serde(try_from/into)]`
   attribute — avoids two-macro ordering/visibility fragility and a hard
   dependency on the caller also deriving `Serialize`/`Deserialize`). So a
   `StrNewtype` carries neither `#[derive(Serialize, Deserialize)]` nor
   `#[serde(...)]`. The generated code references `::serde` paths, which resolve
   at the call site (all newtype homes — `common` — depend on `serde`); the
   `macros` crate itself gains **no** serde _runtime_ dependency (only dev-deps
   for the fixture tests — see Dependencies). Emitting `serialize_str(&self.0)`
   directly reproduces the existing wire form exactly (`"alice"`) _and_ is
   strictly better than today's `#[serde(into = "String")]`, which clones the
   inner value into an owned `String` per serialization; the direct impl
   borrows. Generated code references `serde` via the absolute path `::serde`,
   so it resolves only in a consuming crate that has `serde` as a **direct**
   dependency named `serde` (true for `common`; a standing precondition for any
   future newtype home). Each generated impl carries `#[automatically_derived]`.

3. **Secret variant** — selected by `#[str_newtype(secret)]` (a `str_newtype`
   helper attribute declared by the `StrNewtype` derive). Its surface is
   **tight**: expose the minimum to construct a secret and to let authorized
   code read its bytes by _explicit_ borrow; emit nothing that invites
   accidental escape or non-constant-time comparison. Concretely, secret gets
   **`AsRef<str>` only** for byte access — **not `Deref<str>` and not
   `Borrow<str>`** — because `Deref<str>` makes `&secret` _implicitly_ coerce to
   `&str` (and reopens owned extraction via `str::to_owned`/`to_string`), which
   is precisely the silent telemetry-leak path ADR-0011 forbids;
   `AsRef::as_ref()` forces an explicit call at each read. See the "Generated
   surface" table.

4. **No inherent `as_str()`.** Per ADR-0063's "no more",
   `Deref<str>`/`AsRef<str>` replace it. Consequence: existing `x.as_str()`
   sites stop compiling when a type adopts the derive — the #404-family
   verticals convert them (`&x` / `x.as_ref()`), which is why #404 is split
   per-type (decision recorded under Separable concerns).

5. **`IdNewtype` generates a transparent-i64 serde bridge** — serialize the
   inner `i64`, deserialize an `i64` and wrap (infallible; an ID has no value
   invariant, only the transposition guarantee). Wire form is unchanged
   (`"post_id": 42`), so downstream DTO typing is non-breaking.

6. **Testing:** runtime fixtures in `macros/tests/` exercise the full positive
   surface; `compile_fail` doctests on the derives lock the secret negatives and
   input-shape rejection. **No trybuild** (no new dep; no brittle `.stderr`
   snapshots that churn on rustc upgrades). Because `compile_fail` only asserts
   _that_ compilation fails, not _why_, each such doctest must **isolate a
   single offending line** with all surrounding code known-good, and the
   positive fixtures must prove that same surrounding code compiles — so a false
   pass (failing for an unrelated reason, e.g. a missing import) is implausible.
   (Doctests in a `proc-macro = true` crate work: they compile as dependent
   crates and can name the crate's exported derives.)

7. **Input-shape hygiene:** each derive emits a clear `compile_error!` when
   applied to anything but a single-field tuple struct, so misuse fails legibly.

**Considered and deferred — a generated/opt-in "infallible" `FromStr`.** We
weighed having the derive supply an accept-anything `FromStr` (`Ok(Self(s))`),
overridable per type. Rejected for #403: (a) a derive can't emit an
_overridable_ default — a generated `impl FromStr` collides with a hand-written
one (E0119), so it could only ever be an explicit `#[str_newtype(infallible)]`
opt-in, never a silent default; (b) `FromStr` is ADR-0063's single validating
**and normalizing** chokepoint, so a default-accept silently mints invalid-state
/ un-normalized values — the exact hazard this milestone exists to kill — and
per ADR-0063 §1 a genuinely no-invariant string shouldn't be a newtype at all,
leaving almost no type to serve. If a real no-invariant string newtype ever
appears, add the explicit `#[str_newtype(infallible)]` opt-in then (YAGNI); not
now.

## Generated surface (precise)

For `struct X(String)` / `struct X(i64)`. "user-derived" = the user writes it in
`#[derive(...)]`; "generated" = the macro emits the impl.

| Item                                                 | `StrNewtype` (non-secret)                                                       | `StrNewtype` **secret**                                                       | `IdNewtype`                                                      |
| ---------------------------------------------------- | ------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Ord`…  | user-derived                                                                    | user-derived (**omit `Debug`**)                                               | user-derived                                                     |
| `Copy`                                               | —                                                                               | —                                                                             | user-derived                                                     |
| `FromStr`                                            | hand-written (required)                                                         | hand-written (required)                                                       | — (n/a)                                                          |
| redacting `Debug`                                    | —                                                                               | **generated** (`X([redacted])`)                                               | —                                                                |
| `Display`                                            | generated (`write_str(&self.0)`)                                                | **omit**                                                                      | generated (`Display` of the `i64`)                               |
| `Serialize`/`Deserialize`                            | generated (deserialize via `FromStr`, so invalid input is rejected on the wire) | **omit**                                                                      | generated (transparent `i64`; deserialize is an infallible wrap) |
| `AsRef<str>`                                         | generated                                                                       | **generated** (explicit read)                                                 | —                                                                |
| `Borrow<str>`                                        | generated                                                                       | **omit** (map-key use; inert without `Eq`/`Hash`)                             | —                                                                |
| `Deref<Target = str>`                                | generated                                                                       | **omit** (implicit `&str` coercion + `to_owned`/`to_string` escape; ADR-0011) | —                                                                |
| `TryFrom<String>` (`Error = <Self as FromStr>::Err`) | generated                                                                       | **generated**                                                                 | —                                                                |
| `From<Self> for String`                              | generated                                                                       | **omit** (owned plaintext must not escape)                                    | —                                                                |
| `PartialEq<str>`, `PartialEq<&str>`                  | generated                                                                       | **omit** (non-constant-time compare)                                          | —                                                                |
| `From<i64>`, `From<Self> for i64`                    | —                                                                               | —                                                                             | generated                                                        |

Secret therefore emits **exactly**: redacting `Debug`, `AsRef<str>`,
`TryFrom<String>` — and omits `Display`, serde, `Borrow`, `Deref`,
`From<Self> for String`, and value-`PartialEq`. A secret is thus constructible
and its bytes readable **only via an explicit `.as_ref()`**; it can never
Display, (de)serialize, implicitly coerce to `&str`, extract an owned `String`,
or value-compare.

Relative to Password today (which has _none_ of the trailer — only `Clone`, a
redacting `Debug`, `FromStr`, inherent `as_str()`, `hash()`, `verify()`), #404's
Password conversion **preserves** the exact redacting-`Debug` string and adds
only explicit borrowed read (`AsRef`) + fallible owned construction
(`TryFrom<String>`); it **removes** the inherent `as_str()` (decision 4 — a
source change #404 sweeps to `.as_ref()`). No _runtime_ behavior changes; the
surface change adds nothing that can leak or compare the secret.

## ADR amendment (record with `jaunder-adr`)

Amend ADR-0063 to match the resolved design (its status is still "proposed"):

- **Secret-bearing exception** broadened: a secret newtype omits not just
  `Display` but also the **serde bridge**, **`Deref<str>`** and
  **`Borrow<str>`** (`Deref`'s implicit `&str` coercion + `to_owned`/`to_string`
  escape is the telemetry-leak path ADR-0011 forbids),
  **`From<Self> for String`** (owned plaintext extraction), and
  **`PartialEq<str>`/`<&str>`** (non-constant-time comparison); it retains
  redacting `Debug`, **explicit** borrowed access via `AsRef<str>` only, and
  `TryFrom<String>`.
- **Numeric-ID trailer** gains "+ a transparent-i64 serde bridge".
- **Reconcile §3's "generates everything except `FromStr`":** post-issue the
  derive generates the trailer except `FromStr` **and the std `#[derive]`s**
  (`Clone`/`Debug`/`PartialEq`/`Eq`/`Hash`/`Ord`/`Copy`), which stay in the
  user's derive list so per-type variation (Slug no `Hash`, Tag adds `Ord`,
  secrets omit `Debug`) is expressed idiomatically.
- Note the trailer is emitted by the derive as **direct impls** (no `#[serde]`
  attribute), and that no inherent `as_str()` is generated (already implied by
  "no more"; make it explicit so #404 reviewers expect the call-site sweep).

## Divergences from issue #403's "Direction" text (resolved in interview)

The spec refines two phrasings in the issue body; both are internally consistent
and match the real types, but are noted so a spec-vs-issue conformance check
doesn't misread them as gaps:

- Issue: _"`IdNewtype` — Generates `Copy`."_ Spec: `Copy` is **user-derived**
  (decision 1 — the macro owns only non-`#[derive]`-able impls). Every ID type
  writes `#[derive(…, Copy, IdNewtype)]`.
- Issue: secret _"swaps the derived `Debug`/`Display` for a redacting form."_
  Spec: secret gets a redacting `Debug` and **no `Display` at all** (Password
  has none today; a secret should not render).

## Acceptance criteria (observable)

1. `macros` exports `#[derive(StrNewtype)]` and `#[derive(IdNewtype)]`;
   `StrNewtype` declares a `str_newtype` helper attribute accepting `secret`.
2. A fixture `struct Code(String)` with a hand-written validating `FromStr` and
   `#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]` compiles and, in a
   `macros/tests/` test, demonstrably provides: `TryFrom<String>` (ok **and**
   err), `From<Code> for String`, `AsRef<str>`, `Deref` (a `str` method called
   directly + `&code` coercing to `&str`), `Borrow<str>` (a `HashSet<Code>`
   probed with a `&str` key returns a hit), `Display`, `PartialEq<&str>` against
   a literal (`code == "abc"`) **and** `PartialEq<str>` against a `str`-typed
   place (e.g. `code == *s` where `s: &str`), and a serde round-trip through
   `serde_json` where a **valid** string deserializes and an **invalid** string
   yields a serde error (proving wire validation flows through `FromStr`).
3. A fixture secret `struct Secret(String)` with `#[str_newtype(secret)]` and
   `#[derive(Clone, StrNewtype)]` (no `Debug` derive) provides a redacting
   `Debug` whose output equals an exact `Secret([redacted])`-style string and
   contains no substring of the inner value, and provides `AsRef<str>` and
   `TryFrom<String>` (ok **and** err).
4. A fixture `struct Id(i64)` with
   `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IdNewtype)]` provides
   `From<i64>`, `From<Id> for i64`, `Display`, `Copy` semantics (usable after a
   copy), and a `serde_json` round-trip where `Id(42)` serializes to `42` and
   `42` deserializes to `Id(42)`.
5. `compile_fail` doctests demonstrate that, for a secret newtype, **each** of
   `format!("{}", secret)` (no `Display`), `serde_json::to_string(&secret)` (no
   serde), `String::from(secret)` (no owned extraction), `secret == "x"` (no
   value-`PartialEq`), and `let _: &str = &secret;` (no `Deref` coercion) fails
   to compile — each isolated on its own line with known-good surrounding code
   (per decision 6), and each surrounded by the same code proven to compile in
   the positive secret fixture.
6. `compile_fail` doctests demonstrate each derive rejects a
   non-single-field-tuple struct with a `compile_error!`.
7. `cargo xtask validate --no-e2e` is clean. No clippy `#[allow]`/`#[expect]`
   waivers appear in **hand-written** code (generated impls carry
   `#[automatically_derived]`, which suppresses the usual lints on expansions);
   `macros` remains build-time-only per ADR-0062 — no coverage surface.
8. ADR-0063 is amended as above.

## Separable concerns (file in the plan's first task, via `jaunder-issues`)

- **Split #404** ("retrofit all four existing newtypes") into **four per-newtype
  verticals** — `Username`, `Slug`, `Tag`, `Password` — each a self-contained
  sweep of that type's `as_str()`/`String` call sites onto the derive. Decided
  in the interview: dropping inherent `as_str()` makes each retrofit a larger
  mechanical sweep, so per-type keeps each reviewable. #403 does not depend on
  this split; it only needs the derives to exist.
