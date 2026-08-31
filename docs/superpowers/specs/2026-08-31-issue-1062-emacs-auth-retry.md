# #1062 — Reconcile the Emacs publish-retry architecture

Issue: [#1062](https://github.com/jaunder-org/jaunder/issues/1062). Milestone:
Developer tooling & DX.

## Outcome

The authoritative architecture projection accurately states that missing
`auth-source` credentials fail immediately while create publishing retries
transport failures and server 5xx responses.

## Load-bearing decisions

- Commit `0d8837d66` already delivered #1062's requested behavior under #945
  before #1062 was opened; this cycle does not duplicate that implementation.
- The retry boundary remains transport-positive: a signalled `plz-error` without
  an HTTP response is retryable, as is a returned 5xx response.
- Missing credentials and every other non-transport error propagate immediately.
- Existing create retry policy remains unchanged: at most three attempts,
  `sleep-for` delays of one then two seconds, and one Idempotency-Key shared
  across every attempt in the invocation.
- Existing focused tests are the regression contract: non-transport errors make
  one attempt with no sleep, transport errors exhaust three attempts, and a 5xx
  can retry successfully with the same key.
- No new architectural decision is introduced; the projection is reconciled with
  ADR-0143 and behavior already on `main`.

## Acceptance

- `docs/ARCHITECTURE.md` no longer describes missing credentials as a current
  retry defect or says the retry handler catches every signalled error.
- The projection identifies the actual retryable signal and retains the
  three-attempt, one-/two-second backoff, and single-Idempotency-Key contract.
- The existing focused Emacs tests covering non-transport, transport, and 5xx
  paths pass unchanged.
- Repository documentation and static checks applicable to the changed files
  pass.

## Boundaries

- No production or test behavior change.
- No credential prompting, minting, writing, rotation, or persistence.
- No change to credential identity, server App Password semantics, HTTP status
  handling, retry count, backoff, idempotency, or other operations.
