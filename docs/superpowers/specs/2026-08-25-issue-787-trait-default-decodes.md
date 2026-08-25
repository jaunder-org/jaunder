# Issue #787 — scan trait-default SQLx decode sites

## Outcome

`sqlx-newtype-decode` includes calls in trait default-method bodies in its
structurally enumerated decode population. The widened gate fails closed on
unapproved targets, while the site-configuration interface names raw-text reads
explicitly instead of making non-SQLx calls look like row decodes.

## Load-bearing decisions

- Trait default-method bodies join the same structural population as free
  functions and implementation methods. Required trait methods without a body
  contribute no decode sites.
- Decode target precedence remains nearest-declared type first: call-level
  turbofish, then an ascribed `let`, then the enclosing function or trait-method
  return type.
- The scanner reuses its existing function-name, return-type, and `let`
  ascription state while visiting a trait default body; it does not introduce a
  trait-specific inference path.
- `SiteConfigStorage` names its raw-text primitive `get_raw`. This is an
  interface-level distinction between stored site-configuration text and SQLx
  `Row::get`/`try_get`, not receiver-name guessing in the scanner.
- Every `SiteConfigStorage::get` caller migrates to `get_raw`; no compatibility
  alias remains. The seven newly visible aggregate-return sites therefore leave
  the scanner population without `NotADecodeTarget` entries.
- The SQL query inside `get_raw` remains a real decode into `(String,)` and
  retains its existing exact `OpaquePayload` allowlist entry under the new
  function name.
- Existing trait defaults whose return leaves are already approved remain
  approved through the normal type model and receive no allowlist entry.
- Scanner population documentation states explicitly that enclosing
  trait-default-method return types participate. `SiteConfigStorage`
  documentation drops its stale claim that trait default bodies are invisible
  while retaining the independent requirement that `get_smtp_config` perform
  typed SQLx bridge decoding at the query boundary.

## Acceptance

- A synthetic `.get` call in a trait default body is recorded under the exact
  trait-method return target `Result<i64, E>`; removing trait-item visitation
  makes that assertion fail.
- Synthetic tests assert exact target vectors or failure text while proving
  call-level turbofish and typed-`let` precedence over the trait method return,
  and proving a required method without a default body adds no site.
- `devtool run -- cargo test --manifest-path xtask/Cargo.toml sqlx_newtype_decode_check`
  passes with the synthetic trait-default coverage.
- `cargo xtask check --no-test` passes with no new site-configuration
  `NotADecodeTarget` entries and no unallowlisted trait-default site.
- `cargo xtask validate --no-e2e` passes with the widened population.

## Boundaries

- No change to the `.bind` direction, gate-module layout, or site-configuration
  value typing.
- No receiver-name or SQL-text heuristic and no broad function/file exemption.
- No architecture decision record or implementation outline: this routine
  static-gate bug changes no architecture, schema, external protocol, security,
  concurrency, or storage-correctness contract.
