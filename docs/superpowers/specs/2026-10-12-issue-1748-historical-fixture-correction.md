# #1748: Historical ordinary fixture applicability correction

Status: Manager-approved bounded correction; spec committed through the normal
hook before implementation. Source implementation and pure focused tests may
proceed without #1760. Normal Rust commit/push advisory may require legitimate
landed integration. No hook waiver or unmerged dependency copy is authorized.

## Purpose and scope

Maintain applicability of the existing approved #1419 historical fixture, not
replace or expand its acceptance contract. Repair the ordinary fixture/profile/
verifier boundary while preserving modern regression obligations. This spec is
the implementation and conformance-review contract; maintain it while work is in
flight, project enduring conclusions into `docs/production-baseline.md`, then
delete it during normal completion cleanup. Ignored design and evidence remain
unchanged and are not tracked delivery artifacts.

No generic capability framework, wire adapter, fallback, new CLI option, ambient
feature flag, backup policy, product-source change, format change, report schema
change, or styling substitution is permitted. Shared `posts.ts`, public
deployment options, historical source trees, current helper and lockfiles remain
unchanged. Backend parity and all four restore directions remain mandatory.

## Closed source-only selection

Resolve the ordinary product source before harness admission. Exactly
`520034ed6f778a854cd5f8708419c1b2b700ad9f` selects the historical fixture.
Acceptance selects from **source**, never target. All other ordinary sources and
live e2e retain strict modern defaults; styling retains its separate existing
path. Keep selection fixed through upgrade, retained state and every restore.

One private constructor owns this indivisible historical tuple:

- Product source: `520034ed6f778a854cd5f8708419c1b2b700ad9f`.
- Helper source: `78d201e6969ed17b8a54b87d3de24be6afa1f5ee`, the recorded
  successful discovery harness, supplied by its immutable historical flake with
  its own dependencies and locks.
- Graph: `qualification-harness-legacy-1419`, alongside unchanged modern
  `qualification-harness`; current xtask, browser, Node and VM tooling remain
  execution authority.
- Seed authority: historical `pub fn sandbox_profile_manifest` in the selected
  helper's actual source; expected complete authority-file SHA-256:
  `18778e4aa1411fdb784cc689c15dd27fcb5e0049ca4b0452368c198de759f831`.
- Exact historical fixture/profile and Post-create request/response contract.

Members are not independently selectable. Manifest version, response shape,
labels, format numbers and target identity cannot choose the fixture.

## Fixture and operation contract

Keep one shared creation/verification workflow. Historical verification
preserves all 73 seeded Posts and their native title, body, authoring format,
visibility, draft/scheduled/publication state and Media references. Preserve the
real seeded text Media object and independently uploaded PNG, checking
owner/member/content identities, bytes and hashes. Preserve browser/web/AtomPub
operation Posts, public and restricted views, retained authentication, fresh
login, App Password mint/use/revoke, and all three Syndication Feed formats with
inclusion/exclusion assertions. Execute restart, reboot, upgrade and all four
schema-exact restores (SQLite → SQLite, PostgreSQL → PostgreSQL, and both
cross-backend directions). Format-1 admission and exact schema matching are
unchanged (ADR-0174).

Historical expectations do not require later Rendered Title fixtures or the
additional SVG corpus. Modern verification remains strict for its five actual
Media objects and every Rendered Title assertion. Do not skip any actually
created record or remove modern obligations globally.

At the existing typed seed boundary, retain exact modern v2 and historical v1
wire types. The constructor-owned selection chooses a strict parser; no v1-to-v2
conversion or synthesized records. Historical demo requires 73 unique (author,
slug) Posts and one actual `baseline-seeded.txt` Media object with exact owner,
URL, hash and byte count. Preserve exact historical object/null wire semantics;
this demo requires the real object. Modern requires its exact v2 profile and
five-Media/current-title corpus. Validate required types, records and
identities; cardinality alone is not authority. Host requests require the closed
selection, and persisted state must match it before verification.

### Fixture-owned Post-create boundary

Both the **request and response parser** belong to the selected ordinary
fixture, not shared `posts.ts`. Historical requests use the established `post`
request wrapper, native format/body, `slug_override`, publication fields and
explicit `audience: { base, named }` contract. Keep modern
`{ public, subscribers, named }` expectations unchanged.

The fixed historical product declares `create` as `MutationOutcome<SavedPost>`
(`web/src/posts/api.rs` at the product pin); its `common/src/mutation.rs` uses
the externally tagged `Confirmed` envelope. Require
`{ Confirmed: { post_id, slug, published_at, permalink } }` with the exact
domain field types and root-relative permalink, **not** a nested `post` payload,
bare payload or alternative envelope. Indeterminate commits cannot establish
creation success. Validate the selected modern fixture's exact response contract
at the same boundary too. Never infer compatibility from returned shape,
catch/retry with another request, or retain the direct-or-nested payload
adapter. An incompatible upgrade target fails rather than switching contracts.
Focused request/response, browser-control and native-body compatibility proof
precedes positive qualification. The same closed selection owns historical
composer controls inside the shared baseline workflow: direct Format buttons,
Slug input and `#audience-base`, without modern disclosure controls. Modern
creation retains shared `composePost` unchanged. Ordinary App Password minting
uses the same fixed fixture: modern delegates to unchanged `sessions.ts`;
historical creation uses `.j-app-passwords` and reads
`.j-app-password-token code`. Primary creation, manifest-author access and fresh
mint/use/revoke all retain that selection. No DOM probing, retry or fallback
selects a control contract.

## Provenance, guest bindings and evidence

The selected immutable graph pairs its helper executable and seed source from
the same package. Preserve executing-xtask/selected-output verification and
exact helper-override equality; copied/native executables and different
overrides remain refused. Derive the canonical admitted helper package root from
the selected graph executable, not a caller-supplied path.

Use the existing qualification-only VM seam for that admitted helper package:
both guest control PATH and system packages must agree. Assert the **selected**
graph, not hard-coded modern `.qualification-harness`. Keep supported product
module wiring, graph/VM GC roots, drift/source rechecks and #1747 service
identity, environment, quoting and exit behavior intact. Do not reintroduce Git
build-revision equality: unchanged Nix outputs across clean commits remain
admissible.

For each ordinary source backend, observe service identity and publisher-lock
ownership after browser/Media creation and after restart plus read-only
Syndication Feed verification. Use existing guest control and lifecycle records;
retain one restricted structured observation per boundary and emit a
numeric-only sanitized summary. Require canonical service User/Group, resolved
non-root IDs, actual service MainPID/executable, all four `/proc` UID/GID fields
and lock owner to agree. MainPID must be stable within one observation, not
across restart. This observes the running service and lock, not an
already-exited seed/CLI child. Host stand-ins prove command/parsing/retention
only; actual VM proof remains required. No evidence-schema or observer framework
change is permitted.

Evidence schema v2 stays truthful: fixed product source, new current clean
harness commit, actual selected graph derivation/output/NAR identities, actual
selected helper authority-file digest/version, and actual current operation
verifier full-file digest. Never attribute current execution to the historical
operation digest or relabel historical schema1 evidence. Preserve sanitization,
publication allowlist, failure classification and two-file publication checks.

## Focused acceptance tests and remaining admission

- Exact resolved source selects the complete historical tuple for discover and
  accept; different sources, target-only equality, styling and live e2e do not.
- Reject wrong helper/graph/source/authority/digest associations, override
  mismatch, copied/native executables and real drift; accept unchanged selected
  Nix outputs across clean commits. Prove both guest helper bindings and roots
  agree with admission.
- Accept each exact profile only with its explicit selection. Reject swapped or
  malformed profiles, omitted/duplicate Posts, missing Media or required fields,
  wrong identities and persisted-state selection switches before success.
- Reject wrong Post-create request/response shapes, including nested/bare
  historical payloads, malformed fields and indeterminate envelopes. Prove
  historical UI/body compatibility without adapters or retries.
- Prove every historical obligation still executes; modern still rejects missing
  Rendered Title fixtures or any of its five Media. Preserve coordinator
  phase/sequence, seed-process, sanitization and publication proofs.

Actual historical Nix helper realization, source/output metadata, admission,
backend compatibility, browser/body compatibility and a distinct compatible
format-1 target remain unproven. Historical green supports the investigation
pin, not present qualification. Positive runtime proof requires accepted #1747
correction and actual helper/target admission. Nix cost/evaluation/build/VM work
belongs to a separately authorized execution unit. No final #1419 candidate
selection or acceptance is authorized by this correction.
