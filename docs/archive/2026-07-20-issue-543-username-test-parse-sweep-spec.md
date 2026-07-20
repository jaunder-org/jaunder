# Spec — #543: sweep inline `Username` test-parses to `parse_username`

- Issue: [#543](https://github.com/jaunder-org/jaunder/issues/543)
- Milestone: Domain-value type safety (newtypes)
- Governing: the newtype test-helper convention (build test values via
  `common::test_support::parse_<name>()`);
  [ADR-0033](../../adr/0033-shared-db-test-harness-crate.md) (feature-gated
  cross-crate test support)
- Origin: follow-up from #542
- Date: 2026-07-20

## Problem

#542 added `common::test_support::parse_username`, but ~28 `#[cfg(test)]` sites
across the workspace still build a `Username` inline
(`"…".parse::<Username>().unwrap()` / `let u: Username = "…".parse().unwrap()`).
The convention is to route test fixtures through the helper so the parse isn't
re-spelled at every call site. This is a pure mechanical, no-behavior-change
sweep.

## Decision

Replace **test-fixture** `Username` parses with `parse_username(…)`; leave
**runtime** parses and the `Username` type's **own** tests untouched.

### Sweep (test fixtures)

| File                               | Sites                                        |
| ---------------------------------- | -------------------------------------------- |
| `storage/src/posts.rs`             | 1                                            |
| `storage/src/atomic.rs`            | 3                                            |
| `web/src/posts/mod.rs`             | 4                                            |
| `web/src/posts/server.rs`          | 1                                            |
| `web/src/posts/listing.rs`         | 2                                            |
| `web/src/auth/server.rs`           | 7                                            |
| `web/src/feed_discovery/labels.rs` | 1                                            |
| `web/src/subscriptions/server.rs`  | 2                                            |
| `web/src/render/mod.rs`            | 12                                           |
| `web/src/forms/field.rs`           | 1 (`.ok()` form → `Some(parse_username(…))`) |
| `web/src/taglist/markup.rs`        | 1                                            |

Transforms:

- `"x".parse::<Username>().unwrap()` → `parse_username("x")`
- `let u: Username = "x".parse().unwrap();` → `let u = parse_username("x");`
- `var.parse::<Username>().unwrap()` (where `var: &str`) → `parse_username(var)`
- `"x".parse::<Username>().ok()` → `Some(parse_username("x"))` (field.rs)

Each touched test module gains `use common::test_support::parse_username;` (both
`storage` and `web` already carry `common`'s `test-support` dev-dep). Any
`use …::Username` import left unused after the sweep is removed.

### Do NOT touch (out of scope, deliberately)

- **Runtime parses of external/untrusted input** — these must stay fallible
  `.parse()`, not a panicking fixture helper: `storage/src/helpers.rs:195` (DB
  decode → `sqlx::Error::Decode`), `server/src/feed/handlers.rs`,
  `server/src/projector/mod.rs`, `common/src/feed/feed_path.rs`,
  `web/src/pages/posts.rs`.
- **`common/src/username.rs`'s own tests** — they exercise `Username::from_str`
  directly (validation `.is_ok()`/`.is_err()`, normalization, `Display`, serde
  round-trip): the parse _is_ the unit under test, so routing it through a
  helper that merely wraps `.parse().expect()` would obscure what's tested. Left
  as-is.

## Acceptance criteria (observable)

1. **Every listed test-fixture site uses `parse_username(…)`** — no
   `"…".parse::<Username>().unwrap()` / `let _: Username = "…".parse().unwrap()`
   fixture form remains in the swept files' `cfg(test)` code.
2. **Runtime parses untouched** — the "do NOT touch" sites are byte-unchanged;
   `common/src/username.rs` is byte-unchanged.
3. **No unused imports** — swept modules import `parse_username`; a now-unused
   `Username` import is removed (clippy clean).
4. **No behavior change** — tests assert the same things; `parse_username`
   normalizes/validates identically to the inline `.parse()` it replaces.
5. **Gate green** — `cargo xtask check` passes (static + clippy + coverage
   across the affected crates).
