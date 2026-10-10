# Restore spaced Org keywords and repair missing tags (#1724)

## Outcome

Org keywords such as `100 bullets` become valid tags rather than silently
vanishing during publication. An opt-in Emacs repair command restores missing
normalized tags from local Posts without overwriting newer server content.

## Load-bearing decisions

### Keyword normalization

- Retain the existing ASCII tag grammar and canonical identity rules (ADR-0068);
  this issue does not broaden stored tag identity or label validity.
- Already-valid ASCII tag labels after boundary trimming retain their spelling
  and casing. Their existing canonical identity remains unchanged, including
  existing hyphen sequences. Non-ASCII letters such as the Kelvin sign reject at
  this new ingress boundary even if Unicode lowercasing could produce ASCII;
  strict stored-tag validation and existing data remain unchanged.
- For an otherwise-invalid non-empty term, trim surrounding whitespace,
  lowercase ASCII letters, replace runs of ASCII punctuation/whitespace with one
  hyphen, and remove leading/trailing hyphens. The result must satisfy the
  existing tag validator.
- Examples: `100 bullets` -> `100-bullets`, `James S. A. Corey` ->
  `james-s-a-corey`, `old man's war` -> `old-man-s-war`.
- Non-ASCII input that is not already valid, and terms with no usable ASCII
  letter/digit, explicitly reject the complete write. No transliteration,
  truncation, invented fallback tag, or silently discarded invalid category.
- Normalize each comma-separated term independently. Repeated leading Org
  keyword lines compose in source order; whitespace does not split a term.
  Existing empty-comma-term behavior and metadata boundaries remain unchanged.
- Raw Org KEYWORDS ingestion and AtomPub category ingestion use the same
  normalization contract. Emacs continues to retain original local source.
  Existing structured web tag validation is unchanged.
- Deduplicate by canonical identity in input order, preserving the first
  resulting label, and retain the existing distinct-tag cap. No automatic
  disambiguation or suffixes when different terms normalize to one identity.
- Invalid AtomPub categories return a client-visible validation failure rather
  than a successful write with missing tags. Rejection saves no Post changes.

### Repair authority and safety

- Deliver an Emacs repair command plus maintained remediation instructions. The
  command uses configured Protocol Client credentials and the existing
  local-root/remote-Member identity model, not direct database access.
- Default behavior is read-only preview. Corpus scanning uses Org's actual
  parser and the same leading CATEGORY/TAGS/KEYWORDS collection as publishing.
- Repair adds only missing normalized local tag identities to the current server
  tag set. Existing server tags and labels remain authoritative and are never
  deleted or relabelled. Invalid local terms block that Post's repair. Preview
  validates the complete canonically deduplicated union; exceeding the existing
  distinct-tag cap blocks the Post without a write.
- Do not treat `server-ahead` as permission to republish a local Post. Fetch the
  current remote Member and preserve its title, body, format, summary, audience,
  publication time/state, identity, and stored slug/permalink during tag repair.
  Apply only to currently published or scheduled Posts, whose stored slug is
  frozen by ADR-0130; draft Posts are reported blocked rather than risking slug
  re-derivation through AtomPub replacement. No protocol change is added.
  Ordinary mutation timestamps/versions, Post Revision creation, and derived
  rendering/search state may change as part of a successful tag update.
- Require a unique local/remote identity match, a valid current strong ETag, and
  a reviewed snapshot of local bytes plus the intended tag additions. Duplicate,
  missing, Deleted, foreign, ambiguous, or well-formed but invalid Member
  targets block. File, transport, or XML-decoding failures abort the read-only
  preview with retained contextual causes rather than masquerading as a domain
  rejection; no report from that failed read authorizes a write.
- Before applying, revalidate the reviewed local source and remote Member;
  modified visiting buffers, local byte changes, or remote ETag changes block
  rather than silently changing the approved operation.
- Application requires explicit confirmation of the reviewed batch and uses
  conditional writes with `If-Match`. No unconditional writes or stale-marker
  manipulation to bypass ordinary reconciliation.
- Leave local Post files and synchronization markers untouched. The preview and
  runbook explain that successful repair does not itself make local
  reconciliation report `unchanged`.
- Report per-Post outcomes deterministically: no missing identities is
  `unchanged`; eligibility, preflight, revalidation, or conditional-precondition
  refusal (including HTTP 412) is `blocked`; confirmed successful mutation is
  `repaired`; a definite other non-success HTTP response or known pre-dispatch
  failure is `failed`; no trustworthy response after possible write dispatch is
  `unknown-remote-outcome`. A lost write response must not trigger an automatic
  retry or a claim that the remote Post is unchanged.
- Subsequent fresh previews discover remaining work; already-present identities
  require no write. Cancellation stops before the next Post, retaining prior
  outcomes. There is no batch-wide transaction or rollback promise.
- Actual production application is separately approved, never performed as a
  development test or implied by spec/implementation approval.

## Acceptance

- Regression tests prove multi-line and comma-separated keywords, punctuation,
  surrounding whitespace, valid-label stability, canonical deduplication,
  normalization collisions, invalid/unusable terms, and the distinct-tag cap.
- AtomPub create/update and raw Org writes produce the same normalized tags;
  invalid input rejects atomically. Persisted assertions cover both backends.
  Emacs pure/live tests prove publication retains the original local keywords.
- Repair tests exercise the real command against disposable Posts: a
  server-ahead Post retains the enumerated authored/lifecycle fields, identity,
  stored slug/permalink, and existing server labels; absent normalized tags are
  added exactly once; a second preview is a no-op. Include collision-suffixed
  published slugs, scheduled Posts, blocked draft Posts, and over-cap unions.
- Preview performs no writes. Tests cover changed remote ETags, changed local
  files, modified buffers, duplicate identities, invalid keywords, per-Post
  failures, cancellation, and response-loss outcomes without automatic retry.
- Read-only inspection of `/home/mdorman/src/jaunder/radios-appear` using Org's
  parser reports exact affected counts and canonical collisions, excluding body
  examples. The preliminary scan found 151 affected files of 944; production
  preview must determine actual missing tags from fresh remote data.
- The runbook covers backups of local corpus and server, deployment of the fix,
  dry-run review, a separately approved small canary, verification, bounded
  batches, interruption/recovery, and later reconciliation without blindly
  fetching over original local keywords or pushing older local bodies.
- Execution evidence and corpus inventories remain in ignored/session storage or
  the PR. Commit code, tests/minimal synthetic fixtures, and current docs, not
  the private corpus or a point-in-time audit report.

## Boundaries

No Unicode tag expansion, schema migration, application HTTP endpoint, bulk
content overwrite, local corpus rewrite, timestamp reset, reconciliation-policy
relaxation, or production mutation. Other milestone reconciliation/performance
issues remain independently owned. A short implementation outline is required
for the conditional multi-Post remediation and failure/recovery contract.
