# Public Theme Selectors Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch`. This
> outline exists because the work changes durable storage interfaces, operator
> authorization, and the public projector/CSR protocol.

## Scope

In:

- One typed theme domain shared by site and author settings.
- Operator-owned site theme plus authenticated author's optional override.
- Route-aware effective-theme resolution for projector and CSR navigation.
- Operator and author controls on `/profile`.
- Clean removal of browser-local theme persistence.
- SQLite/PostgreSQL parity, rendering coincidence, authorization, and browser
  proof required by the approved spec.

Out:

- New database tables or migrations.
- A Blog aggregate or per-viewer presentation.
- Custom themes, assets, layout controls, and cache-purge infrastructure.

## Task outline

- [x] Task 1: Establish typed theme persistence
  - Contract: add one closed `Theme` value (`terminal`, `studio`, `reader`,
    default Studio); register typed site and user configuration keys; expose
    typed site-theme and optional author-override reads and scoped transactional
    writes through the existing storage interfaces. Invalid site values default
    Studio; invalid author values behave as no override; database errors
    propagate.
  - Verification: closed-key validation and backup/restore expectations pass;
    dual-backend storage tests prove site round-trip, author round-trip/delete,
    fallback, and read-error behavior.

- [ ] Task 2: Carry effective theme through public rendering
  - Contract: introduce one public-presentation envelope that carries a
    server-resolved `Theme` alongside projector seeds and client-navigation page
    data. A deep resolver module hides precedence: aggregate routes use site
    theme; User, author-tag, and permalink routes use author override or site
    fallback. Projector markup, serialized seed, pure renderer, CSR mount, and
    destination navigation consume the same value. No caller reimplements
    precedence.
  - Verification: pure-render and projector tests prove initial byte
    coincidence, route precedence, read-error propagation, same-input ETag
    identity, and ETag change with theme; CSR navigation tests prove the
    destination theme replaces the source theme.

- [ ] Task 3: Replace the settings control with persisted mutations
  - Contract: `/profile` exposes `Your pages theme` to authenticated authors and
    `Site theme` only to operators. The author interface supports `Site default`
    by deleting the override. Server functions derive ownership from the
    authenticated session, hard-guard operator operations, use typed storage and
    `WriteScope`, and return `MutationOutcome`. Confirmed and indeterminate
    outcomes revalidate; controls adopt successful rereads while preserving
    error-like indeterminate feedback.
  - Verification: server-function tests prove anonymous/member/operator
    authorization, no cross-author owner input, confirmed/indeterminate
    reconciliation, and malformed wire-value rejection; focused web tests prove
    control state and reset semantics.

- [ ] Task 4: Complete the public cutover and browser proof
  - Contract: delete `jaunder_theme`, its localStorage adapter and
    theme-specific client telemetry contexts/tests. Keep private cockpit
    rendering on Studio. Update architecture projection and the #1341 follow-up
    wording to reflect site themes plus author overrides; preserve the frozen
    rejected spec and archive this approved replacement during shipping.
  - Verification: browser coverage uses fresh authenticated and anonymous
    contexts to prove both settings persist without localStorage, inheritance
    after `Site default`, route-specific public presentation, prepaint/CSR
    coincidence, navigation, access control, and existing CSS tokens. Run the
    repository check and complete e2e lanes before the commit/review/ship gates.

## Risk checks

- Public theme resolution never reads viewer identity and therefore preserves
  anonymous byte identity and cacheability.
- Mutation ownership comes only from server-authenticated identity; client input
  cannot select another author or bypass operator checks.
- The public-presentation envelope reaches every projector and navigation
  constructor, including permalink and both tag route families.
- Missing/invalid values and operational read errors remain distinct at both
  storage seams.
- Commit-indeterminate outcomes trigger reread without being presented as
  success.
- No browser-local theme alias, fallback, telemetry, test fixture, or stale
  caller survives the cutover.
- `docs/ARCHITECTURE.md` reflects the effective-theme input without inventing a
  new ADR; existing storage, rendering, closed-registry, auth, and write-scope
  ADRs already own the mechanism.
