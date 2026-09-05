# Site-wide media upload capability (#552)

## Outcome

An operator can enable or disable new media uploads explicitly from the existing
site-settings page or typed site-config CLI. Disabling uploads makes existing
media read-only, removes upload discovery and controls, and rejects direct web
and AtomPub upload attempts clearly.

## Load-bearing decisions

- `media.uploads_enabled` is the single site-wide boolean setting. The closed
  site-config registry accepts only `true` or `false` for it.
- An absent setting reads as enabled, preserving existing installations. An
  invalid stored value reads as disabled; database failures still propagate.
- The existing `media.max_file_size_bytes` and `media.user_quota_bytes` remain
  positive limits. Zero never regains a second meaning as a feature switch.
- The capability governs every new media upload, whether it enters through the
  web server function or AtomPub Media Collection. One shared domain check owns
  that policy before the media manager polls a supplied stream, reads quota or
  upload storage, creates a temporary file, or performs durable mutation.
  Transport adapters may still parse request framing or materialize an HTTP body
  before calling the manager.
- Policy is evaluated once when an upload begins. Disabling later blocks new
  attempts but does not cancel an upload already in flight.
- A disabled attempt is a typed domain error that maps to HTTP `403 Forbidden`
  at both public boundaries with the client-safe message
  `media uploads are disabled`. Internal failures continue through the existing
  typed error pipeline.
- Existing media remain readable and manageable. Disabling affects creation, not
  retrieval or deletion.
- `/media` remains available. It shows existing media and usage, removes upload
  controls, and displays `Media uploads are disabled by the site operator.`
- The AtomPub Service Document omits the Media Collection while uploads are
  disabled. A client that posts directly to the known collection URI still
  receives the same `403` domain rejection.
- `/admin/site` gains a separate Media Uploads card with its own persisted
  checkbox and save action. It does not couple media policy writes to site-title
  or base-URL updates.
- The typed site-config CLI remains an equivalent operator control surface:
  `site-config set media.uploads_enabled false` disables uploads and `true`
  enables them.
- The durable cross-protocol policy is recorded in
  `docs/adr/drafts/media-upload-capability-is-site-wide.md`.

## Acceptance

- The typed configuration surface round-trips exact boolean values; absent reads
  enabled, invalid physical data reads disabled, and byte-limit zero remains
  invalid/defaulted rather than disabling uploads.
- An operator can disable and re-enable uploads from the Media Uploads card
  without changing site identity settings.
- Under disabled policy, authenticated web and AtomPub upload requests both
  return `403` with the clear disabled message. Manager-level tests prove the
  supplied stream is not polled and quota/upload storage is not read; boundary
  tests prove no temporary file, media row, quota change, or upload-success
  telemetry is produced.
- Web and AtomPub upload behavior remains identical on SQLite and PostgreSQL.
- `/media` under disabled policy displays the read-only notice and existing
  media/usage but no upload picker or submit control; enabled behavior remains
  unchanged.
- AtomPub discovery advertises the Media Collection only while uploads are
  enabled. Direct requests remain authoritative when discovery omits it.
- An upload admitted while enabled can finish after the setting is disabled; the
  next upload attempt is rejected.
- While disabled, existing media remains retrievable and deletable through both
  web and AtomPub surfaces, with SQLite/PostgreSQL parity where storage applies.
- Existing size-limit, quota, retrieval, deletion, and upload behavior remains
  intact when the setting is enabled or absent.
- Operator documentation, the domain glossary, and the architecture projection
  describe the explicit capability and its two control surfaces.

## Boundaries

- No per-user, per-protocol, media-type, or scheduled upload policy.
- No cancellation or revocation of uploads already in flight.
- No change to media path, filename, content-addressing, deduplication,
  retrieval, deletion, quota, or byte-limit semantics.
- No new admin route, process environment variable, or deployment flag.
- No compatibility interpretation of zero as disabled and no migration that
  rewrites old media limit values.
