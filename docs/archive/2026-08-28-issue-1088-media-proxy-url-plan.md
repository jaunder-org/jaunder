# Media Proxy URL Boundary Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a task when
> useful. This outline exists because the typed query changes observable HTTP
> rejection and redirect semantics on a public endpoint.

## Scope

In:

- Replace the proxy query's raw URL with `MediaSourceUrl` and redirect using its
  canonical serialization.
- Pin authentication/query precedence and valid/invalid request behavior at the
  Rust integration and Playwright endpoint seams.

Out:

- Fetching, caching, address/DNS/SSRF policy, redirect following, media storage
  changes, and unrelated proxy/auth refactors.

## Task outline

- [x] Task 1: Type the proxy query and pin HTTP boundary behavior
  - Contract: `ProxyParams.url: MediaSourceUrl`; extractor order remains
    `auth::User` then `Query<ProxyParams>`; `proxy_handler` retains the user-ID
    equality check and emits the canonical typed URL through the existing
    temporary redirect. The primitive-to-type boundary cites issue #697 and
    ADR-0063.
  - Verification: focused media-handler integration tests cover canonicalized
    valid `Location`, authenticated malformed/relative/non-HTTP(S)/empty-host
    400 responses, unauthenticated malformed 401 precedence, and mismatched-user
    401 behavior.
- [x] Task 2: Add browser-level proxy endpoint conformance
  - Depends on: Task 1's status and `Location` contract.
  - Contract: add `signInAsNewUserRecord(page) -> SeedRecord` in the existing
    helper layer, built from `seedUserViaTool` plus `applySeededSession`; the
    existing username-only helper delegates to it. The proxy test uses the
    returned `userId` and authenticated page request context with redirect
    following disabled, so it cannot contact the external target.
  - Verification: focused e2e coverage observes the canonical temporary redirect
    and an invalid URL's 400 response through the real server/router stack.

## Risk checks

- `MediaSourceUrl` remains the single validator; no duplicate URL parser or
  hand-written scheme/host normalization is introduced.
- Valid query spelling may canonicalize, but redirect status, authentication,
  `user_id`, and coherent stored `MediaSourceUrl` semantics do not change.
- Invalid query handling is proven at extraction, not by calling the handler
  directly or asserting private deserialization mechanics.
- The Playwright request cannot follow or make network contact with the supplied
  external URL.
- Production has no raw proxy URL compatibility field/path after cutover.
- The implementation references issue #697 and ADR-0063; no ADR, architecture
  projection, glossary term, schema change, or lint suppression is added.
