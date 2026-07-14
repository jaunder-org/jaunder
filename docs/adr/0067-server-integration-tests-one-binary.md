# ADR-0067: Server integration tests are one binary

- Status: accepted
- Date: 2026-07-13
- Issue: [#298](https://github.com/jaunder-org/jaunder/issues/298)

## Context

The `server` (`jaunder`) integration tests were six separate test-target crates
— `server/tests/{atompub,feed,misc,projector,storage,web}/main.rs` — each of
which `#[path = "../helpers/mod.rs"] mod helpers;`-cloned the shared helper
module. Because that one file was recompiled independently into all six binaries
and each used a different subset of it, the module could not be made lint-clean:

- `helpers/mod.rs` carried `#![allow(dead_code)]` plus two
  `#[allow(unused_imports)]` (an item dead in one binary is live in another;
  `#[expect]` is impossible because it would be "unfulfilled" in the binary that
  _does_ use the item).
- Each of the six crate roots carried a crate-level
  `#![expect(clippy::unwrap_used, clippy::expect_used)]`, because
  `clippy.toml`'s `allow-*-in-tests` only exempts `#[test]` bodies, not the many
  local non-`#[test]` helper fns (`post_form`, `make_app`, `seed_*`, …) each
  crate defines.

These nine suppressions are structural, not incidental: they exist because Rust
has no way to _share_ a module across separate test-target crates, so the
`#[path]` clone is a hand-rolled substitute for the thing Rust actually provides
for cross-unit sharing — a crate. The alternative of promoting the helpers into
a dedicated `server-test-support` library crate was measured and rejected:
collapsing is time-neutral (inner-loop relink is ~2.7 s either way — the ~300 MB
shared-dependency blob dominates link time and is the same in both), saves ~1.5
GB of disk (six ~330 MB binaries, ~90 % duplicated deps, → one ~380 MB binary),
and keeps the helpers under `server/tests/` where they stay excluded from
CRAP/coverage scoring — whereas a lib crate would pull them into it.

## Decision

The server integration tests are a **single** test binary. `server/Cargo.toml`
sets `autotests = false` and declares one
`[[test]] name = "integration", path = "tests/main.rs"`. `tests/main.rs` is the
crate root: it carries the _single_
`#![expect(clippy::unwrap_used, clippy::expect_used)]`, `mod helpers;` once, and
one `mod <subsystem>;` per former crate. Each old `tests/<x>/main.rs` becomes
`tests/<x>/mod.rs` (a subsystem module); a subsystem whose only test file
matched its directory name (`projector`, `storage`) has that file promoted to
the subsystem's `mod.rs` to avoid `clippy::module_inception`.

With one crate, `helpers` compiles once — every item is reachable from some
subsystem, so no `dead_code`/`unused_imports` suppression is needed — and the
six crate-level `#![expect]`s collapse into the one in `tests/main.rs`. Test
helpers import the both-backend harness directly from `storage::test_support`
(see ADR-0033) rather than through a compat re-export.

New shared server test helpers belong in `server/tests/helpers/`, imported as
`crate::helpers::…`; do not reintroduce a per-subsystem `#[path]` clone or a
second test target.

## Consequences

- The nine test-infra suppressions become one justified crate-level
  `#![expect]`.
- **Lost per-subsystem build isolation:** a compile error in any subsystem's
  tests now fails the whole `integration` binary, not just that subsystem's —
  the accepted cost of the single target.
- This is the third leg of jaunder's test-support split, now explicit: the
  in-process both-backend harness is `storage::test_support` (ADR-0033); the
  out-of-process fixture seeder is the `test-support` binary (ADR-0046);
  server-only integration helpers (`ensure_server_fns_registered`,
  `test_options`, the `post_form*` family, the capturing WebSub client) live in
  this one binary's `helpers` module.
- Follow-ups #429/#430 consolidate the remaining duplicated per-file helpers
  into `helpers/`, now that one shared module exists.
