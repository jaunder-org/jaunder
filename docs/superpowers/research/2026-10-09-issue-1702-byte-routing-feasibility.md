# #1702 Task 1 byte-routing feasibility

Date: 2026-10-09. Test-side research only; Task 1 remains incomplete.

## Scope and result

`testdata/image-metadata-sanitizer/route_image.py` distinguishes identified
families requiring validation, verified private family output, confirmed SVG,
opaque non-image passthrough, and typed claimed/plausible-raster rejection.
`verify_image_routing.py` declares independent static expectations and invokes
both the actual callable and actual CLI. `probe.nix` adds one proof call after
all accepted family witnesses; their sources, grammar and call bodies remain
unchanged. No production service, ingress, runtime packaging or Task 2 work.

The corrected focused proof passed: 765 static goldens, 108 real positive
fixtures, three omitted/misleading-label variants per positive, 821
sensitivities and 11 failure/guard cases, plus three real unsupported-framing
controls. The complete focused Nix witness passed, retaining the unchanged
independent family privacy/fidelity and native test-side consumer proofs. Both
original review axes had blocked the prior 374-case proof with two P1 findings;
the implementation and evidence corrections below await the SAME reviewers'
explicit disposition. Passing counts do not self-clear either P1 or complete
Task 1.

## Frozen routing decisions

| Byte evidence                                                                                 | Identified family              | Canonical MIME after actual successful rewrite |
| --------------------------------------------------------------------------------------------- | ------------------------------ | ---------------------------------------------- |
| Initial `FF D8`                                                                               | JPEG                           | `image/jpeg`                                   |
| Initial `89 50 4E 47` (`89PNG`)                                                               | PNG, including APNG            | `image/png`                                    |
| Initial `GIF8`                                                                                | GIF                            | `image/gif`                                    |
| Initial `RIFF`, `WEBP` at offset 8                                                            | WebP                           | `image/webp`                                   |
| Complete bounded `ftyp`, major `heic`, only `heic`/`mif1`/`miaf` brands                       | HEVC/HEIC requiring validation | `image/heic`                                   |
| Complete bounded `ftyp`, major `mif1`, explicit compatible `heic`, only the same three brands | HEVC/HEIC requiring validation | `image/heic`                                   |

Recognition is **not admission**. The actual accepted HEIF rewriter still
requires the exact `heic`, minor-zero, ordered `mif1/heic/miaf` record and its
closed container/HEVC grammar. A recognized `mif1` major may therefore fail.
There is no generic successful `image/heif` mapping and no repair of original
`0x60/0x61/0x62` configuration arrays. Constructed `0xa0/0xa1/0xa2` fixtures
remain research constructions, not device-native or published conformance proof.

Evidence discovery traverses complete aligned top-level boxes with ordinary
32-bit, extended 64-bit and to-EOF extents. Complete unknown leading boxes are
skipped by their declared extent, so a later known image `ftyp` cannot escape
via opaque passthrough. Generic non-image `ftyp` records do not stop discovery
of a later image record. These are recognition-only framing forms: the closed
actual image helpers still reject unsupported leading/extended/EOF layouts;
there is no repair or admission expansion.

Generic structural invalidity is not raster evidence. Extended/to-EOF/truncated
`isom` records, bare `free` padding and literal text `1234free` remain opaque
non-image under neutral or SVG labels. Raster MIME/filename claims still reject.
Malformed `ftyp` with a complete known-image major or compatible brand at its
aligned field rejects, including physically available brands in a truncated
record. A short major/compatible prefix such as `hei` alone is ambiguous. For
undersized framing, only an available complete major field establishes strong
evidence, not hypothetical subsequent compatible fields. A complete extended
header establishes its own 16-byte field offsets; an incomplete extended header
does not establish a brand field.

Complete framing advances by at least eight physical bytes; brand inspection
uses only four-byte slices and fixed booleans, never a list/set of arbitrary
unknown brands. Work is bounded by the already bounded physical input length,
not an image admission limit. Discovery therefore continues after more than
65,536 generic records or a generic brand list larger than 64 KiB. Once image
evidence appears, an image `ftyp` beyond the record admission bound or with an
oversized/malformed brand list rejects. There is neither limit-exhaustion to
non-image fallback nor unconditional generic-media rejection. The fixture file
cap remains 32 MiB. These are prototype work/accounting guards, not Task 2
allocation/deadline/resource proof.

No payload/interior scanning or guessed resynchronization is performed. An
unknown box's payload, including an apparent inner `ftyp`, remains opaque; a
to-EOF unknown box owns the remaining bytes. Invalid generic framing without
aligned image evidence stops boundary traversal and stays opaque absent raster
claims. This is preservation of existing behavior, not verification of safety.

Known unsupported image evidence includes `heix/hevc/hevx/heim/heis/hevm/hevs`,
`mif1` without explicit `heic`, `msf1`, `avif/avis`, and unsupported
combinations including image brands beneath a video major. `miaf` alone does not
prove HEIC. The registration-derived reject-only additions below prevent known
image containers from escaping through generic `ftyp`. None receive a canonical
MIME or a verified result. Unknown truly non-image brands, including ordinary
`isom/mp42` without image evidence, remain opaque non-image.

Raster byte evidence wins over SVG labels and names. Aligned strong prefix cases
are explicit, not fuzzy magic matching: exact PNG first four bytes plus a
damaged/missing tail and JPEG SOI plus invalid following structure request
actual family validation and reject if invalid. `RIFF[length]WEB` ending at that
marker is strongly evidenced truncated WebP and rejects. `RIFF` alone, WAV/AVI
forms, `FF` alone, `89PN`, `GIF`, `RIFF[length]WE` and PNG's last four bytes
alone are ambiguous without a raster claim and remain non-image. No single-byte
heuristic or arbitrary interior-signature scan is promised. Conflicting initial
evidence is routed by the aligned initial family, whose complete validator
rejects the invalid structure; there is no second-family or original fallback.

For bytes without recognized/plausible raster evidence, unknown `image/*` MIME
is a raster claim, excluding only `image/svg+xml`. MIME comparison uses the
case-folded, trimmed base type before parameters; this covers `image/pjpeg`,
`image/x-png`, HEIC/HEIF sequence aliases, TIFF, AVIF and unknown image
subtypes. The case-folded final filename suffix claims raster for
`jpg/jpeg/jpe/pjpeg/png/apng/gif/webp/heic/heif/heics/heifs/hif` and the common
unsupported suffixes `bmp/dib/tif/tiff/avif/avifs/ico/cur/jp2/j2k/jpf/jpx/jxl`.
Other arbitrary suffixes do not create an attachment allowlist. Labels are
claims, never byte-family proof. Recognized bytes override all labels, even
raster or SVG labels, but the original filename string is returned unchanged.
Opaque/SVG passthrough preserves original bytes and supplied MIME exactly.
Filename privacy/normalization is outside this task.

SVG confirmation requires the first root name to be unnamespaced legacy `svg` or
`{http://www.w3.org/2000/svg}svg`, with a literal default/prefixed namespace
binding. A bounded UTF-8/BOM lexical inspection skips declarations, processing
instructions, comments and DOCTYPE with quote/internal-subset tracking; it never
expands entities, resolves external resources or decodes pixels. DTD and
entity-bearing ordinary SVG remains unchanged. Foreign namespace SVG, nested SVG
under HTML, SVG labels alone, entity-derived namespace bindings, unsupported
encodings such as UTF-16 and unestablished roots do not confirm SVG. With no
raster evidence/claim these remain opaque non-image rather than acquiring a new
generic rejection rule. This is not full XML validity, SVG sanitization or an
SVG security guarantee.

## Authoritative registration evidence, not normative codec proof

Owner: MP4 Registration Authority (MP4RA). Official registry surface:
https://mp4ra.org/registered-types/brands. Actual first-party registry source
retrieved 2026-10-09:
https://raw.githubusercontent.com/mp4ra/mp4ra.github.io/main/data/brands.csv.
Saved raw CSV: `.xtask/task1-mp4ra-brands.csv`, SHA256
`137cb40ebf58fd9c39981c8c4ac62a8420ed8b93a4a4b7d034d2b66cb7e200e0`. The source
URL's branch is mutable; the frozen raw-byte hash and evidence packet identify
precisely the registration snapshot used. There is no online lookup in the
prototype. The parent independently inspected this same snapshot and approved
the reject-only set before implementation.

The following are registered descriptions/specification labels, not claims to
have read the underlying standards. HEIF-coded image/sequence registration is
distinct from unrelated image collections/profiles such as JPSearch or JPEG XR.
All 42 additions reject only; every one has major and compatible placement
controls under omitted, video and SVG labels. Exact brand case and trailing
spaces are significant; `$20` below denotes one actual ASCII space.

| Exact brand ($20 = space) | Registered description                                                                              | Registry specification label |
| ------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------- |
| `1pic`                    | Single intra-coded picture                                                                          | HEIF                         |
| `avci`                    | AVC image and image collection brands                                                               | HEIF                         |
| `avcs`                    | AVC image sequence brands                                                                           | HEIF                         |
| `avio`                    | AV1 intra-only brand                                                                                | AVIF                         |
| `evbi`                    | EVC Baseline coded image                                                                            | HEIF                         |
| `evbs`                    | EVC Baseline coded image sequence                                                                   | HEIF                         |
| `evmi`                    | EVC Main coded image                                                                                | HEIF                         |
| `evms`                    | EVC Main coded image sequenc                                                                        | HEIF                         |
| `heoi`                    | OMAF HEVC image profile                                                                             | OMAF                         |
| `j2ki`                    | JPEG 2000 image and image collections in ISO/IEC 23008-12 files                                     | J2KHEIF                      |
| `j2ks`                    | Motion JPEG 2000 in ISO/IEC 23008-12 files                                                          | J2KHEIF                      |
| `j2is`                    | JPEG 2000 image sequence in ISO/IEC 23008-12 files                                                  | J2KHEIF                      |
| `J2P0`                    | JPEG2000 Profile 0                                                                                  | JPEG2000                     |
| `J2P1`                    | JPEG2000 Profile 1                                                                                  | JPEG2000                     |
| `jp2$20`                  | JPEG2000 Part 1                                                                                     | JPEG2000                     |
| `jpeg`                    | JPEG-specific still image brand                                                                     | HEIF                         |
| `jpgs`                    | JPEG image sequence brands                                                                          | HEIF                         |
| `jpm$20`                  | JPEG 2000 Part 6 Compound Images                                                                    | JPM                          |
| `jpoi`                    | OMAF legacy image profile                                                                           | OMAF                         |
| `jpsi`                    | The JPSearch data interchange format, for the exchange of image collections and respective metadata | JPSearch                     |
| `jpx$20`                  | JPEG2000 Part 2                                                                                     | JPX                          |
| `jpxb`                    | JPEG XR                                                                                             | JPXR                         |
| `jxl$20`                  | JPEG XL                                                                                             | JPEG XL                      |
| `jxs$20`                  | Still Image File Format for JPEG XS                                                                 | JPXS                         |
| `jxsi`                    | JPEG XS image and image collections for HEIF                                                        | JPXS                         |
| `jxss`                    | JPEG XS image sequences for HEIF                                                                    | JPXS                         |
| `MA1B`                    | AVIF Baseline Profile                                                                               | AVIF                         |
| `MA1A`                    | AVIF Advanced Profile                                                                               | AVIF                         |
| `MiAB`                    | Multi-Image Application format brand for MIAF AVC Basic Profile                                     | MIAF                         |
| `MiAn`                    | Mutli-Image Application format brand for animation                                                  | MIAF                         |
| `MiBu`                    | Multi-Image Application format brand for burst capture                                              | MIAF                         |
| `mif2`                    | Image file format structural brand CICP alpha and depth                                             | HEIF                         |
| `MiHA`                    | Multi-Image Application format brand for MIAF HEVC Advanced Profile                                 | MIAF                         |
| `MiHB`                    | Multi-Image Application format brand for MIAF HEVC Basic Profile                                    | MIAF                         |
| `MiHE`                    | Multi-Image Application format brand for MIAF HEVC Extended Profile                                 | MIAF                         |
| `MiPr`                    | Multi-Image Application format brand for progressive decoding and rendering                         | MIAF                         |
| `pred`                    | Image file format brand for predictively coded image items                                          | HEIF                         |
| `vvic`                    | VVC coded image item                                                                                | HEIF                         |
| `vvis`                    | VVC coded image sequence                                                                            | HEIF                         |
| `vvoi`                    | OMAF VVC image profile                                                                              | OMAF                         |
| `jaii`                    | JPEG AI coded image                                                                                 | JPEGAI                       |
| `tmap`                    | Tone-map derived image item present                                                                 | HEIF                         |

Unconfirmed proposed codes `j2im/j2km/unci` were absent from this snapshot and
are not asserted/added. `jxsc`'s generic codestream description does not
establish image-only rather than video scope. Neighboring video codes
`avc1/av01/hevd/hevi/hvci/hvce/hvcx/vvci/mj2s/mjp2`, ambiguous MIAF `MiAC/MiCm`,
`jxsc`, and incorrect-case/incorrect-space controls remain non-image without
other evidence. This is not a universal registry or parser-completeness claim.

An ancillary attempted `data/specifications.csv` source URL returned HTTP 404
(curl exit 22). No specification text was obtained from that guessed filename;
no retry/provider substitution or standards assertion was made. Its logs and
complete current source snapshot are preserved under
`.xtask/recovery/task1-routing-mp4ra-specifications-1791500540/`. Registration
claims rely only on the successfully retrieved actual `brands.csv`.

## Executable evidence and failure semantics

The independently named golden census is separately declared from the cases and
from detector tables. It includes every added brand in both placements and all
three label variants, video/non-image neighbors, SVG prolog/entity/namespace
cases, strong/damaged/truncated prefixes and raster claims. It also includes the
five immutable parent counterexample vectors with neutral/raster-claim/SVG
pairs; ordinary/extended/EOF framing, exact later image brands, width/extent/
truncation boundaries, no-interior/no-resync controls, record/brand admission
boundary/excess and image evidence appearing after excessive generic work. Every
registered reject-only brand is also tested behind a complete unknown leading
box in major/compatible placement. Deleting each of the 765 goldens fails the
whole verifier; 42 individually unrecognized registered brands and wrong
kind/family/MIME/filename/output mutations also fail it.

Seven actual parser source mutations run the SAME whole executable verifier on
private copied sources. Generic-invalid rejection, unknown-leading early stop,
extended/EOF early stop, record/brand-limit fallback and malformed-known-image
fallback each fail with an actual static-golden AssertionError. Their source
hashes, command/exit/stdout/stderr and complete partial invocation trees are
retained. No weaker replacement predicate or import/permission failure counts as
mutation sensitivity. The three real supported HEIF coded inputs wrapped with
unknown-leading, extended-ftyp and EOF-ftyp framing reach actual
recognition/rejection and cannot produce a successful original fallback.

The independent real-fixture inventory comprises 66 JPEG, 8 PNG/APNG, 7 GIF, 10
WebP and 17 bounded constructed HEIF positives. For each, direct actual family
rewriting supplies a dispatch-equivalence reference; callable and CLI outputs
under omitted/PDF/SVG labels must match it exactly. These references are not a
second independent metadata oracle. The unchanged family witnesses in the same
complete Nix build remain the independent privacy/fidelity proofs. Input byte
hashes, output hashes, exact filenames, canonical MIME, deterministic same-byte
label routing and already-rewritten idempotence are asserted.

Expected family failures become `RasterRejected` with the original typed cause;
the CLI exits 2 and retains unsuppressed causal diagnostics. Prototype PNG/GIF/
ICC generic `ValueError` is adapted only for their explicit domain prefixes;
arbitrary `ValueError`, `RuntimeError`, I/O/missing-output failures and errors
with partial private output propagate instead of becoming image-domain success.
The CLI missing-source proof exits 1 with its actual traceback. Existing-output
and same-input/output guards fail; accepted unsupported progressive JPEG,
original HEIF configuration and auxiliary-alpha fixtures cannot fall back via
SVG labels. Every child command, actual exit, stdout and stderr is retained in
the invocation-owned tree. Private partial outputs from injected failures are
retained as test evidence, not published or returned as successful results.

This routing prototype is **not** workspace ownership/cleanup/cancellation,
TOCTOU, deadline, allocator, isolation, production output-observer, quota,
identity, storage or platform proof. The validator is a trusted explicit
callable seam, not a sandbox; no public upload result is minted here.

## Reproduction and durable evidence

The original complete 55-file packet and its full diff were hash-verified before
edits, then copied with complete review/parent RED evidence into
`.xtask/task1-routing-review-baseline-01/`. The five immutable parent vectors
were rerun with only a fresh output-directory substitution: actual exit 1 (64
ms) on the original source, then actual exit 0 (117 ms) on the correction. The
original fixed-directory script/tree remains untouched. New independent
expectations first failed the SAME whole verifier, exit 1/89 ms. Complete RED
sources, trees and diagnostics are preserved under
`.xtask/recovery/task1-routing-evidence-red-1791502253/`.

A focused corrected run passed, but the first complete Nix attempt genuinely
failed exit 1/585,247 ms: `shutil.copytree` preserved read-only Nix-source
permissions in the test mutation copy, causing PermissionError. This was a test
infrastructure failure, not image-domain success. Complete partial fixtures/
reports, source and focused tree were preserved before correction at
`.xtask/recovery/task1-routing-evidence-nix-permission-1791503060/`. The
smallest correction makes only the invocation-owned copied mutation route file
writable; original/accepted sources are never chmodded or changed. The same
protocol was then rerun in fresh focused/full directories.

Final focused command (exit 0, 132,057 ms, empty wrapper stderr):

```text
devtool run -- python3 -B testdata/image-metadata-sanitizer/verify_image_routing.py .xtask/heif-residual-witness-01/fixtures .xtask/task1-routing-evidence-green-02 testdata/image-metadata-sanitizer/route_image.py
```

Final full focused command, run once serially after the final test-source
correction (exit 0, 543,687 ms, timeout 3600):

```text
devtool run -- nix build --impure --out-link .xtask/task1-routing-evidence-witness-02 --print-out-paths --file testdata/image-metadata-sanitizer/probe.nix
```

Final store output:
`/nix/store/i3k0rlmxmmghgp7qrrpglxapsnksq163-issue-1702-image-sanitizer-feasibility-witness`.
Routing report: `reports/image-routing/routing-proof.json`. Focused/full command
logs: `1791503096366-1017087` and `1791503235887-1052755`; durable copies live
in `.xtask/task1-routing-evidence-correction-packet-1791503920/`.

The earlier completed source and focused/full evidence before the reject-only
brand correction are frozen in `.xtask/task1-routing-original-completed/`. Their
full build passed exit 0/491,318 ms; it was neither cancelled nor silently
replaced. The subsequent source change and fresh proof were explicitly
supervisor-approved to close a known recognition gap.

The correction packet freezes all 55 helper/note sources and hashes, a complete
tracked plus untracked `full.diff`, and `correction.diff` against the verified
original 55-file packet. Only routing code, controls and this note changed in
this review correction; the existing additive probe call and all accepted
helpers are unchanged. Durable baselines, registry snapshot, complete focused/
full reports, actual commands/exits/diagnostics and source-mutant trees are
included. Old complete proof and failures remain preserved, never overwritten.
The separate record-layout note remains byte-identical at SHA256
`3cd90f79a639f6c0e413548d655291c9bcd3f1d8eaab5322db8ab9b2da94de98`. No staging,
commit, push, worktree, broad gate or production integration occurred.

## Remaining barriers

Parent reproduction and independent review remain required. Licensed
non-personal device-native originals, published ISO configuration-array syntax,
broader codec/layout/ICC/HDR/alpha/grid coverage and exact production
runtime/dependency/resource/platform closure remain unresolved. No Task 1
checkmark, general HEIF acceptance, published conformance or production
readiness claim follows from this bounded routing proof.
