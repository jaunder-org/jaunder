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
indices and expansion count, not a full pixel raster. The current corpus does
**not** prove full/deferred-clear twelve-bit dictionaries: the prototype admits
only minimum code size 2 and three/four-bit states, rejecting before the first
five-bit transition (including if the next code would be end/clear). Higher
minimum sizes and wider/full/deferred-clear states cannot succeed. This is a
fixture-supported subset, not a viable general GIF envelope yet. Interlace,
ANIMEXTS, user-input controls, Plain Text and XMP/unknown applications reject
until proved. Other GIF structures, metadata limits, previews, rendering
variants and platform/runtime safety remain unproven. Reports:
`gif-rewrite.json` and `gif-controls.txt` under the witness's `reports/`.

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
