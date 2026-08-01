# Spec — issue #761: ordering joins the newtype standard trailer

- Issue: [#761](https://github.com/jaunder-org/jaunder/issues/761)
- Date: 2026-07-31
- Amends: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md) (in
  place)

## Problem

ADR-0063 §2 says to "derive `Clone, Debug, PartialEq, Eq, Hash` (add `Ord` when
the type is used as a sort/map key)", and §3 makes "the std derives stay in the
user's `#[derive(...)]` list" the mechanism for per-type variation. So ordering
is opt-in, spelled per type, and nothing steers an author toward it.

#711 hit the resulting trap. `MediaRef` needed an ordering (extraction collects
into a `BTreeSet` so a body yields one deterministic, deduplicated row set).
None of its three members implemented `Ord`, so the first attempt **hand-wrote**
`Ord` over their string views — even though all three derive it trivially, and
`ByteSize` in the same file already opts in by hand. The standard trailer exists
to prevent exactly that reflex everywhere else; for ordering it does not.

Two facts make the default worth flipping rather than documenting harder:

- **A string newtype is a string.** Lexicographic order on the inner value is
  the only sane ordering and is what any hand-written impl would spell anyway.
- **`Borrow<str>` already imposes the obligation.** `Borrow`'s contract requires
  `Ord`/`Eq`/`Hash` to agree with the borrowed form. Every non-secret
  `StrNewtype` is `Borrow<str>` today, so _if_ such a type has `Ord` it must
  already agree with `str`'s — which is what delegating to the inner `String`
  gives, and what a hand-written impl can get wrong.

## Decision

**Every non-secret newtype orders on its inner value, emitted by the macro.**
One rule across all three macros; the only escape is an explicit opt-out.

### 1. What the macros emit

`StrNewtype` (`Kind::Default` and `Kind::Infallible`), `NumNewtype`, and
`IdNewtype` each emit `impl PartialOrd` + `impl Ord` delegating to the inner
value. To satisfy clippy's `non_canonical_partial_ord_impl`, `partial_cmp` reads
`Some(self.cmp(other))` and `cmp` delegates to the inner value.

`Kind::Secret` emits neither. A secret already takes the tight surface (no
`Display`, `Deref`, `Borrow`, `PartialEq`), and ordering secret material is both
meaningless and the wrong affordance to hand out by default.

The impls are hand-written blocks, not additions to the user's `#[derive(...)]`
list — a derive macro cannot append to that list, and rustc does not show it the
`derive` attribute, so the macro also cannot detect what the author already
derived.

**Consequence: `PartialEq + Eq` becomes mandatory.** `Ord: Eq` and
`PartialOrd: PartialEq`, so every `NumNewtype` and `IdNewtype` must now derive
both, with no escape, and every `StrNewtype` must unless it takes `no_ord`.
Every production type in the tree already complies. Three **doc fixtures** do
not and must be updated: `macros/src/lib.rs:50` (`Ok1`, `Kind::Default`),
`macros/src/lib.rs:146` (`Inf`, `infallible`), and `macros/src/lib.rs:178`
(`Id`, `IdNewtype`) each derive `Clone`(`, Copy`) and nothing else. They gain
`PartialEq, Eq`.

### 2. `#[str_newtype(no_ord)]` — the opt-out

`no_ord` lives on `str_newtype` only. Its one **production** user is `RawToken`
(`common/src/token.rs`), which is `Kind::Default` and deliberately derives no
`PartialEq`/`Eq` — the bearer-token profile keeps the type distinction rather
than value comparison. Unconditional emission would not compile there.

`no_ord` is valid with `infallible` (an infallible newtype can lack `Eq` for the
same reasons a default one can) and **invalid** with `secret` (already
unordered), matching the existing `no_sqlx` treatment exactly.

`no_ord` is **not** added to `num_newtype` or `id_newtype`. `IdNewtype` has no
attribute parser at all today, and an unused branch in either macro would be
surface with no caller that the uncovered-line gate would still demand a fixture
test for. Note this rests on a **narrower** claim than "every num/id newtype
already derives `Eq`" — the `IdNewtype` doc fixture at `macros/src/lib.rs:178`
does not, and is fixed by adding the derives (§1) rather than by an opt-out. Add
the flag when a real type needs it.

### 3. Enforcement is structural — for non-secrets

For every type where the macro emits ordering, both failure modes the issue
wants to prevent become compile errors on their own: a hand-written `impl Ord`
collides with the emitted one, and an `Ord`/`PartialOrd` left in a derive list
collides too. That collision makes removing the two existing opt-ins
**mandatory**, not cleanup.

**The secret side has no such guarantee.** Because the macro emits nothing for
`Kind::Secret`, `#[derive(PartialOrd, Ord, StrNewtype)] #[str_newtype(secret)]`
compiles cleanly. "A secret does not order" is therefore a **default, not a
structural invariant**. We accept that: the macro cannot see the derive list, so
the only alternative is a bespoke xtask gate, which is disproportionate to a
hazard nobody has hit. ADR-0063 states it as a convention, and the acceptance
criteria scope the no-`Ord`-in-derive-lists sweep to non-secret types
accordingly.

### 4. Capability, not intent

`StrNewtype`s that omit `Hash` split into two groups. **Seven** state a
rationale that this change falsifies — "never a map/set key": `Slug`, `Bio`,
`AudienceName`, `DisplayName`, `SessionLabel`, `PostSummary`, `TagLabel`.
**Four** omit `Hash` silently, with no rationale to correct: `SmtpUsername`,
`SiteTitle`, `BackupSchedule`, `DestinationPath`. Separately, **three** types
make an explicit "no `Ord`" claim: `AbsoluteUrl` ("URLs are not sorted"),
`RootRelativeUrl`, and `FeedPath` ("feed paths are never sorted").

Default-on ordering makes all of them `BTreeMap`/`BTreeSet` keys, so "never a
key" would survive only for `HashMap` — not a coherent line.

ADR-0063 resolves this by stating that the trailer grants **capability** and
never encoded intent: trait omission was never enforcement (any author can add
`Hash` in one line). `Hash` stays a per-type decision because a hash has a real
cost story — it is not free to compute and invites the type into hot paths. A
total order does not.

Types with a **hand-written** trailer (`RenderedHtml`, `ProfferedFilename`) do
not use these macros and are unaffected by design.

### 5. Sequencing with #711

#711 is in flight and adds `PartialOrd, Ord` to
`ContentHash`/`Filename`/`MediaSource` — exactly the derive-list entries this
change makes illegal. #711 lands on its own terms; it needs those derives to
compile today and is not blocked on a convention change. The removal task here
is written against whatever is on `main` at ship, and the sweep is **re-run as
the last act before merge** rather than trusting the inventory taken at spec
time.

## Acceptance criteria

Each is stated so a conformance review can tell delivered from not.

**AC1 — string default trailer orders.** For a `Kind::Default` fixture newtype
in `macros/tests/str_newtype.rs`: `a < b`, `vec.sort()`, and `BTreeSet`
insertion compile and agree with the same operations on the inner `&str`. These
three spellings are the discriminators; `a.cmp(&b)` is **not** — it compiles
today through `Deref<Target = str>` and so proves nothing.

**AC2 — infallible trailer orders.** The same three discriminators hold for a
`#[str_newtype(infallible)]` fixture in the same file.

**AC3 — secret does not order.** `compile_fail` doctests (not `trybuild` — it is
not a dependency and was explicitly rejected for this macro family) show `a < b`
failing for both a `#[str_newtype(secret)]` and a
`#[str_newtype(secret, serde)]` fixture.

**AC4 — numeric and id trailers order.** A new `macros/tests/num_newtype.rs` and
the existing `macros/tests/id_newtype.rs` each show `a < b`, `sort()`, and
`BTreeMap`/ `BTreeSet` insertion agreeing with the same operations on the raw
inner integer. (Note `PageSize::MIN` is typed `#inner`, not `PageSize`, so
`PageSize::MIN <= x` is ill-typed and must not be used as the assertion.)

**AC5 — `no_ord` opts out.** A `#[str_newtype(no_ord)]` fixture retains the full
default trailer (`Display`, `Deref`, serde, sqlx) but fails `a < b` in a
`compile_fail` doctest. A `#[str_newtype(infallible, no_ord)]` fixture compiles,
proving the pair is accepted.

**AC6 — `no_ord` + `secret` is rejected.** The pair produces a spanned
`compile_error!` reading
`a `secret`newtype is already unordered;`no_ord` is redundant/invalid` — the
wording of the existing `no_sqlx` + `secret` error (`macros/src/str_newtype.rs`)
with the two nouns swapped.

**AC7 — `RawToken` compiles unchanged.** `common/src/token.rs` carries
`#[str_newtype(no_sqlx, no_ord)]`, still derives no `PartialEq`/`Eq`, and **all
four** of its existing `compile_fail` doctests still fail to compile.

**AC8 — redundant derives removed.** No `#[derive(...)]` list on a
**non-secret** newtype names `Ord` or `PartialOrd` alongside `StrNewtype`,
`NumNewtype`, or `IdNewtype`. Scoped to non-secrets deliberately: a secret
opting in, and a `no_ord` type re-adding derived `Ord`, are both legal by §2/§3.
Verified by sweep, re-run immediately before merge. Known at spec time: `Tag`
(`common/src/tag.rs`), `ByteSize` (`common/src/media.rs`), plus whatever #711
lands.

**AC9 — no hand-written ordering on newtypes.** No `impl Ord` /
`impl PartialOrd` exists for any macro-derived newtype. (Zero exist today; the
criterion pins that the change does not introduce one.)

**AC10 — falsified doc comments corrected.** Specifically:

- the three explicit "no `Ord`" claims — `AbsoluteUrl`
  (`common/src/absolute_url.rs`), `RootRelativeUrl`
  (`common/src/root_relative_url.rs`), `FeedPath`
  (`common/src/feed/feed_path.rs`);
- the seven "never a map/set key" claims named in §4;
- `Tag`'s "Keeps `Hash`/`Ord`" note (`common/src/tag.rs`), which will describe a
  derive list that no longer says it. (`ByteSize` carries **no** such note —
  nothing to fix there.)
- the macro crate's own contradicting docs: `macros/src/lib.rs:164` and
  `macros/src/lib.rs:193`, plus the module docs at
  `macros/src/str_newtype.rs:3`, `macros/src/id_newtype.rs:4`, and
  `macros/src/num_newtype.rs:8`, all of which state that `Ord` stays in the
  user's derive list.

The four silent `Hash`-omitters (`SmtpUsername`, `SiteTitle`, `BackupSchedule`,
`DestinationPath`) need no edit. Catch-all: no comment in the tree claims a
newtype cannot be ordered, or is "never a key", on the strength of an omitted
derive.

**AC11 — ADR-0063 amended in place.** §2's "add `Ord` when the type is used as a
sort/map key" and §3's std-derive list are updated to state the new rule, the
`no_ord` escape, the secret exclusion (as a convention, per §3 above — not as a
guarantee), and the capability-not-intent principle. The amendment cites #761,
matching the in-place precedent of #400/#458/#535/#537/#545/#696. No new ADR and
no draft. `docs/README.md` is not hand-edited.

**AC12 — the gate is green.** `cargo xtask validate --no-e2e` passes with no new
`cov:ignore`.

## Non-goals

- **Making `Hash` default-on.** Tempting for coherence, but it reverses ADR-0063
  §3's own exemplar and is well past what #761 authorized. §4 states why `Hash`
  stays per-type.
- **Giving `RawToken` `PartialEq`/`Eq`.** Adding value comparison to a bearer
  token to unlock a convention is a security-adjacent change that must not ride
  along.
- **Adding `no_ord` to `num_newtype`/`id_newtype`.** No production caller.
- **An xtask gate pinning secrets un-ordered.** §3 records why.
- **Introducing orderings on hand-written-trailer types** (`RenderedHtml`,
  `ProfferedFilename`).

## Behavioral impact

No call site changes meaning. The naive claim — "ordering two newtypes does not
compile today" — is **false**: for any `StrNewtype` with `Deref<Target = str>`,
`a.cmp(&b)`, `a.partial_cmp(&b)`, `a.lt(&b)`, and `sort_by(|p, q| p.cmp(q))` all
compile today, resolving through `Deref` to `str`'s impls. Only the `<`
operator, `Vec::sort()`, and `BTreeSet` fail. So the change _shadows_ those
existing resolutions with inherent impls rather than enabling something new.

That is safe because both spellings yield identical lexicographic order on the
inner value, and a sweep found no such call site in
`common`/`host`/`web`/`client`. This is a `preparatory refactor` in ADR-0063's
sense, but the reason is the equivalence of the two orderings — not the absence
of a resolution.

## Risks

- **Coverage attribution.** The change adds two emitted impls per type across
  ~41 types. The existing trailer already emits ~13 impls per type
  (`PartialEq<&str>` for `BackupSchedule` is surely never called) and the
  uncovered-line gate is green, which implies proc-macro output collapses onto
  the derive-site span rather than creating per-impl regions. **This is an
  inference, not a verified fact** — it is confirmed empirically at the first
  gate run, and if wrong the change needs a per-type ordering exercise or a
  different emission strategy before proceeding.
