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

`png-sanitized.png` and `jpeg-sanitized.jpg` are byte-stable golden outputs from
the linked native editing engine and their corresponding owned originals. Their
update required independent ExifTool private-tag inspection and conventional
decoded-rendering, orientation and full-ICC equality checks, not merely
regenerating expected bytes. The JPEG additionally checks byte-idempotence.
Manager and HTTP tests compare actual public bytes to these outputs, including
edited digest and size. The native PNG SHA-256 is
`fd0e88136fda66661f07ab7f269c9f6a28726f434cc91721ee7fd608fd481620`; the JPEG is
`ed8b5251799b0381863da2b50ff7ee22927503f08947d3a39a98bcaccc89113c`. The owned
original PNG also provides a small production startup probe: the same-executable
worker must produce its canonical sanitized digest before serve/demo-seed roots
accept the adapter.

`heic-named-original.heic` is the owned HEIC control from the native feasibility
corpus. Its metadata item name contains the synthetic marker
`native-private-item-name`; the earlier test-only little-exif study control
added that name while adjusting container sizes and metadata locations. The
maintained regression test requires the checked parser to find that actual name,
checks blanking and preserved metadata framing, and compares every byte outside
the checked metadata regions. It is a supplemental item-name control, not a new
format or a replacement for the exact-nine format/rendering observer population.
This boundary test does not independently certify decoding or device rendering.

`inspect_rendering.py` is a test-only observer built on conventional Pillow and
ImageMagick/libheif decoders. The Rust format table independently reads planted
private tags before/after, compares orientation and rendering tags, and compares
whole-profile SHA-256/length plus decoded RGBA frame hashes, frame sizes,
timing/disposal/blending, animation loop/default-frame state and alpha extrema.
The animated cases must actually expose multiple frames, and the corpus must
exercise non-opaque alpha. No custom image/container parser is used. Normal Nix
behavioral-test and dev/CI inputs provide pinned Pillow, ImageMagick and
ExifTool as independent observers. Product binaries and their dependency
preparation do not inherit those tools.

Fixtures are compiled into tests (and the small startup probe) and explicitly
admitted by the normal application and coverage source filters. Editing tests
inject a real `test-support` executable through `JAUNDER_IMAGE_WORKER_BIN`:
`cargo xtask test-local` builds it explicitly, and hermetic coverage supplies
its reviewed package. Libtest executables never act as production worker
substitutes. Observers use their separate test-only tool paths; production never
discovers an editor through PATH or environment. No ignored `.xtask` fixture
directory is needed for maintained tests.
