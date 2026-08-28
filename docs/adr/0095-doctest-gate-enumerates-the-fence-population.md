# ADR-0095: The doctest gate enumerates the fence population

- Status: accepted
- Date: 2026-08-01
- Issue: [#763](https://github.com/jaunder-org/jaunder/issues/763)

## Context

The repo carries rustdoc code fences, many of them `compile_fail`. They are how
it proves the negative type properties no unit test can express: that `RawToken`
does not convert to `TokenHash`, that a bare `String` cannot masquerade as a
`ContentHash`/`Filename`/`ContentType`/`ETag`/`RenderedHtml`, that a `PostBody`
is not a `RenderedHtml`, that `common::media` exposes no public decoded filename
intermediate (the structural half of
[ADR-0084](0084-media-filename-encoded-canonical.md)), and that the
[ADR-0063](0063-domain-value-newtype-convention.md) secret surface omits
`Display`, serde, owned-`String`, `PartialEq`, `Deref` and `Borrow`.

**No gate ran the root workspace's doctests**, including the compile-fail
proofs. `cargo nextest` structurally cannot run doctests, the package build sets
`doCheck = false`, and `--doc` appeared nowhere in `flake.nix`, `xtask/src`, or
`tools/devtool`.

### Running them is not enough, and the issue's own evidence shows why

The obvious fix — run `cargo test -p common -p macros --doc` — was measured and
reported as "common: 18 passed", offered as proof the property held. But
`common/` has **21** fences. The three missing ones are the `RenderOutput`
private-field proofs inside `mod sanitized`, gated
`#[cfg(feature = "sanitize")]`, an optional feature that is off by default.
Under that invocation they do not fail. They **vanish**, and 21 − 3 = 18 is
exactly the number quoted as evidence of health.

That is [ADR-0085](0085-static-type-safety-gates-enumerate.md) principle 6
verbatim: _"a gate that quietly shrinks its own population reports green for the
one reason it must never report green."_ A proposal that invoked ADR-0085 as its
motivation would have violated it.

Five ways a doctest population silently shrinks, all confirmed on this branch:

1. **A `#[cfg(feature = …)]` gate** the run does not enable — the `sanitize`
   case.
2. **A `#[cfg]`-gated module.** rustdoc sets `cfg(doctest)`, **not**
   `cfg(test)`, so a module reached only under `cfg(test)` is never compiled for
   the doc run.
3. **An unrecognized fence info string.** Probed: a wholly unrecognized word
   makes rustdoc treat the block as non-Rust and skip it **with no warning at
   all**. A typo deletes a proof and reports green forever.
4. **A crate outside the run's reach** — `xtask/` is filtered out of the flake
   `src`; `tools/` is a separate virtual workspace.
5. **A crate with no lib target.** cargo collects doctests from lib targets
   only.

### Running them is also not sufficient in a second way

A `compile_fail` passes if its snippet fails to compile _for any reason_ — a
renamed path, an import that stopped resolving — so it can rot into vacuous
truth while still reporting green. The repo already knew this and defended
against it in exactly one place. Everywhere else it did not: 18 of `common/`'s
20 blocks sat in doc comments with no passing fence at all, and the one partial
companion imported a different set of names than the negatives it nominally
covered.

Two proofs were worse than uncompanioned — they were **circular**. Three
ordering `compile_fail` blocks derived no `PartialEq`, so `a < b` would have
failed to compile even if the macro _did_ emit ordering. Their prose said so and
named a unit test as "the actual guard", while that unit test named the
doctests. Neither guarded anything.

## Decision

**The doctest gate enumerates the fence population and reconciles it against the
run, in both directions. Running the doctests is half of it; the other half is
proving the run saw everything.**

1. **Reconcile, do not merely run.** Every fence read from the source must
   appear in the run output, and every run entry must match a scanned fence. The
   first catches all five shrink vectors — whatever the cause, the proof was not
   evaluated. The second catches the gate shrinking its **own** population
   through a scanner gap, which is principle 6 turned inward.
2. **A standalone check, not folded into `coverage`.** The coverage producer is
   contractually unable to fail (gating goes through `status.json` → a
   consumer), and its `failed_tests` field means "nextest test ids". Doctests
   get their own producer/consumer pair with their own sentinel.
3. **A closed fence vocabulary of three exact strings** — plain, `compile_fail`,
   `text` — compared with whitespace removed. Everything else fails. This denies
   by default rather than modelling rustdoc's collection rules, which matters
   because two rejected forms are actively dangerous: `ignore` **is** collected
   and reported, so a presence check would accept it as a one-word
   self-exemption; and an unrecognized word is dropped silently.
4. **`text` is the only way to say "not a proof", and there is no exemption
   marker for a `compile_fail`.** A `text` fence stops _claiming_ to be a proof
   — it renders as non-Rust and reads as illustration — where an `ignore` fence
   silences one while leaving the appearance of it. `-compile_fail` / `+text` is
   a reviewable diff hunk, exactly like deleting the fence. No gate can stop a
   human deleting a proof; the job is to stop it happening _silently_.
5. **Every `compile_fail` must carry a `#`-hidden prelude, every line of which
   appears in a plain fence in the same doc comment.** The hidden prelude _is_
   the fixture, so matching it to a companion is what proves the negative fails
   for the stated reason. Scoping to one doc comment is load-bearing: a
   file-wide rule would let one companion cover negatives whose fixture it
   shares nothing with — the region-scoped exemption ADR-0085 principle 4
   forbids.
6. **The run is `cargo test --workspace --doc`, never package-scoped.** Package
   scoping is precisely what made the original measurement wrong; under
   `--workspace`, feature unification enables `common/sanitize` via `storage`.
7. **Parse with `syn`, do not scan lines.** ADR-0085 principle 5, and the
   companion rule is scoped to a doc comment — a multi-line unit whose
   boundaries a line scan must guess. A `///` line desugars to one `#[doc]`
   attribute per source line, so the fence opener's span _is_ the line libtest
   prints.
8. **Fail on input the gate cannot read.** An unparseable or unreadable file, or
   a missing scan root, is a hard failure rather than a silent skip.
9. **Doctests do not feed the coverage gate.** `llvm-cov --doctests` is unstable
   and [ADR-0050](0050-stateless-coverage-gate.md)'s stateless gate measures
   nextest only, so `--doc` runs outside instrumentation and contributes no
   profraw.

### Conformance to ADR-0085

- **Population read structurally**: every fence `syn` can see in every `.rs`
  file under an enumerated scan root — not a search for what a bad fence looks
  like.
- **Denies by default**: three accepted info strings; everything else fails.
- **No automatic exemption**: the only exemption is a marker a human writes at
  the site, in the source, visible in the diff.
- **Site-scoped**: the marker _is_ the site. There is no allowlist to scope
  wrongly.
- **Parses rather than scans**, per principle 5.
- **Fails on unreadable input**, per principle 6 — including its own scan roots.

### What this gate cannot read, stated rather than papered over

- **Fences it can see but no run can reach**, which it therefore forces to
  `text` rather than skipping: fences in a crate with no lib target
  (`tools/devtool` today), and fences under `#[cfg]` combinations no scan root's
  run enables (wasm-only modules, e.g. `web/src/reactive/scope.rs`). A cfg-gated
  fence is **inside** the population — `syn` reads it regardless of cfg — so it
  is a hard failure whose remedy is the marker, not an invisible skip.
- **Fences inside a multi-line `#[doc = "…"]` value** are rejected, not
  supported. libtest keys them by the attribute's line plus a markdown offset
  rather than by any line a fence opens on, so the reconciliation key cannot
  address them. A single-line `#[doc]` is indistinguishable from `///` and is
  allowed.
- **A source path containing a literal `" - "`** would truncate in the
  run-output parser, surfacing as a spurious orphan plus a spurious not-run —
  never as a silent pass. None exists; recorded because the first version of
  that parser defended this case with a test that could not fail.
- **The `text` population is uncounted**, so principle 4's multiplicity clause
  is not enforced for it. A census snapshot was considered and declined as
  machinery disproportionate to the risk, given that every such change is a
  reviewable diff.

### An exemption marker was designed and then dropped

A `compile_fail,intent_only` marker was specified and probed (it survives
rustdoc: because `compile_fail` is recognized, the block still runs and passes)
to formalize the three self-declared non-discriminating ordering proofs. It was
dropped because those three turned out not to need it: giving each fixture
`PartialEq, Eq` makes `a < b` fail **only** for the missing ordering, and a
control fence — the same shape without the suppressing option — proves the
discrimination is real. With no instance left, the marker would have been
machinery for a case that does not exist, and it would have enshrined the
circular claim rather than resolving it. If a genuinely non-discriminating proof
ever appears, add the marker then, with a real case to justify it.

> **Annotation (2026-08-27).** As of #847, doctest examples naming the prior
> common-owned rendering and sanitization call shape were historical; the
> compile-fail gate's enumeration policy remained unchanged. Current ownership:
> [ARCHITECTURE.md](../ARCHITECTURE.md).

## Consequences

**What this commits us to.** Every new `compile_fail` costs a positive companion
carrying its fixture. `ignore` is no longer available anywhere in the tree. A
fence that no run can reach must say so with `text`, at the site, in the diff.

**What it creates.** Test data that happens to be Rust must not be named `.rs`,
or the gate will police it — the fixture crates under `tools/doctests/testdata/`
are `.rs.txt` for exactly this reason. That is a structural exclusion, not an
exemption.

**What it rules out.** Gates that run a suite without establishing what the
suite covered; `ignore` as a way to park a fence; an allowlist or census as the
mechanism for saying "this one is fine".

**What it does not claim.** That a companion makes a `compile_fail`
unfalsifiable — it proves the negative fails for the stated reason, not that the
stated reason is the right one. And a marker change is still a human deleting a
proof; the gate makes that visible, not impossible.
