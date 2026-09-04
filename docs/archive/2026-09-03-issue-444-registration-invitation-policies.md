# Registration invitation policies (#444)

## Outcome

Jaunder exposes four registration policies whose names state both whether a new
user needs an invitation and who may issue one. Eligible inviters can discover
and use the existing invitation-email flow at `/invites`; no public invitation
request flow is added.

## Load-bearing decisions

- The registration policies are `Closed`, `OperatorInvites`, `MemberInvites`,
  and `Open`.
- Their configuration values are exactly `closed`, `operator_invites`,
  `member_invites`, and `open`; each parses and serializes as its corresponding
  policy.
- `Closed` rejects every registration attempt, including one carrying an
  otherwise valid invitation, and permits nobody to issue or list invitations.
  It remains the default when the setting is absent or invalid.
- `OperatorInvites` accepts registration only with a valid invitation and
  permits only operators to issue or list invitations.
- `MemberInvites` accepts registration only with a valid invitation and permits
  every authenticated user to issue invitations. Ordinary members do not see the
  global invitation metadata ledger; operators do. The ledger never exposes raw
  invitation codes.
- `Open` accepts registration without consuming an invitation and permits nobody
  to issue or list invitations. A supplied invitation code is ignored and
  remains unused.
- Invitation authorization is enforced by both web server operations and the
  local `jaunder user-invite` command. Route and navigation visibility mirror
  web authority but are not security boundaries.
- The existing invitation email remains the delivery mechanism: the inviter
  supplies the prospective user's address and that address receives a link
  carrying the invitation code.
- `/invites` remains the single capability route because invitation issuance is
  not always an administrative operation.
- The old `InviteOnly` name and `invite_only` configuration value are removed in
  a clean cutover. There are no deployed instances whose stored policy needs
  migration or compatibility parsing.
- The durable policy model is recorded in
  `docs/adr/drafts/registration-policy-separates-invitation-authority.md`.

## Acceptance

- `Closed` rejects direct and invited registration; both invitation policies
  reject direct registration and accept a valid invitation; `Open` accepts both
  request shapes without consuming a supplied invitation.
- Invitation creation and email delivery succeed only for an operator under
  `OperatorInvites`, and for any authenticated user under `MemberInvites`.
  Anonymous and unauthorized authenticated calls are rejected.
- The local `jaunder user-invite` command acts with operator authority: it may
  issue under `OperatorInvites` and `MemberInvites`, and rejects `Closed` and
  `Open`.
- Invitation listing succeeds only for operators under either invitation policy
  and never returns raw invitation codes.
- `/invites` is discoverable in authenticated navigation exactly when the
  current user may issue invitations.
- The invitation page is unavailable under `Closed` and `Open`. Under
  `OperatorInvites`, it is unavailable to non-operators and shows both the send
  form and metadata ledger to operators. Under `MemberInvites`, it shows the
  send form to every authenticated user and the metadata ledger only to
  operators.
- UI absence cannot bypass server authorization: direct calls to invitation
  creation and listing enforce the same policy matrix.
- Both SQLite and PostgreSQL behavior remains identical where storage-backed
  registration and invitation tests apply.
- Existing invitation-email and invite-link acceptance behavior remains intact.
- Configuration round-trips the four exact values, rejects `invite_only`, and
  preserves `Closed` as the absent-or-invalid default.
- The domain glossary and architecture view state the four-policy model and cite
  its decision record.

## Boundaries

- No unauthenticated invitation-request form, request queue, operator inbox,
  operator-email setting, or public-form rate limiter.
- No registration-policy settings UI; operators continue to configure the policy
  through the existing configuration surface.
- No invitation recipient or issuer persistence and no member-visible invitation
  history.
- No compatibility alias for `InviteOnly`, `invite_only`, or the old policy
  semantics.
