# #1702 — GIF/WebP primary-source feasibility

**Primary research and a bounded owned GIF pilot, not production proof.** Task 1
remains incomplete. WebP remains source research only. The approved spec/outline
remain binding. Research artifacts were produced independently for GIF and WebP;
this note records the primary facts needed for incremental owned-fixture
witnesses.

## GIF

The [GIF89a specification](https://www.w3.org/Graphics/GIF/spec-gif89a.txt)
defines logical screen, global/local palettes, image rectangles/interlace, GCE
scope, transparent index, disposal, user-input flag, hundredth-second delay, LZW
data sub-blocks and final trailer. Those are rendering information, not
removable descriptive metadata. Plain Text Extension is a **graphic-rendering
block**: preserve and prove it or reject it, never delete it as a comment.

Application extensions have an eight-byte identifier and three-byte
authentication code. Their semantics are application-defined. De-facto
NETSCAPE2.0/ANIMEXTS1.0 loop controls cannot be blanket-dropped; absence, finite
count and zero/infinite looping are distinct. The
[Wuffs decoder](https://github.com/google/wuffs/blob/main/std/gif/decode_gif.wuffs)
and
[giflib notes](https://android.googlesource.com/platform/external/giflib.git/+/4d7e65f86f50437fac1d91edac7bd2950abb3960/doc/whatsinagif/bits_and_bytes.html)
provide implementation evidence, not a replacement for cross-consumer proof.

The [ICC convention](https://www.color.org/specification/ICC1v43_2010-12.pdf)
and [ExifTool GIF tags](https://exiftool.org/TagNames/GIF.html) identify
`ICCRGBG1`/`012` for a complete ICC profile in sub-blocks. Scrub its
descriptions without changing transforms. `XMP Data`/`XMP` is recognized XMP,
not authority to remove unknown rendering requirements. Comments can be removed;
unknown applications must reject unless their semantics are independently
proved.

[Pillow's GIF reader](https://pillow.readthedocs.io/en/stable/_modules/PIL/GifImagePlugin.html)
composites canvases and may promote palette mode. Its writer can optimize
palettes/frames: use it only to construct owned inputs or decode **all** frames,
not to sanitize. Compare palettes, compressed streams and controls separately;
use a second consumer for accepted looping/disposal variants.

The first pilot should use owned global/local-palette frames, transparency,
partial rectangles, disposal/delay variations, known NETSCAPE looping, comments
and ordinary ICC 2/4. LZW structural validation must check clear/end codes,
dictionary growth, expansion count and all emitted palette indices without
materializing a full raster. ANIMEXTS, Plain Text, XMP/application variants and
broader structures remain unproven until their own fixtures pass.

## Executed GIF pilot

The pinned Nix witness passes for `make_gif_fixtures.py`, `rewrite_gif.py`,
`verify_gif.py` and `verify_gif_controls.py`. These are **fixture-only Python**,
not a selected production runtime. The 32 MiB file cap is a fixture limit, not
an upload-policy change; task-2 execution/resource/platform proof is unfinished.

- Seven owned inputs, **15 paired displayed canvases**: bare GIF87a, GIF89a
  static transparency and finite/infinite NETSCAPE animations with ICC2/4.
  Partial rectangles, distinct global/local palettes, delay and disposal1/2/3
  are exercised. Exact screen/palette/image/LZW/control/loop bytes survive.
- Independently implemented byte inspection plus **Pillow and ImageMagick**
  compare every coalesced source/output frame. Six extracted profile pairs pass
  independent ICC field/header/color inspection and LittleCMS all-intent checks.
  The bare87 file stays byte-identical; separate invocations and output
  reprocessing prove determinism/idempotence for every input.
- Pinned ExifTool13.59 `-all= --icc_profile:all` removes comments from six
  inputs but leaves their original descriptive ICC bodies unchanged. It is not a
  sufficient privacy candidate.
- **45 malformed/unsupported cases** reject with an explicit GIF/ICC domain
  error, unchanged input and no output. They cover header/palette/rectangle, GCE
  flags/scope/transparency, unknown/plain-text/application data, duplicate or
  late loop/profile, sub-block truncation, LZW dictionary/index/expansion/end
  and trailer failures. Infrastructure errors do not count as expected
  rejection.
- Additional known-code fixtures independently decode the LZW KwKwK special
  case, a three-to-four-bit dictionary-width transition, the maximum admitted
  four-bit boundary, and a clear/reset back to three bits. Valid inputs using
  minimum code sizes 3–8 or entering the first five-bit state are independently
  decoded by Pillow and ImageMagick before verifying explicit no-output
  rejection by this narrower prototype. A valid last-frame pixel mutation
  changes only the final rendered canvas and retained image bytes, demonstrating
  the observer does not check only frame0.

LZW validation tracks dictionary string lengths, first symbols, maximum palette
indices and expansion count, not a full pixel raster. The initial pilot admitted
only minimum size 2 and three/four-bit states. The independently proved
dictionary extension below supersedes that restriction, not the other GIF
feature restrictions or production caveats. Interlace, ANIMEXTS, user-input
controls, Plain Text and XMP/unknown applications reject until proved. Other GIF
structures, metadata limits, previews, rendering variants and platform/runtime
safety remain unproven. Reports: `gif-rewrite.json` and `gif-controls.txt` under
the witness's `reports/`.

## Executed LZW dictionary extension (2026-10-08)

The primary GIF89a Appendix F specifies minimum+1 initial width, clear+2 first
entry, LSB-first packing and maximum code 4095 at twelve bits. Its **cover-sheet
Deferred Clear Code** clarification requires a full dictionary to remain frozen
at twelve bits until clear; code 4096 cannot be represented. The validator now
implements those states, using 4096 length/first/maximum entries without
decoding a raster. No container feature acceptance changed.

`verify_gif_lzw_envelope.py` constructs explicit code-word/width schedules by
**literal ordinal formulas**, not the validator's dictionary algorithm. After N
literals, highest allocated entry is clear+N and entry k is the pair of literals
at ordinals k-clear-2 and k-clear-1. Known next-entry specials append two copies
of the previous literal; chained specials explicitly yield strings of lengths
2/3/4. Golden palette indices and RGBA canvases are constructed directly from
those authored symbols. Neither golden construction nor packing imports the
rewriter. Both pinned Pillow **12.3.0** and ImageMagick **7.1.2-29**
independently check every source and rewritten canvas exactly, and Pillow checks
exact palette and indices. Rewrites preserve every byte for these metadata-free
controls, including compressed sub-blocks, through repeat and second-pass
invocations.

| Minimum | Executed positive rows | Admitted widths | Transitions checked |
| ------- | ---------------------- | --------------- | ------------------- |
| 2       | 82                     | 3–12            | 3→4 through 11→12   |
| 3       | 74                     | 4–12            | 4→5 through 11→12   |
| 4       | 66                     | 5–12            | 5→6 through 11→12   |
| 5       | 58                     | 6–12            | 6→7 through 11→12   |
| 6       | 50                     | 7–12            | 7→8 through 11→12   |
| 7       | 42                     | 8–12            | 8→9 through 11→12   |
| 8       | 34                     | 9–12            | 9→10 through 11→12  |

For **every** minimum/width pair, rows terminate, clear/reset, reference an
allocated entry or exercise KwKwK immediately before and after the allocation
boundary. Twelve-bit after-boundary KwKwK is intentionally absent: the table is
full and no next code exists. All seven minima allocate and reference entry
4095, freeze the dictionary while referencing its last and earlier entries and
literals, then clear at twelve bits and exercise reset KwKwK. Separate chained
specials prove a previous dictionary string rather than only a previous literal.
Schedules (`*.words.json`) and independently authored `*.golden.rgba` are
retained beside each fixture, distinct from consumer evidence in the reports.
These are state witnesses, not validator runtime traces.

**406 positive rows and 222 new malformed rows execute.** The latter cover
initial/after-clear unallocated codes, missing initial clear, premature end,
forward codes at all non-full transition states, truncated variable-width ends,
expansion overflow and stale entries after reset at every width, plus
unsupported minima 0/1/9/12/255. All reject via the explicit GIF domain error,
unchanged input and no output; infrastructure failures cannot satisfy those
assertions. The original 45-case suite retains all **38 malformed/unsupported**
cases; the six valid minimum3–8 streams and valid first-five-bit stream are
migrated to named positive golden controls, not deleted as malformed. Original
seven inputs/15 paired animation canvases, six ICC
privacy/all-four-LittleCMS-intent pairs, compressed/frame-control fidelity and
late-frame observer checks remain green.

Before widening, the 301-row initial consumer-only matrix passed with the old
rewriter and all 45 original rejection controls (Nix output
`/nix/store/p8pfx94lyah723i10h2fd1g9xz3rkxak-issue-1702-image-sanitizer-feasibility-witness`).
The final focused full witness passed with 406 rows, including additional normal
entry references at every width and chained specials. Final output:
`/nix/store/m1gr20x2gk6hdxjsyqw055sh9z6hvbsa-issue-1702-image-sanitizer-feasibility-witness`.
Reports `gif-lzw-consumers.json`, `gif-lzw-rewrite.json`, `gif-rewrite.json`,
`gif-controls.txt` and `gif-consumer-packages.txt` provide per-row results and
pinned consumer identities. A separately executed census reconciles all 406
required row identities and the 222 rejection records. Invocation:

```sh
devtool run -- nix build --impure --out-link .xtask/image-metadata-witness --print-out-paths --file testdata/image-metadata-sanitizer/probe.nix
```

This is a synthetic grammar/state envelope, **not all-GIF certification**, pixel
steganography detection or entropy-safety proof. Strict initial-clear and
zero-padding/no-post-EOI-byte restrictions remain. Interlace, user-input, Plain
Text, ANIMEXTS, XMP and unknown applications remain closed. File/pixel/frame/
record guards are fixture prototype guards: no measured production allocation,
deadline, cancellation, isolation or supported-platform safety follows. No
product integration, task-1 completion or shared-upload policy change occurred.

## WebP

The
[official RIFF specification](https://developers.google.com/speed/webp/docs/riff_container)
defines RIFF declared extent, little-endian chunk lengths, zero odd-byte
padding, VP8/VP8L/VP8X, feature/reserved bits, ICCP, ALPH, EXIF/XMP and
animation. Metadata changes require corresponding VP8X feature changes, not
stale flags. Retain exact compressed image/alpha bytes and all presentation
information. Unknown chunks cannot silently retain private data or be discarded
when their rendering meaning is unknown; reject initially.

ANIM stores BGRA background and loop count. ANMF stores half-resolution offsets
(actual position = stored value times two), rectangle, millisecond duration,
blend/disposal and nested padded image chunks. Preserve controls and payloads,
not merely frame count. A source RIFF cannot represent an odd pixel offset;
libwebp mux APIs snapping odd _API arguments_ is a separate fixture-construction
hazard, not a native-source odd-offset field.

[libwebp demux.h v1.5.0](https://chromium.googlesource.com/webm/libwebp/+/refs/tags/v1.5.0/src/webp/demux.h)
and
[mux.h v1.5.0](https://chromium.googlesource.com/webm/libwebp/+/refs/tags/v1.5.0/src/webp/mux.h)
are pinned **reference sources**, not proof of the nixpkgs package version.
Demux/animation APIs are useful independent consumers. Mux assembly success
alone proves neither privacy nor byte fidelity; encoders are unsuitable for
production metadata removal. WebP does not use GIF's LZW algorithm.

[Pillow WebP](https://pillow.readthedocs.io/en/stable/_modules/PIL/WebPImagePlugin.html)
provides an all-frame decode consumer, not a metadata-only editor or automatic
ICC/HDR oracle. Exif orientation requires a separately validated
orientation-only rewrite and display comparison.
Alpha/frames/background/duration/loop/control bytes, extracted ICC transforms
and compressed VP8/VP8L/ALPH bytes need separate proofs. Ordinary eight-bit
profiles do not establish HDR support.

## Candidate and source caveats

[ExifTool RIFF tags](https://exiftool.org/TagNames/RIFF.html) and GIF tags
document recognized metadata families, not lossless sanitizer behavior. Some
official ExifTool pages returned HTTP 403 to research tools, so those claims
depend on lower-confidence official search/source passages. Execute the
**already-pinned 13.59 binary** before selecting it; live documentation/version
labels are not source/runtime acceptance. The current witnesses already pin
dependencies via nixpkgs; research does not authorize ambient installs or
re-resolution.

## Next proofs and remaining scope

Build independent container inspectors, all-frame consumers, ICC oracles and
adversarial controls on owned inputs. Compare exact palettes/compressed ranges,
all default/displayed frames and every control; prove metadata removal,
determinism and idempotence. Failures must be explicit domain rejections, never
original fallback or infrastructure errors misclassified as unsupported data.

This note freezes no production envelope. Native HEIF licensing, broader PNG,
JPEG orientation/presentation, resource/platform boundaries, shared ingress,
quota/cleanup/identity and one-off remediation remain unfinished. No production
systems or external image assets were accessed by this research.
