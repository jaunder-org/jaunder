# #1702 HEIF coded-data ownership checkpoint

**Status: blocked diagnostic checkpoint, not a sanitizer.** The supervisor
approved stopping before rewriting/publication after the source-backed
counterexample below. No HEIF admission envelope is certified. Task 1 remains
incomplete; rejecting every device photo would not satisfy the approved spec.

## Observed owned input

Starting checkout: `0f8bc7c3`, initially clean. The unchanged existing
`testdata/image-metadata-sanitizer/probe.nix` generated original CC0 artwork (a
32×24 gradient, not a device photograph), encoded it through pinned libheif
1.23.1, and attached synthetic metadata using ExifTool 13.59. The existing
fixture root resolves to
`/nix/store/rs8kygwdnw2zfjqzj6hs3qscggvck3ds-issue-1702-image-sanitizer-feasibility-witness/fixtures`.

| Object                                         | Bytes | SHA-256                                                            |
| ---------------------------------------------- | ----: | ------------------------------------------------------------------ |
| Original `input/device-like.heic`              |  4110 | `32ef665e9e89f18e91985c79d8a1984b5375eec33e82be20137637ebb109c994` |
| Existing ExifTool `candidate/device-like.heic` |   617 | `68c6c64a5ae005c590ed90b250e84e6b4c9871e6d222a0ec2ce1f7315f1c29f0` |
| Private suffix counterexample                  |   665 | `00de9d6e1762e528a3b535237d8ae001dcb1fbd81043a9fd80da3e11a68311eb` |

Observed shape, **not a general admitted envelope**:

- Top-level `ftyp`, `meta`, `mdat`; major `heic`, minor version zero, compatible
  brands `mif1`, `heic`, `miaf` in that order.
- `meta` version zero, children `hdlr`, `iloc`, `iinf`, `iref`, `pitm`, `iprp`.
  `pitm` identifies item 1; `infe` version 2 identifies item 1 `hvc1`, item 2
  `Exif`, item 3 `mime` (`application/rdf+xml`). `cdsc` edges are 2→1 and 3→1.
  Candidate keeps the metadata descriptors/references but gives both metadata
  items zero-length extents.
- `iloc` version zero, offset/length/base widths 4/4/4, index width zero, one
  extent per item, internal data references. Original primary uses a nonzero
  base offset; the diagnostic applies base plus extent offset, not array
  position. Candidate primary extent is `[554,617)`; both empty metadata extents
  start at 554. No nonempty unowned `mdat` bytes in these objects.
- Properties in `ipco`: `hvcC`, `colr`, `ispe`, `clap`, `pixi`. Primary
  association bytes are `81 02 03 05 84`: hvcC and clap essential; colr, ispe
  and pixi nonessential. Encoded `ispe` is **64×64**, with **32×24 clean
  aperture**, not an uncropped 32×24 encoded image. `nclx` is 1/13/6/full range.
  No irot/imir or embedded ICC observed. This is not an orientation, ordinary
  ICC, HDR, alpha or auxiliary-image proof.
- `hvcC` contains one VPS NAL type 32 (24 bytes), one SPS type 33 (41 bytes),
  and one PPS type 34 (6 bytes). Raw array header bytes are 96/97/98; this
  checkpoint records them without claiming configuration-record reserved-bit
  conformance. The primary item contains one four-byte-length-prefixed
  `IDR_N_LP` NAL type 20, 59 bytes, layer zero, temporal-id-plus-one 1. No
  separate SEI NAL was observed. Absence of a separate SEI is not proof of
  parameter-set/header privacy or complete coded-data ownership.

The old candidate physically removes the observed synthetic descriptions; that
fact does not validate arbitrary compressed-stream bytes. The diagnostic does
not import the rewrite parser or existing verifier. It checks only the expected
fixture framing/typed extent/property observations needed for this control; it
is **not** the complete independent graph/output/privacy verifier required for
successful sanitization.

## Primary-source boundary

Primary source: **Rec. ITU-T H.265 V9 (09/2023)**, discovered through the
[official recommendation index](https://www.itu.int/rec/T-REC-H.265/en) and
[edition page](https://www.itu.int/rec/T-REC-H.265-202309-S/en).
[Actual edition PDF](https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-H.265-202309-S!!PDF-E&type=items)
SHA-256: `30b1f2dc016e6f6f2b76d75e19ac3fabffbee0c06c0e7345cf88fa9ebf713269`. The
PDF was checked for a PDF signature before extraction; no PDF is committed.

The approved transient Nix extractor uses the existing pinned flake import and
Poppler `/nix/store/g0f2man6jdwimdpz383l8p11r1rzx9hs-poppler-utils-26.06.0`.
Extracted text, hash and tool path are retained under `.xtask/h265-extracted/`.
Relevant source clauses (baseline slice syntax, not an assertion of complete
conformance to this edition or equivalence to all other editions):

- §7.3.2.9: `slice_segment_layer_rbsp()` contains slice header, slice data, then
  `rbsp_slice_segment_trailing_bits()`.
- §7.3.8.1: slice data loops over coding-tree units and
  **`end_of_slice_segment_flag ae(v)`**. Termination is arithmetic-coded, not a
  boundary supplied by the NAL length or slice header.
- §7.3.2.10/.11 and §7.4.3.10/.11: after slice data, stop-one/alignment-zero
  bits and optional `cabac_zero_word` words have prescribed values.
- §7.4.2.1 describes extracting the last stop-one bit **when RBSP boundaries are
  known**. Picking the last one-bit does not prove that earlier bytes belong to
  coded picture syntax. Appending bytes can move that presumed end.
- §7.4.7.1 slice-header-extension semantics allow ignored data bytes. Complete
  parameter-set and slice-header inspection would still be necessary, but cannot
  by itself locate arithmetic-coded termination and establish ownership of the
  entire remainder.

The initial guessed `202309-I` URL returned HTTP 200 **HTML Document Not
Found**. Extraction failed with exit 1; this is a source/tool failure, never an
image rejection. The failed response remains `.xtask/dispatch/h265.pdf`,
separately from `.xtask/dispatch/h265-202309.pdf`. No mirror or alternate
transport was used.

## Executable counterexample

New `verify_heif_coded_boundary.py` creates only a private diagnostic fixture.
It appends exactly 48 owned bytes:

```text
Owned synthetic private VCL trailer; not pixels
```

followed by byte `80`. It changes exactly three declared four-byte lengths:

- primary `iloc` extent length at file offset 103: 63→111;
- `mdat` box size at offset 546: 71→119;
- sole NAL length at offset 554: 59→107.

It preserves the entire original 59-byte NAL as a prefix, every rendering
property byte and parameter-set byte, all original descriptors/associations, and
both zero-length metadata extents. The inserted bytes are inside the correctly
declared NAL, item and `mdat` extents, not a new box or unreferenced file
trailer. Their ownership as pixels is **not proven**; they cannot be relabelled
as steganography to escape the metadata contract. This construction is not
asserted to be a conforming HEVC bitstream.

Pinned ImageMagick 7.1.2-29 decoded the candidate and control with exit 0 and
**empty stdout/stderr**. Both displayed 32×24 and produced byte-identical 8-bit
RGBA (3072 bytes), SHA-256
`17efe821ce198e3e699f06bf047449b76642d5d8e4931f0d7bbcd1aab328fe19`. `cmp`
exited 0. This is consumer **tolerance evidence**, not validity or privacy
acceptance. Raw uncropped decode was not tested.

Consumer executable:
`/nix/store/1fj0wg21ba24hv612yg4kqwzxbnyappm-imagemagick-7.1.2-29/bin/magick`.
Its recorded closure includes libheif 1.23.1 at
`/nix/store/jvhqjkdpin2ddnxprvrqa9jzg7dqmg26-libheif-1.23.1-lib` and libde265
1.1.1 at `/nix/store/sych5ppz29934xx2pm7qkqp6plhfgr25-libde265-1.1.1`.
Encoding-side closure includes x265 4.2. These are shared implementation-family
consumers, not independent HEVC grammar oracles. nixpkgs revision is
`d6524aaca2ff07876657ae2b323f24be4874944b`.

## Reproduction and retained evidence

Reuse the existing pinned witness output; choose a fresh gitignored output path
(the diagnostic rejects an already-existing directory):

```sh
devtool run -- python3 -B testdata/image-metadata-sanitizer/verify_heif_coded_boundary.py \
  .xtask/image-metadata-witness/fixtures .xtask/heif-coded-boundary-checkpoint-review
devtool run -- /nix/store/1fj0wg21ba24hv612yg4kqwzxbnyappm-imagemagick-7.1.2-29/bin/magick \
  .xtask/heif-coded-boundary-checkpoint-review/private-vcl-suffix.heic -depth 8 \
  RGBA:.xtask/heif-coded-boundary-checkpoint-review/suffix-display.rgba
devtool run -- /nix/store/1fj0wg21ba24hv612yg4kqwzxbnyappm-imagemagick-7.1.2-29/bin/magick \
  .xtask/image-metadata-witness/fixtures/candidate/device-like.heic -depth 8 \
  RGBA:.xtask/heif-coded-boundary-checkpoint-review/candidate-display.rgba
devtool run -- cmp .xtask/heif-coded-boundary-checkpoint-review/candidate-display.rgba \
  .xtask/heif-coded-boundary-checkpoint-review/suffix-display.rgba
```

Worker diagnostic report: `.xtask/heif-coded-boundary-checkpoint/framing.json`.
The following are **historical execution IDs**, not a promise of retained logs.
`devtool run` parks `.out`/`.err` under `.xtask/run/`, then prunes history; some
listed files were already absent during review. The commands above and concise
controller reproduction record below remain the reproducible evidence.
Gitignored reports, review packets and Nix roots are local artifacts, not
permanent evidence availability guarantees.

| Command                                             | Exit | Log stem                |
| --------------------------------------------------- | ---: | ----------------------- |
| Initial actual box/NAL inspection                   |    0 | `1791476629769-3884724` |
| Original/candidate hashes and residual strings      |    0 | `1791476693881-3896992` |
| Initial clean-tree status                           |    0 | `1791476846901-3914443` |
| Guessed ITU URL fetch (HTTP success, wrong content) |    0 | `1791476858995-3915414` |
| Failed wrong-content extraction, timeout 3600       |    1 | `1791476881317-3917111` |
| Official index fetch                                |    0 | `1791476998326-3923177` |
| Official edition fetch                              |    0 | `1791477005356-3924585` |
| Correct PDF fetch                                   |    0 | `1791477014545-3926415` |
| PDF signature/hash check                            |    0 | `1791477031197-3928470` |
| Correct extraction, timeout 3600                    |    0 | `1791477042368-3930482` |
| Pinned tool/revision evaluation                     |    0 | `1791477143662-3938800` |
| Diagnostic construction/inspection                  |    0 | `1791477306897-3944721` |
| Control display decode                              |    0 | `1791477314406-3944766` |
| Candidate display decode                            |    0 | `1791477320783-3944805` |
| Display RGBA comparison                             |    0 | `1791477325918-3944835` |
| Consumer closure inspection                         |    0 | `1791477431592-3945721` |
| Display dimensions/channels                         |    0 | `1791477440934-3945798` |
| Hash/extent/unchanged-input summary                 |    0 | `1791477449445-3945859` |

An exploratory store file census exited 2 due inaccessible entries
(`1791476647888-3889670`); no domain result or packaging proof is inferred from
it. The first non-silent HTTP invocation was blocked by tool routing before
execution; all actual fetches used file output with error diagnostics retained.

### Independent controller reproduction record

The controller independently inspected both files and the cited primary clauses,
then repeated the diagnostic and the exact pinned consumer/comparison commands
above using a fresh `.xtask/heif-coded-boundary-controller/` directory. The
following concise record is retained here because runner logs are prunable:

| Operation               | Actual exit | stdout bytes | stderr bytes | Historical execution ID |
| ----------------------- | ----------: | -----------: | -----------: | ----------------------- |
| Diagnostic construction |           0 |          183 |            0 | `1791477868158-3965451` |
| Control RGBA decode     |           0 |            0 |            0 | `1791477876810-3965712` |
| Candidate RGBA decode   |           0 |            0 |            0 | `1791477888228-3966911` |
| RGBA `cmp`              |           0 |            0 |            0 | `1791477908870-3968142` |

Controller byte comparison independently confirmed only original offsets
106/549/557 changed, within the three declared four-byte length fields, plus 48
appended bytes. The original NAL prefix at offsets 558–617, parameter sets and
rendering properties remained unchanged. The 665-byte control hash and identical
3072-byte displayed RGBA hash match those recorded above. Both source files
remained unchanged. This repeats tolerance evidence only; it does not measure
decoder byte consumption or certify conformance/privacy.

An ancillary `devtool.run.history_prune` warning during controller Git status
was observed with child exit 0; it was not suppressed, counted as an image
rejection, or interpreted as evidence that a gate was broken.

## Exact remaining decision

The observed framing/header approach and tolerant consumer cannot certify
coded-data ownership/termination. This does **not** prove every HEVC metadata
sanitization approach impossible. Before continuing, the owner must authorize a
source-backed bounded entropy-syntax/termination proof, or select another
independently proved ownership boundary. No unparsed remainder may be labelled
safe pixels and no original fallback is acceptable.

No complete VPS/SPS/PPS/slice-header privacy parser, metadata rewriter,
independent full graph/output verifier, admission/rejection matrix or sanitizer
CLI domain controls was delivered. Unknown NAL/SEI, brands, graph/properties,
orientation, ICC/HDR, alpha/aux/grid/thumbnail variants have **no admission**
from this checkpoint. No partial sanitization output is published.

Existing helpers, `verify.py`, `verify_controls.py`, `probe.nix` and product
code are unchanged. Existing JPEG 892/181 and other accepted witnesses were not
modified or rerun; no full focused corpus build or broad gate was run after the
approved stop. Representative licensed non-personal device-native originals,
production resource/deadline/cancellation isolation, supported-platform proof
and ingress integration remain independent barriers. No stage/commit/push or
Task 1/native/runtime/platform/general-HEVC/HIF readiness claim is made.
