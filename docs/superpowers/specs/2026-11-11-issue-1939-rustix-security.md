# Rustix security repair (#1939)

## Outcome

Remove the known RUSTSEC-2026-0486 rustix 1.1.4 resolution from the root,
`tools` and `xtask` Cargo contexts without suppressing the advisory or changing
application behavior. Restore the required host dependency-security verdict that
blocked PR #1937's merge-group validation.

## Approach and tradeoffs

Use Cargo's existing targeted resolver to select exactly rustix 1.1.6 in each
lockfile. Raise the three first-party rustix minimums from `1.1.4` to `1.1.6`,
preserving all existing features, default-feature choices and target conditions.
These remain normal caret requirements; committed lockfiles supply exact pins. A
lock-only repair would clear the present resolution but leave manifest minimums
allowing the known-affected release after re-resolution. No exact `=`
requirement, custom machinery or broad dependency upgrade is needed.

The implementation surfaces are exactly:

- `Cargo.toml`: `workspace.dependencies.rustix.version` only.
- `tools/traced-run/Cargo.toml`: `dependencies.rustix.version` only.
- `xtask/Cargo.toml`: Unix-target `dependencies.rustix.version` only.
- `Cargo.lock`, `tools/Cargo.lock`, `xtask/Cargo.lock`: rustix package version
  and checksum only; source and dependency entries remain unchanged.

Existing first-party requirements, tempfile 3.27.0 and xattr 1.6.1 permit 1.1.6.
The inspected transitive bounds are satisfied by already-locked dependencies.
Cargo resolution must confirm that no additional package-version change is
necessary. If it requires one, stop for renewed scope approval rather than
broadening the update. Do not hand-edit generated lockfiles as a substitute for
Cargo resolution.

Direct first-party consumers remain host theme-repository filesystem handling,
jaunder Linux shutdown, traced-run monotonic timing, and xtask timing/Unix
lease-owner handling. Transitive xattr/tar remain present; absence of direct
first-party xattr calls is not a safety exemption.

## Acceptance

1. All three lockfiles contain exactly one rustix package at 1.1.6, with the
   registry checksum verified by Cargo. The complete six-file implementation
   diff contains no unrelated package-version, source, feature, policy or API
   changes. Preserve rustix's existing dependency entries.
2. Run the product host security leaf used by Just:
   `cargo run --locked --quiet --manifest-path tools/Cargo.toml -p devtool --bin devtool -- check cargo-deny`.
   Run
   `cargo deny --manifest-path tools/Cargo.toml --config deny.toml --locked check`
   and the equivalent `xtask/Cargo.toml` command. All three must pass with
   advisories included; sandbox checks that omit advisories are not a
   replacement.
3. Run the approved focused product consumer proofs:
   `cargo xtask test-local -- --locked -p host theme_repository` and
   `cargo xtask test-local -- --locked -p jaunder shut_down`.
4. Run the approved auxiliary proofs:
   `cargo test --locked --manifest-path tools/Cargo.toml -p traced-run --lib`,
   `cargo test --locked --manifest-path xtask/Cargo.toml --test invocation_trace`,
   and
   `cargo test --locked --manifest-path xtask/Cargo.toml --lib production_baseline::platform::unix`.
   Every test selection must be nonempty. These test affected consumers, not an
   independent reproduction of upstream unsoundness.
5. Execute commands through `traced-run --`, preserve honest exit status and
   parked logs, and distinguish failures and unavailable evidence from passes.
   Existing supplied PR #1937 failure is the red evidence; do not rerun the
   merge group or unrelated lanes. Normal hooks and PR CI retain their
   authority; do not duplicate full local validation.
6. Keep this working spec through conformance. Proof logs, diff packets and
   reviews belong in ignored/session storage, not tracked evidence archives.
   Final independent Standards/Spec review must inspect the intended tree and
   proof before the normal implementation commit/push/PR lifecycle continues.

## Boundaries and ownership

The human approved `rustix-security-intake-v1` with “Approve bounded repair”,
relayed through Manager background `b0c1713e-1ecf-4649-b0f2-4522aadbbc3a` and
acknowledged by Lead. The approved consumer testing seams are confirmed. This
spec records that approved scope; no outline is required for this mechanical
patch without a new architecture, protocol, authentication or storage invariant.

Preserve the assigned `fix/rustix-rustsec-2026-0486` branch and checkout under
the managed-assignment exception. The current execution phase may commit this
spec before implementation, then stops after focused proof and an immutable
implementation review packet, before the implementation commit, push or PR.

Issue #1898 is independent in another checkout. Coordinate only intersecting
`tools/Cargo.lock` rustix version/checksum and `tools/traced-run/Cargo.toml`
rustix requirement; never replace a whole lockfile or hold its independent work.
Issue #1933 and PR #1937 remain untouched. No advisory suppression, broad
upgrade, resource override, weakened gate, merge, reenqueue or cleanup is in
scope. Exact-head human merge approval remains a separate later gate.
