# Newtype Ordering Trailer — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-31-issue-761-newtype-ord-trailer.md`](../specs/2026-07-31-issue-761-newtype-ord-trailer.md)
— the "what" and "why". This plan is the "how" and does not restate it.

**Goal:** Make every non-secret newtype order on its inner value, emitted by the
macro rather than spelled per type.

**Architecture:** One shared `crate::ord_impls(name)` helper in
`macros/src/lib.rs` emits `PartialOrd` + `Ord` delegating to `self.0`; all three
derives call it. The body is identical for `String` and integer inners, so there
is exactly one copy. `#[str_newtype(no_ord)]` suppresses it for `StrNewtype`
only.

**Tech Stack:** Rust 2021, `syn`/`quote`/`proc-macro2`, `cargo nextest`,
`cargo xtask`.

## Review header

**Scope — in:**

- `macros`: a shared ordering emitter, wired into `StrNewtype` (Default +
  Infallible), `NumNewtype`, `IdNewtype`; a `no_ord` option on `str_newtype`.
- `macros`: **enabling refactor** — `Opts`'s two sqlx bools folded into a
  `SqlxMode` enum. Not named in the spec; it is forced by a clippy ceiling (see
  Task 1) and is the one piece of unauthorized-by-the-spec scope in this plan.
  Behavior-preserving, its own commit, rejectable on its own.
- `common`: `RawToken` opts out; `Tag` and `ByteSize` drop their now-colliding
  derive entries.
- Doc corrections across ~11 code sites, 5 in the macro crate, and 2 ADRs;
  ADR-0063 amended in place.

`host/src/invite.rs:26` is the only `host` newtype and is a `secret` —
unaffected, no task touches `host`.

**Scope — out:** making `Hash` default-on; giving `RawToken` `PartialEq`/`Eq`;
`no_ord` on `num_newtype`/`id_newtype`; an xtask gate pinning secrets
un-ordered; hand-written-trailer types (`RenderedHtml`, `ProfferedFilename`).
One separable concern surfaced and is filed rather than folded in — see Task 0.

**Tasks:**

0. **[done during planning]** File the separable concern —
   [#763](https://github.com/jaunder-org/jaunder/issues/763), run doctests in
   CI.
1. Refactor `Opts`'s sqlx controls into a `SqlxMode` enum — pure refactor, makes
   room under clippy's bool threshold.
2. `ord_impls` helper + `no_ord` option + `StrNewtype` ordering; `RawToken` opts
   out; `Tag` sheds its derives.
3. `NumNewtype` ordering; `ByteSize` sheds its derives; new
   `macros/tests/num_newtype.rs`.
4. `IdNewtype` ordering; doc fixture gains `PartialEq, Eq`.
5. Doc-comment corrections (AC10).
6. ADR-0063 amended in place (AC11).
7. Final sweep + full gate (AC8 re-run, AC9, AC12).

**Key risks / decisions:**

- **Task 1 exists because of a lint, not a whim.** `Opts` carries 3 bools;
  clippy `pedantic` is on with `-D warnings` and `struct_excessive_bools` trips
  at 4. Adding `no_ord` naively fails the gate. The existing comment at
  `macros/src/str_newtype.rs:8-10` already flagged this ceiling.
- **Task 2 is atomic and cannot be split.** The moment `StrNewtype` emits
  ordering, `RawToken` (no `Eq`) stops compiling and `Tag`'s derived `Ord`
  collides. Emission, the opt-out, and both call-site fixes must land in one
  commit.
- **The coverage inference is confirmed at Task 2's gate run**, the first time
  emitted impls hit the uncovered-line gate. If ~82 new impls produce per-type
  uncovered regions, stop and re-plan (spec, Risks).
- **Doctests are not run by any gate.** `cargo nextest` does not execute them,
  and the Nix `nextest` check deliberately disables `cargo test`
  (`flake.nix:371-372`); `--doc` appears nowhere in `flake.nix` or `xtask/src`.
  AC3, AC5's negative half, and AC7 rest entirely on `compile_fail` doctests, so
  **every task that touches one must run `cargo test -p <crate> --doc` by hand**
  — nothing downstream will catch a regression there. This is pre-existing repo
  behavior, not introduced here.

## Global Constraints

- No `Co-Authored-By` trailer on any commit.
- Generated `partial_cmp` must read `Some(self.cmp(other))` — clippy's
  `non_canonical_partial_ord_impl` canonical form.
- No new `cov:ignore` markers.
- No `trybuild` dependency — negative compile checks are `compile_fail` doctests
  (recorded rejection:
  `docs/archive/2026-07-12-issue-403-newtype-derive-macros-spec.md:94`).
- Never hand-edit `docs/README.md` — it is a generated projection.
- ADR-0063 is amended **in place**, citing #761. No new ADR, no draft.
- `no_ord` is added to `str_newtype` only.
- Per-commit gate: run `devtool run -- cargo xtask check` before each commit
  (**`jaunder-commit`**).

---

### Task 0: Separable concern — filed, not deferred

- [x] **Filed [#763](https://github.com/jaunder-org/jaunder/issues/763) — "ci:
      run doctests"** (Task, `tooling`/`dx`, added to Jaunder Backlog #1).

Planning turned up that **no gate runs doctests**: `cargo nextest` cannot, and
the Nix package build disables `cargo test` (`flake.nix:372`). The tree holds 36
doctest blocks, **26 of them `compile_fail`** — the mechanism this repo uses to
prove negative type properties (`RawToken` does not convert to `TokenHash`; a
`String` cannot masquerade as a
`ContentHash`/`Filename`/`ContentType`/`ETag`/`RenderedHtml`; the ADR-0063
secret surface omits `Display`/serde/owned-`String`/`PartialEq`/`Deref`). They
all pass today (33 tests, 0.88s, measured) and none is evaluated by CI.

**This is not #761's work** — it is a flake/CI change with its own
coverage-interaction decision — and #761 does **not** depend on it. Filed up
front so it can be picked up concurrently rather than blocked behind this cycle.

It does sharpen one thing here: this plan adds three more `compile_fail` blocks
to that unrun population, and **AC7 is checkable only by a command a human
remembers to type** (Task 2 Step 7). Until #763 lands, that is a known, accepted
weakness of this plan's verification — not something Task 7's gate covers.

---

### Task 1: Fold the sqlx controls into a `SqlxMode` enum

Pure refactor, no behavior change. `Opts` currently holds `serde`, `sqlx`,
`no_sqlx` (3 bools); Task 2 needs a 4th, which `clippy::struct_excessive_bools`
rejects under `-D warnings`. Collapsing the two sqlx bools into one enum field
takes the count to 1 and follows the precedent already set by `Kind` in the same
file.

**Files:**

- Modify: `macros/src/str_newtype.rs:20-31` (`Opts`), `:95-103` (`sqlx_bridge`),
  `:366-443` (`parse_opts`)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: `enum SqlxMode { Default, Forced, Off }` (private to
  `str_newtype.rs`) and
  `struct Opts { kind: Kind, serde: bool, sqlx: SqlxMode }`. Task 2 adds one
  field to this struct.

- [x] **Step 1: Run the existing tests, verify they pass** — 68 passed, 0
      failed.

The four guard paths are already covered —
`str_newtype_no_sqlx_omits_the_bridge` (`macros/src/lib.rs:578`),
`str_newtype_infallible_no_sqlx_omits_the_bridge` (`:629`),
`str_newtype_no_sqlx_with_secret_emits_compile_error` (`:639`),
`str_newtype_no_sqlx_with_sqlx_emits_compile_error` (`:651`). This refactor must
not change what any of them observe, so they are the contract; no new tests are
written. Establish the green baseline first.

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS

- [x] **Step 2: Introduce the enum and rewrite the struct**

Add above `Opts`:

```rust
/// How the sqlx storage bridge is selected — the two `sqlx`/`no_sqlx` flags collapsed into
/// one field so an invalid combination is unrepresentable in `Opts` (mirroring [`Kind`]) and
/// the struct's bool count stays under clippy's `struct_excessive_bools` threshold.
enum SqlxMode {
    /// No explicit flag: the default-on-except-secret policy applies.
    Default,
    /// `#[str_newtype(sqlx)]` — re-adds the bridge to a secret that genuinely is stored.
    Forced,
    /// `#[str_newtype(no_sqlx)]` — opts a non-secret must-not-store type out.
    Off,
}
```

and change `Opts` to `{ kind: Kind, serde: bool, sqlx: SqlxMode }`.

- [x] **Step 3: Rewrite `sqlx_bridge` against the enum**

To signature
`fn sqlx_bridge(opts: &Opts, name: &syn::Ident) -> proc_macro2::TokenStream`,
preserving today's arm-for-arm behavior: a `Secret` with `SqlxMode::Forced` gets
`sqlx_impls`; any other `Secret` gets nothing; `SqlxMode::Off` gets nothing;
`Infallible` gets `sqlx_impls_infallible`; `Default` gets `sqlx_impls`.

- [x] **Step 4: Keep `parse_opts`' local bools, fold at the end**

`parse_opts` keeps parsing into local `sqlx`/`no_sqlx` bools so all four
existing validation guards (`infallible` × `secret`/`serde`; `serde` without
`secret`; `no_sqlx` with `secret`; `no_sqlx` with `sqlx`; bare `sqlx` without
`secret`) stay byte-identical, including their messages. Only the final
construction changes, mapping `(sqlx, no_sqlx)` to `SqlxMode::Forced` /
`SqlxMode::Off` / `SqlxMode::Default`. Locals are not linted, so the bool count
only matters in the struct.

- [x] **Step 5: Run the tests, verify they still pass** — 68 passed, identical
      to the Step 1 baseline.

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS — identical
results to Step 1

- [x] **Step 6: Commit** — `e3f102e0`, gate green (coverage clean, 22858
      executable lines, 0 failures).

```bash
devtool run -- cargo xtask check
git add macros/src/str_newtype.rs
git commit -m "refactor(macros): fold str_newtype sqlx flags into a SqlxMode enum (#761)"
```

> Note for later tasks: pass commit messages via `git commit -F <file>`, not
> `-m "…"`. These messages contain backticks, which a double-quoted `-m` hands
> to the shell as command substitution — the first attempt here silently ran
> `sqlx` and mangled the message.

---

### Task 2: `ord_impls` + `no_ord` + `StrNewtype` ordering

The atomic core. Emission, the opt-out, and both colliding call sites land
together because none of them compiles without the others.

**Files:**

- Modify: `macros/src/lib.rs` (add `ord_impls`; doc fixtures at `:50` and
  `:146`; new `compile_fail` doctests; new unit tests in `mod tests`)
- Modify: `macros/src/str_newtype.rs` (`Opts`, `parse_opts`, `expand`)
- Modify: `macros/tests/str_newtype.rs` (ordering assertions; infallible +
  `no_ord` fixtures)
- Modify: `common/src/token.rs:79` (`RawToken` → `no_ord`)
- Modify: `common/src/tag.rs:18` (`Tag` drops `PartialOrd, Ord`)

**Interfaces:**

- Consumes: `SqlxMode`, `Opts` (Task 1).
- Produces:
  `pub(crate) fn ord_impls(name: &syn::Ident) -> proc_macro2::TokenStream` in
  `macros/src/lib.rs` — Tasks 3 and 4 call this exact function. `Opts` gains
  `ord: bool` (true unless `no_ord` was written).

- [x] **Step 1: Write the failing tests** — 10 new tests: 6 expansion-level, 4
      integration.

In `macros/tests/str_newtype.rs`, against the existing `Code(String)` fixture
(`:9`):

```rust
#[test]
fn ordering_agrees_with_the_inner_str() {
    let a = Code::from_str("aaa").unwrap();
    let b = Code::from_str("bbb").unwrap();
    // `<` is the discriminator: it does NOT resolve through `Deref<str>`.
    assert!(a < b);
    assert!(!(b < a));
    assert_eq!(a.cmp(&b), "aaa".cmp("bbb"));
}

#[test]
fn sorts_and_keys_a_btreeset() {
    let mut v = vec![
        Code::from_str("c").unwrap(),
        Code::from_str("a").unwrap(),
        Code::from_str("b").unwrap(),
    ];
    v.sort();
    assert_eq!(v[0], Code::from_str("a").unwrap());
    assert_eq!(v[2], Code::from_str("c").unwrap());

    let set: std::collections::BTreeSet<Code> = v.into_iter().collect();
    assert_eq!(set.len(), 3);
}
```

Add an infallible fixture and its ordering test to the same file:

```rust
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
#[str_newtype(infallible)]
struct Label(String);

impl From<String> for Label {
    fn from(s: String) -> Self {
        Label(s)
    }
}

#[test]
fn infallible_trailer_orders() {
    let a = Label::from("aaa");
    let b = Label::from("bbb");
    assert!(a < b);

    let mut v = vec![b.clone(), a.clone()];
    v.sort();
    assert_eq!(v[0], a);

    let set: std::collections::BTreeSet<Label> = v.into_iter().collect();
    assert_eq!(set.len(), 2);
}
```

Add a `no_ord` fixture proving the rest of the trailer survives:

```rust
#[derive(Clone, Debug, StrNewtype)]
#[str_newtype(no_ord)]
struct Unordered(String);

impl FromStr for Unordered {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Unordered(s.to_owned()))
    }
}

#[test]
fn no_ord_keeps_the_rest_of_the_trailer() {
    let u = Unordered::from_str("x").unwrap();
    assert_eq!(u.to_string(), "x"); // Display
    let _read: &str = &u; // Deref
    assert!(u == "x"); // PartialEq<str>
    assert_eq!(serde_json::to_string(&u).unwrap(), "\"x\"");
}
```

And in `macros/src/lib.rs`'s `mod tests`, matching the existing
expansion-assertion style:

```rust
#[test]
fn str_newtype_emits_ordering_by_default() {
    let input: DeriveInput = parse_quote! { struct X(String); };
    let out = str_newtype::expand(&input).to_string();
    // Assert on the method names, not the trait names: "Ord" is a substring of
    // "PartialOrd", so `contains("Ord")` can never fail independently.
    assert!(out.contains("fn partial_cmp"));
    assert!(out.contains("fn cmp"));
}

#[test]
fn str_newtype_infallible_emits_ordering() {
    let input: DeriveInput = parse_quote! {
        #[str_newtype(infallible)]
        struct X(String);
    };
    assert!(str_newtype::expand(&input).to_string().contains("fn partial_cmp"));
}

#[test]
fn str_newtype_no_ord_omits_ordering() {
    // This is the real discriminator for `no_ord` — the compile_fail doctest in Step 6
    // passes both before and after the feature exists (unknown option also fails to
    // compile), so it cannot tell the two states apart. This test can.
    let input: DeriveInput = parse_quote! {
        #[str_newtype(no_ord)]
        struct X(String);
    };
    let out = str_newtype::expand(&input).to_string();
    assert!(!out.contains("fn partial_cmp"));
    assert!(!out.contains("fn cmp"));
    // The rest of the trailer is untouched.
    assert!(out.contains("Display"));
}

#[test]
fn str_newtype_secret_omits_ordering() {
    let input: DeriveInput = parse_quote! {
        #[str_newtype(secret)]
        struct X(String);
    };
    assert!(!str_newtype::expand(&input).to_string().contains("fn partial_cmp"));
}

#[test]
fn str_newtype_no_ord_with_secret_emits_compile_error() {
    let input: DeriveInput = parse_quote! {
        #[str_newtype(secret, no_ord)]
        struct X(String);
    };
    assert!(str_newtype::expand(&input).to_string().contains("compile_error"));
}

#[test]
fn str_newtype_infallible_no_ord_is_accepted() {
    let input: DeriveInput = parse_quote! {
        #[str_newtype(infallible, no_ord)]
        struct X(String);
    };
    let out = str_newtype::expand(&input).to_string();
    assert!(!out.contains("compile_error"));
    assert!(!out.contains("fn partial_cmp"));
}
```

- [x] **Step 2: Run the tests, verify they fail** — FAIL as designed: unknown
      `no_ord` option, `<` not applicable to `Code`/`Label`, `Code: Ord`
      unsatisfied. Notably `a.cmp(&b)` did **not** error, empirically confirming
      it resolves through `Deref<str>`.

Run: `devtool run -- cargo nextest run -p macros` Expected: FAIL — `no_ord` is
an unknown option; no ordering is emitted; `a < b` does not compile

- [x] **Step 3: Add the shared `ord_impls` helper**

In `macros/src/lib.rs`, beside `require_newtype_shape`. The body is written out
here because two invariants no test can express: the UFCS-free `self.cmp(other)`
spelling is clippy's canonical `non_canonical_partial_ord_impl` form, and
delegating `cmp` to `self.0` (not to a `str` view) is what keeps the order
consistent with `Borrow<str>` and with derived `PartialEq`.

```rust
/// The ordering half of the ADR-0063 trailer, shared by all three newtype derives (#761):
/// `PartialOrd` + `Ord` delegating to the wrapped value. Identical for a `String` and an
/// integer inner, so there is one copy. `partial_cmp` is written in clippy's canonical
/// `Some(self.cmp(other))` form (`non_canonical_partial_ord_impl`).
pub(crate) fn ord_impls(name: &syn::Ident) -> proc_macro2::TokenStream {
    quote::quote! {
        #[automatically_derived]
        impl ::core::cmp::PartialOrd for #name {
            fn partial_cmp(&self, other: &Self) -> ::core::option::Option<::core::cmp::Ordering> {
                ::core::option::Option::Some(self.cmp(other))
            }
        }

        #[automatically_derived]
        impl ::core::cmp::Ord for #name {
            fn cmp(&self, other: &Self) -> ::core::cmp::Ordering {
                ::core::cmp::Ord::cmp(&self.0, &other.0)
            }
        }
    }
}
```

- [x] **Step 4: Wire `no_ord` through `str_newtype`**

Add `ord: bool` to `Opts` (bool count is now 2 — `serde`, `ord` — under the
threshold thanks to Task 1). In `parse_opts`, accept a `no_ord` key, extend the
unknown-option message to list it, and add one guard placed beside the existing
`no_sqlx && secret` check:

```rust
if no_ord && secret {
    return Err(syn::Error::new_spanned(
        input,
        "a `secret` newtype is already unordered; `no_ord` is redundant/invalid",
    ));
}
```

Set `ord: !no_ord`. In `expand`, append `crate::ord_impls(name)` to the
`Kind::Default` and `Kind::Infallible` arms when `opts.ord`, and never to
`Kind::Secret`. Every branch above is pinned by a Step 1 test.

- [x] **Step 5: Fix the two colliding call sites and the doc fixtures**

- `common/src/token.rs:79` → `#[str_newtype(no_sqlx, no_ord)]`, with a comment
  recording that `RawToken` derives no `PartialEq`/`Eq` by design so ordering
  cannot be emitted.
- `common/src/tag.rs:18` →
  `#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]` (drops
  `PartialOrd, Ord`).
- `macros/src/lib.rs:50` → `#[derive(Clone, PartialEq, Eq, StrNewtype)]` for
  `Ok1`.
- `macros/src/lib.rs:146` → `#[derive(Clone, PartialEq, Eq, StrNewtype)]` for
  `Inf`.

- [x] **Step 6: Add the negative `compile_fail` doctests**

In `macros/src/lib.rs`, following the existing hidden-fixture style
(`#`-prefixed setup lines). One for `secret` and one for `secret, serde` (AC3),
and one for `no_ord` (AC5):

````rust
/// No ordering on a secret:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let a = Sec("a".to_owned());
/// # let b = Sec("b".to_owned());
/// let _ = a < b;
/// ```
````

Repeat verbatim with `#[str_newtype(secret, serde)]`, and once more with
`#[str_newtype(no_ord)]` (that fixture needs no hidden `Debug`, matching the
`Unordered` shape from Step 1).

Note the `no_ord` doctest is a **documentation** artifact, not a discriminator:
it also passes before this task, because an unknown `no_ord` option is itself a
compile error. `str_newtype_no_ord_omits_ordering` from Step 1 is what actually
pins the behavior.

- [x] **Step 7: Run the tests and the doctests, verify they pass** — macros 78
      passed (was 68); macros doctests 18 (was 15); **common doctests 18,
      unchanged — AC7 verified**; common 454 passed.

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS

Run: `devtool run -- cargo test -p macros --doc` Expected: PASS — nextest does
not execute doctests, so this second command is required

Run: `devtool run -- cargo nextest run -p common` Expected: PASS — confirms
`RawToken` and `Tag` still compile

Run: `devtool run -- cargo test -p common --doc` Expected: PASS — **this is the
only command that verifies AC7.** `RawToken`'s four `compile_fail` doctests live
at `common/src/token.rs:56-74`, and no gate runs them; without this line the
criterion is asserted but never checked.

- [x] **Step 8: Confirm the coverage inference before going further** —
      **CONFIRMED.** Coverage clean at 22917 executable lines (up from 22858), 0
      failures, 0 guard violations, 0 CRAP over threshold. The ~48 emitted
      `StrNewtype` impls produced no uncovered regions, so the spec's inference
      holds and the abort condition did not fire.

      The first run of this step also caught three clippy errors in the new
      fixtures — `nonminimal_bool` on `!(b < a)`, `infallible_try_from` on the
      `Unordered` fixture's `Infallible` error type, and
      `no_effect_underscore_binding` on `let _read`. All three were fixed in the
      fixtures (asserting `b > a`, giving `Unordered` a rejecting `FromStr`, and
      asserting on the deref result), not silenced.

Run: `devtool run -- cargo xtask check` Expected: PASS, with **no new uncovered
lines**. This is the spec's stated risk checkpoint — the first run where emitted
ordering impls meet the uncovered-line gate. If it reports per-type uncovered
regions for the new impls, **stop and report** rather than papering over it with
`cov:ignore`; the spec's abort condition has fired.

- [x] **Step 9: Commit** — `1745d124`, gate green.

```bash
git add macros/src/lib.rs macros/src/str_newtype.rs macros/tests/str_newtype.rs common/src/token.rs common/src/tag.rs
git commit -m "feat(macros): emit Ord/PartialOrd from StrNewtype, with a no_ord opt-out (#761)"
```

---

### Task 3: `NumNewtype` ordering

**Files:**

- Modify: `macros/src/num_newtype.rs` (append `crate::ord_impls` to the emitted
  trailer)
- Create: `macros/tests/num_newtype.rs`
- Modify: `common/src/media.rs:828` (`ByteSize` drops `Ord, PartialOrd`)

**Interfaces:**

- Consumes: `crate::ord_impls(name: &syn::Ident) -> proc_macro2::TokenStream`
  (Task 2).
- Produces: nothing later tasks depend on.

- [x] **Step 1: Write the failing tests**

Create `macros/tests/num_newtype.rs`. `PageSize::MIN` is typed as the **inner**
integer, not the newtype, so it must not appear in an ordering assertion.

```rust
//! Exercises the ordering half of the `#[derive(NumNewtype)]` trailer (#761). The rest of
//! the numeric-value surface is covered by the doctest in `macros/src/lib.rs` and by the
//! real types in `common`.

use macros::NumNewtype;
use std::collections::BTreeSet;
use std::str::FromStr;

// No `default =` and no `max =`: each option this fixture declares must be exercised
// below, or the derive emits a `Default` impl / bound assertion that nothing calls —
// a self-inflicted uncovered region, in the very change whose coverage attribution is
// this plan's stated risk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
#[num_newtype(inner = u32, min = 1)]
struct Count(u32);

#[test]
fn ordering_agrees_with_the_inner_integer() {
    let a = Count::from_str("3").unwrap();
    let b = Count::from_str("7").unwrap();
    assert!(a < b);
    assert!(!(b < a));
    assert_eq!(a.cmp(&b), 3u32.cmp(&7));
}

#[test]
fn min_bound_still_rejects() {
    // Exercises the `min` branch this fixture declares, so the option earns its keep.
    assert!(Count::from_str("0").is_err());
}

#[test]
fn sorts_and_keys_a_btreeset() {
    let mut v = vec![
        Count::from_str("9").unwrap(),
        Count::from_str("2").unwrap(),
        Count::from_str("5").unwrap(),
    ];
    v.sort();
    assert_eq!(v[0].value(), 2);
    assert_eq!(v[2].value(), 9);

    let set: BTreeSet<Count> = v.into_iter().collect();
    assert_eq!(set.len(), 3);
}
```

- [x] **Step 2: Run the tests, verify they fail** — FAIL as designed: `<`/`>`
      not applicable to `Count`, `Count: Ord` unsatisfied.

Run: `devtool run -- cargo nextest run -p macros --test num_newtype` Expected:
FAIL — `a < b` does not compile; `Count` is not `Ord`

- [x] **Step 3: Emit ordering from `NumNewtype`**

Append `crate::ord_impls(name)` to the token stream `num_newtype::expand`
returns, unconditionally — there is no `no_ord` on this macro (spec §2). Every
branch is pinned by the Step 1 tests plus the collision that Step 4 resolves.

- [x] **Step 4: Drop `ByteSize`'s now-colliding derives**

`common/src/media.rs:828` →
`#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]`.

- [x] **Step 5: Run the tests, verify they pass** — 535 passed (78 macros + 3
      new `num_newtype` + 454 common).

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS

Run: `devtool run -- cargo nextest run -p common` Expected: PASS — `ByteSize`'s
existing surface test (`common/src/media.rs:858`) still passes

- [x] **Step 6: Commit** — `86bcd5f3`, gate green (coverage 22918 lines, clean).

```bash
devtool run -- cargo xtask check
git add macros/src/num_newtype.rs macros/tests/num_newtype.rs common/src/media.rs
git commit -m "feat(macros): emit Ord/PartialOrd from NumNewtype (#761)"
```

---

### Task 4: `IdNewtype` ordering

**Files:**

- Modify: `macros/src/id_newtype.rs` (append `crate::ord_impls`)
- Modify: `macros/src/lib.rs:178` (doc fixture `Id` gains `PartialEq, Eq`)
- Modify: `macros/tests/id_newtype.rs` (ordering test)

**Interfaces:**

- Consumes: `crate::ord_impls` (Task 2).
- Produces: nothing later tasks depend on.

- [x] **Step 1: Write the failing test**

Append to `macros/tests/id_newtype.rs` (its `Id` fixture at `:7` already derives
`PartialEq, Eq`):

```rust
#[test]
fn ordering_agrees_with_the_inner_i64() {
    let a = Id::from(3_i64);
    let b = Id::from(7_i64);
    assert!(a < b);
    assert_eq!(a.cmp(&b), 3_i64.cmp(&7));

    let mut v = vec![b, a];
    v.sort();
    assert_eq!(v[0], a);

    let map: std::collections::BTreeMap<Id, &str> = [(b, "b"), (a, "a")].into_iter().collect();
    assert_eq!(map.keys().next(), Some(&a));
}
```

- [x] **Step 2: Run the test, verify it fails** — FAIL as designed: `<`/`>` not
      applicable to `Id`, `Id: Ord` unsatisfied.

Run: `devtool run -- cargo nextest run -p macros --test id_newtype` Expected:
FAIL — `Id` is not `Ord`

- [x] **Step 3: Emit ordering from `IdNewtype`**

Append `crate::ord_impls(name)` to the stream `id_newtype::expand` returns,
unconditionally.

- [x] **Step 4: Fix the doc fixture**

`macros/src/lib.rs:178` → `#[derive(Clone, Copy, PartialEq, Eq, IdNewtype)]`.
Without this the doctest stops compiling, because `Ord: Eq`.

- [x] **Step 5: Run the tests and doctests, verify they pass** — 536 passed;
      doctests 18 (macros) and 18 (common), both clean.

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS

Run: `devtool run -- cargo test -p macros --doc` Expected: PASS

Run: `devtool run -- cargo nextest run -p common` Expected: PASS — the eight id
newtypes in `common/src/ids.rs` all derive `PartialEq, Eq` already

- [x] **Step 6: Commit** — `d16e8d27`, gate green (coverage 22919 lines, clean).

```bash
devtool run -- cargo xtask check
git add macros/src/id_newtype.rs macros/src/lib.rs macros/tests/id_newtype.rs
git commit -m "feat(macros): emit Ord/PartialOrd from IdNewtype (#761)"
```

---

### Task 5: Correct the falsified doc comments

Docs only — no behavior. Satisfies AC10; the enumeration below is the criterion,
verified by the Step 2 sweep.

**Files:**

- Modify: `common/src/absolute_url.rs:21`, `common/src/root_relative_url.rs:21`,
  `common/src/feed/feed_path.rs:16-18` — the three explicit "no `Ord`" claims
- Modify: `common/src/slug.rs:27-28`, `common/src/bio.rs:19-20`,
  `common/src/audience.rs:12-13`, `common/src/display_name.rs:22`,
  `common/src/session_label.rs:25-26`, `common/src/post_summary.rs:20`,
  `common/src/tag.rs:57-58` — the seven "never a map/set key" claims
- Modify: `common/src/tag.rs:14-15` — `Tag`'s "Keeps `Hash`/`Ord`" note
- Modify: `macros/src/lib.rs:164`, `macros/src/lib.rs:193`,
  `macros/src/str_newtype.rs:3`, `macros/src/id_newtype.rs:4`,
  `macros/src/num_newtype.rs:8` — the macro crate's own "`Ord` stays in the
  user's list" statements
- Modify: `docs/adr/0068-tag-identity-label-split.md:59` — "a canonical-slug
  operation on `Tag`, which keeps `Hash`/`Ord`" describes the derive list Task 2
  changes. Falsified the same way as `common/src/tag.rs:14-15`. Not in the
  spec's AC10 enumeration, but squarely inside AC10's catch-all ("no comment
  **in the tree**"), so it is in scope, not scope creep.

**Interfaces:** none produced or consumed.

- [x] **Step 1: Rewrite each comment** — 10 code sites, 5 macro-crate docs, and
      `docs/adr/0068`.

Apply the spec §4 principle — the trailer grants **capability**, and an omitted
derive never encoded intent. Concretely: strike every "no `Ord`" / "never
sorted" claim (all three are now false); for the seven `Hash` comments keep the
`Hash` decision and its reason but drop the "never a map/set key" justification,
which no longer holds now that the type is a `BTreeMap` key; rewrite `Tag`'s
note to say `Hash` is kept and ordering comes from the trailer; and correct the
five macro-crate statements to say ordering is emitted, not user-derived.

`ByteSize` (`common/src/media.rs`) carries no such note — do not invent one.
`SmtpUsername`, `SiteTitle`, `BackupSchedule`, and `DestinationPath` omit `Hash`
silently with no rationale — leave them alone.

- [x] **Step 2: Sweep for anything missed** — clean: the only live hits left are
      this plan/spec and one true statement (a secret really is never sorted).
      `docs/archive/` excluded by the note below.

Use `-U` (multiline) and search every crate plus `docs/`. A single-line pattern
is not good enough here: rustfmt wraps these comments, and the naive
`'Keeps .Hash'` misses `common/src/tag.rs:14-15` — where "Keeps" ends line 14
and "`Hash`/`Ord`" begins line 15 — which is a site this very task is supposed
to edit.

```bash
rg -n -i -U 'map/set\s+key|never\s+sorted|are\s+not\s+sorted|no\s+.?Ord|Keeps\s+.?Hash' \
  common/ host/ storage/ web/ client/ server/ csr/ test-support/ macros/ xtask/ docs/
```

Expected: no hit that claims a newtype cannot be ordered, or is "never a key",
on the strength of an omitted derive. Hits that merely describe `Hash` without
invoking key-ness are fine and stay.

**`docs/archive/` is deliberately excluded.** The sweep surfaces ~12 archived
specs and plans (#350, #399, #409, #459, #472, #475, #478, #545) carrying the
old claims. They are a historical record of decisions _as they were made_;
rewriting them would falsify that record and make the archive useless for
answering "what did we believe when we chose this?". AC10's catch-all is read as
governing **live** documentation — code doc comments and active ADRs — which is
what a future author would consult and be misled by.

- [ ] **Step 3: Commit**

```bash
devtool run -- cargo xtask check
git add common/ macros/ docs/adr/0068-tag-identity-label-split.md
git commit -m "docs(newtypes): correct rationale falsified by the ordering trailer (#761)"
```

---

### Task 6: Amend ADR-0063 in place

**Files:**

- Modify: `docs/adr/0063-domain-value-newtype-convention.md` — §2's trailer
  bullet (`:104-105`), the secret exception (`:112-123`), the **inbound-secret
  variant** (`:125-134`, which carries its own parallel omission list and sits
  outside the secret-exception range), §3's std-derive paragraph (`:260-288`)

**Interfaces:** none.

- [ ] **Step 1: Edit the three sites**

- §2's
  `Derive Clone, Debug, PartialEq, Eq, Hash (add Ord when the type is used as a sort/map key)`
  → ordering is emitted by the derive; `PartialEq`/`Eq` are now **required** on
  every non-secret newtype; `Hash` stays the per-type call.
- The secret exception gains ordering to its omission list, stated as a
  **convention rather than a structural guarantee** — the macro emits nothing
  for a secret, so a hand-added `#[derive(Ord)]` on one still compiles (spec
  §3).
- §3's "the std derives stay in the user's `#[derive(...)]` list ... (Slug omits
  `Hash`, Tag adds `Ord`...)" → drop `Ord`/`PartialOrd` from the user-owned set,
  name `#[str_newtype(no_ord)]` and its `RawToken` rationale, note that `no_ord`
  is `str_newtype`-only, and record the capability-not-intent principle from
  spec §4. The `Tag adds Ord` example is now false and must go.

Cite #761. Do not create a draft, do not hand-edit `docs/README.md`.

- [ ] **Step 2: Verify the ADR gates**

Run: `devtool run -- cargo xtask check` Expected: PASS — `adr-format` and
`adr-readme-parity` both green (this is a body edit, not a status change, so
`sync-readme` should not be needed; if parity complains, run
`devtool run -- cargo xtask adr sync-readme`)

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0063-domain-value-newtype-convention.md
git commit -m "docs(adr): ordering joins the newtype standard trailer (#761)"
```

---

### Task 7: Final sweep and full gate

Verification only — expected to produce no commit. Exists because AC8 requires
the sweep to run against whatever `main` holds at ship, not against the
inventory taken at spec time (#711 may have landed meanwhile, adding three more
derive entries).

**Files:** none by default.

**Interfaces:** none.

- [ ] **Step 1: Rebase onto current `main` and re-sweep for colliding derives**

```bash
git fetch origin
git rebase origin/main
rg -n -U --multiline-dotall \
  '#\[derive\([^)]*\b(?:Partial)?Ord\b[^)]*\)\]' \
  -g '*.rs' --glob '!target/**'
```

Order-independent and wrap-tolerant, both of which matter: the previous
single-line, `Ord`-before-the-derive pattern matched today's two hits only by
luck, and would silently miss `#[derive(StrNewtype, Ord)]` or a rustfmt-wrapped
derive list — which is exactly the shape an unseen branch like #711 may land.

Read every hit; do **not** pipe this through `rg -i 'newtype'` to narrow it. The
worktree path itself contains "newtype" (`…/issue-761-newtype-ord-derives/…`),
so that filter matches every line and only looks like it is narrowing. The
unfiltered list is short — two known non-newtype hits in `xtask/` — so read it.

Expected: no hits on non-secret newtypes. Any hit from a branch that landed
after this plan was written (e.g. #711's `ContentHash`/`Filename`/`MediaSource`)
is removed now, then `cargo xtask check` and commit as
`refactor(media): drop derives subsumed by the ordering trailer (#761)`.

- [ ] **Step 2: Confirm no hand-written ordering exists (AC9)**

```bash
rg -n 'impl(<[^>]*>)?\s+(::core::cmp::)?(Partial)?Ord\b' -g '*.rs' --glob '!target/**'
```

Expected: **exactly one hit** — the `quote!` body inside `ord_impls` at
`macros/src/lib.rs`. Nothing else in the tree hand-writes an ordering.

Do not expect hits in `xtask/src/steps/flaky.rs` or
`xtask/src/server_fn_coverage/extract.rs`: those two carry
`#[derive(..., PartialOrd, Ord, ...)]`, which this pattern does not and should
not match. (The earlier draft of this step predicted them and would have handed
the worker a contradiction.) The `impl(<...>)?` prefix is there so a generic
`impl<T> Ord for …` cannot slip past.

- [ ] **Step 3: Run the full local gate (AC12)**

Run: `devtool run -- cargo xtask validate --no-e2e` Expected: PASS — static,
clippy, and coverage, with no new `cov:ignore`

This is a long, cold run; use the Bash tool's background mode.

---

## Self-review

**Spec coverage** — every acceptance criterion maps to a task:

| AC   | Task    | Where                                                                                                                                                                                                                                                                                                                            |
| ---- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AC1  | 2       | `ordering_agrees_with_the_inner_str`, `sorts_and_keys_a_btreeset`                                                                                                                                                                                                                                                                |
| AC2  | 2       | `Label` fixture + `infallible_trailer_orders`                                                                                                                                                                                                                                                                                    |
| AC3  | 2       | Step 6 `compile_fail` doctests (`secret`, `secret, serde`)                                                                                                                                                                                                                                                                       |
| AC4  | 3, 4    | `macros/tests/num_newtype.rs`, `macros/tests/id_newtype.rs`                                                                                                                                                                                                                                                                      |
| AC5  | 2       | `Unordered` fixture + `str_newtype_no_ord_omits_ordering` (the doctest documents, it does not discriminate). The spec also names sqlx; that is unverifiable inside `macros`, whose `sqlx` feature has no deps and is never enabled (`macros/Cargo.toml:24`) — it is covered instead by `RawToken`, a real `no_sqlx, no_ord` type |
| AC6  | 2       | `str_newtype_no_ord_with_secret_emits_compile_error`                                                                                                                                                                                                                                                                             |
| AC7  | 2       | Step 5 `common/src/token.rs`, verified **only** by `cargo test -p common --doc` in Step 7 — no gate runs doctests                                                                                                                                                                                                                |
| AC8  | 2, 3, 7 | `Tag`, `ByteSize`, then the pre-merge re-sweep                                                                                                                                                                                                                                                                                   |
| AC9  | 7       | Step 2 sweep — expects exactly one hit (`ord_impls`), not three                                                                                                                                                                                                                                                                  |
| AC10 | 5       | the enumerated file list (incl. `docs/adr/0068`) + the multiline Step 2 sweep                                                                                                                                                                                                                                                    |
| AC11 | 6       | ADR-0063 in-place edit                                                                                                                                                                                                                                                                                                           |
| AC12 | 7       | `cargo xtask validate --no-e2e`                                                                                                                                                                                                                                                                                                  |

Spec §1's "`PartialEq + Eq` becomes mandatory" consequence is discharged by Task
2 Step 5 (two `StrNewtype` fixtures) and Task 4 Step 4 (the `IdNewtype` fixture)
— all three doc fixtures the review found.

**Placeholder scan:** no TBD/TODO, no "add error handling", no "similar to Task
N". Every test is written in full; the only body written out is `ord_impls`, and
Task 2 Step 3 states why (two invariants no test expresses).

**Type consistency:** `ord_impls(name: &syn::Ident) -> proc_macro2::TokenStream`
is defined once in Task 2 and called by the same name in Tasks 3 and 4.
`SqlxMode` is introduced in Task 1 and consumed by Task 2's `Opts` edit. `Opts`
ends with fields `kind`, `serde`, `sqlx: SqlxMode`, `ord: bool`.

**Collision inventory, verified tree-wide** (not just `common`): across all 66
real derive sites naming a newtype derive in every crate — including
`#[cfg(test)]` modules, `tests/` dirs, and doctest fixtures — the only
non-secret types lacking `PartialEq`/`Eq` are `RawToken`
(`common/src/token.rs:78`) and the three doc fixtures (`macros/src/lib.rs:50`,
`:146`, `:178`); the only derive lists already naming `Ord`/`PartialOrd` are
`Tag` (`common/src/tag.rs:18`) and `ByteSize` (`common/src/media.rs:828`). Every
`NumNewtype` and all eight `IdNewtype`s already derive `PartialEq, Eq`. So Tasks
2–4 leave the workspace compiling at each commit point, and the three
`NotATuple` fixtures are `compile_fail` shape-guard cases that never reach
emission.

**Method resolution, verified:** in the generated `Some(self.cmp(other))`, the
receiver `&Name` matches `<Name as Ord>::cmp` at the first autoderef step, so it
does not fall through to `str`'s impl — no recursion, no wrong comparison. (That
fall-through is precisely why `a.cmp(&b)` resolves to `str` _today_, per the
spec's "Behavioral impact".) One residual hygiene note for the implementer:
`self.cmp(other)` is the only un-path-qualified call in an otherwise
fully-qualified codegen family, so an inherent `fn cmp` on a future newtype
would silently hijack it. None exists today.

**Known gaps, deliberate:**

- `#[str_newtype(sqlx, no_ord)]` and other novel flag pairs beyond the `secret`
  guard are not separately tested — the existing exclusivity guards are
  untouched and remain covered by the four Task 1 baseline tests.
- Task 1's `SqlxMode` refactor is enabling work the spec does not authorize. It
  is listed in Scope-in, justified by a verified lint ceiling, and isolated in
  its own commit so it can be rejected without touching the rest.
