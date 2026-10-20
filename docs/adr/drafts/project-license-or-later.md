# ADR-DRAFT: License Jaunder's own code under GPL version 3 or later

- Status: proposed
- Date: 2026-10-19
- Issue: [#1702](https://github.com/jaunder-org/jaunder/issues/1702)

## Context

During the native image preparation dependency review, the user explicitly
clarified that Jaunder itself should grant GPL version 3 or later, not merely
allow that license for a dependency. This current decision is recorded as
JAU-LICENSE-FIRSTPARTY-001; it is separate from Licence001's dependency-policy
allowance and does not backdate that earlier authority.

## Decision

License Jaunder-owned code under the GNU General Public License, version 3 or
(at the recipient's option) any later version, using SPDX `GPL-3.0-or-later`.
The root README states the project grant; the GNU license document remains
verbatim. Root Cargo inheritance, independent tooling package declarations and
Jaunder's Nix package metadata express the same grant.

The dependency allowlist retains `GPL-3.0-only` and adds exactly
`GPL-3.0-or-later`. This is not an advisory waiver or a broader source-trust
allowance. Third-party licenses, contributor ownership, separately licensed data
and fixture notices remain unchanged; historical decision records are not
rewritten.

## Consequences

Recipients may choose a later GNU GPL version for Jaunder's own code. Dependency
license and source checks remain necessary on the actual resolved graph. This
records the user's licensing choice, not a copyright ownership audit or legal
clearance certification. The application User's Content License vocabulary and
publication rights are unrelated and unchanged.
