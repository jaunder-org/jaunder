# #1893 — helper Nix bootstrap amendment (A2)

Issue: [#1893](https://github.com/jaunder-org/jaunder/issues/1893)

**Status:** approved in scope by the direct human response **“yes, do it through
nix”** to the qualified A2 v3 proposal; the corrected four-step outline was
separately approved by the direct human response **“approved”** following cold
review `result:a2-outline-review#2` (no findings). These approvals cover only
the stated A2 scope. Exact proposal snapshot and both approval records are
retained at `/tmp/issue-1893-helper-bootstrap-intake/`; the v3 snapshot SHA-256
is `a79f0939488c0b31e3ccce653f50ba597a3ff2109da4bbc9daeeeeb858175b01`.

## Outcome

Build the `ux-sandbox` helper as one independently filtered, debug-profile Nix
package. Have the existing `just sandbox` positional front door run that package
directly, and request its ordinary build in an existing CI job configured with
the repository's current Cachix action. No cache hit, faster warm run, upload,
or substitution is promised by package eligibility alone.

## Approach and tradeoffs

Use the existing Crane workspace closure/source-filter and placeholder
conventions to keep the helper derivation bounded to `ux-sandbox`, its local
Cargo path-dependency closure, required workspace manifests/configuration, root
lockfile, and pinned toolchain. Vendor against that filtered source and build
only the helper; do not inherit the broad application source, CSR/public inputs,
or product `cargoArtifacts`. Keep one runnable `packages.ux-sandbox` output at
the helper's current debug profile.

Replace only the Cargo build/exec in `Justfile` with
`exec nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox -- "$@"`.
Preserve `[ positional-arguments ]` and literal argv forwarding. The package
does not change application artifact selection or any helper lifecycle, profile,
storage, lock, signal, generation, seed, or status behavior.

The CI request uses an existing job after `.github/actions/setup-ci` and leaves
the Cachix action/configuration unchanged. The current `pushFilter` excludes
`jaunder-coverage` and `jaunder-e2e`; it is not a whitelist, so the helper name
is eligible for ordinary publication. A requested build or eligible output is
not proof of actual upload or later substitution.

Cargo normally reuses up-to-date local helper artifacts. This change does not
assert that the Cargo route recompiles each time or that Nix is faster on every
warm invocation. Relevant changed manifests, lock/toolchain files, or local
dependencies can invalidate the helper derivation. No performance benchmark or
cache-hit test is in scope.

## Acceptance

- `nix build`/`nix run` resolve one runnable helper output with the debug
  profile and bounded helper-only source/dependency closure; no product debug
  artifacts, application assets, or CSR inputs are inherited.
- A finite derivation/source-boundary proof shows a tracked helper Rust edit
  changes the helper derivation, while unrelated application Rust, CSR,
  public-asset, and documentation edits do not. Required root/workspace
  manifests, lockfile, toolchain, or helper-dependency edits may change it; no
  claim is made that all such changes leave the derivation unchanged. In bounded
  observations using only task-owned temporary inputs, verify tracked-dirty and
  intent-to-add helper inputs participate in Git-flake capture, while a new
  untracked helper input is omitted without being refused. Preserve exact
  baseline bytes/status; do not touch another checkout/worktree. Do not add a
  reusable framework, production staging/refusal, or broad input matrix.
- `just sandbox` invokes the current checkout's package with exact literal
  caller argv and no Cargo helper build. Existing source admission and
  application artifact/lifecycle behavior remain unchanged.
- Existing Cachix-configured CI requests and completes `.#ux-sandbox` build.
  Only direct retained CI evidence can support an actual upload/substitution
  claim.
- Update current `CONTRIBUTING.md`, architecture view, and standalone-sandbox
  ADR draft after conformance, without restoring the retired A3 spec/outline or
  promoting the ADR.

## Boundaries and limitation

Tracked dirty and intent-to-add inputs participate in the Git-flake snapshot.
New relevant untracked helper inputs are neither automatically included nor
refused by the Just `nix run` front door; users must `git add` or `git add -N`
them before Nix can include them. Verify this with bounded task-owned temporary
inputs and baseline preservation, not another worktree/checkout. Do not claim a
pre-bootstrap admission check, auto-stage, or refusal for omitted files.

Do not add cache trust/filter/token/publication machinery, generic frameworks,
dependencies, Python, resource settings, test-verdict caching, or
application/product behavior changes. Do not change Nix source filters to admit
unrelated files. No modification to #1923 staging permissions or shared SQLite
advisory handling is included; any shared SQLite repair stays with its
Manager-coordinated owner. The exact approved proposal and qualification remain
in the external snapshot above.
