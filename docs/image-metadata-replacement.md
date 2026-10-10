# Existing-image metadata replacement (#1702)

## Scope and authorization

The owner approved deferring actual historical-original retirement to
[remediation-script issue #1714](https://github.com/jaunder-org/jaunder/issues/1714)
on 2026-10-09. #1702 provides new-upload protection and this truthful rehearsal;
it does not claim historical erasure.

New uploads are sanitized; existing Media is not rewritten or backfilled. This
procedure is preparation, not permission to access production. Obtain separate
approval identifying at least the two affected images, exact old Media
identities, owners and referencing Posts before any real operation. Do not infer
production targets from the synthetic fixtures below.

Never overwrite a hash-addressed file, edit database rows manually, bypass
ownership/reclaim guards, or treat an upload success as erasure of old bytes.
The public hash, MIME, size and ETag must describe the sanitized replacement.

## Supported replacement sequence

1. Inventory each exact old identity (source, SHA-256, canonical filename), its
   per-user Media Records and current/Deleted Posts and Post Revisions. Include
   other authors and theme references. Save the current Post Member ETags so
   updates are conditional. Keep this sensitive inventory private.
2. Keep the author's local original private and unchanged. Upload it through the
   normal web Media picker or AtomPub Media collection. Verify the new public
   bytes independently: sensitive fields removed, presentation retained, MIME
   byte-derived, hash/strong ETag matching those bytes. Do not merely change a
   filename or assume an existing URL has been sanitized.
3. Update current Post references to the new URL using the normal editor or
   conditional AtomPub PUT. Check the saved/rendered references, and verify the
   old hash-addressed file was not modified. Uploading a replacement does not
   update references automatically.
4. Review the old identity's deletion report. Normal deletion protects retained
   owner history. Only the web Media library offers the explicit owner-history
   override; AtomPub does not. Obtain approval for its consequences separately.
   Global-safety refusal is never overridable. A qualifying cross-user reference
   has an independent Media Record; deleting one owner's record is not deleting
   every owner's record or every public copy.
5. Verify the outcome at both surfaces: the owner's AtomPub Media Member and the
   anonymous public URL. **An owner record deletion is not byte erasure.**
   Current storage conservatively retains the public file when Post history
   still names it, even after an explicit owner-history override. The owner's
   Member can return 404 while the original public URL still returns 200. Do not
   declare privacy remediation complete in that state.

## Disposable rehearsal evidence

`server/tests/web/web_media.rs::historical_image_replacement_rehearsal` uses
SQLite and PostgreSQL test backends and temporary storage only. It places two
owned synthetic pre-policy originals (`png-original.png`, `jpeg-original.jpg`)
with their genuine original hashes and Media Records. This placement is explicit
historical test setup, not a production upload bypass.

The rehearsal uploads through the supported web endpoint, checks exact golden
sanitized bytes/new identities, updates current references through conditional
AtomPub PUT, and verifies the original files remain byte-identical. Normal old
record deletion refuses on retained history. Explicit owner override deletes one
owner's record (Member 404), but retained history keeps that public file (URL
200). A legacy cross-user retained reference without an independent Media Record
causes non-overridable global-safety refusal for the second image, even with
force; its record and original file remain. For the first image, a normal
cross-user AtomPub Post create materializes an independent Media Record with
matching source metadata. That author's normal delete refuses on their retained
Post. Deleting the source owner's record leaves the independent owner's Member
and original public file readable: replacement is still incomplete retirement.

Focused proof:
`devtool run -- cargo xtask test-local -- -p jaunder historical_image_replacement_rehearsal`
(with the development shell's pinned ExifTool, or explicit test runtime path).

This demonstrates the replacement and refusal paths, **not removal of the two
historical originals from public access**. Complete historical privacy removal
is tracked in [#1714](https://github.com/jaunder-org/jaunder/issues/1714),
including its separately approved retention/remediation decision; this feature
does not add a history purge, physical-force unlink or migration. Do not weaken
ADR-0183 to obtain a green rehearsal.

## Copies and limits

Author originals retain their metadata. Emacs pulls verified sanitized Local
Media Copies for new identities and republishes those identities without
modifying originals; an older Local Media Copy is not magically replaced. After
any separately authorized reclaim, links naming that reclaimed identity may
break; update current references first and accept retained-history impact
explicitly. Browser/proxy caches, exported Posts, backups and previously
obtained downloads are separate copies. This procedure does not invalidate all
caches, rewrite backups or recall downloads, and makes no such guarantee.
