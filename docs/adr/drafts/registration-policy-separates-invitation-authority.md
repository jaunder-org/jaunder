# ADR-DRAFT: Registration Policy Separates Invitation Authority

- Status: proposed
- Date: 2026-09-03
- Issue: [#444](https://github.com/jaunder-org/jaunder/issues/444)

## Context

The registration policy previously mixed two questions. `Open` admitted direct
registration, `InviteOnly` required an invitation code, and `Closed` rejected
registration. Separately, the invitation endpoint allowed every authenticated
user to mint and email an invitation even though the UI and issue language
called it an operator flow.

That model could not express an instance where registration requires an
invitation and only operators may issue one. Naming both operator-issued and
member-issued behavior “invite only” also hid a material authorization choice. A
public “request an invitation” form was considered and rejected: automatically
minting a link for any submitted address is email-verified open registration,
not operator-controlled invitation.

## Decision

Registration policy has four explicit values and configuration spellings:

- `Closed` / `closed`: registration and invitation issuance are disabled.
- `OperatorInvites` / `operator_invites`: a valid invitation is required to
  register, and only an operator may issue or list invitations.
- `MemberInvites` / `member_invites`: a valid invitation is required to
  register, and any authenticated user may issue one; only operators may list
  the global metadata ledger.
- `Open` / `open`: registration is allowed without consuming any supplied
  invitation, and invitation issuance is disabled.

Absent or invalid configuration remains `Closed`. Authorization is enforced at
both the web server boundary and the local administration command boundary;
navigation and page rendering only project web authority. The single `/invites`
capability route is retained because issuance is not always operator-only.

The old `InviteOnly` / `invite_only` value is removed rather than aliased or
migrated. Jaunder has no deployed instances requiring data compatibility, and a
clean vocabulary is safer than preserving ambiguous semantics.

## Consequences

The policy value now states both the registration gate and invitation issuer.
Operators can run invitation-only sites without granting issuance to every
member, while communities may deliberately choose member-issued growth. `Closed`
remains a real stop state that also blocks outstanding invitation codes, and
`Open` remains direct registration rather than an email-verification flow. The
local `jaunder user-invite` command acts with operator authority and follows the
same policy, rather than retaining a bypass that would make “issuance disabled”
false.

Invitation creation and listing need distinct authorization: members may create
under `MemberInvites` but never receive the global ledger, and the ledger never
contains raw invitation codes. UI navigation must therefore depend on both
authenticated identity and current policy, while every server operation repeats
the full check. Existing configuration and tests using the removed value must
move to one of the explicit policies.
