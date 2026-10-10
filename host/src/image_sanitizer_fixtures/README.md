# Owned image metadata fixtures

These are generated, non-personal fixtures, not camera originals or downloaded
photographs. The JPEG, PNG/APNG, GIF, WebP and HEIC inputs were generated during
this repository's earlier feasibility experiments and reused for small
conventional ExifTool tests. The PNG/APNG, GIF and WebP artwork generators
remain in `testdata/image-metadata-sanitizer/`; the retired custom codec
machinery is available only in Git history, not as a runtime or shipping
prerequisite. Descriptions, camera identifiers and GPS values are synthetic.
Retained profile descriptions are outside the ordinary metadata-removal policy.

`hdr-signal.png` is a two-by-two RGB image with a planted private comment and a
PNG cICP signal (primaries 9, transfer characteristics 16, matrix coefficients
0, full-range flag 1). Its test independently reads those fields before and
after editing and checks removal of the comment. This checks representative HDR
signaling, not HDR display/device certification. It was generated with pinned
Pillow's `PngInfo.add` and `Image.new`, without a custom image parser.

`png-sanitized.png` and `jpeg-sanitized.jpg` are byte-stable golden outputs
created from the corresponding owned originals with pinned ExifTool 13.59:
`-config '' -overwrite_original -all= --icc_profile:all -tagsFromFile @ -Orientation`.
Manager and HTTP tests compare the actual public bytes to these golden outputs,
including their edited digest and size. The already-clean PNG also provides a
small production startup probe: the editor must process it unchanged before
serve/demo-seed roots accept the runtime.

`inspect_rendering.py` is a test-only observer built on conventional Pillow and
ImageMagick/libheif decoders. The Rust format table independently reads planted
private tags before/after, compares orientation and rendering tags, and compares
whole-profile SHA-256/length plus decoded RGBA frame hashes, frame sizes,
timing/disposal/blending, animation loop/default-frame state and alpha extrema.
The animated cases must actually expose multiple frames, and the corpus must
exercise non-opaque alpha. No custom image/container parser is used. Normal Nix
host-test and dev/CI inputs provide pinned Pillow, ImageMagick and ExifTool.

Fixtures are compiled into tests (and the small startup probe) and explicitly
admitted by the normal application and coverage source filters. Runtime tests
use the pinned ExifTool provided by the development/CI shell or explicit
`JAUNDER_EXIFTOOL` injection; no ignored `.xtask` fixture directory is needed.
