# ADR-0084: Media filename — the encoded form is canonical, with an extractor-private decoded-segment door

- Status: accepted
- Date: 2026-07-30
- Issue: [#720](https://github.com/jaunder-org/jaunder/issues/720)

## Context

ADR-0080 established one media path layout, percent-encoded, identical on disk
and in URLs — and deliberately kept the **database `filename` column raw**, so
the media library could show the name the user typed. That left exactly one
derivation, `DB → disk`, and it is the derivation in the dangerous direction: a
missed or mis-encoded transform yields a wrong _path_ — a 404, or with a
filename collision, the wrong file.

Three later pieces of work all pay for that arrangement. The motivating one is
the post→media reference table (#711): it extracts references from rendered
HTML, which carries the **encoded** name, and must match them against media
records. With a raw column that comparison needs a transform — and a transform
at a comparison point is precisely the bug class #675, #708 and #711 have each
been an instance of. Orphan reclamation (#721) then asks "is this on-disk entry
referenced?", and an on-disk entry is named by the encoded form.

The decisive observation is not a transform count — both layouts have
derivations. It is **which side of the system is allowed to be intricate**.

Producing a stored filename is genuinely intricate: `sanitize_filename`
normalizes backslashes, strips path components and maps NUL to `_`;
`truncate_to_budget` then splits stem from extension so a dotfile survives,
reserves room for the extension _and_ a minimal stem so a single grapheme
cluster cannot truncate a name to bare `.jpg`, walks grapheme clusters, and
substitutes `TRUNCATED_STEM` when nothing usable remains. Every step is a lossy
policy choice. Decoding, by contrast, is
`percent_decode_str(…).decode_utf8_lossy()` — total, with no choices.

And `truncate_to_budget` **already computes the encoded form**: `encoded_len` is
the metric its budget is measured against. Today the intricate door computes the
encoded cost, discards the encoded value, stores the raw name, and every later
derivation recomputes the encoding.

## Decision

**The percent-encoded form is canonical.** It is what `media.filename` holds,
what the file is named on disk, and what the URL segment contains — byte for
byte. `media_path`, `media_url`, and the AtomPub media member URL stop encoding
and simply interpolate. Decoding happens only for display.

**One type, distinguished on the provenance axis.** `Filename` remains a single
type, now meaning "the canonical, percent-encoded safe leaf". Its ADR-0063
`StrNewtype` trailer exposes the encoded bytes with no deviation from the
derive, so every load-bearing consumer — path construction, URL construction,
the sqlx bind, #711's comparison — is correct by default. Display takes the
single explicit opt-out, `Filename::decoded() -> Cow<'_, str>`.

We rejected splitting into `Filename`(display) + `EncodedFilename`(canonical).
That places the type distinction on the _presentation_ axis, where a mistake is
cosmetic, rather than the provenance axis, where it is not.

**`Filename::from_str` enforces canonicity and keeps the safe-leaf guard.**
`StrNewtype` routes both serde and sqlx through `FromStr`, making it the door
for every database read and every wire value. It now accepts `s` only when
`s == encode(decode(s))`, in addition to the existing non-empty / `.` / `..`
checks (both survive encoding, since `.` is unreserved) and a length bound that
is now a **plain byte count**. Without the canonicity check a raw name arriving
from a hand-edited row, a restored backup, or a `#[server]` wire argument would
be accepted, and the write path would create a file under a name no URL can ever
produce — #721's orphan class arriving by a new route.

Critically, the safe-leaf oracle survives, **relocated to the decoded form**:
`sanitize_filename(decode(s)) == decode(s)`. Canonicity does not imply a safe
leaf — `a%2Fb.jpg`, `a%00b.jpg` and `a%0D%0Ab.jpg` are all canonical, short, and
neither `.` nor `..`, yet decode to `a/b.jpg`, `a\0b.jpg` and `a\r\nb.jpg`.
Dropping the oracle would weaken precisely the path-traversal / header-injection
guard that justifies this type at all (ADR-0063 §1), and a NUL recovered by the
display decode is not a legal XML character, so it would let an Atom feed be
emitted that no parser accepts. Running the oracle on the _encoded_ form does
not work: `sanitize_filename("a%2Fb.jpg")` is `"a%2Fb.jpg"`, so it passes
vacuously.

**Decoded route segments use an extractor-private door into `Filename`.** axum
percent-_decodes_ path parameters before a handler runs, and offers no
un-decoded extractor (`RawPathParams` is "raw" only as in undeserialized; its
values are `PercentDecodedStr`). Three routes therefore receive a decoded name:
the media serve route, and the AtomPub media member `GET` and `DELETE`. They
cannot share `Filename`'s `FromStr`, because encoding is **not idempotent** —
`encode("my%20photo.jpg")` is `"my%2520photo.jpg"` — so the decoded route
segment needs a distinct door from the already-encoded canonical input.

That door is common-owned but does **not** expose a public decoded filename
type. `Filename::from_decoded_segment` applies the same safe-leaf oracle to the
decoded segment Axum has already produced, performs the dumb encode exactly
once, rejects encoded values over the filesystem component budget, and returns
canonical `Filename`. The conversion remains fallible at the door and leaves no
decoded representation behind; after extraction, handler, domain, storage, DTO,
and web surfaces can hold only `Filename` or a validated address struct
containing it.

Keeping the oracle at this door also means the existing miss behaviour is
**unchanged** — a member-route segment like `a%5Cb.png`, which decodes to
`a\b.png`, is still rejected pre-handler as a 400 rather than quietly becoming a
404 lookup miss.

It **checks but never repairs**: it keeps the cheap non-empty / `.` / `..` /
length rejections and never truncates. An encoded name over 255 bytes cannot
exist on disk — that is the filesystem's per-component limit — so rejecting it
is a provably correct early miss. Truncation is a lossy _repair_, and repair
only makes sense at intake, where the alternative is losing the upload. This
reuses ADR-0080's existing reject-vs-truncate split between the two doors rather
than inventing a new one.

Any server-private wrapper/intermediate around the decoded segment deliberately
takes a minimal surface — no `Display`, `Serialize`, `Deref`, `AsRef`, sqlx
bridge, or standard `StrNewtype` trailer. A public second filename type would be
a second _storable_ spelling of a filename; `Display`/`Serialize` would invite a
broken round-trip because the door stores a value different from its input; and
an ergonomic string surface would make the one-hop representation usable far
beyond extraction. Privacy makes the double-encode hazard structurally
impossible rather than merely gated.

A dumb re-encode can miss but can never resolve to a _different_ file, because
percent-encoding under a fixed set is injective.

**Intake order stays `sanitize → truncate → encode`.** Truncating in encoded
space would mean never splitting a `%XX` escape, never splitting the escape run
of one multi-byte character (`ä` is `%C3%A4`; a cut after `%C3` decodes to
invalid UTF-8), and still never splitting a grapheme cluster — strictly harder,
for no gain.

## Consequences

- **Amends [ADR-0080](0080-media-path-naming-correspondence.md)** on two points.
  Its "the database `filename` column keeps the raw name" decision is reversed —
  that is this ADR's central subject. The rest of ADR-0080 stands unchanged: one
  layout, `media_path` as its single definition, and the encode set as
  `NON_ALPHANUMERIC` minus the unreserved marks `-._~`.

- **ADR-0080's encode-set coupling note is narrowed, not removed.** Because
  intake still truncates in raw space measured by `encoded_len`, widening
  `MEDIA_SEGMENT_ENCODE_SET` still grows a raw name's encoded cost, so fewer
  typed names survive intake intact. What changes is _where_ the coupling lives:
  it is a property of `Filename::sanitized`'s budget, no longer a property of
  `Filename`'s invariant. Strict canonicity does put a new encode-set reference
  into `FromStr`, but it is a better dependency — about _which spelling is
  canonical_, not _how much fits_.

- **Amends [ADR-0063](0063-domain-value-newtype-convention.md):** the filename
  case is the first non-secret inbound representation, but it is not a public
  `Proffered*` twin. The lesson is narrower: when an external protocol presents
  a different representation of a domain value, `common` owns the validating
  conversion into the domain type, while any decoded intermediate stays private
  to the extractor seam. The inbound-secret `Proffered*` profile and
  `proffered_secret_check` remain unchanged.

- **`encode_filename_segment` is deleted** and `MEDIA_SEGMENT_ENCODE_SET`
  returns to a private const with no public escape hatch. "Only
  `common::media`'s doors encode" therefore becomes a compile fact via module
  privacy, requiring no gate.

- **The old `proffered-filename-position` static check is deleted.** A decoded
  filename intermediate is no longer a public `common` type, so `xtask` has no
  cross-crate type-position surface to police. The boundary is Rust privacy plus
  the public `Filename::from_decoded_segment` door: `common::media` exposes
  canonical `Filename`, and server extractors retain only `Filename` past
  extraction.

**Media serve extractor amended on 2026-08-15 and 2026-08-24.** The strict
media-address decision in `docs/adr/0140-strict-media-address-extraction.md`
supersedes #504's media-specific `SoftPath<ProfferedFilename>` policy and is
later refined by #1149's extractor-private filename seam. The public media route
establishes source, hash, both hash prefixes, and canonical `Filename` during
strict Axum extraction; the AtomPub strict extractors and every
canonical-filename decision above are unchanged.

- **Sites that read _inside_ a name take the decoded form.**
  `detect_content_type` is given `.decoded()` at both callers: sniffing the
  encoded form happens to work only because `.` is unreserved and every
  extension in the table is ASCII alphanumeric, which is a coincidence of the
  encode set and exactly the silent dependency this ADR is trying to stop
  relying on. `MediaLinkEntry.title` stays typed `Filename` and the Atom
  renderer decodes at the single render site.

- **`UploadedMedia.filename` stays canonical.** It is a lookup key, passed to
  `get_media` by the AtomPub collection handler, not a display value.

- **Legibility regresses slightly.** Backup NDJSON and direct database
  inspection show `my%20photo.jpg`. This is the affordance ADR-0080 optimised
  for, and it is the real price of this decision.

- **A pre-#720 backup restores "successfully" and fails later at read time.**
  Restore is untyped and per-table generic — it reads rows as
  `serde_json::Map<String, Value>` and binds each cell as text, never
  constructing a `Filename` — so raw `filename` rows are written unchallenged
  and the canonicity check first bites as a sqlx `Decode` error on read: a 500
  from the media library, or a silently skipped row via `list_media`'s
  skip-the-malformed-row path. This is deliberately recorded as **not** loud at
  the restore boundary, because the tempting assumption is the opposite. It is
  theoretical — this is adopted with no legacy media data, as ADR-0080 was — and
  making restore validate typed columns is left as a follow-up rather than
  folded in here.

- **Unblocks #711**, whose HTML-reference-to-record comparison becomes byte
  equality, and through it #721.
