# Username-or-Email Password Reset Requests

Issue: #611

## Outcome

The forgot-password form accepts either a Username or an Email and gives every
structurally valid request the same immediate, neutral success response.
Eligible Users receive reset mail without exposing account existence through
response content or awaited SMTP latency.

## Load-bearing decisions

- The request boundary uses one password-reset-scoped identifier enum with
  `Username` and `Email` variants, not an ambiguous string or two optional
  fields. This is local request vocabulary, not a new domain aggregate or
  glossary term.
- An input containing `@` is parsed as an Email; every other input is parsed as
  a Username. Once classified, invalid input fails typed boundary validation and
  does not fall back to the other variant.
- Username parsing retains the canonical lowercase Username boundary. Email
  parsing retains the existing Email boundary: its DNS domain is canonicalized
  to lowercase while its local part is preserved.
- The form is one control labelled `Username or email`. It does not lowercase
  raw input before typed parsing, so it cannot corrupt a case-sensitive Email
  local part.
- A Username request selects its one matching User. An Email request selects
  every User whose stored Email matches the canonical Email exactly and is
  verified. Email is not made unique, and no matching User is selected
  arbitrarily.
- Every structurally valid request returns the same success-shaped response,
  including unknown identifiers, Users without a verified Email, unavailable
  mail delivery, and internal lookup, token, or mail failures. The response does
  not say whether work was scheduled or completed.
- Account lookup, reset-token issuance, and SMTP delivery run outside the
  request response path. The request captures owned dependencies, starts
  detached best-effort work, and returns without awaiting account-dependent or
  SMTP work. Request-scoped dependency lookup and borrowed transactions do not
  escape into that work.
- Detached delivery is intentionally best effort: process termination may lose
  unfinished work. This issue does not introduce a durable outbox or delivery
  status. A future durable queue may replace this policy without changing the
  public request contract.
- Each selected User is processed independently. Failure to create or send one
  User's reset mail does not prevent attempts for other Users sharing the Email.
- Each successful delivery receives a reset token for that User and the existing
  one-hour expiry and absolute reset-confirmation link. No token is created for
  an ineligible User.
- Detached-work failures are observable only through generic internal telemetry.
  Telemetry never records the submitted Email, reset token, password, or other
  prohibited secrets or PII; no internal error crosses the public
  server-function boundary.
- The server-function endpoint remains the existing password-reset request
  endpoint. The changed typed request field is a clean wire cutover: all callers
  use the identifier contract, with no legacy username alias.
- Storage behavior and coverage remain identical for SQLite and PostgreSQL.
- These rules preserve the enumerable-identifier timing boundary of ADR-0018,
  the public error boundary of ADR-0017, the telemetry restrictions of ADR-0011,
  the typed-value convention of ADR-0063, canonical Username ownership from
  ADR-0134, exact dependency injection from ADR-0016, and backend parity from
  ADR-0019 and ADR-0053.

## Acceptance

- In the browser, the forgot-password control is labelled `Username or email`,
  accepts a mixed-case Email without lowercasing its local part, and accepts a
  Username with existing canonicalization.
- A valid Username for a User with a verified Email causes one reset message for
  that User; the existing full reset flow still succeeds.
- A valid Email for one User with that verified Email causes one reset message
  whose absolute link completes the existing reset-confirmation flow.
- A valid Email shared by multiple Users causes an independently usable reset
  message for every matching User with that Email verified. Unverified matches
  receive no message and do not suppress verified matches.
- Unknown Usernames, unknown Emails, Users without a verified Email, and
  eligible Users all receive the same public success-shaped result. Integration
  coverage gates account lookup and token issuance for eligible and unknown
  identifiers and proves the public response completes before either gate is
  released; no account-dependent work is awaited by the response path.
- Malformed Email-looking and malformed Username-looking inputs are rejected by
  the appropriate typed boundary before detached work starts.
- Detached lookup, per-User token, and mail failures do not alter the public
  response; a per-User failure does not prevent another matching User's delivery
  attempt. Integration coverage proves that each failure reports once through
  the existing bounded `swallowed/server` host-error path without recording an
  Email, token, password, or other prohibited PII or secret.
- Server integration coverage proves lookup, verification filtering,
  duplicate-Email fan-out, neutral public results, failure isolation, stable
  endpoint routing, and SQLite/PostgreSQL parity.
- End-to-end coverage exercises the Email-based request path through receipt and
  use of the emitted absolute reset link.
- The password-reset flow documentation and architecture view describe the
  username-or-email request, neutral response, and detached best-effort delivery
  boundary.

## Boundaries

- Reset confirmation remains token plus new password; its validation, storage
  composition, session revocation, and UI do not change except where the
  existing end-to-end flow consumes a token issued from an Email request.
- This issue adds no Email uniqueness constraint, schema migration, durable mail
  outbox, delivery-status UI, retry policy, rate limiter, or public account
  discovery surface.
- Registration, login, Email verification, password policy, SMTP configuration,
  and non-password-reset mail remain unchanged.
- No new ADR or `CONTEXT.md` term is required: accepted decisions already govern
  the security, dependency, typed-boundary, and storage-parity contracts.
