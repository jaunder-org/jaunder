# HEVCDecoderConfigurationRecord published-syntax follow-up

**Research date:** 2026-10-09  
**Scope:** primary-source follow-up for the exact published/accepted
`HEVCDecoderConfigurationRecord` array-header bit layout. This note is source
research only; it changes no admission, conformance, code, specification, or ADR
decision.

## Outcome

The requested authoritative published/accepted syntax excerpt remains
unavailable through the public routes actually reachable in this run. I found no
public ISO/IEC preview or free former-deliverable containing the complete
array-header syntax (field widths, order, and reserved prescribed value).
Consequently the original libheif bit-6 bytes (`0x60/0x61/0x62`) versus the
constructed bit-7 bytes (`0xa0/0xa1/0xa2`) remains unresolved for published
conformance. No standards-malformed or upload-admission claim follows.

The accessible official material does provide useful edition/status and route
provenance, recorded below. Older accepted syntax would be useful only with an
edition-qualified excerpt; it must not automatically be relabeled as current
edition conformance.

## Actual official routes and results

### ISO ITTF former-public-deliverables notice

- **Owner/title:** ISO, “Publicly Available Standards”.
- **URL:** https://standards.iso.org/ittf/PubliclyAvailableStandards/index.html
- **Retrieved:** 2026-10-09 using pinned
  `devtool run -- curl --silent --show-error --fail --location --output`.
- **SHA-256:**
  `b290a047cce42fd5d63199330b52772bc3cdfb7402cdfb70a93d790752f59b95`.
- **Actual small excerpt:** “The ISO/IEC Information Technology Task Force
  (ITTF) web site is now closed.” The page says former deliverables are now
  available at no charge on the ISO and IEC webstores and links
  `https://www.iso.org/store.html` plus an IEC webstore products URL.
- **Status/finding:** this is a route/index notice, not a standard preview or
  syntax source. It contains no Part 15 record syntax.

### ISO Webstore route

- **URL derived directly from the ITTF page:** https://www.iso.org/store.html
- **Attempt:** pinned curl with the approved silent/show-error/fail/location/
  output flags.
- **Reported result:** HTTP 403 (`curl` exit 22), historical runner diagnostic
  `.xtask/run/1791505248515-1154503.err`:
  `curl: (22) The requested URL returned error: 403`.
- **Qualification:** this is an access/infrastructure result, not evidence that
  the standard is absent. No authentication, CAPTCHA bypass, paid purchase, or
  private copy was attempted.

### IEC Webstore route

- **URL provenance:** the ITTF HTML contains a safelink whose decoded target is
  the IEC products catalogue URL beginning
  `https://webstore.iec.ch/en/products/?p=1&f=` with the filter token supplied
  by the ITTF page. The decoded target was recorded transiently in
  `.xtask/iec-url.txt`; it was not guessed.
- **Finding:** this is an official catalogue/filter route, not a Part 15
  document or preview. No public syntax excerpt was obtained from it in this
  run. A malformed manually copied variant returned HTTP 200 but was not used as
  evidence; it is explicitly excluded from the source record.
- **Qualification:** catalogue landing/SPA HTML would not count as a standard
  text or PDF. No paid purchase or authentication was attempted.

### Official ISO search route

- **Candidate URL:** https://www.iso.org/search.html?q=14496-15
- **Provenance:** official ISO host and query path, attempted as a public
  catalogue/search route (not treated as a document URL).
- **Reported result:** HTTP 403 (`curl` exit 22), historical runner diagnostic
  `.xtask/run/1791505275454-1155828.err`.
- **Qualification:** no catalogue record or standard ID was inferred from the
  failure; no archive ID was guessed.

## MPEG primary index and accepted-edition status

### MPEG Part 15 index

- **Owner/title:** MPEG, official MPEG-4 Part 15 standards index.
- **URL:** https://www.mpeg.org/standards/MPEG-4/15/
- **Retrieved:** 2026-10-09.
- **SHA-256:**
  `bf53887cfec21e981f6abb56e206fac48d96446df59577db18ba7081758b5799`.
- **Actual index text:** it lists Edition 4 as
  `ISO/IEC 14496-15:2017 [Edition 4]` with status “published”; Editions 5, 6,
  and 7 as “released”; and Edition 8 as “ongoing”. The page displays
  `Publication Year: 0` for these entries, so it does not establish reliable
  dates for later editions.
- **Status/finding:** authoritative status/index evidence, but no syntax table
  or downloadable published edition was exposed by the page.

The page's meeting-document links were followed only where relevant. It links
current and historical working/amendment documents, but the previously inspected
amendment-only documents are not repurposed as array-layout proof.

### MPEG 155 Edition 8 draft (negative, already-known class)

- **Owner/title:** MPEG, “Draft text of ISO/IEC 14496-15 8th edition Carriage of
  network abstraction layer (NAL) unit structured video in the ISO base media
  file format”.
- **URL:**
  https://www.mpeg.org/wp-content/uploads/mpeg_meetings/155_Geneva/w26537.zip
- **Official index date:** 2026-08-11.
- **Archive SHA-256:**
  `9cf59684819946e86fc1b6c6e5bc3c0702ca2ca37c87e07160f6440f35121556`.
- **Edition/status qualification:** extracted cover identifies
  `ISO/IEC 14496-15:20xx(E)`, date 2023-02, “Draft text of DAM stage”, and warns
  that it is not an ISO International Standard and is subject to change.
- **Finding:** its amendment semantics do not provide the requested complete
  array-header field layout. It is ongoing/draft material, not accepted-current
  conformance evidence.
- **Transient evidence:** `.xtask/heif-mpeg-w26537.zip`, extracted text under
  `.xtask/heif-mpeg-w26537*`; no full archive or document is committed.

### MPEG 141 DAM2 draft (negative, existing source)

- **Owner/title:** MPEG, “Draft text of ISO/IEC 14496-15 6th edition DAM 2
  Picture-in-picture support and other extensions”.
- **URL:**
  https://www.mpeg.org/wp-content/uploads/mpeg_meetings/141_OnLine/w22325.zip
- **Official index date:** 2023-02-13.
- **Status:** draft DAM, not an International Standard.
- **Existing extracted source:** `.xtask/part15-extracted/w22325.txt`.
- **Actual relevant text:** §8.3.2.1.3 gives `array_completeness` semantics;
  §8.4.1.1.1 gives the `hvc1` parameter-array completeness constraint.
- **Finding:** semantics only; it does not establish array-header bit positions.
  It is not relabeled as a syntax table or current conformance source.

## First-party implementation corroboration (non-normative)

- **Owner/version:** struktur AG/libheif 1.23.1.
- **URL:**
  https://raw.githubusercontent.com/strukturag/libheif/v1.23.1/libheif/codecs/hevc_boxes.cc
- **Retrieved source SHA-256:**
  `6792b6dbb5d0bca7c498812b9588dca5423c765028dd396954d157a02be1a2b4`.
- **Actual small excerpts:** line 79 reads
  `array.m_array_completeness = (byte >> 6) & 1`; line 80 reads
  `array.m_NAL_unit_type = (byte & 0x3F)`; line 157 writes completeness at
  `<< 6`; line 374 initializes completeness to `1`.
- **Status/finding:** current implementation evidence for the known bit-6
  behavior only. It is not an ISO/IEC normative source and cannot resolve
  whether the implementation is erroneous, edition-specific, or corrected by an
  inaccessible erratum.

## Preserved infrastructure/provenance evidence

- The reported ISO Webstore/search HTTP 403 failures are separate from negative
  source findings. The research delegate also reported a failed `sha256sum`
  because those requests created no output files; historical run diagnostics
  were `.xtask/run/1791505278767-1155873.{out,err}`. These runner logs are
  prunable, not durable evidence or an availability promise. The small reported
  HTTP diagnostic is recorded above; missing original logs cannot independently
  substantiate their complete contents.
- Earlier repository `find`/`fd` failure is preserved in the committed prior
  note; this follow-up used `rg` through `devtool` instead.
- No image fixture was downloaded. No source, helper, spec, ADR, code, or
  admission behavior was changed. No stage, commit, push, worktree switch, or
  broad gate was run.

## Independently verified IEC catalogue and preview route

The controller's new IEC evidence was independently inspected and re-hashed. The
ordinary public catalogue search result is not itself a syntax source, but it
provides legitimate product IDs and lifecycle metadata without accessing the
paid STANDARD deliverable.

### Current published seventh edition

- **Owner/title:** IEC Webstore catalogue record `ISO/IEC 14496-15:2024`,
  publication ID `102880`.
- **Catalogue product URL:** https://webstore.iec.ch/en/publication/102880
- **Catalogue facts:** edition 7.0, publication date 2024-10-31, status
  `PUBLISHED`, publication type `STANDARD`. The catalogue distinguishes the paid
  STANDARD file (`/pub/pdf/iso/isoiec14496-15{ed7.0}en.pdf`) from the public
  PREVIEW file.
- **Catalogue/search evidence:** `.xtask/iec-catalogue-controller.html` SHA-256
  `14cbdd9c96f36604bec672d9b0bd4a2b35481e13a59c30c8b4445fd22b95bd93`;
  `.xtask/iec-part15-query.json` SHA-256
  `3622642b4ee71689fdc883e43f4c69b7442932b4dcc80ff0d0664dbaa4cb6aa8`;
  `.xtask/iec-part15-results.json` SHA-256
  `47151ca839fc7b2a1e3fa3203cb8d311c3236000d22c1aca61dc2e6a2be87228`. The result
  records the lifecycle IDs below. The catalogue HTML supplies the public
  `POST https://webstore-search-api.iec.ch/api/search` route and request schema;
  the request JSON preserves the actual query. None is an ISO syntax excerpt.
  The controller's earlier catalogue hash
  `a7bf5703f484aac3d85da5a78b87e858af3de4e95ab1ed5bf52722128887d60c` was of
  LF-normalized text: the raw HTML contains 71 CRLF pairs. Its raw hash is the
  `14cb…` value above, not evidence of changed source bytes.
- **Public preview URL derived from the actual product page:**
  `https://webstore.iec.ch/en/iec_catalog/product/preview/?id=` followed by
  base64 of `/pub/pdf/preview/info_isoiec14496-15{ed7.0}en.pdf`.
- **Preview evidence:** `.xtask/iec-part15-ed7-product.html` SHA-256
  `de401bf2c61952d37ab118dd45893ce9170aebbe5e2713476d9508f229dd2f78`;
  `.xtask/iec-part15-ed7-preview.pdf` SHA-256
  `47d2e7eaf934da993b96dc72f109ef4c1dfe73b8a01cab72f1217387865e149b`;
  `.xtask/iec-part15-ed7-preview.txt` SHA-256
  `38bb67cca6a2ad4dd97caf9972e67739eecbda93df9dc9d0b061573847246f91`. The PDF is
  `%PDF-1.4`, 1,223,618 bytes, and the pinned Poppler extraction has 11
  form-feed pages and 63,727 UTF-8 bytes.
- **Actual small excerpts and boundary:** the preview says “This is a preview -
  click here to buy the full publication”; its cover identifies
  `ISO/IEC 14496-15:2024(en)`, seventh edition, `2024-10`; its contents lists
  §8.3.2 “Decoder configuration information” at printed page 77. The extracted
  preview ends in front matter/printed page 1 and does not include §8.3.2 or the
  requested array syntax. Search for `array_completeness`, `NAL_unit_type`, and
  `HEVCDecoderConfigurationRecord` returned no matches.
- **Qualification:** this is an official current-edition preview and confirms
  the relevant clause exists, but it does not disclose the clause text or table.
  The paid STANDARD path was not fetched.

### Older accepted editions and corrigendum routes

The actual lifecycle in the public result identifies these edition-qualified
records (all status `REVISED` once superseded, except the current `PUBLISHED`):

| Edition/record                     | IEC publication ID | Catalogue publication date | Public route result                                                                                    |
| ---------------------------------- | -----------------: | -------------------------- | ------------------------------------------------------------------------------------------------------ |
| ISO/IEC 14496-15:2014, 3rd edition |               9961 | 2014-06-24                 | Preview available; only front matter through printed page 1                                            |
| ISO/IEC 14496-15:2014/COR1:2015    |              22565 | 2015-03-16                 | Product page price is 0 and offers electronic PDF options, but exposes no public preview function/path |
| ISO/IEC 14496-15:2017, 4th edition |              60035 | 2017-02-23                 | Preview available; only front matter through printed page 1                                            |
| ISO/IEC 14496-15:2022, 6th edition |              79563 | 2022-10-11                 | Product page derives a preview path, but endpoint returned HTML rather than PDF                        |
| ISO/IEC 14496-15:2022/AMD1:2023    |              89704 | 2023-10-27                 | Product page has no public preview function/path                                                       |
| ISO/IEC 14496-15:2024/AMD1:2025    |             105294 | 2025-01-29                 | Product page has no public preview function/path; current amendment is a paid product                  |

For the 2014 and 2017 editions, the legitimate preview URLs were derived from
actual product-page JavaScript, not guessed:

- 2014: `https://webstore.iec.ch/en/iec_catalog/product/preview/?id=` plus
  base64 of `/p-pub/preview/info_isoiec14496-15{ed3.0}en.pdf`. Product HTML
  SHA-256 `c08b867ff0e336927c9d64e7c71ba3155a44582e4f98b1767e14a5ddaaaf49c3`;
  preview PDF SHA-256
  `c756a9e49c7a146da05a35519dd936c045d9bcc8ad9087704fd426cd4e0cbcbb`; extracted
  text SHA-256
  `cc78999faa97c066614d676177043798e02f61e8c2ddeb6ebb9cce26160f9eef`. The cover
  states `ISO/IEC 14496-15:2014(E)`, third edition, 2014-07-01; §1 says the
  format includes HEVC, but the preview's contents and body stop before the
  relevant later clauses. No array-header syntax is present.
- 2017: `https://webstore.iec.ch/en/iec_catalog/product/preview/?id=` plus
  base64 of `/p-pub/preview/info_isoiec14496-15{ed4.0}en.pdf`. Product HTML
  SHA-256 `dd3834a6186ce56363d29c3ed5b5955aed58fef5b5ef30f1641e6ac036f1b54b`;
  preview PDF SHA-256
  `6fb2903d770688e5fbcbbc080023f5a4ff0c97e2ef4a81194af66c13b95749cc`; extracted
  text SHA-256
  `1483d3d681fd64253153f607854fde99fc3fa13e6f99e5db3ad15448fc880eaf`. The cover
  states `ISO/IEC 14496-15:2017(E)`, fourth edition, 2017-02; §1 says the format
  includes HEVC. The contents lists §8.3.3 “Decoder configuration information”
  at printed page 70, but the 8-page preview ends at printed page 1, before that
  clause. No array-header syntax is present.

The historical product pages were saved for independent inspection:
`.xtask/iec-part15-ed3-product.html` (SHA-256 above),
`.xtask/iec-part15-ed3-cor1-product.html` SHA-256
`7dd5a68f07fec4e961d2bb219dd151b73b1105b7ef554100d88558a5e010a3d2`,
`.xtask/iec-part15-ed4-product.html` (above),
`.xtask/iec-part15-ed6-product.html` SHA-256
`95fc2f081cc7525b3486306b420a2d84d24a51f4b611f9fb5935832cef77a487`,
`.xtask/iec-part15-ed6-amd1-product.html` SHA-256
`07f28b923ead32c4891cd2c93368fd1d60ed2ca0554d0f683a6cf517f3e31ea1`, and
`.xtask/iec-part15-ed7-amd1-product.html` SHA-256
`baeaf3281d82d9dac2dbdecf098ae4c2447a123d2e7affcb8eec0ae4ae6a63e3`. The 2022
preview response is `.xtask/iec-part15-ed6-preview.pdf`, SHA-256
`3763f0b4fe7c20bdb98216c6356b763101096751bad1575919073c473fb50e33`; `file`
identified it as HTML, and the delegate reported Poppler exit 1. The original
runner stderr `.xtask/run/1791506022279-1184887.err` was already unavailable
(`ENOENT`) when the controller inspected it. The controller preserved the
complete delivered note before correction, then explicitly reproduced the
extraction failure against the identical saved HTML response using the same
pinned Poppler, exit 1. The fresh 6,118-byte diagnostic was copied out of the
prunable runner directory to
`.xtask/recovery/task1-iec-ed6-controller-poppler.err`, SHA-256
`3dd863aca73e16e6c19f7381f1d8d2a98bf07927b293b7bc154c3ddd11c5601f`. Its first
line is `Syntax Warning: May not be a PDF file (continuing anyway)`. This is
fresh reproduction, not recovered original stderr, source absence, standard
conformance evidence, or successful image-domain rejection.

## Remaining gap

The parent still needs an openly inspectable, edition-qualified ISO/IEC 14496-15
syntax excerpt (or official corrigendum) that states the array-header bit fields
in order, including widths and the reserved prescribed value, with clause/table
identity and source bytes/hash. The official IEC catalogue now establishes the
current published edition (2024, ID 102880), historical accepted edition IDs,
and legitimate preview URLs, but every public preview inspected stops before (or
does not expose) the relevant clause. The 2014/COR1 product is labelled price 0,
but its public page exposes no freely downloadable file or preview; no ambiguous
direct file path was guessed. Obtaining a paid, private, authenticated,
CAPTCHA-bypassed, or unlicensed copy is outside this research authorization.
Until a legitimate public excerpt is available, keep both bit layouts unresolved
and make no current published-conformance or upload admission claim.
