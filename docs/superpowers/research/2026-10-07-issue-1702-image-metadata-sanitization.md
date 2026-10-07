# Research: #1702 image metadata sanitization

**Status: primary-source follow-up verified 2026-10-07.** This note records
web-tool verification and source-backed feasibility. It is not fixture or
implementation proof.

## Bottom line

Lossless metadata removal is feasible for JPEG, PNG, GIF, and WebP by rewriting
their format containers while retaining compressed image/frame payloads. It is
not established that one generic command covers all metadata structures.
ExifTool is a useful candidate, not the sanitizer specification.

HEIC/HEIF remains the principal gap. ExifTool documents HEIC/HEIF writing and
`-All=` removal, but that proves supported writable tags, not complete
traversal/removal of every ISO-BMFF item, box, UUID/vendor payload, auxiliary
image, or preview. libheif exposes HEIF metadata and security-limit APIs and
supports auxiliary/depth/thumbnail use cases, but its documented decoder/encoder
workflow is not proof of lossless metadata-only rewriting. Device-native
HEIC/HEIF should be accepted only after a pinned candidate and graph-aware
verifier prove preservation of required rendering and removal of required
descriptive metadata; otherwise reject the covered input.

The settled product constraints are: no lossy re-encoding; preserve source
formats; scrub descriptive ICC fields while preserving color transforms; reject
covered inputs if safety cannot be proved; SVG remains accepted unchanged and
excluded from sanitization. Do not silently drop displayed frames or rendering
auxiliaries. Embedded thumbnails/previews must be removed; other optional camera
editing extras such as depth maps may be discarded. These product choices are
settled; exact safe HEIF graph rewriting remains an outline-time feasibility
question.

## Web-tool verification and limitations

The required tools were registered and usable as follows:

- `web_search`: **passed** with `workflow: "none"`, three bounded queries,
  provider `openai`; response id `muyi3ps1e2gx6z`. It returned official
  ExifTool, W3C PNG, and upstream libheif results.
- `get_search_content`: **passed** for that response and the critical
  source-check response; it returned bounded passages/snippets, including
  ExifTool's HEIC `-all=` and ICC guidance.
- `source_check`: **passed with status unclear/confidence 0.30**, response id
  `muyi43a8e06k7w`; it retrieved passages but explicitly reported that automated
  semantic assessment was unavailable. The passages were manually interpreted
  below.
- `fetch_content`: **failed** for `https://exiftool.org/faq.html#Q32` and
  `https://exiftool.org/exiftool_pod.html` with HTTP 403. Therefore this run
  does **not** claim a direct fetch of ExifTool HTML; ExifTool claims are
  limited to official search/source-check passages. W3C/upstream evidence
  already present in this note remains source-backed from the previous fetch
  record.

No ExifTool or libheif binary was run; no fixture corpus was rewritten; no
independent verifier was built. This is research, not fixture proof.

## Findings

### ExifTool

- [ExifTool application documentation](https://exiftool.org/exiftool_pod.html)
  is returned by the official-source search/source check with:
  `-all= image.heic`; `-All=` deletes all metadata ExifTool can remove;
  `-all:all=` is equivalent. The same passage recommends preserving
  ICC/color-space data for HEIC. **Direct source passage via search artifact;
  not direct HTML fetch.**
- The official [ExifTool FAQ Q32](https://exiftool.org/faq.html#Q32) result says
  JPEG metadata removal can alter color rendition and gives a command retaining
  ICC and restoring color-space tags. **Direct source passage via search
  artifact; not direct HTML fetch.**
- The official
  [ExifTool format documentation](https://exiftool.org/ExifTool.html) result
  lists HEIC/HEIF as writable; the official forum result records HEIC write
  support beginning with v11.33. This establishes write support, not lossless
  completeness.
- ExifTool's default backup behavior is documented in the returned passages as
  creating a `_original` backup unless overridden. A server must keep such
  originals private and never expose them as a public fallback.

**Conclusion:** do not treat `-All=` as “remove every possible descriptive
byte”. Use a version-pinned invocation only behind independent byte/container
verification, reject warnings and unsupported structures, and test the exact
HEIF corpus.

### JPEG

[ITU-T T.81](https://www.w3.org/Graphics/JPEG/itu-t81.pdf) and the
[CIPA Exif specification](https://www.cipa.jp/std/documents/e/DC-008-Translation-2019-E.pdf)
describe marker-segment JPEG and Exif structures. A segment rewriter can retain
entropy-coded scans and progressive structure without re-encoding pixels. It
must nevertheless remove/validate APP1 Exif/XMP, APP13/IPTC/Photoshop, COM, ICC
segments, application segments, and embedded thumbnails according to the
allowlist. Orientation must be normalized or preserved deliberately; color
transforms must be preserved through the approved ICC policy.

### PNG and APNG

The [W3C PNG 3rd Edition](https://www.w3.org/TR/png-3/) identifies `iTXt`,
`tEXt`, `zTXt`, `eXIf`, ICC/profile data, and unknown ancillary chunks.
Ancillary chunks may be ignored by decoders, but that is not a privacy
guarantee. Retain required critical chunks and explicitly approved
color/animation chunks; remove textual, Exif, private, and unapproved unknown
ancillary chunks. For APNG, preserve animation-control/frame data, frame count,
timing, disposal, blend, transparency, and palette semantics. A decode/re-encode
path is not proof of preservation.

### GIF

The [GIF89a specification](https://www.w3.org/Graphics/GIF/spec-gif89a.txt)
defines image descriptors, Graphic Control Extensions, Comment Extensions, and
Application Extensions. Remove comments and unapproved application payloads,
while preserving all frames, palettes, delay, disposal, transparency, and any
permitted loop behavior. Block-level rewriting is preferable to decode/re-encode
when animation preservation is mandatory.

### WebP and animated WebP

Google's
[WebP RIFF container specification](https://developers.google.com/speed/webp/docs/riff_container)
identifies `ICCP`, `EXIF`, `XMP `, `ANIM`, and `ANMF` chunks, plus `VP8X`
feature flags and ordering/size rules. Remove EXIF/XMP and clear corresponding
flags; retain compressed VP8/VP8L/ALPH and animation chunks. Recompute
RIFF/chunk sizes and padding deterministically. Preserve ICC only after the
approved descriptive-field policy and color tests.

### HEIC/HEIF and libheif

The
[libheif v1.23.1 release](https://api.github.com/repos/strukturag/libheif/releases/tags/v1.23.1)
identifies security fixes involving malformed transforms, auxiliary-alpha
dimensions, uncompressed tile slicing, and empty sequences. The tagged
[README](https://raw.githubusercontent.com/strukturag/libheif/v1.23.1/README.md)
describes an ISO/IEC 23008-12 HEIF/AVIF decoder and encoder and lists alpha,
depth maps, thumbnails, auxiliary images, and Exif/XMP reading.

The current upstream
[metadata API header](https://raw.githubusercontent.com/strukturag/libheif/master/libheif/api/libheif/heif_metadata.h)
exposes metadata enumeration, raw data, and add APIs for Exif, XMP, and
proprietary metadata. It does not document a complete remove-all operation or a
complete privacy traversal of every BMFF box/item. The current
[security API header](https://raw.githubusercontent.com/strukturag/libheif/master/libheif/api/libheif/heif_security.h)
exposes limits for pixels, tiles, items, profile size, memory, child boxes,
sequence frames, and brands. The v1.23.1 tagged versions of these separate
header paths returned 404 in the earlier fetch; current headers must not be
presented as tag-pinned evidence.

**Verified feasibility versus inference:** HEIF's documented
auxiliary/depth/thumbnail/sequence model means the sanitizer must reason about
the complete item/reference graph, not merely an Exif block. It does not follow
that every HEIF contains such items. libheif decoding and encoding do not prove
preservation of original HEVC/AV1 payloads, all items, orientation, ICC
transforms, or vendor boxes. The product may reject files whose graph cannot be
safely rewritten, but it must not assume rejection is the only solution until a
lossless candidate is tested. Do not discard displayed rendering auxiliaries
without an explicit product decision.

### ICC, appearance, and descriptive data

The
[ICC v4 specification](https://www.color.org/specification/ICC1v43_2010-12.pdf)
defines descriptive fields including profile description, manufacturer, model,
and copyright. Retaining a profile can preserve color transforms while retaining
descriptive/device strings. The settled requirement is to scrub descriptive ICC
fields while preserving transforms, but this requires a validated profile
rewrite and color-regression tests; dropping ICC is not equivalent and may alter
appearance. “All metadata removed” must not be claimed if arbitrary retained ICC
bytes remain.

### Rust/native alternatives

[`image`](https://docs.rs/image/latest/image/) is primarily a decoder/encoder
and is not evidence of complete metadata-preserving container rewriting.
[`kamadak-exif`](https://docs.rs/kamadak-exif/latest/kamadak_exif/) reads Exif;
it is not a HEIF sanitizer. [`rexiv2`](https://docs.rs/rexiv2/latest/rexiv2/)
binds native gexiv2/Exiv2; it adds native dependency, ABI, update, and license
review. [Exiv2](https://github.com/Exiv2/exiv2),
[libheif](https://github.com/strukturag/libheif), and ExifTool each require
exact runtime/license/patent review. No alternative was verified here as
satisfying the complete HEIF requirement.

## Required operational contract

1. Detect by magic bytes and strict parser validation, not filename or declared
   MIME.
2. Use per-format allowlists and retain compressed source payloads; never use
   general decode/re-encode for the no-loss requirement.
3. Bound upload bytes, dimensions/decoded pixels, frame count, chunk/box lengths
   and counts, nesting, memory, CPU, and wall time. Use libheif limits as one
   layer, plus a restricted subprocess/sandbox for native tooling.
4. Fail closed on warnings, unsupported codecs/items/brands, unknown structures
   the verifier cannot account for, limit breaches, write errors, or post-write
   mismatch.
5. Verify removal of GPS, device, timestamps, author, comments, and embedded
   thumbnails in covered structures, including retained ICC/Exif/XMP and every
   retained image/auxiliary reference. This does not erase visible PII or
   arbitrary steganography.
6. Test deterministic output, idempotence, orientation/color rendering,
   animation timing/disposal/transparency, and the server's new SHA URL/ETag
   behavior. Preserve author-local originals; server working input is private
   and temporary, not a retained unsanitized original. Remediation creates new
   identities rather than overwriting existing bytes/identities.

## Outline-time feasibility and proof questions

- Which HEIF brands/codecs/box types are covered, and what unknown/vendor-box
  policy applies?
- How does the chosen HEIF path remove thumbnails/previews and optional editing
  extras while preserving required alpha/HDR/color and all displayed frames?
  Unsupported unsafe variants reject; blanket rejection is not HEIF delivery.
- Which profile validation and color-regression proofs establish that
  descriptive ICC fields are scrubbed while their color transforms remain
  intact?
- What exact byte, pixel, frame, metadata, memory, and time limits and sandbox
  are mandatory?
- What pinned ExifTool/libheif versions and transitive license/patent/codec
  policy are acceptable? See
  [ExifTool license](https://exiftool.org/license.html),
  [libheif COPYING](https://raw.githubusercontent.com/strukturag/libheif/v1.23.1/COPYING),
  and [Exiv2 LICENSE](https://github.com/Exiv2/exiv2/blob/main/LICENSE).

## Sources and evidence limits

Primary sources are linked inline. Search response ids used for this follow-up:
`muyi3ps1e2gx6z` (`web_search`) and `muyi43a8e06k7w` (`source_check`). ExifTool
direct fetching was blocked by HTTP 403, so those claims use official
search/source-check passages and are labeled accordingly. No implementation,
dependency, tracker, Pi configuration, fixture, or corpus changes were made;
only this research note was updated.
