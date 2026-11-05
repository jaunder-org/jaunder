# #1885 — Isolate the end-to-end census leaf

## Outcome

The existing end-to-end code census has its own hermetic Nix verification leaf,
independent of TypeScript compilation. It retains the same census population and
pass/fail behavior while remaining part of the existing authoritative hermetic
validation path.

## Approach and tradeoffs

Move the existing `node end2end/e2eCensusContract.ts` invocation and its
`devtool provision-node-modules` setup out of `static-code` into one dedicated
leaf. Keep the existing script, command, configuration behavior, and exact
census population; do not replace it with `tsc` or duplicate TypeScript
compilation.

Give the leaf the complete inputs required to execute the census: the census
script and the existing `end2end` source/configuration population that
Playwright `--list` reads (including `playwright.config.ts`, selected projects,
fixtures/helpers, and test sources); the package manifests/lock inputs used to
provision that population; and the pinned Node, `devtool`, Playwright CLI/test
package and their transitive dependency environment. Provision dependencies once
in this leaf using the existing `devtool provision-node-modules` path. This is
the census input boundary, not the broad `static-code` source closure: unrelated
Elisp, CSS, Rust, or static-check inputs must not become direct census sources.
A census script, Playwright configuration/project selection, census-relevant
test/helper source, package/lock input, or pinned tool/dependency change must
invalidate the leaf; unrelated static-code-only source changes should not. Valid
Nix output reuse remains allowed. This narrow derivation boundary is preferred
to a new runner or verification framework because the existing command already
owns the contract.

Request the leaf exactly once from the existing hermetic validation
catalog/consumer, so a failed derivation fails validation rather than yielding
an unconsumed status artifact. Keep existing host checks and their populations
required and unchanged. Preserve required CI selectors, hooks, pure ERT, and
product Clippy. No schema, command framework, or ADR is needed for this bounded
leaf extraction.

## Acceptance

- The isolated leaf runs the existing census command against the unchanged
  population and preserves its current pass/fail semantics, independently of
  `tsc`.
- Its declared inputs include the census script; the `end2end`
  source/configuration population read by Playwright listing (configuration,
  project selection, fixtures/helpers, and selected test sources); provisioning
  manifests/lock inputs; and the pinned Node, devtool, Playwright and
  provisioned dependency closure. Relevant changes to each category alter the
  leaf derivation identity, while unrelated static-code-only sources do not
  broaden its direct source boundary. Legitimate valid-output reuse remains
  possible.
- The existing hermetic validation consumer requests the leaf once and
  propagates a deliberately failing leaf to a failed validation result.
- Existing host verification remains present and unchanged; maintained
  verification documentation describes the new boundary. Source/invalidation
  assertions cover the new leaf and its relevant inputs.
- No required-check selector, hook policy, census population, pure ERT check, or
  product Clippy behavior changes.

Proof is through focused source/invalidation assertions and a controlled
failure-propagation test seam in the existing validation path; execute the
relevant existing gates during implementation. Do not require a heavy full
validation run merely to draft or approve this spec.

## Boundaries

This is one incremental #1864 family carve-off, not completion of #1864. It does
not change `end2end/e2eCensusContract.ts`, add or remove census entries, alter
host checks, or take on #1886's separate Elisp scope. It does not import
sibling-branch commits or overlap #1880, #1882, or #1884. Any material host/Nix
policy or required-CI change requires separate approval.
