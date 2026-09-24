# Issue #1640 — Safe reconciliation conflict resolution

## Outcome

A User can explicitly resolve a true `conflict` in the Emacs reconciliation
report by keeping the reviewed local Post, keeping the reviewed remote Post, or
merging their authored content. No choice is automatic; stale evidence blocks
instead of silently overwriting either side.

## Load-bearing decisions

- Only a uniquely matched `conflict` row is eligible. An `inventory-conflict`,
  `unclassifiable` row, or other state is not a shortcut around identity or
  classification checks. Keep-local and keep-remote accept selected rows in
  display order with one confirmation and an independent terminal result per
  row; merge requires exactly one selected conflict row and explicit completion.
- A report is preview evidence, not mutation authority. The selected row fixes a
  local path and byte digest, Post ID, Member edit identity, and strong remote
  ETag. After confirmation, revalidate the unique match, local bytes and ID,
  remote Member identity and strong ETag against that reviewed evidence, and any
  destination safety required by the existing matched-pull contract. A modified
  visiting buffer blocks all three operations: save it and refresh the report
  first. A remote edit after the report, even if discovered while Ediff is open,
  blocks the operation until a fresh review; never silently adopt a newer ETag
  as authorization. Do not trust Collection metadata as the final mutation
  check.
- Keep-local sends the reviewed authored local Post with a fresh, matching
  strong Member ETag as the conditional `If-Match` precondition. The publish
  path must accept this explicit reviewed precondition while leaving the
  recorded `JAUNDER_SYNCED` value untouched before the PUT; preparation must not
  pre-write synchronization or other local Post metadata. Write back the
  server-confirmed ID, slug, ETag, and synchronization state only after a
  successful response. Do not bypass publication validation, local-link/media
  trust, or existing conditional server safeguards.
- Keep-remote stages and validates the complete current Member and any Local
  Media Copies, then applies the matched-Post pull contract in
  [ADR-0200](../adr/0200-revalidated-matched-post-pull.md): final local and
  remote revalidation, modified-buffer and destination checks, atomic
  replacement, and recoverable canonical-slug rename. A clean visiting buffer
  follows the installed file; a modified one is never silently reverted.
- Merge is two-way Ediff between the reviewed local Post and the freshly staged
  remote Post. There is no saved common ancestor: the last sync ETag is not
  content. Ediff's editable result holds authored fields (including title, body,
  summary, tags, audience, date, and publication state); Post identity,
  canonical slug, and synchronization markers remain client-managed. Neither
  Post is changed before the User explicitly finishes. Finishing rechecks local
  and remote evidence and conditionally publishes the merged authored result;
  only a confirmed server success permits local installation and synchronization
  write-back. An initial staging failure creates no editable result and leaves
  both Posts unchanged. Once an editable result exists, cancellation never
  publishes and retains that scratch work, as do blocked completion,
  indeterminate, or partial outcomes; discard requires an explicit User choice.
- A blocked preflight, cancelled choice, or rejected conditional PUT does not
  change either **Post**. Local Media Copies verified while staging and Media
  uploaded while preparing a publish are durable, reusable side effects that are
  not rolled back; describe them honestly. A transport loss after a PUT may
  conceal a committed remote change: report **remote outcome unknown**, preserve
  the local Post and merge scratch, do not automatically retry, and require
  fresh reconciliation. A confirmed remote commit followed by failed local
  write-back or rename is **partial success**, not a failure claiming both Posts
  unchanged; report the committed side and recovery steps, never attempt an
  automatic rollback. These recoverability boundaries extend
  [ADR-0047](../adr/0047-emacs-publish-orchestration.md) and
  [ADR-0200](../adr/0200-revalidated-matched-post-pull.md).

## Acceptance

1. A true conflict row exposes clear keep-local, keep-remote, and merge actions;
   other states cannot be resolved through them. Confirmation names the chosen
   direction and scope; a batch retains ordered per-Post success, blocked,
   partial, and unknown outcomes when independent rows differ.
2. Keep-local conditionally updates only after a fresh matching Member and
   local-byte/identity check, without pre-writing `JAUNDER_SYNCED` or other
   local Post metadata. A rejected PUT leaves the local Post and remote Post
   unchanged; a confirmed success writes the new synchronization evidence.
3. Keep-remote stages and revalidates the Member, its Media, the local path,
   digest, identity, visiting buffer, and canonical destination before
   atomically installing the remote Post under the ADR-0200 recovery contract.
4. A single-row merge shows both actual authored representations in Ediff and an
   editable result. Initial staging failure leaves both Posts untouched; once a
   result exists, cancellation and blocked completion also preserve its edited
   scratch work. Explicit completion sends a conditional update against freshly
   revalidated evidence and installs locally only after confirmed success.
5. Local edits, identity/duplicate drift, modified buffers, stale or malformed
   remote ETags, destination collisions, HTTP precondition failures, lost PUT
   responses, and post-commit write-back/rename failures yield distinct,
   actionable outcomes. No ambiguous network result is labelled unchanged.
6. Every successful keep-local, keep-remote, and completed merge refreshes the
   report from new inventory while retaining an ordered terminal result summary
   for the selected Post(s); partial and unknown outcomes remain visible rather
   than being hidden by refresh.
7. Pure ERT tests cover state/action eligibility, confirmation, multi-row order,
   edits during Ediff, local and remote drift, safety checks, unknown outcome,
   partial success, and cancellation. Live Emacs integration demonstrates
   keep-local and keep-remote against concurrent remote edits and a successful
   resolution. The README explains the meaning of a `conflict`, all supported
   choices, and their failure/recovery semantics.

## Boundaries

- No automatic conflict decision, heuristic matching, three-way merge without an
  ancestor, server protocol/storage change, or broad new publishing format.
- Do not treat a staged or Collection ETag as a lease: the final conditional
  request and fresh checks remain authoritative. No cross-Post transaction,
  rollback of Media side effects, or guarantee that a committed remote Post can
  be reverted after a local write-back failure.
