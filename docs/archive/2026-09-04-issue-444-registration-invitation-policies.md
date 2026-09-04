# Registration Invitation Policies Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch`. This outline exists because issue #444 changes public
> configuration values and authorization semantics.

## Scope

In:

- Cleanly replace the three-value registration policy with the approved
  four-policy model across configuration, telemetry, fixtures, web, and CLI.
- Enforce invitation authority at every issuance/listing interface and project
  it through registration UI, `/invites`, and authenticated navigation.
- Preserve the existing direct invitation email, metadata-only ledger, invite
  capability handling, backend parity, and checked documentation bundle.

Out:

- Public invitation requests, request persistence, issuer/recipient persistence,
  policy settings UI, rate limiting, compatibility aliases, and historical ADR
  or archived-plan rewrites.

## Task outline

- [x] Task 1: Establish the four-policy domain interface and configuration
      cutover
  - Contract: `common::registration::RegistrationPolicy` exposes `Closed`,
    `OperatorInvites`, `MemberInvites`, and `Open`; exact tokens are `closed`,
    `operator_invites`, `member_invites`, and `open`. Its shared pure interface
    is `requires_invitation(self) -> bool`,
    `may_issue_invitation(self, is_operator: bool) -> bool`, and
    `may_list_invitations(self, is_operator: bool) -> bool`; web and CLI callers
    consume these predicates rather than reproduce the matrix.
  - Consumers: typed site configuration, host registration telemetry, storage
    accessors, test setup builders, raw seeds, and generic site-config CLI
    validation all move to the new values. `invite_only` is rejected; absent or
    invalid configuration remains `Closed`; telemetry retains its separate
    `CliBypass` determinant only for flows unrelated to policy authorization.
    This task also owns the four-policy `CONTEXT.md` glossary, proposed ADR
    draft, `docs/ARCHITECTURE.md` projection/citation, and approved spec bundle.
  - Verification: policy parse/display/serde and predicate matrix tests; both
    storage backends round-trip all four values and fall back to `Closed`; CLI
    site-config behavior accepts the exact new tokens and rejects the removed
    token; repository documentation formatting, links, and ADR bundle checks
    accept the complete projection.

- [x] Task 2: Enforce registration and invitation authority at hard interfaces
  - Depends on: Task 1 policy interface.
  - Contract: registration rejects every shape under `Closed`, requires and
    consumes a valid invitation under both invitation policies, and under `Open`
    accepts a supplied code without validating or consuming it. Web invitation
    creation authenticates first, then checks policy and operator status before
    URL validation, storage mutation, or mail. Listing requires an operator and
    either invitation policy. `jaunder user-invite` acts as an operator and
    rejects `Closed`/`Open` before minting.
  - Verification: dual-backend server integration matrices cover direct/valid
    invite registration, code consumption, anonymous/member/operator create and
    list calls, unauthorized no-side-effect behavior, metadata responses without
    raw codes, mail delivery, and local CLI policy gating.

- [x] Task 3: Project invitation authority through the CSR interface
  - Depends on: Tasks 1 and 2.
  - Contract: `/invites` remains the single route. `Closed`/`Open` render it
    unavailable; `OperatorInvites` gives operators the send form plus ledger and
    rejects members; `MemberInvites` gives every authenticated user the send
    form but fetches/renders the ledger only for operators. Authenticated
    navigation shows `/invites` exactly when the advisory session role and
    authoritative policy permit issuance; server checks remain authoritative.
    Registration guidance treats both invitation policies alike.
  - Verification: sidebar catalog/filter tests cover policy × role visibility;
    component/integration coverage proves the page never dispatches a forbidden
    list call; Playwright exercises operator-only, member-invite, Closed, and
    Open route/navigation states plus the existing email-link registration round
    trip.

## Risk checks

- One policy interface owns the role matrix; server functions and CSR callers
  consume it rather than maintaining divergent matches.
- Unauthorized create calls stop before base-URL lookup, transaction creation,
  mail dispatch, and telemetry success recording.
- `Closed` does not redeem outstanding codes; `Open` does not validate or
  consume supplied codes; both invitation policies retain cheap
  precheck-before-Argon2 behavior from ADR-0022.
- Invitation list wire data remains metadata-only and never exposes raw codes.
- Both SQLite and PostgreSQL prove storage-backed behavior; e2e policy mutation
  remains serial and restores the singleton test state.
- Current docs ship together: `CONTEXT.md`, `docs/ARCHITECTURE.md`, the proposed
  ADR draft, and the approved spec. `docs/README.md` remains generated, and
  historical ADRs/archives remain historical.
- Each task reaches `jaunder-commit` only after its focused behavioral evidence;
  the commit hook owns the single `precommit` gate. No lint suppression is added
  without explicit approval, and commits carry no `Co-Authored-By` trailer.
