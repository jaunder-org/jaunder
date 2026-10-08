# #1702: bounded HEVC syntax ownership and HEIF byte-copy rewrite

## Status and scope

This continues the
[coded-boundary checkpoint](2026-10-09-issue-1702-heif-coded-boundary-checkpoint.md).
Its header/framing-only approach failed: a private VCL suffix survived while
decoding identically. That is not an impossibility result for syntax/entropy
validation. This tranche adds an actual syntax-only reader and an actual bounded
container rewriter, with independently authored arithmetic/bitstream controls
and native test-side observations.

**This is a constructed, owned, closed-layout research tranche, not Task 1
completion, general HEVC/HEIF support, normative conformance certification,
device-native acceptance, or ingress/runtime/platform readiness.** The original
libheif writer layout remains unsupported. Licensed, non-personal device-native
originals are still required separately. Generated CC0 artwork and constructed
containers do not replace them.

No production Rust, backend, storage, route, dependency-injection, or public API
integration is included. The earlier helpers and verification calls remain
unchanged; `probe.nix` adds the new focused proofs after the existing calls.
Nothing was staged, committed, or pushed.

## Implementation and ownership boundary

- `hevc_headers.py` reads VPS/SPS/PPS/slice syntax and exact trailing bits;
  unknown extensions and unproved optional parameter shapes fail closed.
- `hevc_cabac.py` reads arithmetic decisions, bypass symbols, and termination.
  Its boundary is semantic bit consumption, not native byte prefetch.
  `hevc_intra_syntax.py` reads CU/intra/SAO/transform/residual syntax, including
  coefficient positions, levels, and signs, with fixed syntax-neighbour state
  and bounded scalar arrays. It performs no sample prediction, dequantization,
  inverse transforms, filtering, pixel reconstruction, or native decoder calls.
- `validate_hevc.py` is a private four-NAL JSON test adapter, not a public API.
  It publishes no result until complete validation. Only documented image-domain
  failures exit 2; tool, I/O, programming, and instrumentation failures are not
  successful image rejections.
- `rewrite_heif.py` validates the typed item/property/reference graph, exact
  physical extent ownership, descriptive schemas, and the entire retained codec
  syntax before constructing output. It copies the complete primary coded extent
  and rendering-bearing property/configuration bytes unchanged. It removes
  descriptive payloads, descriptors, names, and references physically as well as
  logically, rebuilds parent sizes and item locations, and emits neutral
  strings. It never repairs the original writer's array headers, reconstructs
  pixels, reencodes, or falls back to the original.
- `heif_metadata.py` admits only bounded known Exif/XMP descriptive forms and
  neutral technical defaults. Unknown rendering schemas, non-neutral Exif
  orientation, DTD/entities, non-XMP processing instructions, extra IFDs,
  unowned nonzero TIFF data, and unproved encodings fail closed. A metadata
  label alone does not authorize discarding unfamiliar contents.
- `verify_heif.py` is a separately authored observer with no rewriter import.
  The SAME complete observer verifies positives and all sensitivity mutations;
  checks are not selectively disabled to demonstrate a failure.

The frozen HEVC shape is one 64×64, 8-bit 4:2:0 IDR_N_LP I picture, profile 3,
compatibility `0x70000000`, main tier, level 30, one layer/temporal sublayer, no
inter-reference pictures, tiles, WPP, PCM, scaling lists, transform skip, range
extensions, or extra slice-header bits. MinCB/CTB are 8/16; min/max TB are 4/16;
declared inter/intra hierarchy depths are 0/1. Actual CU and TU splitting are
rejected: exercised luma/chroma transforms are 16/8. SAO is enabled; PPS initial
QP is 26, sign hiding and depth-0 CU QP delta are enabled; slice QP is 22 with
luma/chroma SAO enabled. CU QP delta admission is −25 through +25.
`hevc_headers.py` and the actual-validator mutation matrix define the remaining
exact flags; this paragraph does not authorize broader branches.

The container envelope is internal direct single extents (`iloc` v0/v1), one
`hvc1` primary, known Exif/RDF-XML descriptive items and rooted `cdsc`
references, and closed `hvcC`/`ispe`/`nclx`/`pixi` properties with the exercised
clean aperture, rotations, and mirrors. Associations bind by typed IDs, not
array position. Unknown boxes/properties/NAL arrays, derived or auxiliary
images, external/constructed locations, overlapping or unowned `mdat` bytes, and
non-default reserved fields remain unsupported. ICC/HDR and additional layouts
are not silently admitted. The code's byte, box, extent, metadata, and symbol
budgets are prototype bounds, not production resource/deadline proofs.

## Independent evidence

Full focused command, actual terminal exit 0, 360,908 ms, timeout 3,600 seconds:

```sh
devtool run -- nix build --impure --out-link .xtask/image-metadata-witness --print-out-paths --file testdata/image-metadata-sanitizer/probe.nix
```

Parked run: `.xtask/run/1791492409746-488402.{out,err}`. Immutable result:

```text
/nix/store/d1np99r02fk1wsd46n35lq4zly3dbn9l-issue-1702-image-sanitizer-feasibility-witness
```

Reports `reports/hevc-ownership/ownership.json` and
`reports/heif-rewrite/heif-proof.json` carry per-case assertions, hashes, typed
diagnostics, and native observations. All earlier focused witness calls
completed in the same derivation.

### HEVC

63 positives, 39 actual-validator typed negatives, and 1,008 independently
authored arithmetic controls passed. Arithmetic controls span every reachable
state/polarity/decision and all four quantized-range classes, plus bypass and
termination. Authored streams exercise zero/DC residuals, coefficient levels
2/3/7/8/17, negative level 17, hidden sign, every signed CU QP delta −25…+25,
and SAO BO/EO boundary offsets, band, and class. Headers are bit-authored, not
an observed RBSP constant.

Native libde265 observations agree with CTU semantic bit endpoints, CU/residual
shapes, and scalar/position/sign coefficient hashes; signed QP census also
agrees. Original census: 472 bins, 456 complete RBSP bits, 16 CTUs/CUs, seven
residual transforms, 26 coefficients; coefficient SHA256
`5990a315246d5ae2110c18d200dd092c8429f7c07ca75b57a669bd7efced678a`. Both the
actual private suffix and a distinct suffix fail semantic ownership, without
marker matching. Missing terminal stop, nonzero alignment, CU/TU split gates,
parameter shape/flag changes, level/compatibility mismatches, and truncations
fail the real CLI with exit 2 and no success result.

Native tracing shares the adapted syntax family and is **not** a normative
oracle. `hevc_normative_model.py` supplies the separate H.265-authored
arithmetic and codeword axis; native traces supply actual implementation
observations. Neither axis alone establishes general conformance.

### HEIF

17 positives, 38 typed negatives, seven SAME-observer sensitivity mutations
passed. Positives exercise metadata removal, different typed IDs,
metadata/reference and physical ordering, rooted reference chains, reserved
padding, separate-process determinism/idempotence/input preservation, direct
`iloc` v1, and supported transformation combinations. Strict libheif 1.23.1
consumes real raw/display RGBA before and after; 64×64 raw and 32×24/24×32
displayed output are identical. An independently authored rotation/mirror model
matches pinned native behavior. Supported opaque cases have no alpha. Real
auxiliary-alpha input exposes varying decoded alpha and is rejected, not
flattened.

Consumer-valid unsupported controls include the original owned writer layout,
auxiliary alpha, private prefix SEI configuration, unknown properties and
rendering metadata, and non-neutral Exif orientation. No consumer tolerance is
substituted for syntax/graph ownership. Sensitivity cases detect leaked
names/items/physical payloads, changed coded bytes or rendering properties,
incorrect IDs, and locations using the complete verifier.

Original 4,110-byte input remains SHA256
`32ef665e9e89f18e91985c79d8a1984b5375eec33e82be20137637ebb109c994`. The
constructed base input is SHA256
`222dcf7ada88fc5bc93d93de16b9356d844f37469b744adb7435c9bfff5369b2`; its
canonical output is 479 bytes, SHA256
`56f53de69c61c3e0bb1b9c9ce29e1d0401f5dcedfb6d8c29bcda8f78295f9179`. All 63
primary compressed bytes and all five property byte strings/associations remain
identical; coded-extent SHA256 is
`7e182f80de852f3219b82edb65159e3aab4388b184b7c6c3b43bf74ebd20e525`. Descriptive
items 7/9, payloads, names, and references are absent in output; primary ID 5
remains correctly bound.

## Primary sources, adaptations, and limitations

ITU-T H.265 V9 (09/2023),
[official PDF](https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-H.265-202309-S!!PDF-E&type=items),
SHA256 `30b1f2dc016e6f6f2b76d75e19ac3fabffbee0c06c0e7345cf88fa9ebf713269`:
parameter/slice/residual syntax in §7.3, semantic termination in §7.3.8.1,
arithmetic processes/tables in §9.3, terminal-stop interpretation in §9.3.4.3.5,
and encoder flush Figure 9-15. The normative model is authored from these facts,
not imported from production tables. Standards archives/extracted text stay
gitignored, not redistributed here.

Adapted constants and bounded intra/residual traversal retain provenance,
attribution to struktur AG/Dirk Farin/Min Chen, modification notes, and
LGPL-3.0-or-later notices. `HEVC-LGPL-3.0.txt` contains the LGPL text;
repository `LICENSE` supplies GPL text. Exact libde265 1.1.1 source NAR is
`sha256-ZHfPC86oylqt2bwWMJRWVjdMEEmX6UOKR7XkR0HPyok=`, rooted at
`.xtask/hevc-libde265-source` (store
`/nix/store/rpbjbvsk47na5zgi7g91xx1fr09zyv4w-source`). Native instrumentation is
test-only, fail-loud on nonunique seams, and changes observations rather than
parsing/reconstruction. `dec265` omits threading flags, relying on documented
default `nThreads=0`.

**Native delta-range discrepancy:** proof05 failed on authored −26, retaining
real `coded parameter out of range` and CTB concealment warnings; one
reconstructed frame was not accepted. H.265 V9 §7.4.9.14 (not the older clause
number in upstream's comment) permits −26…+25 at 8-bit. Pinned
`slice.cc:3630–3645` reads sign, then incorrectly bounds absolute delta at 25
regardless of sign. Preguard trace reports `HEVC_QP 0 0 26 1 -26`. No warning or
decoder logic was suppressed/changed. All 51 admitted delta states were
independently authored and warning-free before narrowing. −26 is explicitly
**unsupported by this native-proved envelope, not proven malformed**; −27/+26
violate the normative scalar range. Recovery
`.xtask/recovery/hevc-minus26-1791490700` preserves the failed proof and partial
source; native diagnostic run `1791490848280-364379` preserves the original
warnings and trace.

**Configuration-record evidence remains incomplete:** exact libheif 1.23.1
`libheif/codecs/hevc_boxes.cc:79,157,374` reads/writes completeness in bit 6;
original array headers are `0x60/0x61/0x62`. Constructed fixtures deliberately
select `0xa0/0xa1/0xa2` while retaining original NALs. That is test
construction, never upload repair or relabeling of the original as conformant.

The [official MPEG Part 15 page](https://www.mpeg.org/standards/MPEG-4/15/)
links MPEG155, published 2026-08-11,
[w26537](https://www.mpeg.org/wp-content/uploads/mpeg_meetings/155_Geneva/w26537.zip),
draft eighth edition. Verified ZIP SHA256
`9cf59684819946e86fc1b6c6e5bc3c0702ca2ca37c87e07160f6440f35121556`; the
inspected text is amendment-style and does not establish the record bit layout.
It is an ongoing draft, not an accepted published standard.

Official MPEG141
[w22325](https://www.mpeg.org/wp-content/uploads/mpeg_meetings/141_OnLine/w22325.zip),
sixth-edition draft DAM2, verified ZIP SHA256
`f5cb4f96a94909944c15a0b1831cceefc569a2c436d0475e8385638d72c86e84`, supplies
§8.3.2.1.3 completeness semantics, §8.4.1.1.1 mandatory completeness 1 for
`hvc1` parameter arrays, and explicit prefix-SEI-in-configuration semantics. It
also quotes/proposes §8.3.2.1.1 record level ≥ parameter levels, motivating the
level-30 freeze. It does **not** establish bit position. Therefore no
published-edition record conformance or standards-malformed classification of
the original writer is claimed. Both archives passed signature/CRC checks;
extracted DOCX stays under `.xtask/part15-extracted`.

## Failure preservation and handoff

Earlier nonunique instrumentation, no-WPP threading warning, normative
transcription error, wrong source-output rooting, original array rejection,
split-output linker failure, encoder spawn failure, and navigation/provider
failures remain distinct recovery checkpoints under `.xtask/recovery/`. No
infrastructure failure was reclassified as image-domain rejection. The encoder
retry realized only the same pinned libheif 1.23.1 `^bin` output. HTTP
interception was a pre-execution refusal; subsequent file-output fetches used
the supervisor-approved MCP-unavailable escape, not an undisclosed transport
substitution.

Current flake/gate Prettier is verified 3.9.6, matching
`/nix/store/ibqbhja47bwv6a0pr6swj4ldpnrhqb9f-prettier-3.9.6/bin/prettier`; the
historical AGENTS version anecdote is not a substitute pin. No ambient nixfmt,
wrapper, package installation, or broad gate was used.

Parent independent reproduction, source review, and integration acceptance
remain required. Continue #1702 after this tranche: resolve the published record
syntax, acquire licensed non-personal device-native fixtures, widen syntax only
through independently authored/native proofs, and separately prove production
resource bounds, runtime/platform behavior, ingress error mapping, and
no-original-storage/fallback behavior. This checkpoint is neither Task 1
completion nor a new user/operator gate.

## Review correction: independent residual coverage (P1/P2)

The original 63-positive/39-negative proof above was reproduced by the parent,
then **blocked** by Spec reviewer f1b9c7dc: luma DC/level variations did not
independently cover the admitted residual grammar. Native agreement shared the
adapted family and could not fill that gap. Standards reviewer a13b5175
requested `tt` → `transform_tree`; all method references now use that spelling,
with no traversal behavior change. These corrections require the SAME reviewers
to resume; this note does not self-clear either review or Task 1.

`hevc_residual_vectors.py` is an independently authored semantic-map-to-codeword
model, with no actual/native residual, scan, or context implementation import.
Its inputs are signed coefficient coordinate maps, not observations extracted
from the adapted decoder. Explicit 4×4/2×2 diagonal orders and coordinate prefix
codebook, algebraic map projections, and primary equations define expected
syntax tokens and scalar maps. Numeric I initialization facts are separately
transcribed from H.265 V9 Tables 9-28/29; arithmetic uses the earlier
independent encoder/tables. Native output is corroboration, never the
expected-map oracle.

### Frozen reachable matrix

| Obligation                                                                                                  |                  Independently declared named vectors | Primary basis                            |
| ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------: | ---------------------------------------- |
| All luma16/Cb8/Cr8 last coordinates, x/y prefix and suffix edges                                            |                                                   384 | 7.3.8.11, 7.4.9.11, 9.3.4.2.3            |
| Group neighbours, coded/uncoded/inferred groups, inferred/explicit DC, significance masks/buckets and flags | 96 neighbourhood patterns plus coordinate/count cases | 7.3.8.11, 9.3.4.2.4–5, equations 9-35…55 |
| Counts 1…16 in every group, including 8→9 base-level change, multiple groups                                |                                                   144 | 7.3.8.11                                 |
| First greater1 position 0…7, greater2 0/1, c1 reset/saturation and previous-group sets                      |                   48 plus 6 origin-late-greater cases | 9.3.4.2.6–7, equations 9-56…62           |
| Rice k0…4, every precision-reachable unary prefix, suffix low/high edges                                    |                480 plus 3 saturation/count9 sequences | 9.3.3.11, equations 9-24…28              |
| Positive32767/negative32768 precision edges                                                                 |                                                     6 | 7.4.9.11                                 |
| Explicit gap3/hidden gap4 and gap15, odd/even parity, signed values                                         |                                                    18 | 7.3.8.11, 7.4.9.11                       |
| Y→Cb→Cr shared-context ordering                                                                             |                                                     1 | 7.3.8.10–11, 9.3.4.2.5–7                 |

The independent census is **1,186 names**, separately declared in
`required_names()`, not taken from the producer's dictionary keys. Required
coverage is 511 branch/context/value labels; 550 are observed by normative
projection. Last-position maps themselves also have coordinate/census
assertions.

Directional scans are **unreachable**, not alleged tested-supported branches:
H.265 V9 7.4.9.11 permits them for intra luma TB8 or TB4/chroma TB4. Actual
header plus CU/TU no-split guards admit only luma TB16 and chroma TB8, so every
intra mode yields diagonal scan. Generic horizontal/vertical, TB4,
luma8/chroma16, transform-skip and width1 significance paths were removed or
closed at actual entry points. Direct-call controls reject non-diagonal/other
component-size/Rice5 states before bit, bin, or transform counters change. No
new layout was admitted. At signed15-bit precision, Rice remains0…4, unary
prefix is at most17−k and suffix at most14 bits; longer prefixes are rejected
before precision acceptance. Scalar values between edges use the same bounded
bypass primitives already checked by the 1,008 independent arithmetic controls,
not an opaque remainder.

For EVERY new vector, the actual validator CLI and real callable complete report
agree under JSON tuple/list normalization. The real callable's exact decision
name/context/value, bypass and termination sequence equals the independent
codeword token sequence. Actual coefficient count/hash AND native coordinate/
value/sign trace independently equal the declared semantic map; native warnings,
errors, recovery and nonzero exits still fail. The owned original remains
supported through the current grammar; all old positives, private/distinct
suffix rejections, negative controls and HEIF complete-verifier/raw/display
checks are retained. Its native-family agreement is not relabeled an independent
oracle: the new primary-authored branch/codeword axis is the correction.

### Corrected proof and sensitivity

Focused proof03 actual exit0, 68,374 ms:

```sh
devtool run -- python3 -B testdata/image-metadata-sanitizer/verify_hevc_ownership.py .xtask/image-metadata-witness/fixtures .xtask/hevc-residual-proof-03 .xtask/hevc-native-consumer-v3/bin/dec265 testdata/image-metadata-sanitizer/validate_hevc.py
```

Exactly one serial full focused Nix proof after corrections, timeout3,600,
terminal exit0, 461,253 ms, parked `1791497044093-707754.{out,err}`:

```sh
devtool run -- nix build --impure --out-link .xtask/heif-residual-witness-01 --print-out-paths --file testdata/image-metadata-sanitizer/probe.nix
```

Immutable output:
`/nix/store/1yw647a91prqh014wc3vzvngi6z41r84-issue-1702-image-sanitizer-feasibility-witness`.
HEVC **1,249 positive (all63 old +1,186 new), 42 typed negative (all39 old +3
precision/prefix), 1,008 arithmetic**, and **14 sensitivity/guard controls**.
HEIF remains **17 positive/38 negative/7 SAME-observer mutations**, and original
SHA256 remains
`32ef665e9e89f18e91985c79d8a1984b5375eec33e82be20137637ebb109c994`.

`reports/hevc-ownership/independent-residual-matrix.json` preserves every
expected signed coordinate map, normative token sequence, case coverage and
required coverage. `ownership.json` preserves actual/native per-case
observations and exact sensitivities. Mutating the REAL callable's last
position, significance context, remainder scalar, omitted residual or greater1
state fails the same complete assertions (domain desynchronization is detection,
not successful mutant decoding). Bad independent oracle, omitted coordinate
vector AND omitted Rice edge cannot remain green. Six actual entry-point
controls prove unreachable gates before consumption. JSON normalization
regression accepts nested tuple/list wire equivalence, rejects changed
shape/value, string count and extra field; it neither drops keys nor supplies a
semantic oracle.

Failures remain failed evidence: first branch assertion missed origin-only GT1
contexts2/3 positive, recovered in
`.xtask/recovery/heif-residual-coverage-1791496390` before adding
preceding-all-one / origin-late-greater maps from equations9-56…59. A
structured-edit nonunique-seam refusal occurred before mutation and was retried
only with the parent-approved exact unique anchor. Focused proof01 then caught
tuple-vs-JSON-array report comparison, preserved full source/failed proof/log in
`.xtask/recovery/heif-residual-wire-1791496650`; exact complete-report wire
normalization and mismatch regression fixed that programming failure.
Preliminary callable maps were not reported as full CLI/native acceptance. No
diagnostic suppression, new provider/tool/pin, defaults substitution, pixel
reconstruction, reencode, native sanitizer dependency, original fallback or
guarantee relaxation was introduced. LGPL notices/provenance remain intact.
Published ISO record bit layout, device-native fixtures and production readiness
remain separate gaps.
