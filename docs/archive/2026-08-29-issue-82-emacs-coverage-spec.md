# Issue #82 — Emacs Protocol Client coverage

Issue: [#82](https://github.com/jaunder-org/jaunder/issues/82). Milestone: Emacs
blogging front-end.

## Outcome

Jaunder measures the production Emacs Protocol Client with a hermetic,
repository-local line-coverage gate. The authoritative verdict combines the fast
pure ERT suite with the live server-backed ERT suite and runs in CI through
`cargo xtask validate --no-e2e`.

## Load-bearing decisions

- Undercover is the pinned Emacs Lisp coverage engine. It instruments source
  before any measured module loads and emits LCOV plus a human-readable summary;
  it never uploads results to an external service.
- One hermetic NixOS VM check runs both ERT populations and merges their
  observations into one report. Neither population must cover every line by
  itself; their union is authoritative.
- The denominator starts from a pre-test census of every production module and
  every top-level source form under `elisp/`. Each form must produce one or more
  Edebug executable stop points. A form that produces none contributes its
  opening line as an uncovered synthetic census point. Every ordinary census
  point must reconcile with one LCOV record. A missing module, form, or point is
  a gate failure rather than an implicit exclusion. Tests, test helpers, batch
  runners, vendored dependencies, generated files, and byte-compiled files are
  outside the census.
- The consumer automatically classifies only a zero-stop form whose census
  contains exactly that single synthetic opening-line point as structural:
  `require`, `provide`, `declare-function`, `defgroup`, and `cl-defstruct`; plus
  `defvar`, `defconst`, and `defcustom` with an inert initializer. The closed
  inert grammar is an absent initializer; `nil` or `t`; a number, string,
  character, or keyword; a quote or function-quote; or a literal vector.
  Computed calls, variable references, backquote/unquote, and every other
  evaluated or unknown initializer remain measurable or require an individual
  justification. A classifiable form with an ordinary point or LCOV observation
  is a guard violation, not an exemption.
- Enforcement is stateless and strict: every executable census line is covered
  or carries a trailing `;; cov:ignore: <reason>` on that same physical line.
  The reason is trimmed and must remain non-empty. Structural exclusions are
  counted as ignored/exempt without a source marker. A malformed marker or a
  marker on a covered, non-executable, or structurally excluded line fails, so
  an obsolete justification cannot silently remain. There is no block marker,
  baseline, ratchet, or percentage threshold.
- Undercover or Edebug limitations do not silently narrow the denominator.
  Production macro forms receive suitable Edebug instrumentation where
  practical; remaining executable or unclassified zero-stop forms need an
  individual reason-bearing marker on their synthetic opening-line census point.
- The Emacs verdict remains separate from Rust LLVM coverage and Rust CRAP
  analysis. The checks share policy—stateless, source-local justification—not a
  report format or denominator.
- The combined check replaces the existing `e2e-elisp-integration` check and
  amends ADR-0035's gate placement. It runs once in `validate --no-e2e` and the
  CI static lane; full `validate` inherits that verdict without rerunning the
  live suite or defining a second coverage population.
- This engineering decision changes no domain term in `CONTEXT.md`. Its durable
  rationale is recorded in `docs/adr/0162-elisp-stateless-coverage-gate.md`.

## Acceptance

- The pinned coverage engine is supplied by the repository's Nix-managed Emacs
  toolchain; the check does not resolve packages from the network at runtime.
- A single hermetic check runs the pure and live ERT suites with production
  modules source-loaded, then realizes `$out/elisp-coverage/lcov.info`,
  `$out/elisp-coverage/summary.txt`, and `$out/elisp-coverage/status.json` for
  every controlled outcome.
- The producer distinguishes successful execution, ERT failure, instrumentation
  failure, and invalid report data without losing its diagnostic artifacts. A
  separate consumer maps those statuses and uncovered census lines to the
  failing xtask verdict. An uncontrolled Nix or VM infrastructure failure
  remains an ordinary derivation failure.
- Census/report reconciliation proves that every production Emacs Protocol
  Client module and top-level form is present, every non-structural form yields
  Edebug stop points or a synthetic point, and every ordinary point has exactly
  one LCOV record; only the agreed non-production populations are absent.
- Fixtures prove that only zero-stop `require`, `provide`, `declare-function`,
  `defgroup`, and `cl-defstruct`, plus `defvar`, `defconst`, and `defcustom`
  with the closed inert initializer grammar, are automatically structural. Those
  exclusions are counted without markers only when the census has exactly the
  single synthetic opening-line point.
- A fixture with an uncovered executable or unclassified production line fails
  the consumer. The same fixture passes when exercised by either ERT population
  or when its uncovered line carries `;; cov:ignore: <reason>` with a non-empty
  trimmed reason.
- Fixtures prove that missing modules, missing forms, missing or duplicate LCOV
  points, empty or malformed markers, markers on covered, non-executable, or
  structural lines, and an otherwise structural form with an ordinary point or
  LCOV observation fail. A production macro cannot disappear silently from the
  census.
- `cargo xtask validate --no-e2e` runs the producer and authoritative consumer;
  the existing CI static lane therefore enforces the same verdict without a new
  external coverage service, while full `validate` does not repeat live ERT.
- Existing pure and live ERT behavior remains green under instrumentation, and
  the current interim coverage-exemption prose is replaced with the enforced
  policy in contributor and Emacs-client documentation.

## Boundaries

- No Emacs Protocol Client behavior, AtomPub contract, or test population is
  redesigned by this work.
- Rust coverage, CRAP scoring, browser/Wasm coverage policy, and Playwright e2e
  coverage are unchanged.
- No Codecov, Coveralls, badge, token, upload step, committed baseline, or
  percentage target is introduced.
- Test files and support infrastructure are not coverage targets merely because
  they execute inside the producer VM.
