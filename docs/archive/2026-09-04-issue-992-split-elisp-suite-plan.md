# Issue #992: Split the Elisp suite by package module — implementation outline

**Approved specification:**
`docs/superpowers/specs/2026-09-04-issue-992-split-elisp-suite-spec.md`

## 1. Capture the pre-split suite manifest

**File:** `elisp/test/jaunder-test.el` (read-only baseline)

- Extract the ordered 144 `ert-deftest` names and the five helper/fixture
  declarations from the current file before moving code.
- Record each declaration's complete top-level form and defining production
  owner. Treat the production function invoked by the test as authoritative;
  dependencies used to construct inputs do not make a test cross-owned.
- Use this manifest only as verification data. Do not add a generated manifest
  or alter the runner.

## 2. Create the transport, configuration, and datetime suites

**Files:** new `elisp/test/jaunder-transport-test.el`,
`elisp/test/jaunder-config-test.el`, and `elisp/test/jaunder-datetime-test.el`

- Move transport-owned URL/auth/auth-source/plz/request/curl/error tests into
  `jaunder-transport-test.el`.
- Move blog resolution/validation/normalization, active context, and accessor
  tests into `jaunder-config-test.el`.
- Move numeric offsets, zone resolution/discovery, UTC rendering, and
  zone-mismatch warning tests into `jaunder-datetime-test.el`.
- Give each file lexical binding, matching commentary, `(require 'ert)`,
  `(require 'jaunder)`, and its own matching `ends here` trailer. Keep required
  helpers local and before first use; do not rely on another test file.

## 3. Create the Org, Atom, and media suites

**Files:** new `elisp/test/jaunder-org-test.el`,
`elisp/test/jaunder-atom-test.el`, and `elisp/test/jaunder-media-test.el`

- Move Org-to-entry/header/body tests, including publication projections whose
  invoked behavior is `jaunder--org->atom`, plus Org link, property/keyword
  mutation, and `jaunder--ensure-date-tz` tests into `jaunder-org-test.el`; move
  the entry fixture with them.
- Move Atom XML serialization and direct response-harvesting tests into
  `jaunder-atom-test.el`.
- Move MIME, upload/preflight, local-link eligibility/collection,
  substitution/localization, Git media discovery, and untracked-media warning
  tests into `jaunder-media-test.el`; move the media collection helper with them
  and duplicate the small warning-capture macro locally where required.
- Preserve every top-level form and intra-file order within each owner,
  including temporary-file cleanup, dynamic bindings, and skip conditions.

## 4. Create the service and publish suites; remove the monolith

**Files:** new `elisp/test/jaunder-service-test.el`, new
`elisp/test/jaunder-publish-test.el`, remove `elisp/test/jaunder-test.el`

- Move service-document parsing/fetching, cache, capability warning, and service
  fixture forms into `jaunder-service-test.el`; duplicate the response helper
  and warning-capture macro locally before their first uses.
- Move publish validation, Location ID extraction, force-draft,
  rename/write-back, idempotency/retry, publish-path warning orchestration,
  request identity, and interactive publish/new-post tests into
  `jaunder-publish-test.el`; move its response helper and duplicate the small
  warning-capture macro locally.
- Remove the original file only after every declaration has an owner. Do not
  create `jaunder-entry-test.el`, `jaunder-warn-test.el`, a shared support file,
  or a residual `jaunder-test.el`.

## 5. Prove exact relocation and behavior

- Compare the working-tree union of the eight new files with the pre-split
  `HEAD:elisp/test/jaunder-test.el` using the Emacs Lisp reader. Compare
  normalized complete top-level forms for every `ert-deftest` and each original
  helper/fixture, not names alone: exactly 144 unique test forms, empty
  missing/extra/duplicate test sets, and exact semantic equality of bodies,
  assertions, cleanup, bindings, and skip forms. Permit only the intentional
  local duplication of the warning-capture macro and response helper; confirm
  every helper is defined before use within each consuming file.
- Confirm `elisp/scripts/run-tests.el`, all production files, integration files,
  and existing dedicated pure suites are byte-for-byte unchanged in the diff.
- Run the actual unchanged batch runner:
  `devtool run -- emacs --batch -Q -l elisp/scripts/run-tests.el`.
- Run the Elisp formatting verifier: `devtool run -- devtool check elisp-fmt`.
- Run `devtool run -- cargo xtask check --no-test` once for the broader static,
  ERT, and Clippy surface; apply formatter changes before staging.
- Review the final diff against the approved spec, with particular attention to
  owner assignment, test-body retention, self-containment, no load-order
  dependencies, and untouched runner/production/dedicated suites.
- Stage the complete suite replacement and commit it. The pre-commit hook is the
  authoritative repository gate; if it changes files, inspect and restage only
  intended changes before retrying.

## 6. Deliver and archive

- Move the approved spec and this outline from `docs/superpowers/` to
  `docs/archive/` as the last semantic change, format them with pinned Prettier,
  stage, and commit through the same pre-commit gate.
- Push the branch, open a PR referencing #992, and monitor it with
  `cargo xtask pr watch` until it reports `ready-to-land` or a concrete failure.
- Stop at the explicit merge-approval gate. Do not run `cargo xtask pr land`
  without per-PR human approval.
