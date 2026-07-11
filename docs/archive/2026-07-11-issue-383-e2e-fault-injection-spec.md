# Spec — #383: `failServerFn` + audiences fetch-error branch coverage

**Issue:** [#383](https://github.com/jaunder-org/jaunder/issues/383) **Status:**
proposed **Relates to:** #372 (added the members `Some(Err)` branch), #348 (the
list `ListState::Error` branch), #346 (owns the roster branch — **excluded**
here).

## Problem

The audiences fetch-**error** UI branches are unexercised: `#[component]` +
`cov:ignore` exemptions push them to e2e, but a read server-fn only `Err`s if
the DB breaks, and the suite had no way to force that. So `list_my_audiences` →
`ListState::Error` and `list_audience_members` → `Some(Err)` render
`<p class="error">` that no test drives.

## Approach

Playwright route interception forces the server-fn POST to fail in the browser
(the server fn never runs). A shared helper makes each error test one line:

```ts
// end2end/tests/helpers.ts
export const failServerFn = (page: Page, endpoint: string) =>
  page.route(`**/api/${endpoint}`, (route) =>
    route.fulfill({ status: 500, body: "boom" }),
  );
```

The client `Resource` resolves `Err` → the error branch renders. The route must
be registered **before** the intercepted fetch fires.

## Acceptance criteria

1. **Helper** _(structural)._ `failServerFn(page, endpoint)` exists in
   `helpers.ts` and fulfils `**/api/${endpoint}` with a 500 so the client
   resource resolves `Err`.

2. **Audience-list error** (observable). With `list_my_audiences` forced to fail
   before navigating to `/audiences`, the page renders the list error
   `<p class="error">` — and **not** "No audiences yet." nor a stuck "Loading…".
   (Guards `ListState::Error`.)

3. **Members error** (observable). With `list_audience_members` forced to fail,
   a created audience's `MemberChecklist` renders `<p class="error">` — and
   **not** an empty checklist, "No active subscribers yet.", nor a stuck
   "Loading members…". (Guards `sticky`'s `Some(Err)` → the branch #372 added.)

4. **Roster excluded** _(structural)._ `list_my_subscribers` is not tested here
   — it is #346's (which fixes the current swallow and tests it). No production
   code changes in this issue.

5. **Gate** _(structural)._ The new tests pass under nix-e2e (both browsers);
   tsc + prettier clean.

## Non-goals

- No production-code change (audiences renders these errors already,
  post-#348/#372) — this is test-only.
- No other verticals — the helper generalizes but this issue is audiences-only
  (refocused).
- Not #346's roster branch.

## Risks / to verify

- **Does a bare 500 drive the leptos client `Resource` to `Err` cleanly (no
  panic)?** The client may hit a deserialization error rather than a structured
  `ServerFnError` — either resolves `Err` (fine), but a panic would trip the e2e
  zero-panic gate (ADR-0032). If a raw 500 misbehaves, fulfil with a proper
  server-fn error envelope instead. Verify in nix-e2e.
- **Route timing.** Register `failServerFn` before the fetch: for the list,
  before `goto`; for members, before the audience whose checklist fetches is
  created/mounted.
