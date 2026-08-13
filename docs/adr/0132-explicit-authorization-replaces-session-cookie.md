# ADR-0132: Explicit Authorization replaces ambient session state

- Status: accepted
- Date: 2026-08-13
- Issue: [#936](https://github.com/jaunder-org/jaunder/issues/936)

## Context

Jaunder accepts an HTTP-only `session` cookie for browsers, Bearer credentials
for API callers, and HTTP Basic app passwords for AtomPub
([ADR-0007](0007-auth-mechanisms.md),
[ADR-0014](0014-atompub-authentication.md)). A request can carry both the
ambient browser cookie and an explicit `Authorization` header. Cookie-first
resolution silently selected the browser identity even when the caller
deliberately supplied a different machine credential. That is a confused-deputy
risk and makes the meaning of explicit credentials route-dependent.

Authentication is also used on optional-auth reads. Treating every extraction
failure as anonymous erases the distinction between no credential and a present
credential that failed, allowing malformed, unknown, or mismatched explicit
credentials to continue as an anonymous viewer.

## Decision

The presence of any `Authorization` header expresses explicit authentication
intent and makes that header authoritative:

- Supported Bearer and Basic values are parsed and authenticated before any
  session cookie is considered.
- A malformed Bearer/Basic value, unsupported scheme, invalid token, missing
  session, or Basic username mismatch rejects the request. None falls back to a
  valid cookie.
- Without an `Authorization` header, the existing session-cookie path is
  unchanged, including its missing/invalid behavior.

After successful Bearer or Basic authentication, if the request also carried a
`session=` cookie, the response retires that cookie with `Max-Age=0` using the
deployment's `Secure` setting. This happens only after token authentication and
the Basic username check succeed, including when cookie and header contain the
same token. One request-scoped marker and outer router middleware apply the rule
to Leptos server functions and raw Axum/AtomPub routes. The middleware appends
the expiry `Set-Cookie`; it never replaces a handler's existing cookie headers.

Optional-auth reads remain anonymous only when credentials are absent or when a
cookie-only credential is missing, invalid, or no longer exists. Every failure
attributable to a present Bearer or Basic credential propagates as an
authentication rejection. Successful explicit credentials resolve the
corresponding authenticated viewer and participate in the same cookie retirement
behavior.

## Consequences

- Explicit caller intent has one meaning across all routes: Bearer or Basic
  identity replaces ambient browser identity.
- A bad explicit header cannot borrow a valid browser session or silently obtain
  anonymous behavior.
- Successful machine authentication logs the browser out by expiring its cookie;
  the server-side session represented by that cookie is not revoked.
- Existing response cookies are preserved because retirement uses header append
  semantics. A handler may therefore emit its own expiry alongside the shared
  middleware expiry.
- Clients that accidentally send stale or malformed `Authorization` headers must
  remove or correct them; a valid cookie no longer masks the error.
