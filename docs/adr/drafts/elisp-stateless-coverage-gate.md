# ADR-DRAFT: Elisp stateless coverage gate

- Status: proposed
- Date: 2026-08-29
- Issue: [#82](https://github.com/jaunder-org/jaunder/issues/82)

## Context

[ADR-0031](../0031-elisp-separately-tested-subproject.md) deliberately kept
Elisp outside the Rust coverage gate: `cargo-llvm-cov` cannot instrument Emacs
Lisp, leaving ERT's "a test per pure function" as an unenforced expectation.
[ADR-0035](../0035-elisp-live-integration-harness.md) subsequently established
both pure and live ERT populations, but they still have no shared line-coverage
verdict. The production Emacs Protocol Client needs an enforceable coverage
contract that covers its pure behavior and its real-server behavior without
measuring test infrastructure as product code.

The Rust gate's stateful predecessor was replaced by the stateless model in
[ADR-0050](../0050-stateless-coverage-gate.md): a strict verdict derived only
from the report and source tree, with reviewable, source-local exemptions. The
Elisp gate should preserve that property rather than introduce a baseline,
ratchet, or an approval artifact whose meaning drifts as files move.

Some top-level forms are inherently declarative and produce no executable
coverage stop. Requiring a repeated source marker for those forms obscures the
actual coverage decisions. Conversely, a declaration head alone cannot prove
that initialization is inert: `defvar`, `defconst`, and `defcustom` may evaluate
production behavior. The gate therefore needs a source-parsed, closed
classification that automates only the declarative cases and fails visibly when
instrumentation contradicts that classification.

## Decision

Add a pinned Undercover-based line-coverage gate for the **production Emacs
Protocol Client modules only**. Test files, test helpers, runners, vendored and
generated sources, and byte-compiled files are outside its denominator.

Before either suite runs, the gate will census every in-scope module and every
top-level source form in those modules. Every ordinary Edebug census point must
have exactly one LCOV record. A zero-stop form with exactly its single synthetic
opening-line point is automatically structural only when it is `require`,
`provide`, `declare-function`, `defgroup`, or `cl-defstruct`; or when it is
`defvar`, `defconst`, or `defcustom` with an inert initializer. The closed inert
initializer grammar is absent; `nil` or `t`; a number, string, character, or
keyword; a quote or function-quote; or a literal vector. Computed calls,
variable references, backquote/unquote, and any other evaluated or unknown
initializer remain measurable or need an individual justification. A candidate
structural form with an ordinary point or LCOV observation is a guard violation,
not an exemption. A missing module, form, or point, or a duplicate point, is a
failure; neither Undercover nor Edebug limitations silently narrow the
population.

One hermetic NixOS VM producer will run and merge both ERT populations: the
serverless pure suite and the live suite that uses ADR-0035's self-booting
server harness. Their union is authoritative. For every controlled outcome, it
will realize `$out/elisp-coverage/lcov.info`, `$out/elisp-coverage/summary.txt`,
and `$out/elisp-coverage/status.json`, preserving diagnostics for successful
execution, ERT failure, instrumentation failure, and invalid report data. An
uncontrolled Nix or VM infrastructure failure remains an ordinary derivation
failure and makes no artifact promise. A separate consumer will map producer
status and uncovered census points to the failing `cargo xtask` verdict.

This combined producer and consumer replace the existing `e2e-elisp-integration`
check. That explicitly amends ADR-0035's placement: the live suite is no longer
a separate e2e aggregate member or CI job; it runs once through
`cargo xtask validate --no-e2e` and the CI static lane, and full `validate`
inherits that verdict without rerunning it or defining a second coverage
population.

The verdict is stateless and strict: every executable or unclassified census
line must be covered or have `;; cov:ignore: <reason>` trail that same physical
line. The reason is trimmed and non-empty. Automatic structural exclusions
increment ignored/exempt accounting without source markers. There is no block
form. A malformed marker, a marker on a covered, non-executable, or structural
line, or any failure to determine whether a line belongs in the instrumented
population fails closed.

Macro expansion does not make an unmeasured escape hatch. A production macro
must have an Edebug specification that permits its lines to be instrumented, be
one of the narrowly defined structural declarations, or carry an individual
source-local, reasoned justification on its synthetic opening-line census point.
Broad file- or directory-level exclusions are not an alternative to that
per-form decision.

Rejected alternatives:

- Retain ADR-0031's ERT-discipline-only rule: it cannot distinguish an omitted
  test from an intentionally unmeasurable line and does not fail a regression.
- Produce pure and live coverage separately or aggregate it outside the hermetic
  producer: that splits one denominator across environments and makes the final
  verdict depend on host state rather than a single reproducible run.
- Retain `e2e-elisp-integration` beside the combined check: it would run the
  live population twice and leave ADR-0035's placement ambiguous.
- Reuse a committed baseline, ratchet, or generated allowlist: these repeat the
  state and merge/re-anchoring problems ADR-0050 removed.
- Accept a block, a marker without a trimmed non-empty reason, a malformed
  marker, or a marker on a covered or non-executable line: each can silently
  turn an executable line into permanent unreviewed coverage debt.
- Exclude unsupported macro forms wholesale: it conceals production behavior;
  instrumentation or an individually reasoned exception keeps the scope
  reviewable.
- Exempt every zero-stop form by head alone: that would classify `defvar`,
  `defconst`, and `defcustom` with computed initializers as declarative even
  though their evaluation is production behavior, silently removing it from the
  denominator. The closed inert initializer grammar preserves an automatic
  exemption only where evaluation is not the behavior being measured.

## Consequences

- ADR-0031's interim coverage exemption ends for the defined production module
  population. Its pure ERT expectation becomes an enforced line-coverage
  obligation, while its separate-subproject boundary remains intact.
- ADR-0035 is amended at its gate-placement boundary: its live suite remains the
  integration proof and becomes a coverage input, but the combined producer
  replaces its former standalone e2e check and CI job.
- The repository gains diagnostic LCOV, text, and status artifacts at fixed
  paths for every controlled producer outcome, but no committed coverage
  baseline or generated approval file. A fresh checkout's verdict depends only
  on the source, pinned tooling, and the producer result; uncontrolled Nix or VM
  infrastructure failures remain ordinary failures.
- A new production macro requires an Edebug instrumentation decision at the
  point it is introduced. Executable and unclassified zero-stop forms remain
  visible work, not a blanket exclusion; inherently declarative forms receive
  the narrowly guarded automatic structural exemption.
- The authoritative consumer runs with `validate --no-e2e` and the CI static
  lane; full `validate` inherits its verdict without repeating the live suite.
