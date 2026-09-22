# Open Registration CTA

## Outcome

The anonymous Local masthead advertises registration only when the instance's
current Registration Policy is `Open`. Closed and invitation-based instances
continue to offer Sign in without presenting a generic Register action that
cannot directly create an account.

## Load-bearing decisions

- The generic Register CTA is visible if and only if Registration Policy is
  `Open`.
- `OperatorInvites` and `MemberInvites` do not expose the generic CTA.
  Invitation recipients continue to enter registration through the invitation
  URL carrying its code.
- `Closed` does not expose the CTA.
- Sign in remains visible to anonymous visitors under every Registration Policy.
- The affected surface is the Local masthead only. Permalink, User, Site Tag,
  and User Tag mastheads do not currently offer registration actions and remain
  unchanged.
- Current Registration Policy travels with the Local presentation state used by
  the public projector and browser. A seeded cold mount renders that policy
  synchronously and is not a policy-loading state.
- On unseeded browser navigation, the Local masthead may render once Site
  Identity is available: Sign in remains present, while Register is withheld
  until a successful policy read resolves to `Open`. A policy-read failure
  leaves Register absent.
- Registration authority remains server-owned. Navigation and markup only
  project the current policy; they do not replace endpoint enforcement.

## Acceptance

- The anonymous Local masthead renders the Register link, with its existing
  label, destination, and styling, under `Open`.
- The Local masthead omits the Register link under `Closed`, `OperatorInvites`,
  and `MemberInvites` while retaining Sign in.
- Initial projected Local HTML and the seeded browser cold mount agree for all
  four policies.
- Unseeded browser policy loading and policy-read failure do not flash or retain
  a Register link; Sign in remains available once the Local masthead renders.
- Direct invitation links still reach the existing invited-registration
  experience under either invitation policy.
- Direct `/register` behavior and registration endpoint authorization remain
  unchanged.
- Focused host and browser coverage pins the four-policy visibility matrix and
  the no-flash behavior.
- Comparable transient before/after screenshots show the Closed Local page using
  the built-in Studio theme, a signed-out viewer, a fixed Site Identity, and one
  fixed public Post at `1440×900` and `390×844`. Capture waits for the Local
  heading, fixed Post, and `document.fonts.ready`, with no loading or error
  state; the after evidence shows only the intended Register CTA removal.

## Boundaries

- No public invitation-request or approval flow.
- No redirect, replacement page, or new messaging for direct `/register` visits.
- No changes to Registration Policy values, persistence, invitation authority,
  or server-side registration checks.
- No changes to authenticated navigation or to the action-empty permalink, User,
  Site Tag, and User Tag mastheads.
- No new stable Style Contract hook or unrelated masthead restyling.
