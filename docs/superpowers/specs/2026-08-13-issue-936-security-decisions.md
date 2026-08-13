# Issue #936: Record and enforce four security decisions

- Issue: [#936](https://github.com/jaunder-org/jaunder/issues/936)
- Date: 2026-08-13
- Scope: Four decision records, their architecture/domain projections, and the
  credential-transport behavior required by the precedence decision

## Problem

Four security-relevant invariants are built and documented as current behavior
but have no decision record: cheap test-only password hashing, hashed-at-rest
bearer-equivalent tokens, credential transport precedence, and
lowercase-canonical usernames. The missing rationale makes each invariant
vulnerable to accidental reversal.

Credential precedence additionally needs correction. Today a valid `session=`
cookie wins over an explicit `Authorization` header. A request carrying Alice's
ambient session cookie and Bob's explicit Bearer token therefore authenticates
as Alice. Explicit authentication must instead express the request's identity,
and successfully switching to it must retire the ambient browser session.

## Decisions

### 1. Test-only cheap KDF fails closed in three layers

The `cheap-kdf` feature may reduce Argon2 hashing cost only for test and
coverage builds. Three complementary safeguards remain architectural invariants:

1. Production dependency edges do not enable the feature; test-only dependency
   edges may.
2. A cheap-KDF build with `debug_assertions` disabled fails at compile time.
3. Any `jaunder` server binary linked with cheap KDF exits before CLI parsing or
   application startup.

The compile-time and startup guards are not redundant. The former blocks
ordinary optimized artifacts; the latter catches a debug-assertions-enabled
artifact deployed as production. Password verification remains
feature-independent because the stored PHC string carries its parameters.

### 2. Bearer-equivalent tokens are hashed before persistence

Session tokens and app passwords are bearer-equivalent secrets. Only their
SHA-256 `TokenHash` may enter persistent storage or storage lookup keys.
`RawToken` remains a distinct type with no sqlx encode/type bridge, making
direct SQL binding fail to compile. Raw-token debug output remains redacted.

Hash-before-store remains an explicit conversion rather than a
generated/raw/hashed type-state API. This preserves a visible, small boundary
and records the previously declined stronger design from #554. The ADR must
state the honest limit: the type prevents direct sqlx binding, while correct
conversion before storage is still expressed at call sites.

### 3. Explicit Authorization is authoritative

Any request containing an `Authorization` header expresses explicit
authentication intent:

- The header is evaluated before any session cookie.
- Supported Bearer and Basic credentials authenticate through the existing
  session-token path.
- A malformed Bearer/Basic value, an unsupported authorization scheme, a failed
  token lookup, or a Basic username mismatch rejects the request. None may fall
  back to a valid session cookie.
- With no `Authorization` header, the existing session-cookie path remains
  unchanged.

After Bearer or Basic authentication succeeds, if the request also carried a
`session=` cookie, the response appends an expiry `Set-Cookie` for that session.
This applies even when the cookie and header contain the same token. Cookie
retirement occurs only after the explicit credential and any Basic username
check succeed; invalid or attacker-supplied headers must not log out a valid
browser session.

The behavior applies to every route using `AuthUser`, including raw Axum
API/AtomPub handlers, Leptos server functions, and optional-auth extraction. It
must be implemented once at the shared authentication/response seam rather than
separately in handlers. Existing response cookies must be preserved; the expiry
header is appended, not substituted.

Optional-auth paths preserve current cookie-only behavior: absent, malformed,
unknown, or expired session cookies resolve anonymously. When an `Authorization`
header is present, any resolution or authentication failure propagates as
rejection rather than being converted to an anonymous viewer. A successful
explicit credential resolves the authenticated viewer and participates in the
same cookie-retirement behavior.

### 4. Username is lowercase-canonical

A **Username** is a case-insensitive account identifier whose canonical stored,
compared, serialized, displayed, and URL form is lowercase ASCII. Accepted
characters remain `[a-z0-9_-]`; mixed-case ASCII input is accepted and
normalized exactly at `Username::from_str`. Entry points do not pre-normalize,
and interior equality remains direct `Username` equality.

`CONTEXT.md` will define this domain term. No Unicode username support or
separate case-preserving display form is introduced.

## Deliverables

1. Four numberless ADR drafts under `docs/adr/drafts/`, one for each decision
   above, all linked to #936.
2. `docs/ARCHITECTURE.md` projects each draft into the relevant current
   architecture section and removes the four #936 bullets from **Un-ADR'd
   reality**.
3. `CONTEXT.md` defines **Username** using the canonical semantics above.
4. Credential resolution carries enough provenance to distinguish Cookie,
   Bearer, and Basic and represents explicit-header resolution failures without
   collapsing them into “missing credential.”
5. Shared request/response integration expires a simultaneous session cookie
   only after successful Bearer/Basic authentication and preserves every
   pre-existing `Set-Cookie` value.
6. Existing affected call sites and documentation are migrated cleanly; no
   compatibility alias or deprecated precedence path remains.

## Acceptance criteria

1. In a normal server build, cheap KDF is absent from production dependency
   edges; in test builds it remains available. A non-debug cheap-KDF build fails
   compilation, and a debug server carrying cheap KDF exits before parsing CLI
   arguments.
2. A `RawToken` cannot be bound directly through sqlx; session/app-password
   persistence receives only `TokenHash`; raw-token debug formatting does not
   disclose the token body.
3. With both a valid session cookie and a valid Bearer token for different
   users, the request authenticates as the Bearer user.
4. With both a valid session cookie and valid Basic credentials for a different
   user, the request authenticates as the Basic user, subject to the existing
   Basic username check.
5. Any present but unsupported, malformed, invalid, or username-mismatched
   `Authorization` credential rejects authentication and never falls back to the
   valid cookie.
6. A successful Bearer or Basic request that also carries `session=` appends
   `session=; HttpOnly; SameSite=Lax; Path=/[; Secure]; Max-Age=0` using the
   deployment's cookie security setting.
7. The expiry cookie is emitted when header and cookie tokens are identical, is
   not emitted when header authentication fails, and does not replace another
   `Set-Cookie` header produced by the handler.
8. The cookie-expiry behavior is demonstrated through a Leptos server-function
   route, a raw Axum/AtomPub route, and an optional-auth route. On the
   optional-auth route, a valid explicit credential resolves the authenticated
   viewer and retires a simultaneous cookie, while a present invalid explicit
   credential is rejected rather than converted to anonymous.
9. With no `Authorization` header, valid cookie authentication and the existing
   empty/invalid-cookie behavior remain unchanged.
10. Parsing mixed-case ASCII usernames produces the lowercase canonical value;
    invalid characters remain rejected; Basic username comparison remains
    case-insensitive by direct canonical `Username` equality.
11. Each of the four ADRs is cited by the architecture view, and none of the
    four decisions remains listed as un-ADR'd reality.
12. The repository documentation and ADR gates pass after draft promotion at
    ship.

## Out of scope

- Changing Argon2 production parameters or removing cheap KDF.
- Introducing token encryption at rest, expiring sessions, transport-scoped
  tokens, or the type-state design declined in #554.
- Supporting multiple Authorization headers or authentication schemes beyond
  Bearer and Basic.
- Unicode usernames, case-preserving display names, or username migrations.
- Any behavior change unrelated to resolving explicit Authorization versus an
  ambient session cookie.
