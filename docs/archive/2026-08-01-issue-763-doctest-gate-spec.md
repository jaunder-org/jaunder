# Spec — issue #763: gate the doctests, and enumerate the fence population

- Issue: [#763](https://github.com/jaunder-org/jaunder/issues/763)
- Date: 2026-08-01
- Branch: `worktree-issue-763-doctest-gate`

## Problem

The repo carries 43 rustdoc code fences, 31 of them `compile_fail`. They are how
the repo proves the _negative_ type properties no unit test can express: that
`RawToken` does not convert to `TokenHash`, that a bare `String` cannot
masquerade as a `ContentHash`/`Filename`/`ContentType`/`ETag`/`RenderedHtml`,
that a `PostBody` is not a `RenderedHtml`, that `ProfferedFilename` has no
`Display`/`Serialize` (the structural half of ADR-0084), and that the ADR-0063
secret surface omits `Display`, serde, owned-`String`, `PartialEq`, and `Deref`.

**No gate runs the root workspace's doctests** — which is 41 of the 43 fences,
including every one of the 31 `compile_fail` proofs. `cargo nextest`
structurally cannot run doctests, the package build sets `doCheck = false`
(`flake.nix:315-318`), and `--doc` appears nowhere in `flake.nix`, `xtask/src`,
or `tools/devtool`.

(The two `xtask` fences are the exception: `xtask/src/steps/host_tests.rs:16`
already runs `cargo test --manifest-path xtask/Cargo.toml`, which collects them
— as _ignored_, since both are `ignore` blocks. So they are reached by a gate
and assert nothing.)

### What the issue got wrong, and why it matters

The issue proposes `cargo test -p common -p macros --doc` and cites its own
measurement — "common: 18 passed" — as proof the property holds today. Measured
on this branch:

```
cargo test --workspace --doc
  common:  21 passed, 0 ignored
  macros:  17 passed, 2 ignored
  web:      0
  (client, csr, host, server, storage, test-support: 0)
```

`common/` has **21** fences, not 18. The three missing ones — `render.rs:505`,
`:511`, `:518`, the `RenderOutput` private-field proofs — live inside
`mod sanitized`, gated `#[cfg(feature = "sanitize")]`, an optional feature that
is off by default (`common/Cargo.toml:52`). Under `-p common` they do not fail.
They **vanish**, and 21 − 3 = 18 is exactly the number the issue quotes as
evidence of health.

That is ADR-0085 principle 6 verbatim: _"a gate that quietly shrinks its own
population reports green for the one reason it must never report green."_ The
issue invokes ADR-0085 as its motivation and then proposes a gate that violates
it.

Under `--workspace` the three run without any explicit `--features`, because
`storage/Cargo.toml:12` requires `common` with `["sqlx", "sanitize"]` and
feature unification does the rest.

### Five ways the population silently shrinks

The `sanitize` case is not unique. All five are confirmed on this branch:

1. **A `#[cfg(feature = …)]` gate** — `common/src/render.rs:505/511/518`, above.
2. **A `#[cfg]`-gated module** — `web/src/reactive/scope.rs:16` is reached only
   via `#[cfg(any(target_arch = "wasm32", test))] mod scope;`
   (`web/src/reactive/mod.rs:16`). rustdoc sets `cfg(doctest)`, **not**
   `cfg(test)`, so that fence is invisible to any host `--doc` run. Confirmed:
   `web` reports 0 doctests.
3. **An unrecognized fence info string** — probed directly. A fence tagged with
   only a wholly unrecognized word (` ```intent_only `) is **not collected at
   all**, and rustdoc emits no warning:

   ```
   running 3 tests
   test src/lib.rs - (line 23) - compile fail ... ok    ← HTML-comment marker + compile_fail
   test src/lib.rs - (line 4)  - compile fail ... ok    ← plain compile_fail
   test src/lib.rs - (line 10) - compile fail ... ok    ← compile_fail,intent_only
   ```

   Line 17 is absent, silently. (A _near-miss_ of a known attribute does warn:
   ` ```compile-fail ` trips `rustdoc::invalid_codeblock_attributes`. The silent
   case is the wholly-unknown word, which is the one nobody is looking for.)

4. **A crate outside the run's reach** — `xtask/` is excluded from the flake
   `src` filter (`flake.nix:272`) and sits outside the root workspace; `tools/`
   is a separate virtual workspace. A Nix check over the workspace cannot see
   either.
5. **A crate with no lib target** — cargo collects doctests from lib targets
   only. `tools/devtool` has `src/main.rs` and no `src/lib.rs`, so a fence added
   there can never appear in any run.

Arithmetic closes exactly: 43 fences − 38 run − 2 ignored = 3, being
`web/src/reactive/scope.rs:16` and `xtask`'s two. Nothing unexplained.

### Vacuous passing

A `compile_fail` passes if the snippet fails to compile _for any reason_ — a
renamed path, an import that stopped resolving. `macros/src/lib.rs:64-79`
already defends against this and says why: _"The positive companion shows the
identical fixture compiles — and that `serde_json` resolves, so the serde
`compile_fail` below fails for the missing `Serialize`, not an unresolved
crate."_

In `common/`, **18 of the 20 `compile_fail` blocks sit in doc comments
containing no passing fence at all** (8 doc comments: `etag.rs`@23,
`media.rs`@67/@134/@336/@750, `post_body.rs`@3, `render.rs`@47, `token.rs`@38).
The other two (`render.rs:511`, `:518`) share the plain fence at `:505`, which
imports `{PostFormat, RenderOutput}` while they import
`{render, PostFormat, RenderOutput}` — so it would not notice an unresolved
`render`.

Running the doctests fixes regressions-in-the-property. Only the companion
pattern fixes vacuous passing. Both halves are needed for the gate to mean what
it appears to mean.

### Corrections to the issue's inventory

| Issue says                           | Actually                                                                                                                                                       |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 36 fences, 26 `compile_fail`         | 43 fences, 31 `compile_fail` (+7 plain, +5 `ignore`)                                                                                                           |
| 10 uncompanioned blocks in `common/` | 20 blocks across 8 doc comments; 18 with no companion, 2 with an inadequate one                                                                                |
| 4 `ignore` blocks                    | **5** — it missed `macros/src/lib.rs:353` (`text_enum`)                                                                                                        |
| `flake.nix:372`                      | `flake.nix:315`, and its "separate `nextest` check" is stale naming: there is no `nextest` check; the suite runs inside `coverage` via `devtool coverage emit` |
| `macros/src/lib.rs:244`, `:76-133`   | `:298`, `:50-140` — #761 shifted the file ~54 lines                                                                                                            |
| ADR-0063:118                         | ADR-0063:140 — #761 shifted it                                                                                                                                 |

## Decisions

Settled in the design interview. Each is recorded in a new ADR (AC18).

1. **Enumerate, do not merely run.** The gate reconciles the fence population
   read from the source against what the doctest run reported, in both
   directions. Running alone would inherit all five shrink vectors above.
2. **Standalone check, not folded into `coverage`.** `coverage`'s producer is
   contractually unable to fail (`flake.nix:1184-1186`); gating goes through
   `status.json` → `coverage-gate`. Folding doctests in would require a
   libtest-output classifier, a precedence rule against
   `classify_nextest_output`, and widening `failed_tests[]` — a field that today
   means "nextest test ids" — to carry a second kind of thing. It would also tie
   the cheapest proof in the repo (38 tests, ~1.3s) to the slowest lane
   (instrumented workspace build + ephemeral PostgreSQL).
3. **Shape mirrors `coverage`.** Producer derivation → `status.json` → gate
   consumer → xtask steps. Same idiom, its own status file, so nothing touches
   `failed_tests[]`.
4. **No allowlist. Exemption is the fence marker, written by a human at the
   site.** A ` ```text ` fence declares an illustration, not a proof.

   _Why this is not the self-exemption ADR-0085 principle 3 forbids, given that
   `ignore` is banned for being exactly that._ Principle 3's target is
   **automatic** exemption — a site escaping because it matched a pattern nobody
   has to write down. A marker edit is not automatic; it is "an entry a human
   wrote", which is what principle 3 asks for. The distinction from `ignore` is
   what the marker _claims_: a `text` fence renders as non-Rust and reads, to
   every subsequent reader, as illustration — it stops asserting anything. An
   `ignore` fence renders as a greyed-out Rust block that still looks like a
   test, so it silences a proof while leaving the appearance of one. And
   `-compile_fail` / `+text` is a visible diff hunk, reviewable exactly like
   deleting the fence outright. No gate can stop a human from deleting a proof;
   the gate's job is to stop it happening _silently_, and a marker change is not
   silent.

   **Accepted limitation:** the `text` population is not counted, so principle
   4's multiplicity clause is not enforced for it. A census snapshot was
   considered and declined as machinery disproportionate to the risk, given that
   every such change is a reviewable diff. AC18 records this as a stated limit
   rather than an oversight.

5. **`ignore` is banned.** Per decision 4's reasoning. Banning it also makes the
   issue's scope item 5 permanent rather than a one-time cleanup.
6. **There is no exemption marker for a `compile_fail`.** The companion rule
   (decision 7) is absolute: every `compile_fail` must carry a matched hidden
   prelude. `text` is the only way to say "not a proof", and it says so by
   ceasing to claim otherwise.

   A `compile_fail,intent_only` marker was designed and probed (it survives
   rustdoc: because `compile_fail` is recognized the block still runs and
   passes) to formalize what `macros/src/lib.rs:141-144` does in prose — three
   ordering proofs that "document intent rather than discriminate", because
   their fixtures derive no `PartialEq`, so `a < b` would fail to compile even
   if the macro _did_ emit ordering.

   It was dropped because those three turned out not to need it. Giving each
   fixture `PartialEq, Eq` makes `a < b` fail **only** for the missing ordering,
   and a control fence — the same shape without the suppressing option — proves
   the discrimination is real. Probed: the control orders, all three negatives
   fail. With no instance left, the marker would be machinery for a case that
   does not exist, and it would have enshrined a circular claim: the doctest at
   `:141-144` cites a unit test as "the actual guard", while
   `macros/tests/str_newtype.rs:242` cites the doctest. Neither guarded
   anything.

   If a genuinely non-discriminating proof ever appears, add the marker then,
   with a real case to justify it.

7. **The companion rule matches hidden preludes.** A `compile_fail` fence must
   carry at least one `#`-hidden line, and every hidden line must appear
   verbatim — hidden or visible — in some plain passing fence in the same doc
   comment. No exceptions (decision 6).

   This is `macros/src/lib.rs:69-139`'s pattern made mechanical: the hidden
   prelude _is_ the fixture, and matching it to a companion is precisely what
   proves the negative fails for the intended reason rather than because a path
   stopped resolving. A weaker "at least one plain fence per doc comment" rule
   was rejected: `macros/src/lib.rs`'s doc@16 holds 9 `compile_fail` and 4 plain
   fences, so it would pass while `no_ord`'s `Unordered` fixture (`:173`) has no
   related companion at all — a region-scoped exemption of exactly the kind
   ADR-0085 principle 4 forbids. Verified against three cases: `:130` (Deref)
   passes, `:173` (`Unordered`) fails, `render.rs:511` fails and thereby demands
   the `:505` repair this spec argues for.

8. **The scanner parses with `syn`.** ADR-0085 principle 5 forbids a line-based
   scan for a multi-line invariant, and both the companion rule and the
   hidden-prelude match are scoped to a doc comment — a multi-line syntactic
   unit whose boundaries a line scan must guess. An unparseable `.rs` file under
   a scan root is a hard failure (principle 6).
9. **Multi-line `#[doc = "…"]` values are banned.** Probed: for a `#[doc]`
   attribute on line 22 whose value spans several markdown lines, libtest
   reports `(line 24)` — the attribute line plus a markdown-relative offset, not
   any line a fence opens on. The reconciliation key therefore cannot address
   them, and an offset rule would be fragile across raw strings, `concat!`, and
   multiple `#[doc]` attributes.

   The rule is precise because the two forms are exactly distinguishable in
   `syn`: a `///` line desugars to **one `#[doc]` attribute per source line**,
   whose value never contains a newline, so its span start _is_ the fence-opener
   line. A value containing a newline is a multi-line literal and is rejected. A
   _single-line_ `#[doc = "…"]` is indistinguishable from `///`, keys correctly,
   and is therefore allowed. Zero multi-line ones exist today.

10. **Reconciliation key is `(file, opening line)`.** Verified against real item
    docs: `common/src/token.rs - token::RawToken (line 56)` matches the fence
    opener at `token.rs:56`, and likewise `:59/:64/:69`, `etag.rs:35/:38`,
    `post_body.rs:15/:19`. Holds for `///` item docs and `//!` module docs
    alike. Paths are reported relative to the invoked manifest, so each scan
    root carries a prefix rule (a host-side `--manifest-path xtask/Cargo.toml`
    run prints `src/steps/…`, not `xtask/src/steps/…`).
11. **Doctests do not feed coverage.** `llvm-cov --doctests` is unstable and
    ADR-0050's stateless gate measures nextest only. The doctest run stays
    outside llvm-cov instrumentation, so no profraw from it reaches the coverage
    profile.

## Acceptance criteria

### The gate runs and fails honestly

**AC1.** `cargo xtask check` and `cargo xtask validate --no-e2e` each report a
`nix-doctests` step and a `nix-doctests-gate` step in `.xtask/last-result.json`.
CI's existing `Validate (no e2e)` job therefore runs both; **no new CI job and
no new required-status context is added**, so `xtask/src/pr/` is untouched.

**AC2.** The Nix half's run is `cargo test --workspace --doc`, asserted by a
test on the constructed command. A run scoped to named packages
(`-p common -p macros`) fails that test. This pins the spec's central
correction: `--workspace` is what makes feature unification pull in
`common/sanitize` via `storage`, and a conformance reviewer can tell the two
invocations apart.

**AC3.** A `compile_fail` block that starts compiling fails the gate:
`validate --no-e2e` exits non-zero and the output names the offending fence by
file and line.

**AC4.** A plain doctest that starts failing fails the gate, likewise named.

**AC5.** `--no-test` skips `nix-doctests`/`nix-doctests-gate` and nothing else.
The host-side half (AC8) runs in **every** mode, matching the documented
behaviour of `xtask/src/steps/host_tests.rs:6-10`. The gate's own docs state
this asymmetry — under `--no-test` the `xtask`/`tools` population is gated and
the workspace population is not.

### The population is enumerated, in both directions

**AC6.** A fence present in the tree but absent from the run output fails the
gate, naming file and line. Demonstrated against all five shrink vectors: a
`#[cfg(feature)]` gate whose feature the run does not enable; a
`#[cfg(test)]`-gated module; a fence whose info string is a wholly unrecognized
word; a crate outside every scan root; a fence in a crate with no lib target.

**AC7.** The reverse direction is also checked: a doctest in the run output with
no corresponding scanned fence fails the gate, naming it. Without this, a
scanner bug or an unhandled doc form shrinks the gate's own population silently
— principle 6 turned on the gate itself.

**AC8.** `xtask/` and `tools/` fences are scanned and reconciled host-side,
extending the existing `xtask/src/steps/host_tests.rs:11-28` steps rather than
duplicating them.

**AC9.** The union of the gate's scan roots covers every `.rs` file in the
repository. A test walks the repo and asserts each `.rs` path falls under
exactly one scan root; a file under no root is a failure, not a skip.

**AC10.** An unparseable `.rs` file under a scan root fails the gate with a
parse error naming the file — it is never skipped.

### The vocabulary is closed

**AC11.** The gate accepts exactly these forms, and denies by default:

- **Collected as Rust by rustdoc** (empty info string, or one containing a
  recognized Rust attribute): must be plain or `compile_fail`. `ignore`,
  `no_run`, and `should_panic` are hard failures. (Zero of the latter two exist,
  so that half of the ban costs nothing.)
- **Not collected**: the info string must be `text`. Any other tag — a language
  name, a wholly unknown word — is a hard failure. The permitted set grows only
  by a deliberate edit to the gate, never by a fence tagging itself.
- **Inside a multi-line `#[doc = "…"]` value**: a hard failure regardless of
  marker (decision 9). Single-line `#[doc = "…"]` attributes are permitted,
  since they key identically to `///`.

**AC12.** Zero `ignore` fences remain in the tree:

- `xtask/src/steps/proffered_filename_check.rs:19` → `text`
- `xtask/src/steps/proffered_filename_check.rs:107` → `text`
- `macros/src/lib.rs:298` (`#[macros::server]`) → `text`
- `web/src/reactive/scope.rs:16` → `text`
- `macros/src/lib.rs:353` (`#[text_enum]`) → **promoted to a real, passing
  doctest** with concrete variants, `sqlx` dropped from the attribute and the
  sqlx bridge noted in prose instead.

**AC13.** Controls pin that the ordering proofs of AC17 actually discriminate —
that the same fixture shape **without** the suppressing option orders, so the
negatives fail for the missing ordering rather than for a missing `PartialEq`.
Without a control the three proofs would be indistinguishable from the vacuous
ones they replace. Two halves, because the claim has two:

- the **compiler** fact — `PartialEq + Eq` alone does not admit `<` — pinned by
  a dependency-free fixture crate alongside the gate's other fixtures (AC14);
- the **macro** fact — an un-suppressed `StrNewtype` orders, while `no_ord` and
  `secret` suppress it — pinned by a control fence in `macros/src/lib.rs`'s own
  doc comment, where it runs in the real gate and sits beside the negatives it
  justifies.

The macro half deliberately **is** in a production crate: a control that a
reader of those proofs cannot see does not do the job the control exists for.

**AC14.** The synthetic fixtures backing AC6, AC7, AC10, AC11 and AC13 live
under a dedicated fixture tree owned by the gate's tests (not in a shipped
crate, so they never enter the real population). Each AC names the fixture that
demonstrates it.

### The proofs are not vacuous

**AC15.** Every `compile_fail` in the tree carries at least one **non-empty**
`#`-hidden line, and each hidden line appears verbatim in a plain passing fence
within the same doc comment. There is no exemption (decision 6). Violations fail
the gate, naming the fence and the unmatched line.

Two details the pre-ship review found were load-bearing, both closed:
**non-empty** (a bare `/// #` is a hidden _blank_ line, and a blank needle
matches nothing, so accepting it would let one invisible character opt a block
out of the whole rule), and the scanner must classify hidden lines from the
**trimmed** text (rustdoc's `map_line` trims before testing for `# `, so an
indented `///   # …` is part of the compiled fixture — a scanner that called it
visible would leave a real prelude line outside the matched set).

Concretely in `common/`: 8 doc comments gain a companion (`RawToken`;
`ContentHash`, `Filename`, `ProfferedFilename`, `ContentType`; `ETag`;
`RenderedHtml`; `PostBody`), and the existing partial companion at
`render.rs:505` is repaired to import `render` so it covers `:511`/`:518`. All
20 `common/` blocks are restructured into the hidden-fixture form so their
preludes are matchable — which is what issue scope item 4 asks for.

**AC16.** `macros/src/lib.rs` gains a `compile_fail` proving the secret surface
omits `Borrow<str>` — the one omission ADR-0063:140 names that has no proof. The
positive companion at `:69-79` gains `use std::borrow::Borrow;` so the new
negative cannot pass on an unresolved import.

**AC17.** The three fences that today document in prose that they do **not**
discriminate — `macros/src/lib.rs:145`, `:158` (both governed by the paragraph
at `:141-144`) and `:173` — are made discriminating instead of marked exempt.
Each fixture gains `PartialEq, Eq`, so `a < b` fails only for the missing
ordering, and each gains a matched companion. The `:141-144` prose and the
reciprocal claim at `macros/tests/str_newtype.rs:242` are both rewritten: they
currently cite each other as "the actual guard", so neither guards anything.

### Recorded

**AC18.** A numberless ADR draft in `docs/adr/drafts/` records decisions 1–11,
argues the gate's conformance to ADR-0085, and — discharging ADR-0085's honesty
obligation — states:

- the classes the gate can **see but never run**, which it therefore forces to
  `text` rather than skipping: fences in crates with no lib target, and fences
  under `#[cfg]` combinations no scan root's run enables (e.g. wasm-only
  modules);
- that fences inside multi-line `#[doc = "…"]` values are rejected rather than
  supported, and why;
- that the `text` population is uncounted, so principle 4's multiplicity clause
  is not enforced for it (decision 4's accepted limitation);
- that no exemption marker for a `compile_fail` ships, why one was designed and
  then dropped, and what would justify adding it later (decision 6).

Note that a cfg-gated fence is **inside** the population — `syn` reads it
regardless of cfg — so AC6 makes it a hard failure. The unreadable class is not
"cfg-gated fences" but "fences no run can reach", whose remedy is the `text`
marker.

**AC19.** The coverage decision (11) is stated in the scanner module's doc
comment, so a reader of the code does not have to re-derive why doctests are
absent from the coverage numbers.

**AC20.** `cargo xtask validate` is green on the branch, and the ADR-0050
stateless coverage gate returns the same verdict and the same executable-line
count as on `wt-base-issue-763` — confirming the doctest run contributes no
profraw.

## Out of scope

- Re-enabling `cargo test` in the package build. `flake.nix:315-318` rejected
  that as a redundant compile + run and that reasoning still holds for unit
  tests. Doctests are the genuine exception because nextest structurally cannot
  run them.
- Feeding doctests into the coverage gate (decision 11).
- Rebuilding `sqlx-newtype-bind` as an enumerating gate — that is #716.
- A census/multiplicity lock on the `text` population (decision 4's accepted
  limitation).
