# Verified original Media destinations on matched Org pull

Issue: [#1703](https://github.com/jaunder-org/jaunder/issues/1703)

## Outcome

Pulling a matched Org Post preserves its existing relative Media file
destination when the original file is unambiguously identified and its bytes
equal the verified remote Media. Regeneration and other remote edits do not
create a redundant Local Media Copy or replace a usable authored destination.

## Load-bearing decisions

- Apply the same policy to server-ahead pull, keep-remote, and the staged remote
  representation used by merge. Server-only pull and Markdown/HTML are
  unchanged.
- Consider only actual body-level relative Org file links in the matched local
  Post. Exclude Local Post Link candidates, attachment links, absolute paths,
  queries, and Org search targets. Never search directories for matching files.
- Resolve from the Post's location, require containment in its configured root,
  and reject symlinks in the target or its path components and non-regular
  files. A preserved relative destination must still resolve to the verified
  file at the final Post location, including a canonical-slug rename.
- Byte equality, not filename equality, proves reuse. Preserve the original file
  destination's exact authored spelling, including an explicit or implicit file
  scheme and encoding. The remote body's labels, fragments, reference additions
  and removals, and other authored bytes remain authoritative.
- A remote Media hash may use an original destination only when exactly one
  distinct eligible local file destination matches it. Uniqueness uses exact
  authored destination spelling excluding the fragment: `file:images/a.png`,
  `images/a.png`, and `./images/a.png` are distinct even if they resolve to one
  file. Repeated occurrences of one spelling are not ambiguous. Ambiguous,
  missing, changed, unreadable, or unsafe originals use existing `local-media/`
  localization without overwriting any original file.
- Retain the existing remote trust chain: canonical active-origin URL, matching
  Member/media instance UUIDs, anonymous direct 200 response, strong hash ETag,
  and response bytes matching both ETag and URL hash. Invalid remote evidence
  still fails the action; local reuse must not bypass remote verification.
- For server-ahead and keep-remote, verify original file bytes again at the
  final local-install boundary. If reuse eligibility changes, generate the
  normal verified Local Media Copy fallback before installing the Post. A
  successfully reused file creates no redundant copy; fetching remote bytes for
  verification is still permitted.
- Preserve all reviewed Post identity, local digest, clean-buffer, unique-match,
  remote ETag, destination-collision, and recoverable replacement/rename checks.
  Merge verifies reuse during initial remote staging. Thereafter its links are
  ordinary authored merge content subject to existing publish and conflict
  safety rules; no automatic fallback rewrites the completed merge result.
- Expected ineligibility permits fallback, but unexpected I/O/infrastructure
  errors remain visible failures. Retain ADR-0160's documented residual race
  after final filesystem checks and its non-transactional Media side effects.

## Acceptance

- Pure and live Emacs tests show an unchanged relative Org Media file survives
  server-ahead pull and keep-remote with exact file-destination spelling, remote
  body edits intact, and no redundant Local Media Copy. Merge stages the same
  localized remote representation.
- Tests cover duplicate occurrences, alias-spelling and distinct-file ambiguity,
  renamed local filenames with equal bytes, remote link additions/removals,
  encoding, fragments, code/non-link text, and excluded link forms.
- Missing, changed, unreadable, non-regular, symlinked, and cross-root originals
  demonstrably fall back; originals are never modified. A change during staging
  also falls back at server-ahead/keep-remote installation, without a dangling
  installed link. Merge completion retains authored links and existing publish
  validation rather than silently substituting the staged remote version.
- Canonical-slug rename preserves destination resolution; occupied destinations,
  stale Post/Member evidence, and modified buffers retain their blocking
  behavior.
- Existing invalid remote-instance/hash/ETag/redirect tests stay fail-closed
  even when a matching local file exists. Server-only and non-Org localization
  retain their existing behavior and durable-copy safety guarantees.

## Boundaries

No server changes, persisted upload provenance, filesystem-wide hash index,
external downloads, general source rollback, Media cleanup, or new report UI.
The decision supplements ADR-0160's mandatory-copy policy only for proven
original destinations; see
`docs/adr/drafts/emacs-verified-original-media-reuse.md`.
