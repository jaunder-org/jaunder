# MPEG accepted-edition source-map follow-up

**Research date:** 2026-10-09  
**Scope:** bounded official MPEG route research for an accepted/published
ISO/IEC 14496-15 HEVCDecoderConfigurationRecord array-header syntax excerpt.
This is research only; it changes no existing note, code, admission, spec, ADR,
or fixture/conformance decision.

## Result

Inspection of the official MPEG Part 15 index and its linked MPEG 135 page
identified no downloadable published/accepted base-edition or final-text syntax
source. This conclusion concerns those pages and displayed link metadata, not
the uninspected contents of every linked working document. Its edition index
labels Edition 4 (ISO/IEC 14496-15:2017) `published`, Editions 5–7 `released`,
and Edition 8 `ongoing`, but the linked material visible in the index is meeting
material: working drafts, draft amendments, technology-under-consideration
papers, and related meeting pages. No linked MPEG document in the bounded check
was an accepted/final Part 15 text containing the requested array-header field
widths/order/reserved value.

This is a bounded observed route result, not a claim that no legitimate source
exists globally. The exact bit layout remains unresolved; no current
conformance, malformed-writer, repair, or upload-admission claim follows.

## Official Part 15 index

- **Owner:** MPEG.
- **HTML title:** “Standards – MPEG”.
- **Body heading (H1):** “MPEG-4: Carriage of Network Abstraction Layer (NAL)
  Unit Structured Video in the ISO base Media File Format”.
- **Description:** MPEG-4 Part 15 standards index.
- **URL:** https://www.mpeg.org/standards/MPEG-4/15/
- **Retrieved:** 2026-10-09 with pinned curl using
  `--silent --show-error --fail --location --output`.
- **Raw artifact:** `.xtask/mpeg-part15-index-accepted-source-map.html`.
- **SHA-256:**
  `8117374582533b490c89182a1579521cc90217d815a7f007b940b822e73dcb92`.
- **Edition/status excerpt:** the index displays “Edition - 4: ISO/IEC
  14496-15:2017 [Edition 4]” with “Status: published”; Edition 5 with “Status:
  released” and an objective to integrate the 2017 AMD/COR material; Editions 6
  and 7 with “Status: released”; and Edition 8 with “Status: ongoing”. The page
  displays `Publication Year: 0` for those entries, so it is not a reliable
  publication-date source for those edition rows.
- **Source-link scope:** after the edition rows, the page has a “Meeting
  documents” section. The visible links are MPEG meeting pages and document
  archives, not a published ISO/IEC base/final text download. The page itself
  does not expose a syntax table or accepted-edition PDF.

## Bounded linked route: MPEG 135 meeting page

- **Owner:** MPEG.
- **HTML title:** “MPEG 135 – MPEG”.
- **Body heading (H1):** “MPEG 135”.
- **Description:** MPEG 135 meeting page.
- **URL:** https://www.mpeg.org/meetings/mpeg-135/
- **Why followed:** this is an actual meeting-page href exposed by the Part 15
  index's historical meeting-document navigation.
- **Raw artifact:** `.xtask/mpeg-135-index.html`.
- **SHA-256:**
  `e0837a08971e98ecf06656b05bbdf5ecd3851d5859a686ed4501bf83672c1b90`.
- **Observed status/content:** the page is a general MPEG 135 report. Its
  visible body discusses other standards and has an “Output documents published
  in MPEG 135” section, but no ISO/IEC 14496-15 base/final/accepted document
  link or HEVCDecoderConfigurationRecord syntax. A representative excerpt is the
  heading “Output documents published in MPEG 135”; the listed example is
  MPEG-I/2, not Part 15.
- **Result:** route stopped because the actual page body cannot supply the
  requested Part 15 syntax. It is not treated as a negative finding about the
  standard itself.

## Linked-document classification and exclusions

The Part 15 index's actual linked documents were classified by their displayed
titles/status before any reuse:

- MPEG 155 `w26537.zip`: “Draft text ... 8th edition”, draft DAM-stage text;
  already excluded amendment semantics, not accepted text.
- MPEG 153 `w25954.zip`: “Draft text ... 8th edition”, same draft class; not
  fetched or relabeled as accepted.
- MPEG 141 `w22325.zip`: “Draft text ... 6th edition DAM 2”; existing research
  records its completeness semantics only, not bit positions.
- Other visible links are titled “WD”, “CDAM”, “AMD”, “Technologies under
  Consideration”, “Potential Improvements”, or “Preliminary WD”. These statuses
  do not establish an accepted/published ISO/IEC edition and were not used as
  normative bit-layout evidence.

The bounded source map therefore found no substantively different official
base/final/accepted Part 15 document link to retrieve. No guessed archive ID,
meeting filename, standard-store path, paid STANDARD file, private copy, or
mirror was used.

## Evidence integrity and residual risk

The index and MPEG 135 HTML were retained only as transient `.xtask` artifacts;
no copyrighted PDF, ZIP, or full standard text was committed. HTTP 200 HTML was
treated as HTML, not as a standard/PDF. No extraction failure occurred on these
two HTML artifacts; previously recorded failures from IEC and earlier research
remain separate evidence and were not retried here. No image fixture,
implementation source, or consumer agreement was used as normative evidence.

The remaining gap is an edition-qualified, publicly inspectable syntax excerpt
from an accepted/published ISO/IEC 14496-15 text or official corrigendum that
states the array-header fields in order, including widths and the reserved
prescribed value, with clause/table identity and source-byte hash. Older
accepted syntax, if legitimately found later, must remain edition-qualified and
cannot silently establish current-edition conformance.
