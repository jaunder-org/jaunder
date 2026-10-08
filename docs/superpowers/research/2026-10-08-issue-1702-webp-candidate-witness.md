# #1702 WebP candidate witness

**Task-1 research witness only; not a WebP sanitizer or production proof.** It
uses deterministic owned 32×24 RGBA artwork. The independently implemented
Python-standard-library observer checks a bounded container/header envelope, and
Pillow decodes every canvas. Pillow/libwebp is therefore shared between fixture
construction and consumer observation; this is not independent entropy-decoder
proof.

## Exact transparent-RGB construction and corpus

The eight inputs are static and three-frame animated lossy-alpha and
lossless-alpha WebP, each with owned ordinary sRGB ICC v2 and v4. They carry
embedded ICC, descriptive synthetic TIFF Exif (orientation 6, GPS, device,
date/time, artist, copyright), and known synthetic XMP. Animated v2 loops twice;
v4 loops infinitely. Every animation records 70/170/110 ms timing, explicit BGRA
background, and full 32×24 ANMF rectangles with no disposal and no blend. That
is observed full-frame encoder output, not partial-rectangle coverage.

Pinned Pillow 12.3.0/libwebp 1.6.0 direct animated `save_all` with `exact=True`
previously decoded alpha-zero pixels with zero RGB for every canvas of both
lossless animations. This is a measured behavior of that path, not a universal
libwebp claim. Static `lossless=True, exact=True` VP8L frames did preserve a
nonzero RGB component under alpha zero. The fixture maker now encodes every
lossless animation canvas through that static exact path and assembles owned
full-canvas ANMF containers with those VP8L payloads, known RIFF sizes/padding,
ANIM/VP8X controls, and owned metadata. Pillow decodes every source and
candidate canvas and records `transparent_colored`; all values are true in the
passing witness. Thus the property is decoded consumer evidence, not pre-encode
artwork. The lossy animated path remains Pillow's encoder and also measures
true. Two metadata-free RGB controls establish plain `VP8 ` and `VP8L` framing.

## Bounded observer and fail-closed regression contract

The observer checks RIFF declared extent/trailer, chunk lengths/padding, known
chunk membership/order, duplicate ICCP/EXIF/XMP/ANIM/VP8X, VP8X flags/reserved
bytes/canvas, ANIM/ANMF controls and nested boundaries, static ALPH/image
placement, and compressed payload identity. Its header subset admits VP8 key
frames versions 0–3 without scaling and with in-bounds first partition, VP8L
version/reserved bits zero, and documented ALPH fields/reserved bits. It does
**not** parse VP8/VP8L/ALPH entropy or prove resource limits, color transforms,
HDR, unknown chunks, native-platform safety, or production behavior.

Twenty malformed controls reject with exact `WebPContainerError` reasons. A
valid alternate final ANMF changes only the final canvas/payload while retaining
timing, loop, orientation, ICC/XMP and Exif controls. Candidate observation is
also fail-closed for every fixture: no structural/decode error; exact ExifTool
stdout content after surrounding-whitespace normalization and empty stderr; only
original ICCP remaining; EXIF/XMP and their flags removed; all other flags,
compressed payloads, canvas/background/frame geometry,
blend/disposal/timing/loop, and raw canvases unchanged; and orientation removed
with displayed dimensions changing 24×32 to 32×24. A separate report-mutation
control changes 27 promised properties (including metadata, ICC, compressed
static/frame payload, flags, geometry, blend/disposal, background, timing, loop,
raw and oriented results, and diagnostics) and requires each to fail. Five
additional source-admission mutations ensure frame count, loop, timing, and
encoded geometry checks cannot be bypassed by an early return.

## Candidate result and boundaries

Pinned ExifTool 13.59 is invoked separately for every private copy as
`-overwrite_original -all= --icc_profile:all`. Each measured invocation returns
`1 image files updated` with empty stderr. It retains ICCP byte-identically,
removes EXIF/XMP and orientation, and preserves all listed rendering payloads
and controls. That is **not successful sanitization**: retained ICCP is a
privacy gap and removing orientation changes display.

The selected Nix report records Pillow 12.3.0 (`MIT-CMU`) and libwebp 1.6.0
(`BSD-3-Clause`). This is package metadata, not a closure/platform/license or
security acceptance audit. Native libwebp demux remains deferred. No product
code, external images, publishing path, fallback, record, quota, or GUI behavior
was touched.
