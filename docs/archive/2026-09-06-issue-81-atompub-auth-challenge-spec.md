# Issue #81 — AtomPub Basic Authentication Challenge

## Outcome

Unauthenticated or unsuccessfully authenticated requests to Jaunder's
authenticated AtomPub surface return HTTP 401 with a Basic authentication
challenge. Protocol Clients can discover the supported scheme and present an App
Password without changing the existing successful authentication flow.

## Load-bearing decisions

- The challenge is scoped to authenticated AtomPub routes: the Service Document,
  Posts Collection and Members, and Media Collection and Members.
- The public RSD discovery route remains unauthenticated and outside this
  contract.
- Every authentication-originated AtomPub 401 carries exactly:
  `WWW-Authenticate: Basic realm="Jaunder AtomPub"`.
- Covered authentication failures include absent credentials, malformed or
  unsupported `Authorization` values, unknown or revoked credentials, and a
  Basic username that does not match the resolved credential owner.
- The challenge advertises HTTP Basic because that is the AtomPub Protocol
  Client mechanism established by ADR-0014. It does not remove the currently
  accepted cookie or Bearer credential paths.
- The response change is header-only. Existing 401 status and empty response
  body remain unchanged.
- Other statuses, including internal authentication failures reported as 500, do
  not receive the challenge.
- The AtomPub boundary owns this protocol projection. Shared authentication
  consumers outside AtomPub—including `/media/proxy`, Leptos server functions,
  and client telemetry—must not begin advertising Basic authentication.
- This is a completion of the existing ADR-0014 authentication contract, not a
  new architectural decision. It adds no domain term and requires no ADR or
  `CONTEXT.md` change.

## Acceptance

- An AtomPub request with no credentials returns 401, an empty body, and exactly
  the selected `WWW-Authenticate` challenge.
- AtomPub requests with malformed or unsupported `Authorization` headers return
  the same 401 challenge without falling back to an ambient session cookie.
- AtomPub requests with an unknown or revoked credential return the same 401
  challenge.
- An AtomPub Basic request whose username differs from the credential owner
  returns the same 401 challenge.
- Successful AtomPub Basic authentication continues to return the route's normal
  response without a `WWW-Authenticate` header.
- Existing successful cookie and Bearer authentication behavior on AtomPub
  routes remains unchanged.
- Authentication failures outside the AtomPub surface retain their existing
  response headers and representations.
- Server integration coverage exercises the AtomPub authentication-failure
  classes and exact header value.
- End-to-end coverage proves that a revoked App Password receives the challenge
  through the running application.
- The MarsEdit acceptance checklist documents a live check that an
  unauthenticated AtomPub connection is challenged and can then authenticate
  with an App Password. A live MarsEdit run is not required to complete this
  issue.

## Boundaries

- No new authentication scheme, credential type, login flow, or token-storage
  behavior.
- No new error body or Atom error document.
- No change to authorization rules, including Basic username matching and
  per-user AtomPub route guards.
- No challenge on the unauthenticated RSD route or on non-AtomPub authentication
  failures.
- No requirement to automate or bundle MarsEdit.
