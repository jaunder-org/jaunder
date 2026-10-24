# #1800 amendment: owning Git repository fixture

## Outcome and authority

Tests retain one repository value that owns its directory and behaves as a path.
Callers must not repeat a separate owner binding and cloned `PathBuf` merely to
use the shared fixture. This refines the test-only implementation approved under
`1800-fixture-ownership-both-v1`; Manager confirmed the User's abstraction
challenge is an in-scope design correction, not a new product requirement.

The original approved contract remains in commit `035a9bb7`. Its cleanup,
Git-environment isolation, and lifetime requirements remain authoritative.

## Approach

Return a small `TestRepo(tempfile::TempDir)` from the existing `temp_repo`
helper. Implement `Deref<Target = Path>` for ordinary path methods/coercions and
`AsRef<Path>` for generic filesystem consumers. Ownership exists before Git
initialization. Keep existing scrubbed Git operations; no operation façade,
custom cleanup, cloneable fixture, ownership-release method, or generic
framework.

Use one owning binding in all six existing caller modules and returning
wrappers. Provenance repositories remain declared before `Request` and alive
through its destruction and subsequent sidecar reads. Path snapshots used to
assert absence after teardown remain legitimate; ordinary caller clones do not.

## Acceptance and boundaries

Existing actual-helper regressions must still prove valid Git while owned and
cleanup after normal return, consumer error, and panic unwinding. Compile and
run those regressions and the six affected native xtask suites, preserving
assertions and exercising both path coercions and generic filesystem consumers.

The media repair stays byte-identical. No production behavior, dependency,
suppression, baseline, configuration, historical-root cleanup, or abort/SIGKILL
guarantee changes. Whole #1800 remains open. Previous PR1830 CI certifies only
its old head; the revised tree needs review and fresh applicable certification.
Retire this amendment after conformance under the normal working-spec lifecycle.
