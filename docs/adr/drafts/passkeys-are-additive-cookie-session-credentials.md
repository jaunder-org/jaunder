# ADR-DRAFT: Passkeys are additive cookie-Session credentials

- Status: proposed
- Date: 2026-09-12
- Issue: [#1455](https://github.com/jaunder-org/jaunder/issues/1455)

## Context

Jaunder authenticates browser users with passwords and establishes an HttpOnly
cookie Session. Bearer tokens and App Passwords serve machine clients through
the same Session storage, but browser endpoints never return token material.
Users want WebAuthn passkeys without weakening that boundary or making
self-hosted relying-party identity depend on attacker-controlled request
headers.

WebAuthn separates a long-lived public-key credential from two transient
registration and authentication ceremonies. Verification depends on a stable RP
ID, an exact expected origin, a fresh challenge, server-held ceremony state, and
a credential lookup. Discoverable credentials can identify the account without a
Username, but therefore require an opaque account handle and global credential
identity. Synced passkeys also make signature counters a risk signal rather than
reliable clone proof.

Passwords already provide registration and email-assisted recovery. Retaining
that independent path limits the first passkey increment and prevents a lost
passkey from becoming a new account-recovery protocol.

## Decision

Passkeys are additive browser credentials. Every account keeps its password and
may own multiple independently labelled and revocable passkeys. Enrollment and
deletion occur only in a Passkeys settings surface authenticated by the ambient
cookie Session; explicit Authorization is rejected without cookie fallback, and
successful current-password verification is required. Password reset leaves
passkeys intact.

Passkeys are discoverable and require user verification. The login page starts
an explicit account-discovering WebAuthn assertion without asking for Username.
Registration requests no attestation and accepts both synced and device-bound
credentials. Conditional UI, username-first WebAuthn, and passkey enrollment at
signup are not part of this decision.

Each account receives a stable random WebAuthn user handle unrelated to its
Username or database UserId. Credential IDs are globally unique. Durable storage
retains the library credential representation and the creation, last-use, label,
counter, and backup properties required to verify and manage multiple
credentials on both SQLite and PostgreSQL.

The WebAuthn origin is exactly the configured `site.base_url` origin and the RP
ID is its hostname. Request Host and forwarding headers never select trust.
Passkeys fail closed and their UI is clearly unavailable when the setting is
absent or is not HTTPS, except for the browser's localhost development case.
While any Passkey exists, every configuration mutation must preserve the
canonical RP hostname: unsetting or changing it is rejected, including through
aggregate and CLI writes. Jaunder supports neither wildcard/multiple origins nor
credential migration between RP hostnames.

Ceremony state is database-backed, purpose-bound, expires after five minutes,
and is atomically claimable once. It binds the exact origin and RP ID used at
start, which must still equal current configuration at finish. The client holds
only a random opaque lookup handle whose stored representation is hashed.
Ceremony rows are excluded from backup and restore; expired and consumed rows
otherwise obey bounded transient retention. Verification covers ceremony type,
challenge, exact origin, RP ID hash, credential/account binding, signature, user
presence, and required user verification.

Jaunder pins `jaunder-org/webauthn-rs` branch `feature/passkey-policy-apis` at
`6d0acc73fbf4436b1ed853fa5c8a219dac304a8e`, an exact revision of the 0.5.5 fork.
The safe `Webauthn` wrapper remains the cryptographic boundary; application code
never calls its unstable expert core. The fork adds only opt-in policy doors for
resident-required Passkey registration without attestation, explicit
discoverable authentication without conditional mediation, and verified
counter-anomaly results. Upstream defaults remain unchanged. Remove the Cargo
patches and paired `deny.toml` rationale when an audited upstream release
exposes equivalent supported policies.

Successful passkey authentication updates the durable credential and creates an
ordinary Session atomically. The browser receives the existing credential-free
`SessionUser` and HttpOnly SameSite=Lax cookie, never a bearer token. Passkeys
do not authenticate Bearer, Basic, or AtomPub transports. Deleting a passkey
atomically revokes every other Session for its User while preserving the current
password-confirmed browser Session.

A cryptographically valid assertion with a zero or non-monotonic signature
counter is accepted because synced authenticators can produce that state. It
updates mutable properties but never decreases the stored nonzero counter
high-water mark, and emits bounded PII-free security telemetry. Credential
material, WebAuthn payloads, account handles, and ceremony secrets never enter
telemetry or public errors.

## Consequences

Users gain passwordless browser sign-in while retaining the existing password
and email recovery model. The login surface has two independent choices, but
both establish exactly the same application Session and advisory client marker.

Passkey credentials and user handles become durable backup data. Ceremony rows
are excluded from backup and restore and form a transient credential domain with
expiry, atomic claim, cleanup, and dual-backend parity requirements. Exact
dependency injection adds passkey storage only to the authentication,
management, configuration-mutation, backup, and cleanup roots that need it.

The fork is maintenance debt, but it is narrower than reimplementing WebAuthn or
depending on an immature verifier. Exact revision pinning, characterization
tests, and an explicit removal condition make that debt visible; accepting a
counter error as undocumented proof of otherwise-successful verification is
forbidden.

`site.base_url` becomes a credential-binding input as soon as the first passkey
is enrolled. Blocking hostname changes prevents a routine configuration edit
from silently orphaning every credential; moving the instance requires users or
an operator to remove passkeys first while password auth remains available.

Current-password checks prevent a stolen ambient Session alone from becoming
persistent account access. Revoking other Sessions on deletion addresses an
already-established Session from a lost credential without logging out the
password-confirmed browser performing remediation.

Rejecting attestation, conditional UI, passkey-only accounts, multi-origin RP
configuration, and hard counter rejection keeps compatibility broad and avoids
new device-trust and recovery systems. Those are separate future decisions, not
implicit extension points in this implementation.
