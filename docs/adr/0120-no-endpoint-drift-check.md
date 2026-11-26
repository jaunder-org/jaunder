# ADR-0120: The server-fn snapshot gate carries no endpoint-drift check

- Status: accepted
- Date: 2026-08-11
- Amended: 2026-11-25 — the seed cross-check is `repository-policy`'s (#2100)

## Context

The server-fn coverage snapshot gate once compared each fn's _declared_
`endpoint = "…"` against the derived `<vertical>/<ident>` — a real cross-check
while an author wrote that literal by hand. #714 removed the hand-written
literal: the inventory computes `endpoint` with the very expression such a check
would compare it to (`server_fns.rs`), so both failure arms — a missing
endpoint, and a declared one that disagrees — are unreachable by construction.

## Decision

The gate performs **no** endpoint-drift check, deliberately: a comparison of a
value against itself passes for the wrong reason, which is worse than no
comparison at all. What verifies the computed endpoint instead is
`server_fn_coverage_check`'s seed cross-check, which compares it against URIs
observed in a real captured run — ground truth produced by the macro's actual
expansion, not a second restatement of the rule.

_Amended by #2100._ The coverage verdict moved to `dev`'s Python, but the seed
cross-check stays live in Rust as a `repository-policy` test, because that
package owns the derivation. It checks the seed's recorded
`(module, span name, URI path)` tuples against the live `web/src` inventory. The
Python extractor's own seed checks run against the seed and an inventory
recorded with it.

## Consequences

- Re-adding a declared `endpoint` attribute would reintroduce the drift class
  without a gate; the macro refuses the key (`macros/src/server_fn.rs`), which
  is the real guard.
- Endpoint correctness is asserted only where real traffic exists (the seed
  cross-check).
