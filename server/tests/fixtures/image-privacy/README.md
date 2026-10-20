# Image upload privacy fixtures

`jpeg-original.jpg` is generated non-personal artwork with planted synthetic
metadata, admitted as BEFORE data under JAU-1702-RESTART-001. SHA256:
`944700d5ffc62c63ac6a93f75001bf24ab942ed2734c9e2b640399d4f0c7fc9c`.

The upload integration test consumes this original and independently reads both
input and retrieved public bytes with the nixpkgs-pinned test-only ExifTool. It
is not a sanitized golden. No historical generator, helper or AFTER data is
used. Changes to the fixture must retain positive ordinary metadata witnesses.

`jpeg-clean.jpg` is a fresh test INPUT for ordinary already-clean uploads and
existing placement/reclamation tests. It was produced in this checkout from the
above owned artwork by the pinned TEST-only ExifTool 13.59 (`-all=`); it is not
an expected output of Jaunder or a historical AFTER artifact. SHA256:
`a7a0d6384e2bdaddd9a476fbf09526d6acf5e923189c4a83f5941b4e18295b3f`. The
positive-witness privacy tracer always uses `jpeg-original.jpg` instead.

The real browser UI and live Emacs upload/pull/republish tests share these JPEG
inputs. They observe public bytes with TEST-only ExifTool and retain originals;
hermetic test composition passes the fixture directory explicitly rather than
copying originals into a second maintained corpus.

The format/fidelity HTTP tracer also consumes these generated, non-personal
BEFORE inputs admitted under the same authority. No historical helpers,
generators or sanitized output goldens are used.

| Input                          | SHA256                                                             |
| ------------------------------ | ------------------------------------------------------------------ |
| `png-original.png`             | `2e6f4abd182f8b4db24ac964f0422a215392355a750a0fa0eb3167e6745ef7c5` |
| `gif-animation-original.gif`   | `e9b9af42701dce5afacbaf3f8b6aef8076814147f490bb47d0beca841ea521fd` |
| `webp-animation-original.webp` | `60b7834bd02c4742971a63fef5586cd1fac296ef79b711d843d755cc01a74d01` |
| `heic-original.heic`           | `32ef665e9e89f18e91985c79d8a1984b5375eec33e82be20137637ebb109c994` |
| `hdr-signal.png`               | `511385f18f0adafb501b512f1146127e16b45b2e34be6b071c56ab6bf44962de` |

The APNG variants are unchanged copies of the separately admitted BEFORE data:

| Input                       | SHA256                                                             |
| --------------------------- | ------------------------------------------------------------------ |
| `apng-default-original.png` | `7c8cc38a7736df26e5da507b619fea9a6d210f8012a08ccb08aaac63eba2ebaf` |
| `apng-first-original.png`   | `b35ccee766d7bd277e99bd4e630b33770e3d8a870263eb419ab279cdc1407126` |
| `png-color-original.png`    | `02b08fd0d94c989b43069c1411ebb4da07413b0a5b182c017e65193d59429d34` |

`png-color-original.png` is a fresh owned BEFORE variation of
`png-original.png`, created with pinned TEST-only ExifTool 13.59 using
`-ICC_Profile=`, `-PNG:Gamma=2.2`, `-PNG:SRGBRendering=Perceptual` and `-o` to a
new path. Removing the ICC profile avoids conflicting color declarations;
ordinary private witnesses remain. Positive Gamma and zero-valued sRGB intent
are required before and after upload. This is not a Jaunder output golden. The
APNG cases compare three decoded RGBA frames, delays/placement and the
independently stripped encoded container, including animation controls; the
default image variant also retains its encoded default image. The original
admitted inputs are untouched.

Tests independently require positive ordinary metadata before uploading, then
inspect retrieved public bytes, exact ICC where present, rendering controls,
encoded content and truthful stored identity. PNG/WebP decoding additionally
compares actual RGBA/dimensions and WebP frames/timing. GIF uses the positive
transparent-index control plus unchanged encoded frames; its alpha samples are
not independently decoded. The small HDR PNG is a cICP signaling control
(primaries 9 / transfer 16 / matrix 0 / full range 1), not a camera/HDR device
certification. The PNG original's ordinary timestamp and textual XMP remain
mandatory removal witnesses; do not fixture-strip them to make an editor pass.

For HEIC, the pinned TEST-only ExifTool's requested `ImageDataHash` with
`ImageHashType=SHA256` hashes encoded image extents, excluding metadata and
previews. Container bytes/offsets are not compared as if they were image data.
Its `-v3 -ImageDataHash` observation identifies this input's 63-byte `hvc1` item
at `0xfcf`. A single sensitivity control guards the complete input hash and byte
`0x80` at `0xfd7`, flips that byte in a disposable memory copy, and requires a
different image-data hash. It does not parse the format or alter this input. The
64×64 coded extent, 32×24 clean aperture and nclx signaling are retained; this
small fixture has no ICC profile or device-specific HDR claim.
