# Issue #1921: Publication time in the latest editing timezone

## Outcome

Show each Post's publication instant in its saved editing timezone, with the
zone's correct date-specific abbreviation (for example, EST or EDT). Every
accepted create or edit records the editor device's supported IANA timezone;
later edits replace it. A timezone-only change never changes the absolute
publication instant.

## Decisions and scope

Human-approved meaning is the **latest editing timezone**, not an immutable
creation timezone. The answer is “Use latest editing timezone” (Manager relay
`b38380f9-6550-4022-83f9-a15e9f54a2e1`, `1921-edit-zone-v3-reask`). The required
NY Org → London web edit → Org reconcile result is: “If I create an article via
org in NY, and then edit via the web in London, and then reconcile my org
directory, I would expect that article's org file to get an update to its
JAUNDER_DATE_TZ. You seem to be suggesting that that wouldn't be the case?”
(answer `eb5aab83-eb34-41ae-835b-840d5fd1bc5a`, `/1921-display-scope-final`).
The approved fallback is “Explicit UTC; report exceptions” (background
`37d74c15-24c7-4df7-9212-00bbc9f4382a`).

The earlier immutable-creation-zone and disconnected local display-property
recommendations are superseded. Use one Post timezone, represented in Org as
`JAUNDER_DATE_TZ`. All approved displays are in scope: public Post cards and
permalinks, scheduled list, edit scheduled notice, and Published timestamps in
Post history. Created, updated, deleted, and captured audit timestamps remain
UTC/instant values and are not converted to the Post timezone.

## Persisted and wire contract

Persist a required IANA-zone string as `posts.timezone` and in every immutable
revision's timezone snapshot. Validate with the application's bundled Jiff IANA
database (`TimeZone::get`); accept named IANA aliases when `iana_name()` is
present, reject `Etc/Unknown` and numeric offsets, and preserve the exact valid
input spelling. Add paired SQLite and PostgreSQL migrations. Existing Post and
revision rows receive explicit `UTC`; report the number backfilled as fallback
exceptions. Backups/restores include both fields while retaining ADR-0174's
exact-schema compatibility contract: do not invent automatic cross-version
backup conversion. Preserve all existing `published_at` values, timeline
ordering, and permalink-date calculations.

The web `PostInputs` JSON create/update object carries optional `timezone`. At
each create or edit submission, the browser captures
`Intl.DateTimeFormat().resolvedOptions().timeZone`, validates it against the
bundled supported IANA zones, and sends it. If capture is absent, numeric, or
unsupported, send explicit `UTC` and visibly warn that UTC was used. An omitted
field from an older client means UTC on create and preserve the stored zone on
update. A present empty/whitespace or otherwise invalid API value is a
validation error, not omission or fallback.

AtomPub uses the existing Jaunder namespace URI
`https://jaunder.org/ns/atompub`: add `<j:timezone>` to member read/write and
add the `timezone` feature token to the existing version-1 `<j:extension>`
capability advertisement. Namespace URI plus local name defines the extension,
not the prefix (ADR-0172). A valid explicit extension value sets the latest Post
timezone. Empty/whitespace or invalid values return bad request. Missing
extension on create uses UTC and reports the fallback; missing extension on
update preserves the saved timezone. Standard Atom `<published>` remains the
absolute instant and is never inferred from or changed by the timezone. Older
clients may ignore the extension.

### Emacs local DATE preparation

This civil-field comparison applies **only to Emacs local-document
preparation**; `JAUNDER_DATE_UTC` is a local comparison marker in that flow, not
a prior-value marker sent over AtomPub. Emacs sends body-only content and
computes standard Atom `<published>` locally, so its marker is not transmitted.
Before an update, compare the local DATE's civil `year/month/day/hour/minute`
fields with DATE rendered from the saved `JAUNDER_DATE_UTC` instant in the
recorded old `JAUNDER_DATE_TZ`. If unchanged, keep the exact original UTC
marker, including fractional seconds and later-fold identity, and re-render DATE
in the latest editor zone. If changed, interpret that authored wall time in the
latest editor zone and use the result as `<published>`. New local Posts have no
prior marker and interpret DATE in the current effective zone.

For an existing published or scheduled local Post whose `JAUNDER_DATE_UTC`
marker is missing or invalid, do not guess, truncate the instant, or reinterpret
DATE. Require a successful pull/reconcile to establish the canonical server
instant and matching local marker first; otherwise stop before PUT with an
actionable message. Existing numeric `JAUNDER_DATE_TZ` may interpret legacy
local DATE during recovery, but is not persisted as a Post timezone. A changed
DATE uses the existing ADR-0182 Jiff local-time policy: choose the earlier fold
instant and reject gaps. A zone-only refresh uses the saved instant directly, so
it retains fractional precision and a later-fold instant exactly.

### Raw Org server-ingress semantics

Raw Org ingress retains ADR-0155's existing contract. `JAUNDER_DATE_UTC`, when
present, is an assertion about the **final effective publication instant**; it
is not a prior DATE comparison marker. Marker-free raw Org remains backward
compatible. Standard structured Atom `<published>` takes precedence over the Org
header's publication lifecycle when both are present, as today. The timezone
transport value only selects the saved Post timezone; it never overrides an
independently established structured `<published>` instant.

For the persisted Post timezone, raw Org uses a valid explicit web
`PostInputs.timezone` or AtomPub `j:timezone` first; absent that, a valid
recognized named-zone `JAUNDER_DATE_TZ` header supplies the value. Legacy
numeric offsets are not valid persisted zones. Current raw DATE validation
rejects numeric zones before structured/header precedence; Emacs-local legacy
numeric baseline recovery remains separate. If neither timezone source is
present in an otherwise-valid raw input, create uses explicit UTC and reports
the fallback, while update preserves its stored zone. Existing raw DATE parsing
and `JAUNDER_DATE_UTC` final-value validation continue unchanged; no Emacs-local
baseline comparison or missing-marker rejection is applied to raw ingress.

On Emacs create/update, capture and validate the current device zone before
building the entry. For an existing Post, use the saved DATE/old zone and
`JAUNDER_DATE_UTC` comparison above; never merely swap `JAUNDER_DATE_TZ` and
resend the same wall DATE. Send the latest zone through `j:timezone`; only after
a confirmed conditional PUT write the server-accepted zone and its paired DATE
back to the Org file. On failed, stale, or uncertain PUT, leave the prior local
DATE/zone untouched. The client resolves a missing/invalid device zone to UTC
with a visible warning; the server rejects malformed nonempty values.

AtomPub GET returns the saved Post timezone and unchanged `<published>` instant.
Pull adopts the server zone; it does not capture a fresh local device zone.
Reconciliation likewise uses server state on pull. On a successful authored PUT,
reconciliation saves the zone that was captured and sent for that PUT, then
renders the DATE from the confirmed publication instant in that zone. Keep
`JAUNDER_ID` and conditional ETag checks. For NY Org → London web edit →
reconcile, the Org file receives London `JAUNDER_DATE_TZ`, London-local DATE,
and the same `JAUNDER_DATE_UTC`; a subsequent unchanged Org publish sends the
same `<published>` instant.

## Updates, revisions, and displays

The storage scalar no-op test includes timezone. A zone-only change persists and
creates a revision even when content and `published_at` are unchanged. The newly
inserted revision is a complete snapshot of the **old** Post, including its old
zone; the current Post receives the new zone. Every other lifecycle path that
creates a revision snapshots the timezone alongside the other prior Post fields.
A true no-op where zone and all other state are equal creates no revision.
Existing revisions are explicitly migrated to UTC and reported with other
fallback rows.

Public cards, permalink, scheduled list, and edit scheduled notice format the
publication instant using the current Post timezone. History Published values
use the timezone on the corresponding revision snapshot; Current state Published
uses the current Post timezone. Other history audit timestamps retain current
instant formatting.

## Existing-original recovery

When the promised local Org path/sample is supplied, provide only a read-only
extractor for exact `JAUNDER_ID` → `JAUNDER_DATE_TZ` pairs. Validate IDs and
supported IANA zones; report duplicate IDs, missing/invalid local properties,
and source files that cannot be mapped. Reporting server Posts absent from the
originals additionally requires a separately supplied, authorized read-only
Post-ID inventory; the local extractor cannot infer that set. Do not access
production or apply an import under this issue. Any later import needs separate
authorization, must target exact current Post IDs, and must decide revision
history treatment separately; it must not blanket-rewrite immutable revisions or
any `published_at` value. Never infer by title, slug, offset, or server-local
timezone.

## Acceptance

- Paired SQLite/PostgreSQL migrations, Post and revision records, queries,
  backup/restore, and exact-schema compatibility preserve timezone state.
- Browser and Emacs capture the latest supported IANA zone on create/edit;
  device capture failure explicitly selects UTC and warns. API omission is
  distinct from present empty/whitespace invalid input. Old-client omission uses
  UTC on create and preserves the stored zone on update. Invalid nonempty zones
  reject; aliases with a named IANA identity are accepted and their exact
  accepted spelling is retained; `Etc/Unknown` is rejected.
- Raw Org tests cover explicit transport timezone over header for the saved Post
  zone, header-supplied zone when transport is absent, and existing raw-ingress
  rejection of numeric-offset DATE zones without mutation. Test create-UTC /
  update-preserve with timezone genuinely omitted from otherwise-valid raw Org;
  numeric local DATE baseline recovery belongs to Emacs, not raw server ingress.
  Independently verify ADR-0155 final `JAUNDER_DATE_UTC` assertions, marker-free
  raw Org, and structured Atom `<published>` precedence over header lifecycle.
- Emacs-local tests alone compare DATE civil fields in the recorded old zone and
  preserve exact `JAUNDER_DATE_UTC` precision/fold identity. Test fractional
  seconds, an unchanged later-fold instant, changed DATE, invalid/missing marker
  recovery diagnostics, and Jiff gap/fold behavior, including scheduled Posts.
- Zone-only update changes the current Post zone and creates one revision of the
  old state/zone without changing `published_at`, permalink date, or ordering.
  All other revision-producing lifecycle paths retain the timezone snapshot; a
  true no-op creates no revision.
- Correct zone and DST abbreviation appear on public cards/permalink, scheduled
  list, edit notice, current history, and revision history. Projected/reactive
  public output agrees; audit timestamps remain unchanged.
- NY Org → London web edit → pull/reconcile rewrites `JAUNDER_DATE_TZ` and
  paired DATE in London while retaining exact UTC marker and `<published>`;
  subsequent unchanged Org publish preserves the instant. Pull does not
  recapture the local device zone. Failed/stale conditional writes do not claim
  local writeback.
- Recovery extractor reports local ID/zone exceptions and preserves exact
  strings. Server-ID absence checks require the separate supplied inventory;
  tests do not access production or mutate Post data.

The numeric raw-ingress acceptance wording above was factually corrected by
Manager clarification following Lead's SLICE3 numeric-ingress discrepancy
report. The original acceptance wording incorrectly assumed raw numeric DATE
support; existing raw DATE validation rejects it before structured/header
precedence. This preserves the approved design's “where existing behavior
permits” and “Existing raw DATE parsing ... continue unchanged” requirements; it
does not add numeric-offset server ingress or change Emacs-local numeric
baseline recovery.

## Boundaries and rationale

No timezone selector, viewer-local display policy, timestamp rewrite, production
access, or production migration execution. The original-file sample/path remains
forthcoming; it does not block this contract. UTC instants continue to govern
ordering and permalink identity. Bundled IANA TZDB supplies date-specific
historical offsets/abbreviations (ADR-0182). Latest editor zone plus paired Org
DATE is required for all selected displays and round-trip reconciliation.
