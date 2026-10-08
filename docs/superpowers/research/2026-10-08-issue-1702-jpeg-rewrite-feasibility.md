# #1702 — owned JPEG orientation/privacy byte-copy witness

Task 1 remains **incomplete**. This tranche is private fixture evidence, not
product integration, a production upload validator, or device-native HEIF proof.

## Implementation and admission

`testdata/image-metadata-sanitizer/rewrite_jpeg.py` does no pixel decoding or
encoding. It validates the bounded baseline coefficient syntax described below,
then copies the complete entropy-coded scan and DQT/DHT/SOF/SOS payloads in
order. Fixture encoding uses pinned Pillow; Pillow and ImageMagick are output
consumers. They may share libjpeg and LittleCMS: two consumer interfaces are not
two independent JPEG codecs.

The admitted envelope is deliberately small:

- SOI, known length-delimited segments, one baseline 8-bit sequential Huffman
  YCbCr frame, one interleaved scan, terminal EOI without a trailer. Component
  IDs 1/2/3, quantizer selectors 0/1/1, 4:4:4 or 4:2:0 sampling, DC/AC selectors
  0/1/1, spectral selection 0–63, no successive approximation.
- JFIF 1.01 immediately after SOI, unitless 1:1 density, no thumbnail;
  optionally the exact Adobe version-100, zero-flags, transform-1 segment. Every
  admitted APP0/APP14 byte remains unchanged. Other
  densities/versions/transforms/application schemas reject rather than being
  silently discarded.
- Ordinary owned sRGB and P3-primary/gamma-2.2 ICC 2.0/4.0 profiles, **not
  standard Display-P3**. Actual embedded multipart APP2 bytes are extracted,
  sequence, counts and duplicates checked, then passed through the existing
  ordinary ICC canonicalizer. Canonical APP2 chunks occupy a deterministic
  location immediately after JFIF APP0. The transform-bearing bytes and semantic
  header fields are retained.
- Exif APP1 with the existing restrictive classic-TIFF IFD0/GPS schema, or its
  canonical orientation output. JPEG removes the six-byte Exif identifier before
  reusing the WebP strict TIFF extent/overlap handling and PNG metadata parser.
  Descriptive GPS, device, date, artist and copyright fields disappear. Output
  contains only the minimal independent-consumer-valid orientation TIFF for
  orientations 2–8; orientation 1 needs no Exif segment.
- Restrictive descriptive XMP uses the existing PNG schema validator through the
  WebP helper. Unknown XMP properties reject. COM is removed (T.81 assigns it no
  image-rendering semantics). Unknown APP1/APP2/other APP data reject.
  APP13/IPTC is explicitly unsupported, not assumed safe to discard. Exif/JFIF
  thumbnails, MakerNote, unknown TIFF graphs and previews reject until
  separately proved.

Progressive construction succeeds in pinned Pillow, and both consumers decode
that source without warnings. The prototype **explicitly rejects progressive**;
there is no progressive entropy/frame-display admission proof. DRI/restarts,
multiple scans, arithmetic, lossless, CMYK/YCCK, HDR and other unsupported
markers also reject. Rendering-bearing state is preserved only inside the
admitted no-restart single-scan envelope; this is not general JPEG grammar
certification.

Structural references (not newly fetched in this run):
[ITU-T T.81](https://www.w3.org/Graphics/JPEG/itu-t81.pdf),
[JFIF](https://www.w3.org/Graphics/JPEG/jfif3.pdf),
[CIPA Exif](https://www.cipa.jp/std/documents/e/DC-008-Translation-2019-E.pdf),
and Adobe's
[DCT filter technical note 5116](https://www.adobe.com/content/dam/acom/en/devnet/dct/pdfs/5116.DCT_Filter.pdf).
The marker framing, scan headers, JFIF density/thumbnail fields, Exif TIFF
layout and Adobe color-transform signal inform this restricted envelope.
Consumers supply the actual display evidence. Header checks alone do not certify
DCT entropy; the additional bounded syntax proof below is separate from both
pixel rendering and general JPEG certification.

## Executable proof

`verify_jpeg.py` independently advances through marker/scan bytes without
importing the rewriting parser. Its callable `verify` checks every output
privacy marker, literal minimal orientation TIFF, APP0/APP14 fields and complete
scan/control bytes. Embedded profiles go through the separate ordinary ICC
reader and the existing compiled LittleCMS consumer (four intents × 4,096 RGB
samples). It asserts all observations before publishing a report.

Owned CC0 32×24 asymmetric RGB art is encoded with all four owned ICC profiles,
all eight orientations and both sampling modes: **64 metadata-bearing
positives**, plus **two metadata-free RGB controls**. Source metadata and
embedded profiles are independently consumed by Pillow. Both consumers prove raw
and oriented source/output raster equality and dimensions. Each consumer's
orientation result is additionally compared with the specified transpose of its
own raw raster. Hashes and dimensions are retained in
`reports/jpeg-rewrite.json`. Separate rewrite processes prove repeated-input and
second-pass byte identity for every metadata-bearing positive; inputs remain
unchanged.

`verify_jpeg_controls.py` exercises **38 CLI typed-domain no-output
rejections**, including malformed markers, metadata
offsets/types/counts/cycle/overlap, ICC sequence/count/duplicates, unsupported
modes, restart and budget failures. A preview-bearing Exif source constructed by
pinned ExifTool is independently consumer-valid before prototype rejection.
Missing input and injected programming `RuntimeError` and `ValueError` escape
the domain oracle, with no output. Only the ICC helper's explicit `invalid ICC `
diagnostic maps to profile rejection, not every `ValueError`.

Fifteen deliberate output mutations must fail the **complete callable
verifier**: leaked Exif/XMP/COM, incorrect orientation, changed
entropy/DQT/DHT/SOF/SOS, changed JFIF/Adobe rendering controls, dirty ICC header
and dirty ICC description, ICC before JFIF or after Exif. These controls neither
suppress consumer warnings nor turn tool/import/I/O failures into expected
malformed-image results.

Controller regression initially failed on `jfif-not-first`: the source
constructor and rewrite accepted metadata before JFIF. The constructor now
retains first APP0, the rewrite requires it, canonical ICC follows it, and the
independent observer checks both JFIF-first and contiguous canonical ICC
positions. Privacy and budget controls insert metadata after JFIF (and output
privacy mutations after ICC), so the new ordering guard cannot mask those
intended checks.

## Baseline entropy grammar extension

The checked metadata tranche's 66 positives, 38 domain negatives and 15 complete
observer mutations remain unchanged. `rewrite_jpeg.py` now parses Huffman
coefficient **syntax**, without dequantization, IDCT, color conversion, raster
allocation, re-encoding or a native-decoder validity shortcut. Its three scalar
DC predictors and streaming stuffed-byte reader do not store an image-sized
coefficient array. Output publication still occurs only after all metadata and
entropy checks succeed; compressed source bytes are never repaired.

### Primary normative evidence

The actual [T.81 PDF](https://www.w3.org/Graphics/JPEG/itu-t81.pdf) was fetched
for this extension (SHA-256
`631031d4ba56b06abee3e312a0f235b9422da9c7267d1c8f7604418795768bf0`). With
explicit controller approval, a transient gitignored `.xtask` derivation using
the witness's exact existing nixpkgs import extracted it with pinned Poppler
26.06.0: `/nix/store/g0f2man6jdwimdpz383l8p11r1rzx9hs-poppler-utils-26.06.0`. No
downloaded normative PDF, extraction tool or production dependency was added to
the fixture source. Host `pdftotext` was absent; the first approved extraction
failed on the renamed `poppler_utils` attribute, then succeeded with the
explicitly approved `poppler-utils` correction. These were tool failures, not
image-domain rejections.

Relevant printed T.81 page numbers and mandatory functions:

- A.2.3–A.2.4, page 26: interleaved component/block order and completion of
  partial MCUs. Replicating right/bottom samples is a **recommendation**, not a
  dummy-block coefficient restriction; the grammar does not impose it.
- B.1.1.5, page 33; E.1.4, page 80; F.1.2.3, page 91: only one-bits complete the
  final Huffman byte, including a stuffed zero if this creates `FF`. Exact
  required blocks must consume the scan apart from these 0–7 fill bits; hidden
  extra entropy bytes reject. The existing no-marker-fill/no-restart envelope
  stays closed, not all T.81 marker layouts.
- Annex C/C.2, pages 50–52: canonical Huffman assignment in increasing
  width/order, maximum width 16, reserved all-ones code space. Prefix tables are
  bounded by the already-validated unique baseline symbols before allocation.
- F.1.1.4–F.1.1.5, pages 87–88; F.2.1.3, page 104: quantized coefficients have
  signed 11-bit precision for 8-bit input, and DC prediction is component-local,
  initialized to zero at scan start. The prototype enforces reconstructed
  quantized DC in `[-1024, 1023]`, **not** an invented bound on the transmitted
  difference. DC differences may use category 11 through magnitude 2047. F.2's
  later IDCT/clamping tolerance is not an input-validity exemption.
- F.1.2.1/Table F.1, pages 88–89; F.1.2.2/Table F.2, pages 89–90;
  F.2.2.1–F.2.2.4, pages 104–110: DC categories 0–11, AC categories 1–10,
  amplitude sign extension, runs 0–15, EOB and 16-zero ZRL, exact zig-zag slots
  1–63. A nonzero coefficient at 63 completes its block without EOB. A ZRL
  exceeding the 63 AC slots or a nonzero run landing beyond 63 rejects as
  overflow. A ZRL exactly covering slots 48–63 is instead **conservatively
  unsupported**: the cited encoder/decoder flow diagrams do not establish a
  mandatory malformed-input ruling for this terminal form. Its normative
  validity remains unresolved; this boundary is not claimed as JPEG overflow.
- F.2.2.5, pages 110–111: MSB-first reads remove only `FF00` stuffing. There is
  no unexpected-marker/restart/native-consumer fallback.

These constrain this prototype's restricted baseline coefficient domain; they
are not a claim of T.81 Part 2 encoder/decoder compliance or IDCT certification.

### Independent required corpus and sensitivity

`verify_jpeg_entropy.py` defines **892 independently enumerated required
positive identities**. Explicit fixed and mixed Huffman codeword/width schedules
are authored independently of the rewriter's canonical compiler. Coefficient
layouts are constructor inputs, not producer/validator traces. A second,
test-only string-prefix observer consumes these words, independently checks the
signed coefficient/block models and terminal bits, and compares its bounded
aggregates/digest with the real compiler/grammar parser. The independent
marker/privacy/rendering observer imports no entropy rewriting code.

The asserted matrix covers all DC categories, signed low/high magnitude edges
(including valid category-11 transitions between -1024 and 1023), all 160
nonzero AC run/size combinations at both signs/magnitude edges, EOB/ZRL, every
nonzero AC position 1–63, a dense 63-AC block, ZRL's last admitted slot
boundary, component-local 4:2:0 predictors, canonical empty-width intervals,
mixed lengths and **actual** consumed Huffman widths 1–16, byte
crossings/stuffing and all final fill counts 0–7. Valid ZRL followed by EOB is
observed as F.2 decoding syntax; it is not rejected merely because F.1's
efficient encoder uses one trailing EOB. A final `FF00` made by fill bits and
amplitude-generated `FF00` both occur in actual positive inputs.

Both 4:4:4 and 4:2:0 use independently enumerated padded MCU counts at
dimensions 1×1, 7×7, 8×8, 9×9, 15×15, 16×16, 17×17, 1×17, 17×1 and 33×25. Each
golden is consumer-valid in pinned Pillow and ImageMagick without observed
warnings; source/output raw rasters and byte streams agree. Hand-authored
zero-DC/EOB canvases additionally equal exact independently specified RGB
`(128,128,128)` pixels in both consumers. Complex coefficient cases use
independent syntax models and actual consumers, not a test-side IDCT pretending
to be the oracle. Two interfaces may share libjpeg; that caveat is unchanged.

**181 additional independently enumerated CLI domain controls** assert unchanged
input/no output and exact targeted diagnostics: partial Huffman codes, every
proper DC/AC amplitude-prefix length at physical byte EOF, unallocated prefixes,
illegal DHT categories/zero-size symbols/all-ones code, DC predictor precision,
AC/ZRL overflow, conservative terminal-ZRL rejection, missing/extra padded
blocks and whole MCUs, hidden extra bytes, every single-zero final-fill position
and malformed stuffing/restarts. Valid JFIF/frame/scan ordering is retained so
header failures cannot mask entropy controls. Missing-input and unrelated
programming-error escape controls in the existing suite remain green. Three
additional deliberate output scan/codeword/fill mutations run through the
**actual complete observer**, requiring its compressed-control mismatch before
consumer calls. No warning/crash is counted as a typed grammar rejection.

The earlier incorrectly classified `zrl-exact-16-trailing` case is retained
byte-for-byte as `unsupported-terminal-zrl-with-extra-block`; its bytes also
included an extra block. Two additional terminal-ZRL controls, one per sampling
mode, have exactly the required remaining blocks. Pinned Pillow and ImageMagick
consume them without observed warnings and produce exactly the same raw/display
pixels as their EOB counterparts before the CLI's precise
`unsupported terminal ZRL` rejection is asserted. Consumer tolerance is not a
normative validity proof. True ZRL overflow and all existing extra-block/MCU
controls remain intact; successful admission has not widened. The report retains
these separately as `unsupported_consumer_controls` rather than presenting them
as proven malformed JPEG syntax. Both review axes blocked the earlier normative
claim; this reclassification addresses that finding.

The report `reports/jpeg-baseline-entropy.json` retains required identities,
per-case independently observed coefficient hashes, actual bit/width/category/
MCU/block/fill/stuff counts, consumer hashes, and exact rejection diagnostics.
All published coverage observations are asserted; no failed observation is
merely serialized as green.

## Bounds and limitations

The prototype reads at most 32 MiB plus one byte, bounds metadata to 8 MiB and
frame dimensions to 100 million pixels, and checks segment/TIFF/profile extents
and counts before output publication. Actual consumer-valid COM fixtures prove 8
MiB metadata acceptance and plus-one rejection. The parser's record guard admits
65,535 segment/scan records and rejects the next (EOI needs guard headroom).
Lowered **test-only** file/pixel limits prove exact-boundary comparisons on
small valid images; actual over-cap inputs also reject. This is not a 512 MiB
allocator ceiling, CPU/deadline/cancellation isolation, general JPEG entropy
validation, or a measured 100-million-pixel consumer proof. The new reader
checks a conservative physical-bit lower bound before dimension-driven MCU work,
bounds each block to 63 AC positions and each code to 16 bits, and keeps bounded
tables and scalar state; this is grammar/work structure, not measured task-2
allocation or deadline enforcement. Those task-2 and platform requirements
remain open.

## Reproduction

```sh
devtool run -- nix build --impure --out-link .xtask/image-metadata-witness \
  --print-out-paths --file testdata/image-metadata-sanitizer/probe.nix
```

Read the parked logs separately. The focused full witness also runs the
unchanged 636 ICC structure controls, PNG/APNG, WebP and GIF suites (406 GIF LZW
positives, 222 new LZW negatives and 38 older GIF negatives). No negative was
removed and no other format helper was edited. No broad gate, commit, stage,
push or task-1 checkbox is part of this evidence.
