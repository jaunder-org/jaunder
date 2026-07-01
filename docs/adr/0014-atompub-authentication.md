# ADR-0014: AtomPub Authentication via App-Specific Passwords

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-05-29

## Context and Problem Statement

Jaunder is adding an AtomPub (RFC 5023) publishing interface whose primary
target client is MarsEdit. For a generic AtomPub account, MarsEdit authenticates
with **HTTP Basic** auth — it prompts for a username and password and sends them
on every request. It does not support bearer tokens, OAuth, or any app-specific
token-exchange flow for generic AtomPub. Jaunder's existing auth (ADR-0007) is
session cookies plus bearer tokens, with no HTTP Basic path. We need an
authentication model for the AtomPub routes that works with MarsEdit's
Basic-auth-only behaviour without exposing the user's primary credential.

## Decision Drivers

- Compatibility: must work with MarsEdit's HTTP Basic auth, unchanged.
- Security: avoid storing the user's login password in a long-lived third-party
  desktop app.
- Revocability: a per-client credential should be individually revocable.
- Reuse: avoid building a parallel credential system if the existing one fits.

## Decision Outcome

Chosen option: **App-specific passwords carried over HTTP Basic and validated
through the existing session-token path.**

A user mints a named **App Password** in the web UI; the raw token is shown once
and entered into MarsEdit as the Basic-auth password. AtomPub requests carry it
in the `Authorization: Basic` header; the credential is resolved through the
same `authenticate(raw_token)` path used for session/bearer tokens.

### Implementation Details

1. An App Password is a **labelled session** — minting calls
   `create_session(user_id, label)`; no new table. Sessions never expire, so the
   credential is long-lived as MarsEdit requires.
2. The `AuthUser` extractor is extended to read credentials from
   `Authorization: Basic`, feeding the password component into `authenticate()`.
3. The Basic **username** must equal the resolved token's user, or the request
   is rejected `401`; combined with the per-user collection URIs, a token for
   user X may only operate on `/atompub/X/*`.
4. Tokens are **interchangeable across transports** — no `kind` marker
   distinguishes app passwords from browser sessions. In a self-hosted,
   single-user trust model both are bearer-equivalent secrets, so
   transport-scoping was judged unnecessary complexity.
5. `sessions.label` is made **non-optional** (with a backfill migration);
   browser logins auto-generate a User-Agent/host label while app passwords
   carry a user-supplied name, distinguishing them in the Sessions UI without a
   dedicated type field.

## Consequences

- Good: Works with MarsEdit (and the planned Emacs client) using standard HTTP
  Basic.
- Good: The login password is never stored in a third-party app; app passwords
  are individually revocable in the existing Sessions UI.
- Good: Reuses the existing session-token storage and `authenticate()` path — no
  parallel credential system.
- Bad: Tokens are interchangeable across transports; a leaked app password is
  usable as a web session and vice versa. Accepted for the self-hosted
  single-user trust model; revisit if multi-tenant or stricter isolation is
  needed.
- Bad: Basic auth transmits the token on every request, so the deployment's
  TLS-terminating reverse proxy is load-bearing for this interface.
