# Temporary SQLx SQLite dependency patch

Issue [#1928](https://github.com/jaunder-org/jaunder/issues/1928) repairs
RUSTSEC-2026-0459 without changing SQLite/PostgreSQL features or application
behavior. Keep this note while the local patch exists.

## Provenance and packaging

`vendor/sqlx-sqlite-0.9.0/` comes from the published `sqlx-sqlite 0.9.0`
`.crate` archive, SHA-256
`488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c`. Its 54
retained files are the normalized `Cargo.toml`, complete unchanged `src/`, and
archive `LICENSE-MIT` / `LICENSE-APACHE` files. The sole driver change is the
normalized manifest's `libsqlite3-sys` bound: `>=0.30.1, <0.38.0` →
`>=0.30.1, <0.40.0`, permitting fixed `0.39.1` (bundled SQLite 3.54.1). Registry
`sqlx-core = "=0.9.0"` is unchanged. Do not copy the nested lockfile, original
manifest, or archive/cache metadata.

The two archive license files contain relative `../LICENSE-*` pointers, not full
notices. Preserve those archive bytes. Two **additional ancillary files**,
`vendor/LICENSE-MIT` and `vendor/LICENSE-APACHE`, supply the full upstream
notices at their referenced paths. These are from the authoritative SQLx
[`v0.9.0` tag](https://github.com/transact-rs/sqlx/tree/v0.9.0), commit
`75bc0487eb661da811bb7a3c5d158f1bd463fef4`, not changes to the 54-file archive
subset. Their SHA-256 hashes are:

- MIT: `5abbdd842310c8f1650fcde3a5a51a3e09412b9e5a6eea87519e0db9e32b339d`
- Apache-2.0: `c8f5453612253e8ea8bad241617fe74c4a6d5d592ba64ff881668c211b50ac99`

Keep both ancillary notices in source distributions and Nix source closures. Nix
must admit only the selected driver files plus those notices. Crane's
`extraDummyScript` preserves the real driver during dependency preparation:
registry SQLx macros require its API even while workspace targets are stubbed.

Any new consumer of the root Cargo metadata must account for the root path patch
even if its selected executable does not use SQLx. Admit directory traversal
through `vendor/`, `vendor/sqlx-sqlite-0.9.0/`, and its `src/` tree; for driver
files, allow exactly `Cargo.toml`, every `src/` descendant, and `LICENSE-MIT` /
`LICENSE-APACHE`. Also admit the two full parent notices at `vendor/LICENSE-MIT`
and `vendor/LICENSE-APACHE`. Reject other driver-root files before generic
Cargo/data filters can admit them. If dependency preparation compiles SQLx
macros, use Crane's existing `extraDummyScript` to restore that driver and its
parent notices after stubbing, not the entire workspace. The separate
`ux-sandbox` consumer owns applying this guidance when integrating the security
repair; it is not part of these three workspace source closures.

## Removal trigger

When adopting a compatible **published SQLx release**, first verify that its
published SQLite driver's bound permits the fixed `libsqlite3-sys` version. Then
remove together:

- The path patches in root, `tools/`, and `xtask/` `Cargo.toml` files.
- The 54-file driver copy and the two ancillary notices under `vendor/`.
- Their Nix admission exceptions/assertions and dummy-source restoration in
  `nix/packages.nix` and `nix/xtask.nix`.
- The isolated metadata-root patch projection in
  `xtask/src/steps/common_host_target_closure.rs` and its patch-specific
  fixture.
- The exact imported-source ownership entry in `tools/doctests/src/roots.rs`.
  These upstream sources are dependency code, not maintained workspace doctest
  targets; the existing workspace/host scan roots remain unchanged.
- This note and its manifest comment link.

Update all three workspace lockfiles to the released registry driver; recheck
package identities, unique SQLite native links, bundled SQLite, advisories, and
offline Nix builds through existing proof lanes. Do not force an incompatible
resolution or switch backend features to remove the patch.
