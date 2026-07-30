# Spec — #720: the percent-encoded filename is canonical in the database

- Issue: [#720](https://github.com/jaunder-org/jaunder/issues/720)
- Date: 2026-07-30
- Status: approved-pending
- Blocks: [#711](https://github.com/jaunder-org/jaunder/issues/711) →
  [#721](https://github.com/jaunder-org/jaunder/issues/721)

## Problem

ADR-0080 made the percent-encoded filename canonical on disk and in URLs while
leaving `media.filename` **raw**, so `DB → disk` is a derivation in the
dangerous direction. #711 must compare references extracted from rendered HTML
(which carry the **encoded** name) against media records; with a raw column that
comparison needs a transform, and a transform at a comparison point is the bug
class #675/#708/#711 have each been an instance of.

The governing principle for this change:

> **Producing a stored filename is allowed to be intricate. Everything else must
> be dumb.**

Intake is genuinely intricate — `sanitize_filename` (backslash normalization,
component stripping, NUL→`_`) then `truncate_to_budget` (stem/extension split so
a dotfile survives, reserving room for the extension _and_ a minimal stem,
grapheme-cluster-safe truncation, `TRUNCATED_STEM` substitution). Every step is
a lossy policy choice. Decoding is `percent_decode_str(…).decode_utf8_lossy()` —
total, no choices.

And `truncate_to_budget` **already computes the encoded form**: `encoded_len` is
its budget metric. Today the intricate door computes the encoded cost, discards
the encoded value, stores the raw name, and every later derivation recomputes
the encoding. This change stores what the intricate step already produced.

## Decisions

### D1 — `media.filename` holds the encoded form

The DB value, the on-disk name, and the URL segment are byte-identical.
`media_path`/`media_url`/the AtomPub member URL stop encoding and interpolate.

### D2 — one canonical type; the distinction is on the _provenance_ axis

`Filename` remains a single type holding the **canonical, percent-encoded safe
leaf**. Its `StrNewtype` trailer (`Display`, `Deref<str>`, `AsRef<str>`,
`PartialEq<str>`, serde, sqlx) exposes the **encoded bytes** with no deviation
from the derive — so `media_path`, the URL builders, the DB bind, and #711's
comparison are all correct by default.

Display is the cosmetic side and gets the single explicit opt-out:
`Filename::decoded(&self) -> Cow<'_, str>`. `Cow` because the decode is free
when there is nothing to decode; **lossy** decoding cannot actually substitute
on a canonical value, because a lone `%FF` fails the D3 canonicity check.

Rejected: `Filename`(display) + `EncodedFilename`(canonical). That puts the type
distinction on the _presentation_ axis, where a mistake is cosmetic, instead of
the provenance axis, where it is not.

### D3 — `Filename::from_str` enforces canonicity **and** keeps the safe-leaf guard

`StrNewtype` routes both serde `Deserialize` and sqlx `Decode` through `FromStr`
(`macros/src/str_newtype.rs:188-193`, `:313-317`), so it is the door for every
DB read and every wire value. It accepts `s` iff:

1. `s == encode(decode(s))` — canonical percent-encoding under
   `MEDIA_SEGMENT_ENCODE_SET`;
2. **`sanitize_filename(decode(s)) == decode(s)`** — the _decoded_ value is a
   safe leaf;
3. `s` is non-empty and is neither `.` nor `..` (both survive encoding, since
   `.` is unreserved, so this check must remain);
4. `s.len() <= MAX_FILENAME_ENCODED_BYTES` — now a **plain byte length**, with
   no encode-set reference.

**(2) is load-bearing and easy to lose.** Canonicity does _not_ imply a safe
leaf: `a%2Fb.jpg`, `a%00b.jpg` and `a%0D%0Ab.jpg` are all canonical, non-empty,
short, and neither `.` nor `..`, yet decode to `a/b.jpg`, `a\0b.jpg` and
`a\r\nb.jpg`. Today's `sanitize_filename(s) == s` oracle
(`common/src/media.rs:204`) rejects all three; dropping it would weaken exactly
the path-traversal / header-injection guard that justifies the type
(`common/src/media.rs:129-134`, ADR-0063 §1). A NUL recovered by `.decoded()` is
not a legal XML character, so it would make `render_media_link_entry` emit an
unparseable feed.

The check must run on `decode(s)`, **not** on `s`:
`sanitize_filename("a%2Fb.jpg")` is `"a%2Fb.jpg"`, so testing the encoded form
passes vacuously.

**Check order is load-bearing:** empty/`.`/`..` → decoded-safe-leaf → canonicity
→ length. A separator value like `a/b.txt` is _both_ an unsafe leaf and
non-canonical, and the existing test `from_str_still_reports_a_bad_leaf_as_such`
(`common/src/media.rs:1197`) pins that it reports the leaf failure. Running
canonicity first would silently reclassify it.

`InvalidFilename` gains a third variant, **`NotCanonical`**, for (1). Reusing
`NotASafeLeaf` would report "filename must be a non-empty safe path leaf" for a
value that _is_ a safe leaf and is merely unencoded — the most likely real-world
case being a raw name on the wire, where that message misdirects. The existing
test `from_str_still_reports_a_bad_leaf_as_such` (`common/src/media.rs:1197`)
already pins that the failure modes stay distinguishable; this extends that
property rather than departing from it.

Without (1) a raw `my photo.jpg` from a hand-edited row, a restored backup, or a
`#[server]` wire argument would be accepted, the write path would create a file
under a name no URL can produce, and that is #721's orphan class arriving by a
new route.

### D4 — `ProfferedFilename` guards the three inbound URL doors

axum percent-**decodes** path parameters before a handler runs, and there is no
un-decoded extractor (`RawPathParams` is "raw" only as in _undeserialized_ — its
values are `PercentDecodedStr`). So these three doors hold a decoded name — and
they are the complete set of routes carrying a filename path segment
(`server/src/media.rs:34`, `server/src/atompub/mod.rs:41`):

| Route                                               | Extractor                          | Miss behaviour |
| --------------------------------------------------- | ---------------------------------- | -------------- |
| `GET /media/{source}/{p1}/{p2}/{hash}/{filename}`   | `SoftPath<_>`                      | soft → 404     |
| `GET /atompub/{username}/media/{sha}/{filename}`    | `Path<(Username, ContentHash, _)>` | strict → 400   |
| `DELETE /atompub/{username}/media/{sha}/{filename}` | `Path<(Username, ContentHash, _)>` | strict → 400   |

They cannot share `Filename`'s `FromStr`: encoding is **not idempotent**
(`encode("my%20photo.jpg") == "my%2520photo.jpg"`), so one door cannot serve
both a decoded and an already-encoded input. Because these extractors select
their door _purely by the type parameter_, a second type is the only mechanism
making the choice compiler-checked.

`ProfferedFilename::from_str(input)` receives the already-decoded segment and:

1. rejects unless `sanitize_filename(input) == input` — the same safe-leaf guard
   as D3(2), applied to the decoded form it already holds;
2. rejects empty / `.` / `..`;
3. encodes: the stored value is `encode(input)`;
4. rejects if the encoded value exceeds `MAX_FILENAME_ENCODED_BYTES`.

So the type holds encoded bytes and `From<ProfferedFilename> for Filename` is a
**rewrap with no logic**. The conversion is **infallible**, and stays infallible
_because_ (1) is present: `decode(encode(input)) == input`, so D3(2) is
satisfied by construction, as are D3(1), (3) and (4).

It **checks but never repairs** — it never truncates. An encoded name over 255
bytes cannot exist on disk (the filesystem's per-component limit), so rejecting
is a provably correct early miss. Truncation is a lossy repair that only makes
sense at intake, where the alternative is losing the upload — this reuses
ADR-0080's existing reject-vs-truncate split rather than inventing one.

**Existing miss behaviour is preserved, not changed.**
`server/tests/atompub/atompub_media.rs:293` pins that `a%5Cb.png` (decoding to
`a\b.png`) is a 400. Under (1), `sanitize_filename("a\b.png")` is `"b.png"` ≠
input, so it is still rejected pre-handler and still 400s. That test needs no
change. Had (1) been omitted, this would have silently become a 404.

**Safety.** A dumb re-encode can miss but can never resolve to a _different_
file, because percent-encoding under a fixed set is **injective**. The
pre-existing aliasing (`/media/…/100%.jpg` and `/media/…/100%25.jpg` both
resolving to stored `100%25.jpg`, because axum's decode is not injective) is
unchanged by this issue.

### D4a — `ProfferedFilename` carries a deliberately minimal trailer

It gets **`FromStr`, `Deserialize`, `Clone` and `Debug` only** — no `Display`,
`Serialize`, `Deref`, `AsRef`, or sqlx bridge. (`Clone` is load-bearing, not
ergonomic: `SoftPath::value()` returns `Option<&T>`, so the serve route needs an
owned value to hand to `From<ProfferedFilename> for Filename`. `Debug` is
required by the extractor error paths.) This is a deliberate ADR-0063 deviation,
justified because the type exists for exactly one hop (extractor → rewrap into
`Filename`) and every other member of the standard trailer is a hazard here:

- A bare `#[derive(StrNewtype)]` takes `Kind::Default` in
  `macros/src/str_newtype.rs`, which emits the **sqlx** `Type`/`Encode`/`Decode`
  bridge — making `ProfferedFilename` bindable as a DB column, i.e. a second
  _storable_ spelling of a filename. The D8 gate could not catch that, since it
  scans type positions, not query binds. Use `#[str_newtype(no_sqlx)]` if the
  derive is used at all.
- `FromStr` stores a value **different from its input**, so
  `Display`/`Serialize` would not round-trip through `FromStr`/`Deserialize`.
  Every other `StrNewtype` in this tree round-trips; omitting the rendering half
  means there is no broken round-trip to document, and no way to re-encode an
  already-encoded value.

This makes the double-encode hazard structurally impossible rather than merely
gated; D8 then covers only the residual leak (a `Deserialize`-only type used as
a DTO field).

### D5 — intake order stays `sanitize → truncate → encode`

Truncating in _encoded_ space would mean never splitting a `%XX` escape, never
splitting the escape run of one multi-byte character (`ä` is `%C3%A4`; cutting
after `%C3` decodes to invalid UTF-8), and still never splitting a grapheme
cluster — strictly harder, for no gain. `truncate_to_budget` keeps walking raw
graphemes with `encoded_len` as its cost function.

The two bounds agree exactly: `truncate_to_budget` bounds
`encoded_len(raw) <= 255`, and the encoded output's `s.len()` **is**
`encoded_len(raw)`, so D3(4) and D5 are the same number.

Therefore the encode-set coupling **survives**, relocated: widening
`MEDIA_SEGMENT_ENCODE_SET` still grows a raw name's encoded cost, so fewer typed
names survive intake intact. It becomes a property of `Filename::sanitized`'s
budget, not of `Filename`'s invariant. ADR-0080's coupling note is **narrowed**,
not deleted.

### D6 — every site that reads or shows the name, enumerated

This is the complete list; each names the form it takes.

| Site                                              | Form        | Note                       |
| ------------------------------------------------- | ----------- | -------------------------- |
| `media_path` / `media_url` / AtomPub member URL   | encoded     | interpolate, no encoding   |
| sqlx bind + `get_media`/`find_by_hash` lookups    | encoded     | byte equality              |
| `MediaRecord.filename`, `UploadResponse.filename` | encoded     | D7                         |
| `MediaItem.filename` (wire DTO)                   | encoded     | client decodes at render   |
| `component.rs:300` link text                      | **decoded** | `.decoded()`               |
| `component.rs:310` hidden `filename` field        | **encoded** | the delete key — see below |
| `content_disposition` argument                    | **decoded** | see below                  |
| `MediaLinkEntry.title` render site                | **decoded** | field stays `Filename`     |
| `detect_content_type`                             | **decoded** | see below                  |

**`component.rs:289` must split into two bindings.** Today one `String` feeds
both the link text (line 300) and the hidden delete-form field (line 310). After
this change they diverge: the link text decodes, and the hidden field must stay
canonical because it round-trips to `delete_media(filename: Filename)`
(`web/src/media/api.rs:133`), whose wire door is D3. D2's "display is the
cosmetic side" framing makes the hidden field _look_ like a display site; it is
not. Getting it wrong breaks deletion for every name needing encoding — loudly,
since D3 rejects the decoded value rather than deleting nothing.

**`content_disposition` takes the decoded name.** It already percent-encodes
internally with the bare `NON_ALPHANUMERIC` set for the RFC 5987 `filename*=`
parameter (`server/src/media.rs:288`). Handing it the already-encoded form would
silently double-encode into a header that still _looks_ well-formed — precisely
the failure this issue exists to eliminate. The two encode sets are deliberately
different and both correct in place (ADR-0080).

**`detect_content_type` takes the decoded name.** Sniffing the encoded form
happens to work only because `.` is unreserved and every extension in the table
is ASCII alphanumeric — a coincidence of the encode set, and exactly the silent
dependency ADR-0080 warns about. Its caller
`MediaManager::get_content_type(content_type: Option<&str>, filename: &str)`
(`media_manager.rs:152`) keeps its `&str` signature — it is `pub` and
unit-tested with string literals at `:474-486` — so the change lands at the two
callers that hold a `&Filename`: `media_manager.rs:115` (`upload`) and `:363`
(`upload_bytes`), plus the serve fallback at `server/src/media.rs:132`.

### D7 — `UploadResponse.filename` stays canonical

It is a **lookup key**, not a display value: `atompub/media.rs:116` passes it to
`get_media`. Renderings of it decode; the field does not. (The issue text's
"display sites decode … `UploadResponse.filename`" is imprecise; this supersedes
it.)

Two assertions invert as a consequence, and both are correct new truths rather
than regressions: `server/tests/web/web_media.rs:325` and
`end2end/tests/media.spec.ts:58-59` (whose comment "The display name stays raw"
also becomes false and must be rewritten).

The elisp AtomPub client is **unaffected**: `elisp/jaunder-media.el:72` harvests
`content-src` from the entry, not `filename`.

### D8 — enforcement gate

`encode_filename_segment` loses its callers and is **deleted**, returning
`MEDIA_SEGMENT_ENCODE_SET` to a private const with no public escape hatch — so
"only `common::media`'s doors encode" becomes a compile fact via module privacy,
needing no gate.

`ProfferedFilename` must be `pub` for the `server` crate's route signatures, so
privacy cannot contain it. A new `xtask` gate, modelled on
`proffered_secret_check.rs`, confines it. **The discriminator is
bare-versus-wrapped, not field-versus-not:** the serve route's legitimate
extractor position _is_ a struct field (`server/src/media.rs:61-68`,
`#[derive(Deserialize)] pub struct ServeParams { … filename: SoftPath<Filename> }`),
so a "no DTO fields" rule would be undecidable.

- **Permitted:** the type appearing wrapped as `SoftPath<ProfferedFilename>` or
  inside a `Path<(…)>` tuple; and its own defining file.
- **Rejected:** a _bare_ `ProfferedFilename` as a struct field, a `#[server]`
  parameter, any return type, or a plain fn parameter.

Naming: `Proffered` currently carries a _secrecy_ meaning (ADR-0063;
`proffered_secret_check` pins `ProfferedInviteCode`/`ProfferedPassword` to
`#[server]` params via a two-name `POLICED_TYPES` list, so a third `Proffered*`
does not trip it). ADR-0063 is amended: `Proffered` = untrusted inbound twin, of
which the secret profile is one specialization carrying an extra gate.

### D9 — decisions recorded as an ADR

One new numberless draft under `docs/adr/drafts/`, which in its Consequences
**amends** ADR-0080 (D1, and the D5 narrowing) and **amends** ADR-0063 (D4a
deviation, D8 naming). This follows ADR-0068's precedent for amending ADR-0063
in a new ADR's Consequences. ADR-0080's coupling note is edited in place to the
narrowed form.

## Acceptance criteria

Each is stated so conformance can be checked from the delivered tree.

**AC1 — the column is the path segment.** For an uploaded `my photo.jpg`: the
`media.filename` value, the on-disk leaf under the storage root, and the tail
segment of `media_url` are all exactly `my%20photo.jpg`. One test asserts all
three against one upload.

**AC2 — encoding is confined by privacy.** `media_path` contains no call to
`utf8_percent_encode`; `encode_filename_segment` no longer exists in the tree;
`MEDIA_SEGMENT_ENCODE_SET` is a private const and `common/src/media.rs` is the
**only** file naming it. (Not "no `utf8_percent_encode` anywhere" —
`server/src/media.rs:269` legitimately uses the bare `NON_ALPHANUMERIC` set for
the RFC 5987 header.)

**AC3 — `Filename::from_str` rejects non-canonical values.** `"my photo.jpg"`
(raw) and `"my%2fphoto.jpg"` (lowercase escape) are `Err`; `"my%20photo.jpg"` is
`Ok`. Tested.

**AC4 — `Filename::from_str` rejects canonical-but-unsafe values.**
`"a%2Fb.jpg"`, `"a%00b.jpg"` and `"a%0D%0Ab.jpg"` are each `Err`, tested
individually. These pass canonicity and the `.`/`..`/length checks, so they fail
only if D3(2) is present — this is the test that would catch its omission.

**AC5 — the length bound is a plain byte check.** `Filename`'s `FromStr` length
test is `s.len() <= MAX_FILENAME_ENCODED_BYTES` with no `encoded_len` or
encode-set reference. `Filename::sanitized` still uses `encoded_len` for its
budget.

**AC6 — `ProfferedFilename` never repairs, and converts infallibly.**
`From<ProfferedFilename> for Filename` is a total `From`, not `TryFrom`. The
discriminating truncation test: store a record whose name sits exactly at the
budget, then request a **longer** segment whose truncation would land on that
stored name, and assert the miss. (Asserting merely that "an over-long segment
does not resolve" passes whether or not truncation was removed.)

**AC7 — `ProfferedFilename` cannot be rendered or stored.** It implements
neither `Display` nor `Serialize` nor sqlx `Type`/`Encode`/`Decode` — asserted
as a compile-fail doctest for the rendering half, the way `Filename`'s
private-field compile-fail tests are written.

**AC8 — each inbound door resolves an encoding-needing name.** Three tests, one
per route in D4: uploading `my photo.jpg`, then requesting the URL `media_url`
produced, serves the file (200 for the serve route; entry retrieval and deletion
for the two AtomPub routes).

**AC9 — miss behaviour is unchanged.**
`server/tests/atompub/atompub_media.rs:293` (`a%5Cb.png` → 400) still passes
**unmodified**. A well-formed segment matching no record 404s on the serve route
rather than 500ing.

**AC10 — the media row's two bindings carry different forms.** For a stored
`my%20photo.jpg`: the row's link text is exactly `my photo.jpg`, and the hidden
`filename` input's value is exactly `my%20photo.jpg`.

**AC11 — the delete round-trip survives.** Deleting the `my photo.jpg` item
through the media library's form succeeds — the end-to-end check that AC10's
hidden field was not decoded.

**AC12 — `Content-Disposition` carries both forms correctly.** For a stored
`my%20photo.jpg`, the header's `filename=` parameter is exactly `"my photo.jpg"`
and its `filename*=UTF-8''` parameter is exactly `my%20photo.jpg`. (Stated as
two exact strings because `filename*=` is _supposed_ to be percent-encoded, so
"shows the raw name" is false for half the header and a double-encoded
`my%2520photo.jpg` would pass a looser check.)

**AC13 — the Atom title decodes.** A rendered `MediaLinkEntry` for
`my%20photo.jpg` contains `<title>my photo.jpg</title>`.

**AC14 — literal `%` round-trips.** Uploading `50%.jpg` stores `50%25.jpg`,
serves at `…/50%25.jpg`, and displays as `50%.jpg`. Uploading `a%2Fb.jpg` stores
`a%252Fb.jpg`, and no path derived from it contains a `/` in the filename
segment. Both tested — these expose a double-encode or double-decode.

**AC15 — `UploadResponse.filename` is canonical on the wire.**
`server/tests/web/web_media.rs:325` and `end2end/tests/media.spec.ts:59` both
assert `my%20holiday%20photo.jpg` / `my photo.jpg`'s encoded form, and the e2e
comment "The display name stays raw" is rewritten to state the new truth.

**AC16 — the gate bites.** The new `xtask` step fails on fixtures placing a
**bare** `ProfferedFilename` in a struct field, a `#[server]` parameter, a
return type, and a plain fn parameter; and passes on fixtures using
`SoftPath<ProfferedFilename>` and
`Path<(Username, ContentHash, ProfferedFilename)>` — asserted the way
`proffered_secret_check`'s own tests assert theirs.

**AC17 — `detect_content_type` receives decoded names.** All three call sites
(`media_manager.rs:115`, `:363`, `server/src/media.rs:132`) pass `.decoded()`;
`get_content_type` keeps its `&str` signature and its literal-driven unit tests.

**AC18 — ADR-0080 is amended, not contradicted.** Its text no longer claims
`Filename`'s _invariant_ depends on the encode set; it states the dependency as
a property of `Filename::sanitized`'s intake budget, and its "the database
`filename` column keeps the raw name" bullet points at the new ADR.

**AC19 — the prose that asserts the reversed fact is updated.**
`common/src/media.rs:21-22` ("The database `filename` column keeps the **raw**
name") and its three-spellings paragraph at `:28-29` state the new arrangement;
`server/src/media.rs:243-248`'s "the serve route's re-encode is not redundant …
`media_path` re-encodes it" comment is rewritten to describe
`ProfferedFilename`'s job. These comments are the next reader's map, so a stale
one is a defect.

## Accepted consequences

- **Legibility.** Backup NDJSON and direct DB inspection show `my%20photo.jpg`.
  This is what ADR-0080 optimised for, so it is a real regression, just a small
  one.
- **The read path gains a dumb transform** at three doors, whose only failure
  mode is a miss (D4, injectivity).
- **A pre-#720 backup restores "successfully" and fails later at read time.**
  Restore is untyped and per-table generic — `storage/src/backup.rs:314` returns
  `Vec<serde_json::Map<String, Value>>` and `:345` binds each cell as text,
  never constructing a `Filename`. So raw `filename` rows are written, and the
  failure surfaces as a sqlx `Decode` error on read (the mechanism pinned by
  `storage/src/media.rs:425`) — a 500 from the media library, or a silently
  skipped row via `list_media`'s skip path. This is **not** loud at the restore
  boundary. It is theoretical, since there is no legacy media data, and making
  restore loud is explicitly out of scope.
- **`Filename`'s semantics change**, so its doc, its doors, and the tests
  pinning them all move. This is the bulk of the work.
- **No migration.** There is no legacy media data.

## Out of scope

- The post→media reference table (#711) and orphan reclamation (#721) — both
  land after this, against the encoded column.
- Making backup **restore** validate typed columns, so a pre-#720 backup fails
  loudly at the restore boundary rather than at first read. Surfaced by this
  spec; file as a follow-up.
