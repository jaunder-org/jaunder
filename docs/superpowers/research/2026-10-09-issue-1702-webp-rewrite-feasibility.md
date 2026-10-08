# #1702 WebP bounded rewrite feasibility

**Task-1 fixture evidence only; not a production sanitizer or upload admission
path.** The executable witness adds a byte-copying RIFF rewrite over the owned
eight-fixture WebP corpus described in the
[candidate witness](2026-10-08-issue-1702-webp-candidate-witness.md). It does
not decode or re-encode pixels while rewriting.

## Proven fixture envelope

`rewrite_webp.py` accepts only RIFF/WebP with exact declared extent and zero
odd-byte padding; known `VP8 `, `VP8L`, `ALPH`, `VP8X`, `ICCP`, `EXIF`, `XMP `,
`ANIM`, and `ANMF` chunks; unique metadata/control chunks; truthful VP8X flags
and reserved bits; the owned key-frame/header subset; metadata ordering; and one
of the witnessed layouts: metadata-free single `VP8 `/`VP8L` RGB, owned static
alpha with metadata, or owned animated metadata with exactly three full-canvas
origin-zero `ANMF` frames whose control byte is `0x02` (no blend, no disposal).
ICCP precedes ALPH/ANIM/image chunks. Metadata-bearing inputs require ICCP and
either both EXIF/XMP or an already-canonical derived output: scrubbed ICCP with
optional minimal orientation TIFF and no XMP. Arbitrary partial metadata
combinations and metadata-free animation reject. `ANIM` is exactly six bytes. It
bounds file bytes (32 MiB), metadata aggregate (8 MiB), total outer+nested
records (65,536), and canvas pixels (100 million) as prototype guards. It
validates nested ANMF extents, geometry and image headers, but it does **not**
parse VP8/VP8L/ALPH entropy.

For ordinary RGB ICC 2/4 profiles, it invokes the existing canonicalizer rather
than deleting ICCP. It removes XMP and descriptive TIFF Exif and writes only a
raw classic-TIFF orientation IFD when orientation is non-1. The owned source
form is raw TIFF, not `Exif\0\0`-prefixed data; other EXIF forms reject. TIFF
next-IFD/thumbnail structures reject. The rewriter copies every compressed image
and frame payload and every animation control byte unchanged, then recomputes
RIFF extent. It always clears XMP; it retains EXIF and its VP8X EXIF feature bit
only when a non-1 orientation has been reduced to the minimal TIFF IFD, and
otherwise removes both.

The `webp-rewrite.json` consumer proof compares exact compressed/frame/control
bytes, all raw and orientation-applied Pillow canvases, timing, looping,
background, geometry and colored fully-transparent pixels. It independently
extracts ICCP, validates the canonical ordinary-profile structure and invokes
the LittleCMS four-intent equivalence consumer. It checks the minimal TIFF IFD,
absence of XMP/descriptive Exif fields, and separate-process determinism plus
idempotence. The typed negative oracle in `webp-rewrite-controls.txt` proves its
explicit `WebPDomainError` and exact reason, not a traceback substring;
malformed RIFF/container, metadata/profile, unknown-chunk and unsafe-variant
inputs write no output and leave input bytes unchanged. It also proves that a
simulated infrastructure exception escapes rather than qualifying as rejection,
all eight orientations on both static and animated controls (including EXIF
removal for orientation 1), simple RGB controls, and a valid late-frame mutation
through the actual rewriter with canvas, background/loop, frame rectangle,
timing, blend/disposal and compressed subchunk identity retained. The current
report records 61 typed no-output domain rejections, retaining the original 20
malformed observer regressions and adding targeted TIFF offset/type/count/GPS,
next-IFD, duplicate/overlapping structures and values, XMP rendering/conflicts,
ICC, chunk-layout, animation and budget cases. Five exact budget edges use small
owned inputs with lowered prototype thresholds; these are guard-logic tests, not
allocation/deadline/platform proofs. Eleven corrupted outputs exercise the same
callable verifier as the published fixture evidence, including incorrect
orientation, leaked EXIF/XMP, dirty ICC header/description, static/frame payload
drift and changed animation timing/blend/background/loop. Both structural IFD
and external-value overlap are checked separately from legitimate inline values.

## Exact remaining blockers

This is not evidence for arbitrary WebP, partial/progressive frames, HDR,
unknown chunks, arbitrary ICC/TIFF/XMP layouts, general animation rectangles, or
a full compressed-entropy safety proof. The fixture caps are not the approved
Task-2 execution limits and do not prove allocation, deadline, cancellation,
platform/native isolation, or malformed-upload safety in a production service.
Pillow/libwebp remains the fixture encoder and canvas consumer, so this is not
an independent entropy decoder. No shared ingress, storage identity/cleanup,
quota, supported-platform/license closure, HEIF device proof, or production
remediation has been implemented. These blockers must not be relaxed into
successful upload admission.
