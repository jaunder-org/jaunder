# HEVCDecoderConfigurationRecord array-header bit layout research

**Research date:** 2026-10-09  
**Repository note path:**
`docs/superpowers/research/2026-10-09-issue-1702-hevc-configuration-record-layout.md`  
**Scope:**
published-edition array bit positions only; this is source research, not a
conformance or admission decision.

## Result

The exact published-edition bit layout for the HEVCDecoderConfigurationRecord
`array` header was **not established from an openly available authoritative
published ISO/IEC edition** in this run. No current published syntax table was
found in the accessible primary material. Therefore this research does **not**
classify the original `0x60/0x61/0x62` writer output as standards-malformed, and
does not authorize changing it to constructed `0xa0/0xa1/0xa2` or admitting HEIF
uploads.

The working hypothesis seen in implementations is that `array_completeness`
occupies the high bit (bit 7), with a reserved bit and a 6-bit NAL-unit type
below it. That hypothesis is explicitly **unconfirmed for a published edition
here**. The pinned libheif implementation instead reads/writes completeness at
bit 6; this is implementation evidence only.

## Primary-source records

### MPEG official Part 15 page

- **Owner:** MPEG (Moving Picture Experts Group), official standards index.
- **URL:** https://www.mpeg.org/standards/MPEG-4/15/
- **Title:** MPEG-4 Part 15 standards page (page text describes the standard as
  “Carriage of NAL unit structured video in the ISO Base Media File Format”; its
  introductory description says “This standard specifies a File Format for
  AVC”).
- **Retrieved:** 2026-10-09 via pinned
  `devtool run -- curl --silent --show-error --fail --location --output ...`.
- **Available edition/status text:** the page lists Edition 4
  (`ISO/IEC 14496-15:2017`) as **published**; Edition 5 as **released** and
  intended to integrate AMD/COR material; Editions 6 and 7 as **released**;
  Edition 8 as **ongoing**. The page gives `Publication Year: 0` for these
  entries, so it does not supply reliable publication dates for the later
  editions.
- **Relevant actual text (small excerpts):** “Edition - 4: ISO/IEC 14496-15:2017
  [Edition 4] ...”; “Status: published”; “Edition - 8: [8th] ...”; “Status:
  ongoing”. The meeting-document index labels `w26537.zip` “Draft text of
  ISO/IEC 14496-15 8th edition ...”.
- **Finding/status:** authoritative first-party index and edition-status
  evidence, **not the syntax text**. It establishes that the accessible
  8th-edition material is ongoing draft material, not a published edition.
- **Confidence/limitation:** high for the page’s displayed status at retrieval;
  it cannot prove the array layout because no published syntax excerpt is
  supplied.

### ISO ITTF public-deliverables landing page

- **Owner:** ISO (International Organization for Standardization), ITTF.
- **URL:** https://standards.iso.org/ittf/PubliclyAvailableStandards/index.html
- **Title:** “Publicly Available Standards”.
- **Retrieved:** 2026-10-09 via the same pinned curl policy.
- **Actual text:** “The ISO/IEC Information Technology Task Force (ITTF) web
  site is now closed.” It says previously available deliverables are now
  available at no charge on the ISO and IEC webstores and links the ISO Webstore
  and IEC Webstore.
- **Finding/status:** confirms the former free-deliverable route is closed and
  the old public archive does not provide the needed edition text in this run.
- **Confidence/limitation:** high for the retrieved landing page; no paid
  purchase, authentication, or private acquisition was attempted.

### MPEG 155 working draft (negative source result)

- **Owner:** MPEG, official meeting-document publication.
- **URL:**
  https://www.mpeg.org/wp-content/uploads/mpeg_meetings/155_Geneva/w26537.zip
- **Title as indexed by MPEG:** “Draft text of ISO/IEC 14496-15 8th edition
  Carriage of network abstraction layer (NAL) unit structured video in the ISO
  base media file format”.
- **Meeting publication date:** 2026-08-11 (the official index date).
- **Document identity:** extracted DOCX cover identifies
  `ISO/IEC 14496-15:20xx(E)`, `Date: 2023-02`, and says “Draft text of DAM
  stage”; its warning says “This document is not an ISO International Standard
  ... subject to change without notice”. (The archive itself was retrieved from
  the official MPEG URL; it is not treated as a published standard.)
- **Inspection:** ZIP was retrieved and inspected locally under
  `.xtask/heif-mpeg-w26537*`; the document text was searched for
  `array_completeness`, `DecoderConfigurationRecord`, `nal_unit_type`, and
  syntax material.
- **Finding/status:** this draft contains amendment semantics, including the
  `array_completeness` meaning and `hvc1` constraints, but did not yield the
  requested complete array-header bit-layout table. It is **not** evidence for
  the published edition’s bit positions.
- **Confidence/limitation:** high for the negative result on this retrieved
  draft; a negative search is not proof that no table exists elsewhere in the
  full published edition.

### MPEG 141 DAM draft (existing local research artifact; negative for bit layout)

- **Owner:** MPEG; official meeting document.
- **URL:**
  https://www.mpeg.org/wp-content/uploads/mpeg_meetings/141_OnLine/w22325.zip
- **Title:** “Draft text of ISO/IEC 14496-15 6th edition DAM 2
  Picture-in-picture support and other extensions”.
- **Meeting publication date:** 2023-02-13 (official MPEG index).
- **Status:** draft DAM text, not an International Standard.
- **Actual relevant text in the approved extracted artifact
  `.xtask/part15-extracted/w22325.txt`:** subclause `8.3.2.1.3` describes
  `array_completeness` semantics (“when equal to 1 indicates that all NAL units
  ... are in the following array ...; when equal to 0 indicates that additional
  NAL units ... may be in the stream”); subclause `8.4.1.1.1` states the `hvc1`
  parameter-set arrays shall have completeness 1.
- **Finding/status:** semantics and sample-entry constraints only; no
  authoritative bit-position table. Do not turn these semantics into a
  bit-position claim.
- **Confidence/limitation:** high for the quoted extracted text and its draft
  status; it is deliberately not a published-edition conformance source.

## First-party implementation corroboration (separate from normative evidence)

### libheif 1.23.1 `hevc_boxes.cc`

- **Owner:** struktur AG / libheif project (first-party project source, not a
  standards publisher).
- **URL:**
  https://raw.githubusercontent.com/strukturag/libheif/v1.23.1/libheif/codecs/hevc_boxes.cc
- **Title/version:** libheif v1.23.1 HEVC box implementation.
- **Retrieved:** 2026-10-09 via pinned curl; saved as
  `.xtask/heif-libheif-hevc_boxes.cc`.
- **Actual text:** line 79: `array.m_array_completeness = (byte >> 6) & 1;`;
  line 80: `array.m_NAL_unit_type = (byte & 0x3F);`; line 157 writes
  `((array.m_array_completeness & 1) << 6) | (array.m_NAL_unit_type & 0x3F)`;
  line 374 initializes completeness to `1`.
- **Status/finding:** direct implementation evidence of the pinned version’s
  bit-6 behavior, consistent with the known current libheif read/write behavior
  and the original `0x60/0x61/0x62` headers. It is not normative proof and
  cannot establish published conformance.
- **Limitations:** implementation may be buggy or intentionally follow an
  edition/erratum not publicly visible here; source code alone cannot resolve
  which published syntax edition governs this upload contract.

## Evidence and preserved limitations

- Existing current research was read in
  `docs/superpowers/research/2026-10-09-issue-1702-heif-syntax-ownership-and-rewrite.md`,
  along with the current spec and outline. That research already records the
  known facts: pinned libheif 1.23.1 reads/writes bit 6; original headers are
  `0x60/0x61/0x62`; constructed fixtures use `0xa0/0xa1/0xa2`; constructed
  positives are not upload repair or conformance proof; MPEG 155 and 141 drafts
  provide semantics but not the requested layout.
- No image fixtures were downloaded, no paid/private standards acquisition was
  attempted, and no code/admission/spec/ADR change was made.
- The initial repository `find` tool attempt failed because its `fd` helper
  could not run on this NixOS host (dynamic-linker error). The failure was not
  reclassified as a research result; repository enumeration continued with
  pinned `devtool run -- rg`.
- Retrieved HTML/ZIP/source are transient `.xtask` research artifacts and are
  not a substitute for a permanently available normative excerpt.
- Controller preservation: research child `597cd92d-a806-462c-909b-e66ef79133ae`
  completed with this report, but its intended repository path was absent when
  independently checked. The controller persisted the delivered report before
  permitting the next writer to edit. That missing-file failure is not
  image-domain evidence or proof of an available published standard.

## Remaining gap / next source target

A reviewer still needs an openly inspectable, edition-qualified syntax excerpt
from the accepted/published ISO/IEC 14496-15 edition governing
HEVCDecoderConfigurationRecord (or an official correction/erratum explicitly
changing that record). The excerpt must include the array-header bit fields and
clause/table identity, not merely `array_completeness` semantics. Until that
source is obtained, retain the current bounded prototype as a research-only
construction and keep the original-vs-constructed layout unresolved for
conformance and admission.
