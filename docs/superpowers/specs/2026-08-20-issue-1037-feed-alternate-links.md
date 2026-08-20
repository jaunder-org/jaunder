# #1037 — centralize feed alternate-link translation

Issue: [#1037](https://github.com/jaunder-org/jaunder/issues/1037). Milestone:
Code quality ratchet.

## Summary

`feeds.spec.ts` repeatedly turns `head link[rel="alternate"]` DOM nodes into
browser-resolved discovery links. Jaunder will centralize that translation in
`end2end/tests/feeds.ts` without changing a navigation, assertion, request, or
polling contract.

## Decision

`feeds.ts` gains a narrow, read-only `readAlternateLinks(page)` helper. It
returns typed records containing:

- `href`: the browser-resolved absolute URL from `HTMLLinkElement.href`; and
- `type`: the DOM link's MIME type from `HTMLLinkElement.type`.

The helper performs only the existing alternate-link DOM read. It does not
navigate, fetch, poll, assert, filter, or introduce a trace action.

Every audited materialized alternate-link read in `feeds.spec.ts` calls the
helper:

1. site and user-page discovery retain their local count checks and MIME-based
   lookup before fetching each link;
2. site and tag-page navigation retain their local href comparison; and
3. the existing live `expect.poll` remains live by calling the helper inside its
   predicate and counting matching `href`s there.

The crawler test remains a raw-HTML response check. It deliberately does not
call the DOM helper because it proves server-rendered discovery without a
browser document.

## Acceptance criteria

- **AC1 — Typed resolved records.** `readAlternateLinks(page)` returns the
  alternate links in document order as `{ href, type }` records; `href` is the
  browser-resolved absolute URL and `type` is the link MIME type.
- **AC2 — One DOM seam.** The four audited materialized alternate-link reads in
  `feeds.spec.ts` use `readAlternateLinks`; no direct alternate-link DOM
  extraction remains there.
- **AC3 — Local assertions remain local.** The discovery count, MIME selection,
  fetch/content assertions, and URL-difference assertions remain at their
  current callers.
- **AC4 — Live navigation predicate remains live.** The client-side tag
  navigation uses `expect.poll` with `readAlternateLinks` inside the predicate;
  it does not replace the convergence wait with a fixed delay or snapshot.
- **AC5 — Crawler scope stays raw.** The crawler’s raw-HTML alternate-link count
  remains independent of the browser DOM helper.
- **AC6 — No observable behavior change.** The affected focused feed tests and
  `cargo xtask check` pass unchanged in behavior.

## Out of scope

- Changing discovery link markup, supported Syndication Feed formats, or feed
  response behavior.
- Moving non-alternate head-link checks into the helper.
- Adding DOM polling, network-idle waits, or navigation behavior to `feeds.ts`.
