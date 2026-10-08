# #1702 — owned JPEG orientation/privacy byte-copy witness

Task 1 remains **incomplete**. This tranche is private fixture evidence, not
product integration, a production upload validator, or device-native HEIF proof.

## Implementation and admission

`testdata/image-metadata-sanitizer/rewrite_jpeg.py` does no pixel decoding or
encoding. It copies the complete entropy-coded scan and DQT/DHT/SOF/SOS payloads
in order. Fixture encoding uses pinned Pillow; Pillow and ImageMagick are output
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
Consumers supply the actual display evidence; header checks do not certify DCT
entropy.

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

## Bounds and limitations

The prototype reads at most 32 MiB plus one byte, bounds metadata to 8 MiB and
frame dimensions to 100 million pixels, and checks segment/TIFF/profile extents
and counts before output publication. Actual consumer-valid COM fixtures prove 8
MiB metadata acceptance and plus-one rejection. The parser's record guard admits
65,535 segment/scan records and rejects the next (EOI needs guard headroom).
Lowered **test-only** file/pixel limits prove exact-boundary comparisons on
small valid images; actual over-cap inputs also reject. This is not a 512 MiB
allocator ceiling, CPU/deadline/cancellation isolation, full entropy validation,
or a measured 100-million-pixel consumer proof. Those task-2 and platform
requirements remain open.

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
