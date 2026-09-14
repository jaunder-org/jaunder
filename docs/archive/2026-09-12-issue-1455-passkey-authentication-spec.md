# Passkey authentication

Issue: #1455

## Outcome

Jaunder users can independently enroll, revoke, and sign in with multiple
Passkeys without a Username. Password auth and recovery remain independent;
ceremonies use configured identity, server state, UV, and cookie-only Sessions.

## Load-bearing decisions

- A **Passkey** is an additive browser credential. Every account retains its
  password and may enroll multiple independently identifiable and revocable
  Passkeys; registration, invitation, and recovery policy do not change.
- Passkeys are discoverable. Login keeps the password form and adds an explicit
  `Sign in with a passkey` account picker that solicits no Username. Conditional
  UI, username-first Passkeys, and page-load prompting are absent.
- Registration and authentication require user verification, not presence alone.
  Registration requests no attestation and accepts synced and device-bound
  Passkeys without a vendor or authenticator-class trust claim.
- Each User receives one random, stable, opaque WebAuthn user handle. It is not
  derived from or interchangeable with Username, UserId, Email, or Display Name.
  Discoverable authentication resolves the account through this handle and the
  globally unique credential ID.
- A dedicated authenticated **Passkeys** page lists each credential's required
  user-supplied label, creation time, and last-used time. Labels are non-blank
  presentation values; duplicates are allowed and never establish identity.
- Enrollment and deletion exist only on the Passkeys page and authenticate
  through the ambient cookie Session; an `Authorization` header is rejected
  without cookie fallback. Each mutation requires current-password verification.
  Enrollment finish stays bound to the same authenticated User and cookie
  Session. Deletion verifies credential ownership, removes it, and atomically
  revokes every other Session while preserving that password-confirmed browser
  Session.
- Password reset keeps all enrolled passkeys. It continues to replace the
  password and revoke Sessions under the existing reset contract.
- A successful passkey assertion creates an ordinary Session in the same
  transaction that applies the credential's authentication update. It returns
  the same credential-free `SessionUser` representation and establishes the
  browser Session only through the existing HttpOnly, SameSite=Lax cookie.
- Passkeys never authenticate Bearer, HTTP Basic, or AtomPub requests and never
  become App Passwords. Existing explicit-Authorization precedence is unchanged.
- The expected WebAuthn origin is exactly the origin of `site.base_url`; the RP
  ID is that URL's hostname. Neither value is derived from request Host or
  forwarding headers. Related-origin requests, parent-domain RP IDs, wildcard
  subdomains, and multiple accepted origins are excluded.
- Passkeys are available only when `site.base_url` is configured with HTTPS, or
  with the localhost development exception accepted by browsers. Password auth
  remains available otherwise; login and Passkeys surfaces explain why the
  passkey action is disabled, and every ceremony endpoint independently fails
  closed.
- While any Passkey exists, every `site.base_url` mutation door must preserve
  the current canonical RP hostname. Unsetting or changing it is rejected;
  scheme and port changes remain configurable, while the independent secure-
  context rule controls whether Passkeys are then available. This applies to
  individual and aggregate, web and CLI writes.
- Ceremony challenge and library state live only in server storage. The browser
  receives an opaque high-entropy state handle whose persisted form is hashed.
- Registration and authentication state is purpose-bound, bound to the exact
  canonical origin and RP ID current at start, expires after five minutes, and
  can be claimed exactly once under concurrency. Finish also requires that
  `site.base_url` still resolves to that same origin and RP ID. Expired state is
  unusable; consumed state is never reusable; cleanup follows bounded retention.
- Finish operations verify the ceremony type, challenge, exact origin, RP ID
  hash, credential/account binding, signature, user presence, and required user
  verification before changing durable credential or Session state.
- Successful authentication persists mutable backup properties and last-used
  time. A valid zero or non-monotonic counter is accepted and audited, but never
  decreases a nonzero stored counter: the stored value remains its high-water
  mark.
- Credential IDs, public keys, user handles, ceremony handles, challenges, and
  WebAuthn payloads never appear in logs, metrics labels, user-visible errors,
  Session bodies, or the advisory local-storage authentication marker.
- Invalid, expired, replayed, unknown, or mismatched authentication attempts
  return one neutral public failure. Internal errors retain typed sources and
  structured decision-path telemetry without exposing account existence.
- SQLite and PostgreSQL share one passkey storage contract and remain schema and
  behavioral peers. Passkey credentials and user handles are durable backup
  data; ceremony rows are transient and are excluded from backup and restore.
- The browser integration belongs in the existing wasm client boundary; web
  server functions remain typed orchestration over exact storage dependencies
  and the existing Session establishment path.

## Acceptance

- On a secure configured instance, an authenticated User can label and enroll
  two discoverable Passkeys after entering the current password; both appear
  independently with creation metadata.
- Wrong password, missing/revoked or explicit-Authorization Session, failed user
  verification, origin/RP/configuration mismatch, expiry, and replay all fail
  without storing a credential.
- From signed out, either Passkey identifies its account through the picker and
  creates a cookie-only Session without Username or password. Unknown handles,
  unknown credential IDs, and a known handle paired with another User's known
  credential all return the same neutral failure without updating a credential
  or creating a Session; both identifiers are unique on both backends.
- Password login, registration, invitations, forgot-password, password reset,
  Bearer authentication, Basic App Password authentication, and explicit-
  credential cookie retirement preserve their existing observable behavior.
- A successful email password reset leaves passkeys listed and usable while
  revoking the User's Sessions as before.
- Deleting one owned Passkey with the current password preserves the current
  cookie Session, revokes every other Session, leaves siblings usable, and
  prevents the deleted Passkey from creating a Session. Wrong password,
  missing/revoked or explicit-Authorization Session, and cross-User credential
  IDs leave every Passkey and Session unchanged under the neutral error
  contract.
- A valid counter anomaly signs in and emits bounded telemetry while preserving
  the stored counter high-water mark; a later regression remains detectable.
  Ordinary assertions persist changed credential properties and last-used time.
- Missing, insecure, or non-localhost HTTP `site.base_url` leaves password auth
  usable, renders passkeys clearly unavailable, and makes ceremony calls fail
  closed. With any Passkey present, web and CLI set, aggregate-update, and unset
  doors reject a resulting absent or different RP hostname. Scheme/port-only
  changes, including HTTPS to HTTP, succeed; insecure results disable Passkeys,
  and every start-configure-finish sequence with a changed origin or RP ID
  fails.
- Concurrent finishes admit at most one state claim and one mutation. On both
  backends, fault injection at credential update, Session creation/commit,
  deletion, and other-Session revocation proves rollback leaves no partial
  credential change, Session row, deletion, cookie, or revocation.
- Backup/restore preserves every durable Passkey field and user handle while
  excluding live, consumed, and expired ceremony rows; the derived backup-set
  guardrail enforces that split. Cleanup reclaims consumed and sufficiently
  expired ceremonies under the existing retention contract.
- Browser verification exercises real registration, authentication,
  cancellation, unsupported-browser messaging, and deletion. Storage and server
  verification cover both backends and the replay, ownership, configuration,
  rollback, and Session invariants.

## Boundaries

- No signup enrollment, passkey-only state, password removal, recovery codes,
  email-less recovery, operator reset, or password-reset policy change.
- No conditional/username-first flow, native API, extensions, attestation
  policy, device inventory, credential rename/caps, or unique/inferred labels.
- No multi/related origin, parent/wildcard RP ID, hostname migration, bearer
  response, alternate Session/transport, generic credential framework, or
  weakened password/capability timing discipline.
