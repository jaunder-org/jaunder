# Centralize Session Cookie Attributes

## Outcome

Session-cookie set and clear headers continue to expose exactly the same public
interfaces and wire values while sharing one private typed formatter in the host
authentication module.

## Load-bearing decisions

- Represent formatting state with a private `SessionCookie<'a>` enum whose
  variants distinguish setting a borrowed `RawToken` from clearing a session.
- Implement formatting through that enum's `Display` implementation; the two
  existing public formatter functions remain the stable callers' interface.
- Preserve the exact attribute order: cookie value, `HttpOnly`, `SameSite=Lax`,
  `Path=/`, optional `Secure`, then clear-only `Max-Age=0`.
- Keep `RawToken` typed through the formatting boundary. Do not replace it with
  an unvalidated string or expose it through debug output.
- Preserve each adapter's existing behavior, including deployment-driven
  `Secure` selection and appending the retirement cookie without replacing other
  `Set-Cookie` headers.
- This refactor does not alter the authentication model established by ADR-0007,
  ADR-0098, ADR-0107, ADR-0132, or the host boundary in ADR-0058.

## Acceptance

- Secure and insecure set headers are byte-for-byte unchanged.
- Secure and insecure clear headers are byte-for-byte unchanged, with
  `Max-Age=0` present only when clearing.
- Both public formatter functions delegate cookie serialization to the single
  typed formatter.
- The cookie name and common attribute sequence are serialized through one
  shared path; only the cookie value and clear-only suffix vary by enum variant.
- Existing focused host and server authentication tests pass.
- `cargo xtask check` passes.

## Boundaries

- Do not change public or test interfaces, response adapters, cookie settings,
  authentication policy, or session-token generation and validation.
- Do not implement or close #677 response plumbing.
- No domain glossary or ADR change is required for this private reversible seam.
