# Issue #2048: debug builds embed their rust-embed trees

## Outcome

Every build of the `jaunder` crate, debug as well as release, compiles the
`Site` and system-artifact trees into the binary. A debug test binary served
from the shared kache store therefore never reads files from another checkout's
`target/`. A full `dev test-local` passes or fails on the checkout's own code.

## Approach and tradeoffs

**Cause.** `server/src/site.rs` and `server/src/system_artifacts.rs` derive
`RustEmbed` with `#[folder = "$OUT_DIR/…"]`. Without the `debug-embed` feature,
rust-embed 8.12's debug expansion embeds nothing. It reads files at run time
from the absolute folder path that the proc macro took from the build
environment. rustc's dep-info does not record that environment read. So kache
(ADR-0256) cannot see the path, and a `jaunder` lib compiled in checkout A is a
cache hit in checkout B, still pointing at A's `OUT_DIR`.

Only one of the two debug builds has this problem:

- The Nix debug build (`mkJaunderBin { debug = true; }`) already passes
  `--features rust-embed/debug-embed` on its command line, so it embeds.
- The host Cargo build under kache (`dev test-local`, `dev prepush`) sees only
  the features in `server/Cargo.toml`, which lack `debug-embed`. The failing
  test binaries in #2048 come from this build.

**Change.** Add `debug-embed` to the `rust-embed` features in
`server/Cargo.toml`, with a comment giving the reason. Every build then uses
rust-embed's `include_bytes!` path, whose inputs rustc records in dep-info.
Scratchpad probes under kache 2.1.1 gave correct results across two directories
in both cases: different contents for the same file, and a second directory
embedding one file more than the first. The second case shows kache does not
serve a smaller cached embed when the staged set grows.

Two follow-on edits:

- The Nix debug build's `--features rust-embed/debug-embed` now does nothing, so
  it is removed.
- `site.rs`'s `embedded_body` keeps its general `Cow` conversion. Its
  `Cow::Owned` arm no longer runs from `serve_site`, so a unit test covers it
  directly (Clippy rejects narrowing the signature). The comments about a
  "runtime disk-read (debug)" are corrected.

Docs that describe debug-embed as a Nix-only special case are updated:
CONTRIBUTING's sandbox paragraph and ARCHITECTURE's sandbox paragraph.

**ADR-0003.** _This contradicts ADR-0003's Consequences, which offer
rust-embed's debug disk reads as the mitigation for tying assets to the build._
That mitigation stopped working when `server/build.rs` began staging assets into
`OUT_DIR`. Any asset change already reruns the build script and recompiles the
crate. ADR-0003's live decision, single-binary embedding, stands. This is
recorded in ARCHITECTURE's asset section and needs no new ADR.

**Alternatives rejected.**

- _Make kache key on the folder path._ We don't own kache's keying, and kache
  has no per-crate setting that could add an untracked environment read.
- _Exclude `jaunder` from caching._ That gives up caching the largest product
  crate to keep a debug read mode that nothing needs.

**Cost accepted.** Every debug link of `jaunder` (the lib tests, the `jaunder`
bin, `integration`, and the instrumented coverage links) now carries the staged
bytes. That is the system artifacts, `public/`, and the CSR bundle that
`dev test-local` always supplies through `JAUNDER_CSR_BUNDLE_DIR`: about 7 MB
including the `.br`/`.gz` variants. Compile time and the space used in `target/`
and the kache store both grow somewhat. Release builds already pay this.

## Acceptance

- **Regression test (commit).** A unit test in each of `site.rs` and
  `system_artifacts.rs` asserts that the embed is non-empty and that every entry
  returns `Cow::Borrowed`, meaning compiled-in bytes. The dynamic debug mode
  returns `Cow::Owned` from a file read.
  - These tests guard the mechanism. They do not reproduce a cross-checkout
    cache hit.
  - They assume a staged bundle, as `dev test-local` and the Nix coverage check
    provide. A bare `cargo test` with no bundle stages no system artifacts. The
    existing bundle-dependent tests make the same assumption.
- **Proof (execute).** Both tests fail before the feature change and pass after
  it under `dev test-local -p jaunder <filter>`. A full `dev test-local` and the
  coverage gate pass.
- No `debug-embed` reference remains in `nix/packages.nix`, and the Nix debug
  sandbox artifacts still build. The CI hermetic lanes cover this.

## Boundaries

- No change to kache configuration, ADR-0256, or the `dev` compile-cache
  environment.
- No change to what is staged or served. This changes only how debug builds
  carry the bytes.
