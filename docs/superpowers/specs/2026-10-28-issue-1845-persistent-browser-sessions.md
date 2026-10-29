# Persistent browser Sessions with rolling inactivity expiry

## Outcome

A User who authenticates in a browser receives a newly created typed browser
Session that remains signed in across browser restarts and can continue an
actively used browser Session. A newly created typed browser Session becomes
invalid after 30 days without authenticated browser use; expired rows are later
removed through bounded cleanup. Legacy/unknown Sessions remain non-expiring.
Machine credentials, including App Passwords, do not expire under this policy.

## Approach and tradeoffs

**Proposal pending human approval:** make the existing `session` cookie
persistent for 30 days and renew its browser lifetime on successful
cookie-authenticated use. Enforce the same 30-day inactivity window server-side
from Session activity; refreshing a cookie must not revive an already-expired
Session. There is no separate absolute maximum lifetime in this proposal: active
Sessions can continue indefinitely, which favors continuity over periodic
reauthentication. The current browser-session cookie is the simplest
alternative, but does not address the reported restart symptom; a fixed 30-day
cookie without renewal would sign out active Users after 30 days. A finite
absolute cap is another policy option, intentionally not combined with idle
expiry here.

This addresses the reported browser-restart failure mode without claiming that
it explains every reauthentication. The issue author clarified via Manager relay
`d7c495bd-52cf-4afd-a24f-89d0640238dc` that the symptom occurs after
closing/restarting the browser and that the desired policy is a 30-day rolling
idle limit: activity renews login, with no separate absolute lifetime cap. This
product direction authorizes drafting only, not this unseen spec, schema,
implementation, or merge. The current cookie has `HttpOnly`, `SameSite=Lax`,
`Path=/`, and the configured `Secure` behavior, but no positive `Max-Age` or
`Expires`; browsers therefore treat it as a session cookie and may discard it
when their session ends. The server stores `created_at` and `last_used_at`,
touches activity at most once per 60 seconds, and has no fixed session-expiry
check in the inspected authentication path. Thus the present server Session
appears valid until revoked even if its browser cookie disappears. No automatic
cookie renewal currently occurs on authenticated use.

Keep the cookie-only browser credential model and existing
explicit-Authorization precedence. Durable credential kind must distinguish
browser Sessions from machine credentials for every new-row creation path
(including login/passkey and App Password creation); neither caller-supplied
labels nor the current request's authentication transport is a discriminator.
Existing rows have no reliable kind discriminator and are explicitly
legacy/unknown: grandfather them unchanged, non-expiring and revocable, and
exclude them from idle cleanup. Existing legacy browser cookies therefore keep
their current session-cookie behavior until a fresh sign-in creates a typed
browser Session. This compatibility carveout is a proposed human decision, not
an established product direction: legacy browser rows may remain until explicit
revocation, so a User may need one fresh login and old rows will not be
automatically reclaimed. Do not infer kind from labels or request transport.

New browser Sessions created at sign-in (including passkey establishment)
receive the proposed policy. On successful cookie authentication, persist
activity only when the existing request-driven at-most-once-per-60-second touch
threshold permits it; do not write on every request. Renew the same cookie
without token rotation only when that successful request actually persists
activity. Set its full 30-day lifetime from the authoritative committed
`last_used_at`, represented as integer Unix seconds rounded down; its expiry
must not outlive the server cutoff. Do not renew a cookie inside the write
throttle. Use one explicit UTC `now` for expiry decisions: a new browser Session
is expired when `last_used_at <= now - 2,592,000 seconds`. A stale cookie or
concurrent activity must never revive an expired or revoked row; if the chosen
storage operation cannot guarantee this atomically, provide an implementation
outline covering the concurrency rule after spec approval and before that work.
Bearer/Basic requests do not renew ambient browser state. App Passwords and
other machine credentials do not expire or renew under this policy.

Semantic expiry is authoritative; physical deletion is separate and repeatable
bounded cleanup in the existing daily transient-data pass, using fixed-size
batches and deleting only expired typed browser Sessions. Cleanup failure is
reported and retried by a later pass, consistent with ADR-0167. Update
ADR-0167's present exclusion of non-expiring Sessions if this proposal is
approved; do not create a generic retention framework.

## Acceptance

- New browser Session creation sets a persistent cookie with a 30-day lifetime
  while preserving `HttpOnly`, `SameSite=Lax`, `Path=/`, and configured `Secure`
  behavior; logout/retirement still clears it. New machine Sessions remain
  non-expiring.
- A successful cookie-authenticated request renews the same cookie without
  rotating its token only when it persists activity at the request-driven
  60-second threshold; the cookie's integer-second expiry is committed
  `last_used_at + 2,592,000`, rounded down, and cannot outlive server validity.
  Controlled-clock tests cover a request inside the throttle followed by
  inactivity. Bearer/Basic authentication does not renew ambient browser state.
  Password and Passkey Sessions follow the browser policy.
- With a controlled UTC `now`, a newly created typed browser Session is valid
  just before `last_used_at + 30 days` and expired at or after it; expired
  Sessions cannot be revived by a stale cookie, and explicit revocation remains
  effective. Tests exercise the 60-second touch threshold and 30-day cutoff
  using the same clock seam.
- Both storage backends test classification of newly created browser and machine
  rows and legacy/unknown rows. Legacy rows remain unchanged, non-expiring and
  revocable, excluded from cleanup; test this alongside new-row expiry/cleanup,
  AppPassword, Bearer and Basic invariants on SQLite and PostgreSQL.
- SQLite and PostgreSQL tests cover renewal, expiry, revocation and cleanup. The
  existing daily cleanup pass removes only expired typed browser Sessions in
  fixed-size repeatable batches, reports domain failure, and retries on a later
  pass without expiring/deleting machine credentials.
- Cookie/auth tests verify 30-day persistence, renewal, expiry boundary, and
  preserved attributes. Browser proof verifies the browser receives and uses a
  persistent cookie; it does not claim every browser retains cookies across
  restarts regardless of session-restore/privacy policy. No production session
  data or credentials are used.

## Boundaries

No absolute Session maximum, configurable retention setting, App Password
expiry, auth-method changes, bearer-token introduction, cookie security
weakening, unrelated credential reaping, or generic retention infrastructure.
Refresh-on-use and inactive-Session cleanup are included only as the proposed
30-day inactivity policy above. Do not implement before explicit human spec
approval. After approval, assess auth, storage, schema, and concurrency risks
against the full lifecycle; obtain approval of an implementation outline if that
assessment triggers one before proceeding. Preserve the atomic no-revival
requirement regardless of whether an outline is triggered. If no outline is
triggered, approval permits normal development within this scope.

## Decision requested

Approve this exact spec, including the legacy/unknown-row compatibility carveout
and migration tradeoff, or return a scope/design correction. The Manager-relayed
human product direction selects 30-day rolling idle expiry and no absolute cap
for proposal drafting only; it does not approve this spec or this carveout.
After approval, assess auth, storage, schema, and concurrency risks across the
full lifecycle and obtain outline approval if that assessment triggers one;
otherwise normal in-scope development may proceed. The atomic no-revival
requirement remains in force regardless. Spec approval is not merge
authorization.
