# Issue #992: Split the Elisp suite by package module

## Outcome

Replace the 1,881-line `elisp/test/jaunder-test.el` with eight self-contained
ERT files named for the production module whose behavior they prove. Preserve
all 144 existing ERT declarations and every assertion unchanged in meaning. The
existing wildcard runner, package code, integration suites, test interfaces, and
observable behavior remain unchanged.

## Load-bearing decisions

- Delete `elisp/test/jaunder-test.el`; do not leave a residual catch-all suite.
  `elisp/jaunder.el` is an aggregate loader with no behavior owned by this
  suite, so an empty or cross-cutting `jaunder-test.el` would not have one named
  responsibility.
- Add exactly these module-owned suites:
  - `jaunder-transport-test.el` owns URL construction, Basic/auth-source secret
    handling, plz response normalization, request headers and curl escaping, and
    transport-error behavior from `jaunder-transport.el`.
  - `jaunder-org-test.el` owns Org-buffer-to-entry mapping, header/body parsing,
    Org link records/substitution, property/keyword mutation, and date-zone
    capture from `jaunder-org.el`. Publication-time projection assertions remain
    here when the function under test is `jaunder--org->atom`; datetime
    primitives used by that mapping remain in the datetime suite.
  - `jaunder-datetime-test.el` owns offset/zone resolution, UTC-to-Org
    rendering, current-zone discovery, and zone-mismatch warnings from
    `jaunder-datetime.el`.
  - `jaunder-config-test.el` owns blog resolution, validation/normalization,
    dynamic active-blog context, and active accessors from `jaunder-config.el`.
  - `jaunder-atom-test.el` owns entry XML serialization and direct
    response-field harvesting from `jaunder-atom.el`.
  - `jaunder-media-test.el` owns MIME selection, local-link eligibility and
    collection, upload/preflight behavior, positional substitution/localization,
    Git media discovery, and untracked-media warnings from `jaunder-media.el`.
  - `jaunder-publish-test.el` owns publish validation, Location ID extraction,
    force-draft, draft rename/write-back, idempotency/retry orchestration,
    interactive publish/new-post behavior, and cross-module warning invocation
    on the publish path from `jaunder-publish.el`.
  - `jaunder-service-test.el` owns service-document feature parsing/fetching,
    capability caching, and missing-format warning behavior from
    `jaunder-service.el`.
- Do not add empty suites for `jaunder-entry.el` or `jaunder-warn.el`. The
  former supplies the entry model exercised by owner tests; the latter supplies
  the warning primitive exercised through the module-specific warning behavior
  in datetime, media, and service. Existing dedicated pull, pull-media,
  reconcile, delete, wait, and coverage suites remain untouched.
- Assign cross-module scenarios to the production function being invoked, not
  every dependency they happen to exercise. In particular, publish-path warning
  independence and byte-identical request behavior stay with publish; they do
  not justify a generic cross-cutting suite.
- Each new file is self-contained after the runner's existing
  `(require 'jaunder)`. Helpers and fixtures move into the only suite that
  consumes them. A small helper used by more than one new suite is duplicated
  locally rather than introducing a test-support load protocol or relying on
  file order. No suite depends on another suite's prior load, mutable state,
  helper, macro, or fixture.
- Every file retains lexical binding, standard Emacs Lisp header/commentary/code
  sections, and an `ends here` trailer matching its filename. Test names remain
  globally unchanged, so external ERT selectors and failure identities remain
  stable.
- ADR-0031 remains exact: the pure ERT suite is still a separately tested Elisp
  subproject, every existing pure mapping/transform assertion stays covered, and
  the host/hermetic gate continues using the same CWD-independent batch runner
  and toolchain.

## Acceptance

- `elisp/test/jaunder-test.el` is absent, and the eight named module suites
  exist with one production owner each.
- The union of the eight files contains exactly the same 144 `ert-deftest` names
  as the pre-split monolith, with no missing or duplicate test declarations.
- Every pre-split helper, macro, fixture, test body, cleanup form, dynamic
  binding, skip condition, and assertion remains with the contract it supports;
  no test relies on another test file's load order or side effects.
- Each test is housed with the module defining its primary function under test.
  Cross-module dependencies used as fixtures do not change ownership.
- `elisp/scripts/run-tests.el` remains byte-for-byte unchanged and continues to
  discover every new suite through its existing `test/*-test.el` wildcard.
- Production files, integration-test files, existing dedicated pure suites, and
  ERT names/selection behavior remain unchanged.
- The unchanged batch ERT command passes with all pre-split tests registered and
  executed, Elisp formatting passes, and the repository pre-commit gate passes.

## Boundaries

- No production Emacs Lisp, runner, load-path, wildcard, test name, assertion,
  fixture semantics, warning behavior, network behavior, filesystem behavior, or
  cleanup behavior changes.
- No shared `jaunder-test-support.el`, generated suite manifest, load-order
  convention, or new runner abstraction is introduced.
- No monolith tests are merged into the existing pull, pull-media, reconcile,
  delete, wait, coverage, or integration suites.
- No new tests are required: this issue relocates the complete existing suite;
  it does not change an observable contract.
- No ADR is needed: this is a behavior-preserving projection of ADR-0031 into a
  test layout matching the production package modules.
