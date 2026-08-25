# Issue #25 — Broaden Emacs publish media upload beyond images

## Outcome

Authors publishing from Emacs can reference local files from an Org post body
and have each readable target uploaded to the AtomPub Media Collection before
the Post is sent. The sent body rewrites those local links to the
server-authored absolute media URL from the upload response; the authoring
buffer remains unchanged.

## Load-bearing decisions

- A qualifying upload candidate remains an Org body link whose type resolves to
  a local path: `file:` links resolved from the live authoring buffer's
  `default-directory`, and `attachment:` links resolved through org-attach.
  Header properties, fuzzy links, `http(s)` URLs, inline data, and non-body
  links stay excluded.
- The extension table stops being the eligibility predicate. Any local-path
  candidate qualifies before filesystem checks; preflight then verifies that
  each target exists, is readable, and is a regular file, failing bad targets
  instead of passing them through silently.
- Content type remains deterministic and local. Known mappings are exactly the
  shared media table: `jpg`/`jpeg`, `png`, `gif`, `webp`, `svg`, `mp3`,
  `ogg`/`oga`, `flac`, `wav`, `mp4`, `webm`, and `pdf`; unknown or extensionless
  files upload as `application/octet-stream`. Upload must not depend on mailcap,
  external commands, or server-side sniffing.
- Upload still happens before the entry send and before all write-back/rename
  steps, preserving ADR-0047's safe-to-resume order: validate → media upload →
  entry send → `JAUNDER_ID`-first write-back → rename.
- Missing, unreadable, or non-regular qualifying files still fail in one
  preflight error listing all bad paths, before any upload is attempted.
- Substitution remains position-based over the body links, not global string
  replacement. Two identical raw links may resolve differently under org-attach;
  one resolved file referenced multiple times uploads once and rewrites every
  occurrence to the same harvested URL.
- The client continues to harvest the binary media URL from the response entry's
  `<content src>` per ADR-0045. It must not use `Location` and must not
  reconstruct `/media/...` paths client-side.
- The AtomPub media collection is treated as a general attachment collection.
  Its service document advertises one AtomPub media-range accept value, `*/*`,
  rather than an image-only accept list. This is a service-document/discovery
  value, not an upload `Content-Type`; do not mint wildcard values through the
  concrete uploaded-media `ContentType` invariant.
- Existing image upload behavior, idempotent re-upload, warning for untracked
  local media, and source-buffer non-mutation are preserved.

## Acceptance

- Pure Emacs tests prove that non-image `file:` and `attachment:` body links are
  collected, preflighted, uploaded once per resolved file, and substituted in
  the sent body while preserving link descriptions and leaving the source buffer
  unchanged.
- Pure Emacs tests prove the explicit content-type table, the
  `application/octet-stream` fallback for unknown/extensionless files, existing
  image mappings, non-local link pass-through, and
  missing/unreadable/non-regular local-file abort before upload.
- Live Emacs integration uploads at least one non-image local attachment through
  the real AtomPub Media Collection and publishes a Post whose sent body
  contains the harvested absolute media URL.
- Server/service tests prove `POST /atompub/{user}/media` still accepts
  non-image content types and the AtomPub service document advertises exactly
  `*/*` for the media collection.
- `docs/ARCHITECTURE.md` reflects the broadened Emacs media qualification rule
  and the retained safe-to-resume publish order.

## Boundaries

- No Markdown or HTML authoring-buffer converter work.
- No download/localize-on-pull behavior for pulled Posts.
- No inline/base64/data URI upload path.
- No calendar, gallery, preview, or Media Library UI.
- No new server media storage model, Media URL layout, filename canonicalization
  rule, or publication lifecycle rule.
- No blocking policy for unversioned local attachments beyond the existing
  warning behavior.
