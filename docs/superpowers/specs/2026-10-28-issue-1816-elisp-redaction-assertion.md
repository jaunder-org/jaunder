# #1816: Keep the redaction assertion independent of private temp paths

## Problem

`jaunder-keyword-repair-apply-terminal-http-and-response-loss` in
`elisp/test/jaunder-keyword-repair-test.el` formats the complete result plist
and rejects any occurrence of `private`. The case deliberately signals
`(file-error '("private response loss"))`; production error-source handling
retains that original source text for diagnostics while the printed result uses
`:source redacted-source`. The test fixture's path is rooted under `TMPDIR`,
which may itself contain `private` (for example `.private-tmp-1811`). The broad
search then fails for a benign path and cannot distinguish that from leakage of
the original error/source.

## Proposal

Make the negative privacy assertion inspect the relevant error/source
representation rather than treating every occurrence of the word `private`
anywhere in the formatted result as leakage. Keep an explicit positive assertion
that `jaunder-keyword-repair-error-source` on the result still equals
`(file-error "private response loss")`, and a negative assertion that the
result's diagnostic/source rendering does not disclose that original private
error text. Keep the path field in the result and allow arbitrary configured
temp-root names; do not normalize, hide, or remove paths as a workaround.

This is a test-only reliability change. It must continue to fail if the original
private response-loss text becomes visible in the redacted result, including
when the test runs under a TMPDIR containing `private`.

## Evidence and reproduction

Issue #1816 is open, unassigned, and was Todo in Jaunder Backlog before this
authorized start; it has no linked PR on the preserved branch
`herdsman/work-55a61149`. The test at
`elisp/test/jaunder-keyword-repair-test.el:403-434` deliberately signals
`(file-error '("private response loss"))`, verifies
`jaunder-keyword-repair-error-source` retains
`(file-error "private response loss")`, then rejects any `private` in
`(format "%S" result)`. Production formats the result with `:path` set to the
fixture path and `:source redacted-source`; it is the path substring, not leaked
source text, that trips the assertion.

Focused reproduction, using the existing test unmodified:

- `devtool run -- env TMPDIR=/tmp/private-1816-repro emacs --batch -Q -L elisp -L elisp/test -l elisp/test/jaunder-keyword-repair-test.el --eval '(ert-run-tests-batch-and-exit "^jaunder-keyword-repair-apply-terminal-http-and-response-loss$")'`
  — exit 1. ERT's exact failure was
  `(string-match-p "private" "(:id \\"42\\" :path \\"/tmp/private-1816-repro/jaunder-repair-testsoJtSC/post.org\\" :source redacted-source :stage put :status unknown-remote-outcome)")`;
  the match is at index 22 in the legitimate `:path`. The preceding
  source-retention assertion passed.
- Same focused command with `TMPDIR=/tmp/1816-repro` — exit 0; ERT reports 1
  test, 1 passed.

Both runs use the standard Emacs test location and supported `TMPDIR` override;
no test or production source was modified. The first attempt via
`elisp/scripts/run-tests.el` invoked all 624 tests rather than filtering; its
exit 1 was not used as the focused proof. The exact focused invocation above
establishes the symptom and the control.

## Non-goals

- No change to production redaction, error-source retention, or path reporting.
- No weakening/removal of the assertion that protects the original private
  error/source text.
- No general ERT/temp-directory redesign or gate changes.
- No work on #1777, #1773, #1811, or #1800.

## Acceptance tests

1. Under both an ordinary `TMPDIR` and one containing `private`, the focused ERT
   test passes and continues checking the existing path field without treating
   its arbitrary contents as a disclosure.
2. `jaunder-keyword-repair-error-source` on the result remains exactly
   `(file-error "private response loss")`.
3. A negative privacy assertion examines the redacted result/diagnostic
   representation (not the whole plist containing `:path`) and fails if the
   original sensitive source value `private response loss` is disclosed there.
   Preserve the positive original-source assertion and production source/privacy
   behavior.
4. No production source changes. Demonstrate the negative assertion is
   leak-sensitive by asserting against the complete sensitive source value;
   inspect that a mutation/leak of that value would violate it.

## Approval requested

Approve this test-only assertion redesign for #1816 before implementation. No
implementation, commit, push, PR, or merge is authorized by this start/spec
task.
