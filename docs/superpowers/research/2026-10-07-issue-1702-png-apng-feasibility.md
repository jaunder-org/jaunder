# #1702 — PNG/APNG primary-source feasibility

**Incomplete task-1 evidence.** Primary-source research and the bounded
candidate probe below do not establish production behavior, HDR support, a
universal PNG envelope, or native-device HEIF feasibility. The approved
[spec](../specs/2026-10-07-issue-1702-image-metadata-sanitization.md) and
[outline](../plans/2026-10-07-issue-1702-image-metadata-sanitization.md) remain
binding; this research does not amend their limits or scope.

## Primary facts and implications

- [PNG chunk layout](https://www.w3.org/TR/png-3/#5Chunk-layout) and
  [ordering](https://www.w3.org/TR/png-3/#5ChunkOrdering) define lengths, type,
  data and CRC, IHDR first, contiguous IDAT and IEND finality. Retain compressed
  IDAT bytes and all decoding-critical header/palette/transparency data.
  [Editor conformance](https://www.w3.org/TR/png-3/#conformance) is not
  authority to discard arbitrary ancillary chunks. Unknown chunks must not
  silently evade the privacy/fidelity policy merely because their safe-to-copy
  bit is set.
- [APNG structure](https://www.w3.org/TR/png-3/#structure),
  [sequence numbers](https://www.w3.org/TR/png-3/#4Concepts.APNGSequence), and
  [animation chunks](https://www.w3.org/TR/png-3/#animation-information) require
  acTL before IDAT, fcTL/fdAT in one consecutive sequence starting at zero,
  valid canvas-contained rectangles, declared frame counts and legal blend and
  disposal operations. Preserve control bytes, delay fractions, loop counts and
  each frame's compressed data. fdAT's sequence word is not compressed data.
- A separate static default image is not an animation frame. The
  [output-buffer rules](https://www.w3.org/TR/png-3/#apng-output-buffer) and
  [Pillow APNG documentation](https://pillow.readthedocs.io/en/stable/handbook/image-file-formats.html#apng-sequences)
  distinguish that case from first-frame-in-IDAT. Decode the default separately
  and enumerate every composited animation frame; seek(0) alone proves neither
  frame association nor disposal/blending fidelity.
- [Color signaling](https://www.w3.org/TR/png-3/#11addnlcolinfo) has precedence:
  cICP, then iCCP, then sRGB, then cHRM/gAMA. Retain validated active/fallback
  rendering information, not arbitrary conflicting profiles or substituted sRGB.
  [Transparency](https://www.w3.org/TR/png-3/#11transinfo) includes alpha and
  tRNS, with palette/sample-depth-dependent structure.
- [iCCP](https://www.w3.org/TR/png-3/#11iCCP) contains a text profile name,
  compression method zero and zlib-compressed ICC. Scrub the profile name as
  well as its descriptive ICC content. Bound compressed and expanded data;
  reject invalid/trailing zlib streams. The existing
  [ordinary ICC witness](2026-10-07-issue-1702-exiftool-13.59-feasibility-witness.md)
  supplies a narrow 2.0/4.0 matrix/TRC prototype, not support for every embedded
  profile. Its validated rendering payloads must remain unchanged.
- [cICP](https://www.w3.org/TR/png-3/#cICP-chunk),
  [mDCV](https://www.w3.org/TR/png-3/#mDCV-chunk) and
  [cLLI](https://www.w3.org/TR/png-3/#cLLI-chunk) are rendering/HDR signaling.
  Ordinary ICC or eight-bit pixel equality does not prove their correctness.
- [eXIf](https://www.w3.org/TR/png-3/#eXIf) contains TIFF bytes beginning II/MM,
  not JPEG's Exif prefix. Deleting it blindly can remove orientation as well as
  GPS, device, time and previews. A proof needs a validated orientation-only
  rewrite, or explicit rejection of variants whose rendering requirements are
  not understood. Orientation preservation includes displayed dimensions.
- [Text chunks](https://www.w3.org/TR/png-3/#11textinfo) include tEXt, zTXt and
  iTXt/XMP; tIME is a timestamp. Bound expansion and remove descriptive content.
  [Other ancillary chunks](https://www.w3.org/TR/png-3/#11addnlsiinfo) such as
  pHYs/bKGD/hIST/sPLT/sCAL are not automatically safe to delete. Signatures,
  C2PA/JUMBF, private rendering data, unknown chunks and trailers need explicit
  fail-closed handling, not an appearance-changing shortcut.

## Candidate and independent tools

[ExifTool PNG tags](https://exiftool.org/TagNames/PNG.html) and the official
[upstream PNG.pm](https://github.com/exiftool/exiftool/blob/master/lib/Image/ExifTool/PNG.pm)
document metadata writing, text removal, CRC processing and ICC handling.
Animation frame/play tags are not a complete writable frame model. Evaluate
**the actual pinned ExifTool 13.59 binary** on fixtures: current upstream source
is not proof of that version's behavior or preservation of fcTL/fdAT. A
successful command is not sufficient, and warnings must remain visible.

[Pillow's PNG reader](https://github.com/python-pillow/Pillow/blob/main/src/PIL/PngImagePlugin.py)
and documentation support independent default/frame navigation and composited
RGBA checks. They do not certify original compressed bytes, automatically apply
ICC, or establish HDR appearance. Use Pillow only as a test consumer, never a
production decode/re-encode sanitizer. Retain a separately implemented chunk
inspector and the independent color-transform proof.

The researcher reported an HTTP 403 for one official HTML retrieval and an
unclear source_check score. Upstream source/spec passages—not that score—are the
basis of this note. Live upstream links are discovery references; executable
proofs must use the repository's pinned tools.

## First bounded fixture tranche

Start with owned synthetic RGB/RGBA eight-bit, non-interlaced streams:

- Static images with fully transparent colored pixels, descriptive text/XMP,
  time, ICC name/descriptions and classic TIFF orientation plus synthetic
  descriptive fields.
- APNG with first frame in IDAT, and APNG with a distinct separate default. Use
  partial rectangles, SOURCE/OVER, NONE/BACKGROUND/PREVIOUS, split frame data,
  nontrivial delays and finite/infinite loops.
- CRC/length/order/duplicate/trailer errors, sequence/count/rectangle errors,
  malformed profile/text/Exif and unsupported unknown chunks. A rejection is not
  an original-byte fallback.

This is an incremental proof slice, **not a frozen production envelope**.
Palette/tRNS, grayscale, sixteen-bit, Adam7 and HDR need subsequent positive and
negative proofs before being included. No source-private field may survive
because an unsupported feature was merely ignored.

## First executed candidate probe

The hermetic witness passes with eight owned input fixtures: RGB, RGBA,
first-frame-in-IDAT APNG and separate-default APNG, each carrying an ordinary
sRGB ICC v2 or v4 profile and classic TIFF orientation 6. The APNG fixtures use
three animation frames, partial rectangles, SOURCE/OVER, all three disposal
operations, split data, a zero delay denominator and finite/infinite looping.

```sh
devtool run -- nix build --impure --out-link .xtask/image-metadata-witness \
  --print-out-paths --file testdata/image-metadata-sanitizer/probe.nix
```

`make_png_fixtures.py` constructs the inputs. The independently implemented
`verify_png_candidate.py` runs separate pinned ExifTool processes with
`-all= --icc_profile:all`, checks chunk CRCs, exact fixture structure and
rendering/image/control bytes, and compares every Pillow-decoded canvas.
`reports/png-apng-exiftool-candidate.json` records:

- Eight fixtures and **18 paired default/animation canvas comparisons**.
- Exact rendering chunks, compressed image/frame bytes and un-oriented RGBA
  canvases preserved in every case; timing, loop, disposal and blend agree.
- **Orientation lost in all eight cases**: explicit Exif-transposed display
  canvases differ after eXIf removal.
- **ICC body and descriptive profile name retained unchanged in all eight
  cases**. The source profiles' descriptive fields were not sanitized.
- Therefore the candidate does **not** satisfy the privacy/fidelity policy.

These are positive structural/consumer observations of an inadequate candidate,
not successful sanitation. Pillow does not automatically apply ICC, and this
probe claims neither HDR fidelity nor production cleanup/quota behavior. The
bounded prototype below supplies that rewrite for the owned slice. Other PNG
variants remain unproven.

## Executed bounded rewrite prototype

`rewrite_png.py` is test-only Python, **not a production sanitizer**. It copies
image/frame chunks, validates deflate/Adler/filter-byte structure in 32 KiB
blocks without undoing filters or reconstructing/rendering pixels, scrubs the
actual embedded ICC via the existing ordinary-profile prototype, sets its name
to `sanitized`, and reconstructs only a classic-TIFF orientation field. Default
orientation 1 requires no eXIf chunk. No original is used as an error fallback.

The narrow TIFF grammar admits the fixture IFD0 descriptions and GPS directory;
unknown fields/graphs, next directories and previews are currently rejected, not
claimed sanitized. Unknown chunks, HDR signaling, palette images, sixteen-bit
samples and Adam7 reject in this slice. Text admission is narrow: standard
descriptive keywords and a closed three-element descriptive XMP schema. Unknown
XMP orientation/HDR/application semantics reject rather than being blindly
deleted. XMP is capped at 4 KiB before XML parsing. PNG3 specifies registered,
case-insensitive [BCP47 language tags](https://www.rfc-editor.org/rfc/rfc5646)
for iTXt. The pilot admits only empty (unspecified), `en`, and `en-GB`,
case-insensitively; malformed tags and all other tags reject. This is a closed
registered subset, not general BCP47 or registry certification. ICC or sRGB
coexisting with gAMA/cHRM rejects in either physical order: matching fallbacks
as well as conflicting ones are unproved, not silently accepted. Other admitted
presentation chunk shapes still lack positive fixtures and are unproven until
the task-1 envelope is frozen. The 32 MiB fixture input/output cap is **not** a
new upload limit; task-2 memory accounting, deadline/cancellation, platform
execution and ownership/cleanup remain unproven.

The complete pinned Nix witness passes. `verify_png_rewrite.py` does not import
the PNG rewriter. Its scanner compares exact rendering/image/frame/control
bytes, independently verifies an exact orientation-only eXIf, inspects scrubbed
ICC fields and preserved color payload/header semantics, and invokes LittleCMS
on profiles extracted from each actual input/output PNG. All four intents pass;
Pillow alone is not the color oracle.

- Eight rewritten fixtures, **18 paired default/animation canvases**, both raw
  and orientation-applied, match; timing/loop/disposal/blend match.
- Separate same-input processes and output reprocessing produce identical bytes.
- **73 malformed/unsupported cases** reject through an explicit PNG/ICC domain
  error and leave no output; source bytes remain unchanged. CRC/order/trailers,
  frame counts/sequences/rectangles/control values, compressed image structure,
  metadata expansion/encoding, TIFF cycles/bounds and unknown rendering
  semantics are exercised. Language controls cover non-ASCII/control bytes,
  empty/invalid subtags, an unsupported registered tag, and both orders of
  ICC/sRGB with gamma/chromaticity fallbacks. Infrastructure exceptions are not
  accepted as expected rejection.
- Five positive empty/English/English-UK language cases verify case-insensitive
  admission, unchanged displayed canvases and removal of descriptive iTXt.
- All eight orientation values preserve every displayed default/animation
  canvas, with only orientation retained (none for default 1).
- An independently constructed valid last-frame pixel mutation leaves the
  separate default and earlier frames unchanged but changes the final rendered
  canvas and compressed-byte comparison. A default-only decoder check cannot
  pass this control.

Reports: `png-apng-rewrite.json` and `png-apng-controls.txt` under the witness's
`reports/`. These assertions establish the bounded owned slice only. This is not
universal PNG conformance, native-device evidence, production ingress,
resource/platform certification or completion of task 1.

## Proof requirements and remaining decisions

Compare exact IHDR, image/frame compressed data, rendering chunks, sequence and
control bytes independently. Compare all decoded default/composited frames,
alpha, loop, delay, blend and disposal; apply and compare orientation
separately. Inspect bounded iCCP expansion and exact scrubbed ICC fields/name.
Run separate same-input invocations and reprocess the output for byte
determinism/idempotence.

Pin the admitted chunk/Exif/color shapes and rejection policy in executable
fixtures and evidence. The approved limits already govern task 2; this note does
not authorize changing them. Production structural validation must not be
replaced by pixel rendering merely to strip metadata, nor by trusting the
sanitizer's own report. Native HEIF, execution/platform boundaries, shared
upload integration and remediation remain separate unfinished work.
