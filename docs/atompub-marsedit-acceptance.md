# MarsEdit acceptance checklist (AtomPub)

Manual acceptance for the AtomPub (RFC 5023) publishing interface, verified
against [MarsEdit](https://redsweater.com/marsedit/). The automated Playwright
suite (`end2end/tests/atompub.spec.ts`) covers the HTTP contract; this checklist
covers real-client behaviour that the suite cannot.

## Prerequisites

1. A running Jaunder instance reachable over HTTPS (MarsEdit requires TLS for
   HTTP Basic credentials). Set the site **base URL** (`site.base_url`) so the
   service document and feeds emit absolute URLs.
2. A user account.
3. An **app password**: log in → **Sessions** → "App passwords" → create one
   with a label like `MarsEdit`. Copy the token (shown once).

## Connect

- [ ] In MarsEdit, add a blog using the user's page URL
      (`https://host/~username`). MarsEdit should discover the endpoint via the
      `<link rel="EditURI">` RSD autodiscovery tag → `/~username/rsd.xml` → the
      AtomPub service document.
- [ ] If autodiscovery fails, set the API endpoint manually to
      `https://host/atompub/service` (System API: **Atom**).
- [ ] Authenticate with the **username** and the **app password** (not the
      account password). A wrong username for the token must be rejected (401).

## List & read

- [ ] MarsEdit's "Refresh" lists existing posts (drafts and published) ordered
      newest-edited first.
- [ ] Each post shows its title; opening one loads the body in the editor.
- [ ] A Markdown/Org post round-trips as **native source** (`type="text"`), not
      rendered HTML — editing preserves the original Markdown/Org. An HTML post
      round-trips as `type="html"`.

## Create

- [ ] Create a new **HTML** post; it appears on the site after publishing.
- [ ] Create a post in the user's **default format** (set under Profile →
      default post format) sent as `type="text"`; confirm it is stored in that
      format and renders correctly.
- [ ] Save as **draft** (MarsEdit "Draft" / `app:draft`); it is NOT publicly
      visible and has no public permalink. Toggling draft→published publishes
      it.

## Edit

- [ ] Edit an existing post's body; the change is reflected on the site.
- [ ] A **title-only** edit preserves the source format (does not convert a
      Markdown post to HTML). _(Open question from ADR-0015 — verify here; if
      MarsEdit re-sends `type="text"` HTML on a title edit, confirm the
      last-write-wins behaviour is acceptable.)_
- [ ] Concurrent-edit protection: editing a post that changed server-side since
      MarsEdit loaded it is rejected (`412 Precondition Failed` via `If-Match`).

## Categories

- [ ] Existing tags appear as category suggestions (from the service document's
      `app:categories`).
- [ ] Adding categories to a post creates tags; removing them untags.
- [ ] An invalid category term (spaces, uppercase, etc.) is silently skipped,
      not rejected.

## Media

- [ ] Insert an image into a post; MarsEdit uploads it and embeds an absolute
      `/media/upload/...` URL that loads on the published page.
- [ ] Re-uploading identical image bytes is idempotent (no duplicate stored).

## Delete

- [ ] Deleting a post in MarsEdit removes it from the site (soft-deleted;
      subsequent fetches 404).

## Notes

- Authentication is app-password-over-HTTP-Basic, validated through the existing
  session-token path (ADR-0014). Revoking the app password in **Sessions**
  immediately disconnects MarsEdit.
- The public syndication feed (M8) and the AtomPub collection are deliberately
  separate serializers (ADR-0015): the feed is rendered HTML for readers; the
  collection is native source for editing.
