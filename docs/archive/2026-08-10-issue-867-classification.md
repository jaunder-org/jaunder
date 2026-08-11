# Classification — every document load in the e2e suite (#867)

## Summary (pre-registered)

**This block is registered BEFORE any timing arm is captured.** No `after` arm
exists at the time of writing; the counts below are derived from the `before`
corpus and from reading every test, not from any post-change measurement.

```
count(removed)        = 40
count(converted)      = 21
count(kept:entry)     = 128
count(kept:declared)  = 42
TOTAL                 = 231   (40 + 21 + 128 + 42)

PREDICTED_TOTAL = 231 - 40 - 21 = 170
saved           = 40 + 21       = 61

ceiling firefox  = 61 x 911 ms = 55.6 s
ceiling chromium = 61 x 689 ms = 42.0 s
floor (60% of ceiling) = firefox 33.3 s / chromium 25.2 s
```

Arithmetic checked: the four class counts sum to 231, the corpus total. The
`kept:entry` count decomposes as 114 test-attributed (exactly one per test that
navigates at all — 114 such tests) plus 14 secondary-page entries.

### Amendment 1 — `PREDICTED_TOTAL` 170 → 169 (Task 6b, still before any arm)

The block above is left as first written; this records what changed and why.

Implementing Task 6b found one row the classification had called `converted`
that cannot be converted, because it was never a navigation at all. In
`draft lifecycle: create, view, edit, and publish`, the second
`goto(page, permalinkUrl)` targeted the URL `page` was **already displaying** —
only `guestPage` had moved in between. It was a full reload of the current page,
so there is no in-app move to replace it with, and `navigateInApp` cannot
express it (its barrier would be vacuous by construction). It is deleted
outright.

Deleting rather than converting removes one more load than predicted, so:

```
saved            = 62          (was 61)
PREDICTED_TOTAL  = 169         (was 170)

ceiling firefox  = 62 x 911 ms = 56.5 s   (was 55.6 s)
ceiling chromium = 62 x 689 ms = 42.7 s   (was 42.0 s)
floor (60%)      = firefox 33.9 s / chromium 25.6 s
```

Nothing else moves. The deletion is safe: the assertions that follow it stash
`__jaunderNoReload` on `window` to prove the publish flip happens _without_ a
document reload, and that sentinel is set after the deleted line, so removing a
reload strengthens rather than weakens the test.

**Measured afterwards (Task 11), closing this amendment's open item.** The #863
test's loads are now pinned: **5 in the `before` arm, 1 in the `after` arm.**
Subtracting its 5 from the fork point's 236 gives **231** — the corpus baseline
to the load, which confirms the fork point is exactly the corpus plus that test.

**Also measured: this amendment was wrong about its own effect.** Deleting the
redundant reload of the already-displayed permalink did remove a real document
load, but it did **not** reduce the trace's navigation count, because the trace
never counted that reload as a navigation. So `PREDICTED_TOTAL` should have
stayed at 170; the observed corpus-comparable figure is 170, and the count check
misses its 169 by exactly this one row. The deletion stands — the reasoning
about the metric did not.

**Two changes that do NOT move the prediction**, recorded so the count check can
be read correctly:

- `scheduling from the edit page shows a Scheduled-for badge on the drafts page`
  (#863) was converted too. It postdates the corpus, so none of its loads are in
  the 231 baseline and none are in `PREDICTED_TOTAL`. The post-change observed
  total will therefore include loads the prediction never counted; Task 11 must
  subtract this test before comparing.
- `editing a post updates tag chips and tag listing pages` had its two
  tag-listing assertions **reordered**. `/tags/xeditd` is reached by clicking
  the post's own chip, which exists only while the page is still on the
  permalink, so it must precede the `/tags/xeditc` load that leaves it. Both
  assertions are intact; only the order changed. Listed under Subject changes.

### Amendment 2 — the pre-paint `/`→`/app` redirect counts 1 on chromium and 2 on firefox (Tasks 8 and 9 review)

Arming the budget (Task 8) made the orphan check fail `authed-flash.spec.ts` ›
`owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app`:
it declared two further loads and only one arrived. Collapsing the declaration
to one then made the **firefox** combo fail the same test the other way, with an
undeclared second load (`booted at /register, then loaded /app`) — the single
allowance had been consumed by `/`.

**Measured on both engines, not inferred.** A throwaway probe listened to both
events on that exact flow under chromium:

```
framenavigated   -> http://127.0.0.1:35173/
framenavigated   -> http://127.0.0.1:35173/app
domcontentloaded -> http://127.0.0.1:35173/app     <- the only one
```

Firefox's budget failure is the counterpart observation: it counted `/` and then
`/app`, so `domcontentloaded` fires for both there.

The pre-paint redirect is a `location.replace` emitted as a JS string from
`web/src/app/render.rs` (ADR-0076 names it the one remaining in-app document
load, and specifically as a string its AST scan cannot see). It runs during head
parsing, so whether the `/` document is **replaced before it reaches
`DOMContentLoaded`** is a race between the engine's parser and its navigation —
chromium replaces it first and never fires the event; firefox fires it.

Three consequences:

- **The count is engine-dependent: 1 on chromium, 2 on firefox.** No fixed
  number of declarations is right — one orphans on firefox, two orphan on
  chromium. **This is why `allowEngineDependentBoot` exists** (Task 9 review):
  the `/app` load always happens and keeps an exact `allowSecondBoot`, while the
  `/` document takes the engine-dependent form, which authorises at most one
  load and is exempt from the orphan rule. On chromium the `/` declaration goes
  unconsumed and that is not a defect; on firefox it is consumed.
- **On chromium the load the budget sees is `/app`, not `/`.** The row that
  reads "the pre-paint redirect script runs only on a cold document load of `/`"
  was describing a load that is real but, on that engine, uncountable.
- **The trace still counts 2.** The corpus rows come from
  `e2e.navigation_top_json`, which is built from navigation records, not from
  `domcontentloaded` — and it lists `/` and `/app` as separate rows for this
  test, which is why the classification predicted two. Both are genuine
  navigations; only one is a document _load_ by the budget's definition.

**The arithmetic does not move.** `PREDICTED_TOTAL` (169) is measured against
the trace's navigation count, and the trace's view of this test is unchanged
at 2. I checked this rather than assuming it: the evidence is the
classification's own two rows for this test, which were derived from the trace.

**What does move is the declaration census, and it is now engine-dependent
too:** the source sites stay at **40** — 39 exact `allowSecondBoot` plus 1
`allowEngineDependentBoot` — but the number _consumed_ per run is **41 on
chromium and 42 on firefox**. So `count(kept:declared)` as an _enforced_ number
matches the trace's 42 on firefox and sits one below it on chromium. The gap is
exactly this row, and it is a definitional difference between the two counters
(and between two engines) rather than a miscount in any of them.

**For Task 11:** the budget census and the trace count are not the same
measurement and must not be reconciled by adjusting one to fit the other.

## Method

**Source.**
`~/measurements/jaunder/issue-866-preload/traces/before-1-sqlite-chromium.jsonl`,
one trace from the certified #866 corpus. All 12 corpus traces agree exactly on
navigation counts, so arm and browser do not change the answer.

**Extraction.** Summed `e2e.navigation_count` over `e2e.test` spans; took the
URL list from `e2e.navigation_top_json`; separately summed
`e2e.navigation_count` over `e2e.page` spans.

**Reproduced:** `tests = 137`, `testNav = 211`, `pageNav = 20`, total `231`.

**`dropped = 0`.** Summed `e2e.navigation_top_dropped` across every span in the
file: zero. The URL lists are complete, not a top-N slice, so every
test-attributed row below carries its real destination.

**Secondary-page rows (`e2e.page`).** These 20 loads carry no
`navigation_top_json` — the attribution gap filed as
[#895](https://github.com/jaunder-org/jaunder/issues/895). They are accounted
for by test, from each page span's `e2e.file` / `e2e.test` /
`e2e.navigation_count`, and reconciled against the test source: every one of the
20 is attributable to a specific `newPage()` (or fixture-setup page) and a
specific navigation in the test body. The reconciliation is shown in the
[Secondary-page loads](#secondary-page-loads-e2epage) section.

**Browser projects.** The trace is chromium. `chromium-admin` (the serial
project) covers `admin-site.spec.ts` and `invite.spec.ts`. Firefox runs the same
tests with the same counts.

**Normalisation.** Generated usernames are written `~user`. Post ids are kept as
traced.

**Corpus vintage.** `posts.spec.ts` has since gained
`"scheduling from the edit page shows a Scheduled-for badge on the drafts page"`
(#863), which is not in this corpus. The classification pins the **corpus**, per
the spec: 231 is the number the prediction is measured against, and the `before`
arm is captured at the branch's fork point.

**Class definitions.** `removed` is the `registeredPage` fixture's
`goto(page, "/")` (`fixtures.ts:486`) in a test that immediately navigates away.
`converted` becomes in-app router navigation. `kept:entry` is a Playwright
`Page`'s one legitimate boot — **the budget unit is the `Page`, not the test**,
so a `context.newPage()` gets its own entry. `kept:declared` is a second
document load on an already-booted `Page` that must stay; its `reason` is the
verbatim `allowSecondBoot(page, "...")` string.

**One rule applied strictly.** Where no real UI control reaches the destination,
the row is `kept:declared`, not `converted` — no affordance is invented. This
bites hardest at `/posts/new`: grepping `web/src` and `server/src` finds **no
link to `/posts/new` anywhere in the app**. Every mid-flow arrival at the
composer is therefore `kept:declared`, not `converted`. See
[Uncertain — review these](#uncertain--review-these).

## Classification

### `admin-site.spec.ts` — 9 loads (project `chromium-admin`)

| test                                                                    | url         | class           | reason                                                                                                                 |
| ----------------------------------------------------------------------- | ----------- | --------------- | ---------------------------------------------------------------------------------------------------------------------- |
| admin site settings page loads and allows updating title and base_url   | /admin/site | `kept:entry`    | the page's one boot, at the URL under test                                                                             |
| admin site settings page loads and allows updating title and base_url   | /admin/site | `kept:declared` | a fresh load reads the persisted title and base URL back through site::get                                             |
| non-operator user is denied access to /admin/site                       | /admin/site | `kept:entry`    | the page's one boot, at the URL under test                                                                             |
| site base URL round-trips, clears via omission, and validates inline    | /admin/site | `kept:entry`    | the page's one boot, at the URL under test                                                                             |
| site base URL round-trips, clears via omission, and validates inline    | /admin/site | `kept:declared` | a fresh load reads the saved base URL back through site::get in its canonical form                                     |
| site base URL round-trips, clears via omission, and validates inline    | /admin/site | `kept:declared` | a fresh load reads the cleared base URL back through site::get to prove the None round-trip                            |
| site base URL warning banner shows when unset and hides once configured | /admin/site | `kept:entry`    | the page's one boot, at the URL under test                                                                             |
| site base URL warning banner shows when unset and hides once configured | /admin/site | `kept:declared` | the warning banner is painted from the boot-time site config, so a fresh load is what proves it hides once configured  |
| site base URL warning banner shows when unset and hides once configured | /admin/site | `kept:declared` | the warning banner is painted from the boot-time site config, so a fresh load is what proves it reappears once cleared |

### `atompub.spec.ts` — 3 loads

| test                                                            | url       | class        | reason                                     |
| --------------------------------------------------------------- | --------- | ------------ | ------------------------------------------ |
| an app password can be minted from the sessions page            | /sessions | `kept:entry` | the page's one boot, at the URL under test |
| full AtomPub publishing flow over HTTP with an app password     | /sessions | `kept:entry` | the page's one boot, at the URL under test |
| RSD autodiscovery link is present on the user page and resolves | /~user    | `kept:entry` | the page's one boot, at the URL under test |

### `audiences.spec.ts` — 7 loads

Every test enters once at `/audiences` and does all its work there; no test
navigates a second time.

| test                                                                               | url        | class        | reason                                     |
| ---------------------------------------------------------------------------------- | ---------- | ------------ | ------------------------------------------ |
| Audiences: a failed subscriber-roster fetch surfaces an error, not an empty roster | /audiences | `kept:entry` | the page's one boot, at the URL under test |
| Audiences: a genuinely empty roster still shows the empty message                  | /audiences | `kept:entry` | the page's one boot, at the URL under test |
| Audiences: a list fetch error surfaces the error node, not an empty list           | /audiences | `kept:entry` | the page's one boot, at the URL under test |
| Audiences: a members fetch error surfaces the error node, not an empty checklist   | /audiences | `kept:entry` | the page's one boot, at the URL under test |
| Audiences: create-name client-side validation gates submit                         | /audiences | `kept:entry` | the page's one boot, at the URL under test |
| Audiences: CRUD + membership toggle re-fetch without list remount or flash         | /audiences | `kept:entry` | the page's one boot, at the URL under test |
| Audiences: refresh pulls a mid-session new subscriber into the checklists          | /audiences | `kept:entry` | the page's one boot, at the URL under test |

### `auth.spec.ts` — 12 loads

Every test enters once and stays; the login/logout transitions under test are
already client-side pushState (#591). The `/login` and `/register` entries are
the ADR-0098 holdouts whose subject is the real auth flow (in-source comments at
`:14`, `:52`).

| test                                                        | url       | class        | reason                                                                 |
| ----------------------------------------------------------- | --------- | ------------ | ---------------------------------------------------------------------- |
| login navigates client-side without a full document reload  | /login    | `kept:entry` | ADR-0098 holdout: the real login flow is the subject                   |
| login page shows form                                       | /login    | `kept:entry` | ADR-0098 holdout: /login's own render is the subject                   |
| login with valid credentials succeeds                       | /login    | `kept:entry` | ADR-0098 holdout: auth::login through the real form                    |
| login with wrong password shows error                       | /login    | `kept:entry` | ADR-0098 holdout: the login error path through the real form           |
| logout navigates client-side without a full document reload | /         | `kept:entry` | the page's one boot; the logout itself is client-side                  |
| logout page logs out                                        | /         | `kept:entry` | the page's one boot; the logout itself is client-side                  |
| register page shows form                                    | /register | `kept:entry` | ADR-0098 holdout: /register's own render is the subject                |
| register rejects a too-short password client-side           | /register | `kept:entry` | ADR-0098 holdout: the register form's client validation                |
| register with open policy succeeds                          | /register | `kept:entry` | ADR-0098 holdout: registration::register through the real form         |
| sidebar footer shows Sign out link when logged in           | /         | `kept:entry` | `registeredPage("/")` — the assertions run at `/`, so `/` is the entry |
| sidebar reverts to signed-out state after logout            | /         | `kept:entry` | the page's one boot, at the URL under test                             |
| sidebar shows only Home nav link when not logged in         | /         | `kept:entry` | the page's one boot, at the URL under test                             |

### `authed-cls.spec.ts` — 1 load

| test                                                                | url | class        | reason                                                                                                                                  |
| ------------------------------------------------------------------- | --- | ------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| authed owner: own-post action column is additive (no content shift) | /   | `kept:entry` | the CLS probe's frozen first paint IS the subject; it holds the wasm so mount never completes (raw `page.goto` at `layout-shift.ts:67`) |

### `authed-flash.spec.ts` — 16 loads

| test                                                                             | url       | class             | reason                                                                                                                                                                                                                |
| -------------------------------------------------------------------------------- | --------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| anonymous: / has no authed sidebar chrome                                        | /         | `kept:entry`      | the page's one boot, at the URL under test                                                                                                                                                                            |
| anonymous: /app bounces to /login                                                | /app      | `kept:entry`      | the page's one boot; the bounce to /login is the app's own redirect                                                                                                                                                   |
| operator: admin chrome is seeded flash-free from the marker                      | /login    | `kept:entry`      | ADR-0098 holdout: `login()` through the real UI is what writes the operator marker                                                                                                                                    |
| operator: admin chrome is seeded flash-free from the marker                      | /         | `kept:declared`   | the pre-paint marker read happens only on a cold boot, and with get_session() failing that boot is the only source of the operator chrome                                                                             |
| owner: /app cockpit boots straight into the personalized feed                    | /         | `removed`         | `registeredPage`'s fixture boot; the test asserts nothing at `/` and goes straight to /app                                                                                                                            |
| owner: /app cockpit boots straight into the personalized feed                    | /app      | `kept:entry`      | the cold boot of /app is the subject — "directly bookmarkable, zero intermediate clicks"                                                                                                                              |
| owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app  | /register | `kept:entry`      | ADR-0098 holdout: `registerViaUi` writes a real marker, which a seeded helper cannot (it never navigates)                                                                                                             |
| owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app  | /         | `kept:declared`\* | \*Amendment 2: a real navigation in the trace, but **not** a document load the budget can see — `location.replace` during head parsing replaces `/` before DOMContentLoaded, so this row carries no `allowSecondBoot` |
| owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app  | /app      | `kept:declared`   | the pre-paint redirect is a location.replace during head parsing, so the only load that lands is /app; it is the script's own redirect, not a test-issued navigation, and it is the subject                           |
| owner: pre-paint auth marks html.authed and / stays the enhanced public timeline | /register | `kept:entry`      | ADR-0098 holdout: registering through the real UI is what leaves a correct marker                                                                                                                                     |
| owner: pre-paint auth marks html.authed and / stays the enhanced public timeline | /         | `kept:declared`   | the pre-paint `html.authed` marking is observable only on a cold document load, and it is the subject                                                                                                                 |
| seeded: logout survives a full navigation (tombstone respected)                  | /         | `kept:entry`      | the page's one boot, at the URL under test                                                                                                                                                                            |
| seeded: logout survives a full navigation (tombstone respected)                  | /         | `kept:declared`   | a full post-logout document load is exactly what pins the tombstone; the pushState logout tests never re-run the init script                                                                                          |
| seeded: pre-paint auth marks html.authed and data-user                           | /         | `kept:entry`      | the page's one boot, at the URL under test                                                                                                                                                                            |
| seeded: re-seed as the same user after logout boots authed                       | /         | `kept:entry`      | the page's one boot, at the URL under test                                                                                                                                                                            |
| seeded: re-seed as the same user after logout boots authed                       | /         | `kept:declared`   | the re-seeded marker is re-applied by the init script only on a fresh document load, and booting authed again is the subject                                                                                          |

### `backup.spec.ts` — 6 loads

| test                                                                       | url            | class           | reason                                                                                                    |
| -------------------------------------------------------------------------- | -------------- | --------------- | --------------------------------------------------------------------------------------------------------- |
| backup destination round-trips and clears via omission                     | /admin/backups | `kept:entry`    | the page's one boot, at the URL under test                                                                |
| backup destination round-trips and clears via omission                     | /admin/backups | `kept:declared` | a fresh load reads the persisted destination path back through backup::get_settings                       |
| backup destination round-trips and clears via omission                     | /admin/backups | `kept:declared` | a fresh load reads the cleared destination back through backup::get_settings to prove the None round-trip |
| backup mode select is generated from the enum variants                     | /admin/backups | `kept:entry`    | the page's one boot, at the URL under test                                                                |
| backup retention field gates submit until a count of at least 1 is entered | /admin/backups | `kept:entry`    | the page's one boot, at the URL under test                                                                |
| backup schedule field gates submit until a valid cron is entered           | /admin/backups | `kept:entry`    | the page's one boot, at the URL under test                                                                |

### `boot-marks.spec.ts` — 1 load

| test                                                    | url | class        | reason                                                                               |
| ------------------------------------------------------- | --- | ------------ | ------------------------------------------------------------------------------------ |
| the harness captures the full boot mark set after mount | /   | `kept:entry` | the cold boot IS the subject — the full boot mark set exists only on a document load |

### `email.spec.ts` — 5 loads

| test                                                     | url            | class           | reason                                                                                                       |
| -------------------------------------------------------- | -------------- | --------------- | ------------------------------------------------------------------------------------------------------------ |
| email form gates submit until a valid address is entered | /profile/email | `kept:entry`    | the page's one boot, at the URL under test                                                                   |
| email verification flow completes successfully           | /profile/email | `kept:entry`    | the page's one boot, at the URL under test                                                                   |
| email verification flow completes successfully           | /verify-email  | `kept:declared` | following the emailed verification link is an arrival from outside the app, exactly as a real recipient does |
| email verification flow completes successfully           | /profile/email | `kept:declared` | a fresh load reads the verified state back through the server, proving email::verify persisted               |
| visiting verify-email with invalid token shows error     | /verify-email  | `kept:entry`    | the page's one boot, at the URL under test                                                                   |

### `example.spec.ts` — 1 load

| test                                       | url | class        | reason                                     |
| ------------------------------------------ | --- | ------------ | ------------------------------------------ |
| homepage has title and links to intro page | /   | `kept:entry` | the page's one boot, at the URL under test |

### `feeds.spec.ts` — 3 loads

| test                                                                         | url    | class           | reason                                                                                                                                                                                            |
| ---------------------------------------------------------------------------- | ------ | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| auto-discovery links are present on site home and user timeline, and resolve | /      | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                                                        |
| auto-discovery links are present on site home and user timeline, and resolve | /~user | `kept:declared` | the document-served head link set for the user timeline is the subject; the sibling test already covers the client-side-nav case, so converting this one would collapse the two into one scenario |
| head discovery links update across a client-side nav, staying a single set   | /      | `kept:entry`    | the page's one boot; the nav under test is already client-side                                                                                                                                    |

### `invite.spec.ts` — 3 loads (project `chromium-admin`)

| test                                                                   | url       | class        | reason                                                         |
| ---------------------------------------------------------------------- | --------- | ------------ | -------------------------------------------------------------- |
| invite link registration completes end-to-end                          | /invites  | `kept:entry` | the page's one boot, at the URL under test                     |
| invite-only /register with no code shows guidance and no submit button | /register | `kept:entry` | ADR-0098 holdout: the invite-only guidance branch of /register |
| invites page shows not-found fallback when not invite-only             | /invites  | `kept:entry` | the page's one boot, at the URL under test                     |

### `media.spec.ts` — 8 loads

Every media test enters once. The three delete-guard tests and the two nav-link
tests deliberately enter at `/` and reach `/media` by clicking the sidebar link
— already an in-app move, already counted as zero extra loads.

| test                                                         | url        | class        | reason                                                               |
| ------------------------------------------------------------ | ---------- | ------------ | -------------------------------------------------------------------- |
| a post embedding the AtomPub member URL blocks deletion      | /          | `kept:entry` | the page's one boot; `/media` is reached by clicking the nav link    |
| a post embedding the raw filename spelling blocks deletion   | /          | `kept:entry` | the page's one boot; `/media` is reached by clicking the nav link    |
| deleting media referenced by a post is refused, then forced  | /          | `kept:entry` | the page's one boot; `/media` is reached by clicking the nav link    |
| media manage page is reachable via nav link                  | /          | `kept:entry` | the page's one boot; reaching /media via the nav link is the subject |
| media nav link appears for authenticated users               | /          | `kept:entry` | the page's one boot, at the URL under test                           |
| the media row decodes its label but not its delete key       | /          | `kept:entry` | the page's one boot; `/media` is reached by clicking the nav link    |
| upload widget on create-post page uploads file and shows URL | /posts/new | `kept:entry` | the page's one boot, at the URL under test                           |
| upload widget on the /app cockpit uploads file and shows URL | /app       | `kept:entry` | the page's one boot, at the URL under test                           |

### `password_reset.spec.ts` — 6 loads

| test                                                                         | url              | class           | reason                                                                                                                                                                    |
| ---------------------------------------------------------------------------- | ---------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| forgot-password for user without verified email shows contact operator error | /forgot-password | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                                |
| password reset flow completes successfully                                   | /forgot-password | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                                |
| password reset flow completes successfully                                   | /reset-password  | `kept:declared` | following the emailed reset link is an arrival from outside the app, exactly as a real recipient does                                                                     |
| password reset flow completes successfully                                   | /login           | `converted`     | redundant: the reset submit already triggers the router's client-side redirect to /login and the test awaits it, so the assertions can simply run where the router landed |
| reset-password rejects a too-short password client-side                      | /reset-password  | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                                |
| visiting reset-password with invalid token shows error                       | /reset-password  | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                                |

### `posts.spec.ts` — 90 loads

30 of the 34 `registeredPage` fixture boots here are `removed`.

| test                                                                     | url                                 | class           | reason                                                                                                                                                      |
| ------------------------------------------------------------------------ | ----------------------------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| authenticated user can create a post through the UI                      | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| authenticated user can create a post through the UI                      | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| authenticated user can create a post with a summary                      | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| authenticated user can create a post with a summary                      | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| authenticated user can create a post with a summary                      | /~user/2026/08/10/summary-test      | `converted`     | click the publish flash's `[data-test="permalink-link"]`; the subject is that the summary persisted, not the cold render                                    |
| authenticated user can delete a draft from the drafts page               | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| authenticated user can delete a draft from the drafts page               | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| authenticated user can delete a draft from the drafts page               | /drafts                             | `converted`     | click the sidebar's `a[href="/drafts"]` nav link                                                                                                            |
| authenticated user can delete a published post                           | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| authenticated user can delete a published post                           | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| authenticated user can delete a published post                           | /~user/2026/08/10/post-to-delete    | `converted`     | click the publish flash's permalink link                                                                                                                    |
| authenticated user can delete a published post                           | /~user/2026/08/10/post-to-delete    | `kept:declared` | after the delete succeeds the app offers no control back to the deleted permalink; the fresh load is what proves it now serves Post not found               |
| authenticated user can delete a published post                           | /~user                              | `kept:declared` | the deleted post's permalink renders a not-found page with no link to the author timeline, so the timeline exclusion must be checked from a fresh load      |
| authenticated user can edit a draft post                                 | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| authenticated user can edit a draft post                                 | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| authenticated user can edit a draft post                                 | /~user/2026/08/10/original-draft    | `converted`     | click the save-summary's permalink link                                                                                                                     |
| authenticated user can edit a draft post                                 | /posts/19/edit                      | `converted`     | click the PostCard's `.j-post-acts a:has-text("Edit")` affordance the test already reads the id from                                                        |
| authenticated user can save a draft through the UI                       | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| authenticated user can save a draft through the UI                       | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| clearing a post summary on edit persists as empty                        | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| clearing a post summary on edit persists as empty                        | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| clearing a post summary on edit persists as empty                        | /~user/2026/08/10/clearable         | `converted`     | click the save-summary's permalink link                                                                                                                     |
| clearing a post summary on edit persists as empty                        | /posts/16/edit                      | `converted`     | click the PostCard's Edit affordance                                                                                                                        |
| clearing a post summary on edit persists as empty                        | /posts/16/edit                      | `kept:declared` | reopening the editor from cold is what proves the emptied summary was persisted as None rather than held in client state                                    |
| cockpit /app shows the authenticated home feed with pagination           | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| create post with tags via UI: tags persist and appear on the post        | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| create post with tags via UI: tags persist and appear on the post        | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| create post with tags via UI: tags persist and appear on the post        | /~user/2026/08/10/tagged-post       | `converted`     | click the publish flash's permalink link                                                                                                                    |
| draft lifecycle: create, view, edit, and publish                         | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| draft lifecycle: create, view, edit, and publish                         | /drafts                             | `converted`     | click the sidebar's `a[href="/drafts"]` nav link                                                                                                            |
| draft lifecycle: create, view, edit, and publish                         | /posts/22/edit                      | `converted`     | click the draft row's `a:has-text("Edit")` the test already reads the href from                                                                             |
| draft lifecycle: create, view, edit, and publish                         | /~user/2026/08/10/lifecycle-draft   | `converted`     | click the draft row's `a:has-text("Permalink")` the test already reads the href from                                                                        |
| draft lifecycle: create, view, edit, and publish                         | /~user/2026/08/10/lifecycle-draft   | `converted`     | click the save-summary's "View post" link after the edit save                                                                                               |
| edit page pre-selects the post's current audience                        | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| edit page pre-selects the post's current audience                        | /audiences                          | `kept:entry`    | the named audience must exist before the composer renders its checkbox, so /audiences is the entry                                                          |
| edit page pre-selects the post's current audience                        | /posts/new                          | `kept:declared` | the app exposes no link to /posts/new anywhere, so the composer cannot be reached by an in-app control                                                      |
| edit page pre-selects the post's current audience                        | /~user/2026/08/10/targeted-draft    | `converted`     | click the save-summary's permalink link                                                                                                                     |
| edit page pre-selects the post's current audience                        | /posts/20/edit                      | `converted`     | click the PostCard's Edit affordance                                                                                                                        |
| editing a post updates tag chips and tag listing pages                   | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| editing a post updates tag chips and tag listing pages                   | /posts/189/edit                     | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| editing a post updates tag chips and tag listing pages                   | /tags/xeditc                        | `kept:declared` | the removed tag's chip no longer exists on the permalink, so nothing in the app links to /tags/xeditc — the empty listing must be checked from a fresh load |
| editing a post updates tag chips and tag listing pages                   | /tags/xeditd                        | `converted`     | click the permalink's `.j-tag-list` chip for `#xeditd`                                                                                                      |
| editing a published post freezes the slug                                | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| editing a published post freezes the slug                                | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| editing a published post freezes the slug                                | /~user/2026/08/10/published-article | `converted`     | click the publish flash's permalink link                                                                                                                    |
| editing a published post freezes the slug                                | /posts/21/edit                      | `converted`     | click the PostCard's Edit affordance                                                                                                                        |
| editing an invalid or nonexistent post shows not-found                   | /posts/abc/edit                     | `kept:entry`    | the cold load of an unparseable edit route is the subject (#487's `None` arm)                                                                               |
| editing an invalid or nonexistent post shows not-found                   | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| editing an invalid or nonexistent post shows not-found                   | /posts/999999999/edit               | `kept:declared` | the cold load of a well-formed-but-nonexistent edit route is a distinct subject (the `Some(id)` server path) from the unparseable one                       |
| inline composer: draft flash links to the draft's canonical permalink    | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: draft flash links to the draft's canonical permalink    | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| inline composer: flash clears when user starts typing                    | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: flash clears when user starts typing                    | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| inline composer: format toggle switches active button                    | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: format toggle switches active button                    | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| inline composer: markdown heading becomes article title                  | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: markdown heading becomes article title                  | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| inline composer: plain body publishes titleless note                     | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: plain body publishes titleless note                     | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| inline composer: publish flash is a link to the post permalink           | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: publish flash is a link to the post permalink           | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| inline composer: published post appears in timeline without page reload  | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| inline composer: published post appears in timeline without page reload  | /app                                | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| over-long post summary shows an inline error and gates submit            | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| over-long post summary shows an inline error and gates submit            | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| per-user timeline lists published posts with pagination                  | /~user                              | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| published post renders at permalink                                      | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| published post renders at permalink                                      | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| published post renders at permalink                                      | /~user/2026/08/10/permalink-story   | `kept:declared` | the cold render of the permalink is what this test asserts                                                                                                  |
| scheduling a post shows a Scheduled-for badge on the drafts page         | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| scheduling a post shows a Scheduled-for badge on the drafts page         | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| scheduling a post shows a Scheduled-for badge on the drafts page         | /drafts                             | `converted`     | click the sidebar's `a[href="/drafts"]` nav link                                                                                                            |
| tag chip on permalink navigates to site tag listing                      | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| tag chip on permalink navigates to site tag listing                      | /~user/2026/08/10/chip-nav-post     | `kept:entry`    | the page's one boot, at the URL under test; the chip click that follows is already in-app                                                                   |
| TagInput autocomplete suggests existing tags                             | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| TagInput autocomplete suggests existing tags                             | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| TagInput: Backspace on empty input removes last chip                     | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| TagInput: Backspace on empty input removes last chip                     | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| TagInput: Escape dismisses autocomplete without adding a chip            | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| TagInput: Escape dismisses autocomplete without adding a chip            | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| TagInput: invalid tag text shows an error                                | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| TagInput: invalid tag text shows an error                                | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| TagInput: keyboard navigation selects autocomplete item                  | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| TagInput: keyboard navigation selects autocomplete item                  | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| unpublishing from a permalink navigates to /drafts without a full reload | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| unpublishing from a permalink navigates to /drafts without a full reload | /posts/new                          | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |
| unpublishing from a permalink navigates to /drafts without a full reload | /~user/2026/08/10/unpublish-me      | `converted`     | click the publish flash's permalink link; the /drafts move that follows is already client-side                                                              |
| unseeded client-nav to / paints Loading with the masthead intact         | /forgot-password                    | `kept:entry`    | the page's one boot; entering on a non-`/` URL is the whole point (a document entered on `/` is projector-seeded and never reaches the Loading arm)         |
| user tag page lists that user's tagged posts                             | /                                   | `removed`       | fixture boot; nothing is asserted at `/`                                                                                                                    |
| user tag page lists that user's tagged posts                             | /~user/tags/utaga                   | `kept:entry`    | the page's one boot, at the URL under test                                                                                                                  |

**`posts.spec.ts` subtotal:** removed 30, converted 19, kept:entry 34,
kept:declared 7 = **90**.

### `profile.spec.ts` — 22 loads

Every test takes the `registeredPage` boot at `/` and immediately goes to
`/profile`. The surviving second (and third) `/profile` loads are idiom 3 — the
re-read **is** the assertion (`profile.spec.ts:26`).

| test                                                          | url      | class           | reason                                                                                             |
| ------------------------------------------------------------- | -------- | --------------- | -------------------------------------------------------------------------------------------------- |
| clearing the bio persists as empty                            | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| clearing the bio persists as empty                            | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| clearing the bio persists as empty                            | /profile | `kept:declared` | a fresh load reads the persisted value back through profile::get                                   |
| clearing the bio persists as empty                            | /profile | `kept:declared` | a fresh load reads the cleared bio back through profile::get to prove the None round-trip          |
| clearing the display name persists as empty                   | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| clearing the display name persists as empty                   | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| clearing the display name persists as empty                   | /profile | `kept:declared` | a fresh load reads the persisted value back through profile::get                                   |
| clearing the display name persists as empty                   | /profile | `kept:declared` | a fresh load reads the cleared display name back through profile::get to prove the None round-trip |
| default post format round-trips through the typed dispatch    | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| default post format round-trips through the typed dispatch    | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| default post format round-trips through the typed dispatch    | /profile | `kept:declared` | a fresh load reads the saved default post format back through get_default_post_format              |
| default post format round-trips through the typed dispatch    | /profile | `kept:declared` | the second flip's fresh load proves the persisted value is the selected one, not a constant        |
| over-long bio shows an inline error and gates submit          | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| over-long bio shows an inline error and gates submit          | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| over-long display name shows an inline error and gates submit | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| over-long display name shows an inline error and gates submit | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| profile update persists a valid bio                           | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| profile update persists a valid bio                           | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| profile update persists a valid bio                           | /profile | `kept:declared` | a fresh load reads the persisted value back through profile::get                                   |
| profile update persists a valid display name                  | /        | `removed`       | fixture boot; nothing is asserted at `/`                                                           |
| profile update persists a valid display name                  | /profile | `kept:entry`    | the page's one boot, at the URL under test                                                         |
| profile update persists a valid display name                  | /profile | `kept:declared` | a fresh load reads the persisted value back through profile::get                                   |

### `theme.spec.ts` — 1 load

| test                                                       | url | class        | reason                                     |
| ---------------------------------------------------------- | --- | ------------ | ------------------------------------------ |
| issue #22: .j-root keeps a real data-theme after CSR mount | /   | `kept:entry` | the page's one boot, at the URL under test |

### `timeline-cls.spec.ts` — 4 loads

All four are the CLS probe's frozen first paint — the raw `page.goto` at
`layout-shift.ts:67`, which deliberately holds the wasm so mount never
completes.

| test                                                                | url               | class        | reason                                              |
| ------------------------------------------------------------------- | ----------------- | ------------ | --------------------------------------------------- |
| / : projector paint does not shift across mount                     | /                 | `kept:entry` | the CLS probe's cold projector paint IS the subject |
| /~:username : projector paint does not shift across mount           | /~user            | `kept:entry` | the CLS probe's cold projector paint IS the subject |
| /~:username/tags/:tag : projector paint does not shift across mount | /~user/tags/~user | `kept:entry` | the CLS probe's cold projector paint IS the subject |
| /tags/:tag : projector paint does not shift across mount            | /tags/~user       | `kept:entry` | the CLS probe's cold projector paint IS the subject |

### `unicode-slug.spec.ts` — 6 loads

| test                                                               | url                             | class           | reason                                                                             |
| ------------------------------------------------------------------ | ------------------------------- | --------------- | ---------------------------------------------------------------------------------- |
| a Unicode-titled post is reachable at its permalink                | /                               | `removed`       | fixture boot; nothing is asserted at `/`                                           |
| a Unicode-titled post is reachable at its permalink                | /posts/new                      | `kept:entry`    | the page's one boot, at the URL under test                                         |
| a Unicode-titled post is reachable at its permalink                | /~user/2026/08/10/caf%C3%A9-... | `kept:declared` | the cold render of the percent-encoded Unicode permalink is what this test asserts |
| an emoji-only title falls back to the 'post' slug and is reachable | /                               | `removed`       | fixture boot; nothing is asserted at `/`                                           |
| an emoji-only title falls back to the 'post' slug and is reachable | /posts/new                      | `kept:entry`    | the page's one boot, at the URL under test                                         |
| an emoji-only title falls back to the 'post' slug and is reachable | /~user/2026/08/10/post          | `kept:declared` | the cold render of the fallback `post` permalink is what this test asserts         |

### `visibility.spec.ts` — 7 test-attributed loads

Secondary-page loads for this file are in the next section — this file carries
12 of the 20.

| test                                                                                   | url                              | class           | reason                                                                                                   |
| -------------------------------------------------------------------------------------- | -------------------------------- | --------------- | -------------------------------------------------------------------------------------------------------- |
| Named audience: assigned member sees a Friends post; an unassigned non-member does not | /audiences                       | `kept:entry`    | the Friends audience must exist and hold X before the composer can target it                             |
| Named audience: assigned member sees a Friends post; an unassigned non-member does not | /posts/new                       | `kept:declared` | the app exposes no link to /posts/new anywhere, so the composer cannot be reached by an in-app control   |
| Private post: hidden from anonymous and non-subscriber, visible to author              | /posts/new                       | `kept:entry`    | the page's one boot, at the URL under test                                                               |
| Private post: hidden from anonymous and non-subscriber, visible to author              | /~user/2026/08/10/private-secret | `converted`     | click the publish flash's permalink link; the subject is that the author can see it, not the cold render |
| Public post is visible to anonymous and appears in the feed; Subscribers post does not | /posts/new                       | `kept:entry`    | the page's one boot, at the URL under test                                                               |
| Public post is visible to anonymous and appears in the feed; Subscribers post does not | /posts/new                       | `kept:declared` | the second post needs the composer again and the app exposes no link to /posts/new                       |
| Subscribers post: visible after Subscribe, hidden again after Unsubscribe              | /posts/new                       | `kept:entry`    | the page's one boot, at the URL under test                                                               |

## Secondary-page loads (`e2e.page`)

These 20 loads sit on `e2e.page` spans that carry **no** `navigation_top_json`
(#895), so no URL is recorded in the trace. Each row's URL below is derived from
the test source, not from the trace, and is marked _(derived)_.

Every one of these pages is a fresh `context.newPage()` (or a fixture-setup
page) and therefore gets its **own** entry boot — the budget unit is the
Playwright `Page`, not the test.

| test (file)                                                                  | page                     | url _(derived)_         | class           | reason                                                                                                                         |
| ---------------------------------------------------------------------------- | ------------------------ | ----------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Audiences: CRUD + membership toggle re-fetch… (`audiences.spec.ts`)          | `xPage`                  | /~author                | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Audiences: a failed subscriber-roster fetch… (`audiences.spec.ts`)           | `xPage`                  | /~author                | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Audiences: refresh pulls a mid-session new subscriber… (`audiences.spec.ts`) | `xPage`                  | /~author                | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| password reset flow completes successfully (`password_reset.spec.ts`)        | `verifiedUser` setup     | /profile/email          | `kept:entry`    | the fixture's throwaway page's own entry boot                                                                                  |
| password reset flow completes successfully (`password_reset.spec.ts`)        | `verifiedUser` setup     | /verify-email?token=…   | `kept:declared` | following the emailed verification link is an arrival from outside the app, exactly as a real recipient does                   |
| draft lifecycle: create, view, edit, and publish (`posts.spec.ts`)           | `guestPage`              | permalink               | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| home page shows local timeline for unauthenticated users (`posts.spec.ts`)   | `guestPage`              | /                       | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Private post: hidden from anonymous… (`visibility.spec.ts`)                  | `expectPostHidden` anon  | permalink               | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Private post: hidden from anonymous… (`visibility.spec.ts`)                  | `expectPostHidden` other | permalink               | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Private post: hidden from anonymous… (`visibility.spec.ts`)                  | `anonPage`               | /~author                | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Subscribers post: visible after Subscribe… (`visibility.spec.ts`)            | `viewerPage`             | permalink               | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Subscribers post: visible after Subscribe… (`visibility.spec.ts`)            | `viewerPage`             | /~author                | `kept:declared` | the "Post not found" page the viewer is looking at links nowhere; the author's profile is where Subscribe lives                |
| Subscribers post: visible after Subscribe… (`visibility.spec.ts`)            | `viewerPage`             | permalink               | `kept:declared` | re-reading the permalink through the server after subscribing is the assertion                                                 |
| Subscribers post: visible after Subscribe… (`visibility.spec.ts`)            | `viewerPage`             | /~author                | `kept:declared` | the permalink page links nowhere back to the author's profile, where Unsubscribe lives                                         |
| Subscribers post: visible after Subscribe… (`visibility.spec.ts`)            | `viewerPage`             | permalink               | `kept:declared` | re-reading the permalink through the server after unsubscribing is the assertion                                               |
| Named audience: assigned member sees a Friends post… (`visibility.spec.ts`)  | `xPage`                  | /~author                | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Named audience: assigned member sees a Friends post… (`visibility.spec.ts`)  | `xPage`                  | friendsPermalink        | `kept:declared` | the author's profile page carries no link to another user's arbitrary permalink; X's admitted read must come from a fresh load |
| Named audience: assigned member sees a Friends post… (`visibility.spec.ts`)  | `yPage`                  | friendsPermalink        | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| Public post is visible to anonymous… (`visibility.spec.ts`)                  | `expectPostVisible`      | publicPermalink         | `kept:entry`    | a freshly created page's own entry boot                                                                                        |
| invite link registration completes end-to-end (`invite.spec.ts`)             | `invitee`                | /register?invite_code=… | `kept:entry`    | a freshly created page's own entry boot; ADR-0098 holdout — invite-gated registration through the real UI                      |

**Secondary-page subtotal:** kept:entry 14, kept:declared 6 = **20**.

Reconciliation: 3 (audiences) + 2 (password_reset) + 2 (posts) + 12
(visibility) + 1 (invite) = 20, matching the 20 loads on the 14 `e2e.page` spans
that carry a non-zero count.

## Coverage movement

Destination pages that currently receive an **incidental** cold render and will
stop receiving one. Each entry names the test that provided it and where the
subject still has cold coverage.

| destination                | test that provided the incidental cold render                                                                                                                                                                | still covered cold by                                                                                                                                                                                             |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/` (public site timeline) | 30 `registeredPage` fixture boots across `posts.spec.ts`, `profile.spec.ts`, `unicode-slug.spec.ts`, `authed-flash.spec.ts`                                                                                  | `example.spec.ts`, `theme.spec.ts`, `timeline-cls.spec.ts` `/`, `boot-marks.spec.ts`, `auth.spec.ts` ×5, `authed-flash.spec.ts` ×5, `media.spec.ts` ×6, `feeds.spec.ts` ×2                                        |
| post permalink (published) | `authenticated user can create a post with a summary`, `create post with tags via UI`, `editing a published post freezes the slug`, `unpublishing from a permalink…`, `Private post: hidden from anonymous…` | `published post renders at permalink` (`kept:declared`), both `unicode-slug.spec.ts` tests, `tag chip on permalink navigates to site tag listing`, and 5 secondary-page permalink entries in `visibility.spec.ts` |
| post permalink (draft)     | `authenticated user can edit a draft post`, `clearing a post summary on edit persists as empty`, `edit page pre-selects the post's current audience`, `draft lifecycle` ×2                                   | `draft lifecycle`'s `guestPage` entry at the draft permalink                                                                                                                                                      |
| `/posts/{id}/edit`         | `authenticated user can edit a draft post`, `clearing a post summary on edit…` (first of two), `edit page pre-selects…`, `editing a published post freezes the slug`, `draft lifecycle`                      | `editing a post updates tag chips and tag listing pages` (entry), `clearing a post summary on edit…` (second load, `kept:declared`), `editing an invalid or nonexistent post shows not-found` ×2                  |
| `/drafts`                  | `authenticated user can delete a draft from the drafts page`, `scheduling a post shows a Scheduled-for badge…`, `draft lifecycle`                                                                            | **nothing.** After this change no test loads `/drafts` cold.                                                                                                                                                      |
| `/tags/{tag}`              | `editing a post updates tag chips and tag listing pages` (`/tags/xeditd`)                                                                                                                                    | `timeline-cls.spec.ts` `/tags/:tag`, and `/tags/xeditc` in the same test (`kept:declared`)                                                                                                                        |
| `/login`                   | `password reset flow completes successfully`                                                                                                                                                                 | `auth.spec.ts` ×4, `authed-flash.spec.ts` `operator: admin chrome…`                                                                                                                                               |
| `/posts/new`               | (none removed — every `/posts/new` load is an entry or a declared second load)                                                                                                                               | 13 entries plus 2 declared loads                                                                                                                                                                                  |

**The one real loss is `/drafts`.** Its three cold loads all become in-app moves
and nothing else in the suite boots there. Two of the three tests
(`delete a draft from the drafts page`, `scheduling a post…`) still assert on
the drafts listing's rendered content after the in-app arrival, so the route's
rendering stays covered — what is lost is coverage of `/drafts` **as an entry
URL** (its projector/shell path on a cold document load). No assertion is
deleted (spec A10). If that is judged too much to lose, the cheapest repair is
to make one of those two tests enter at `/drafts` and reach the composer instead
— a swap, not an addition.

## Subject changes

Tests whose subject moved, as opposed to tests that merely reach the same place
by a different route. Populated as conversions land; quoted into the Task 11
write-up (spec A10). No `expect` assertion has been deleted or weakened in any
of them.

**Task 6b — `editing a post updates tag chips and tag listing pages`.** The two
tag-listing assertions are **reordered**. `/tags/xeditd` (the retained tag) is
now reached by clicking the post's own `#xeditd` chip, and that chip exists only
while the page is still on the permalink — so it must be checked before the
`/tags/xeditc` load, which leaves the permalink behind. Previously both were
independent cold loads in the other order. What each assertion checks is
unchanged; what changed is that one of them now also depends on the chip being a
working in-app link.

**Task 6b — `draft lifecycle: create, view, edit, and publish`.** A reload of
the already-displayed permalink is deleted (Amendment 1). Nothing it asserted is
lost, and the `__jaunderNoReload` sentinel that follows is strengthened by the
removal.

**Task 7 — `password reset flow completes successfully`.** The old- and
new-password login assertions used to run on a **cold** `/login` that the test
loaded itself, immediately after awaiting the router's own client-side redirect
to that same URL. The `goto` is deleted, so they now run on the page the reset
flow actually landed on. The subject moves from "a freshly loaded `/login`
accepts the reset credentials" to "the page the reset flow lands on accepts
them" — which is the more faithful scenario, since that is what a real user
sees. `/login` keeps cold coverage in `auth.spec.ts` (×4) and `authed-flash.ts`,
as the coverage-movement table records.

**Task 7 —
`Private post: hidden from anonymous and non-subscriber, visible to author`.**
The author's own read of the permalink moves from a cold document load to an
in-app click on the publish flash's link. The assertion is unchanged; what it
now also depends on is that the flash's permalink link works as an in-app route.
The four other viewers in this test (anonymous, non-subscriber, and the two
`expectPostHidden` pages) still read the permalink cold, so the gate's cold path
keeps its coverage.

## Uncertain — review these

Nine rows where the conversion was not obviously safe. All are classified
`kept:declared`, which is the conservative choice: a wrong `kept:declared` costs
one document load and a stale declaration, whereas a wrong `converted` produces
a green-but-wrong test or a flake.

1. **Every mid-flow arrival at `/posts/new`** (3 rows: `posts.spec.ts`
   `edit page pre-selects the post's current audience`; `visibility.spec.ts`
   `Named audience…` and `Public post is visible to anonymous…`). Searching
   `web/src` and `server/src` finds **no reference to `/posts/new` at all** —
   the app has no compose link. A router push would technically work, but the
   spec's rule is not to invent an affordance the app does not offer.

   **Confirmed and filed as
   [#896](https://github.com/jaunder-org/jaunder/issues/896).** The route is
   registered at `web/src/app/component.rs:127`, and the sidebar's `NAV_ITEMS`
   table (`web/src/sidebar/markup.rs:9-30`) lists Home, Feed, Drafts, Media and
   Audiences — no compose entry. A cold load genuinely is the only way a user
   reaches that page, so `kept:declared` is not a fudge here; it is the honest
   classification, and these 3 rows **stay** `kept:declared` for this cycle.
   `PREDICTED_TOTAL` remains 170. If #896 resolves by adding an affordance, they
   become `converted` and the total falls to 167.

2. **The two post-delete loads** in `posts.spec.ts`
   `authenticated user can delete a published post` (permalink again, then
   `/~user`). After the delete the page shows a success flash and the permalink
   is gone; nothing in the rendered DOM links back to either destination. A
   same-URL router push may also not re-run the fetch, which would make the
   not-found assertion vacuous.

3. **`/tags/xeditc`** in `posts.spec.ts`
   `editing a post updates tag chips and tag listing pages`. The chip for the
   removed tag is, by construction, no longer on the page — that is what the
   test proves — so no link exists.

4. **The three `viewerPage` profile/permalink loads** in `visibility.spec.ts`
   `Subscribers post: visible after Subscribe, hidden again after Unsubscribe`
   (the two `/~author` loads via `subscribeTo`/`unsubscribeFrom`, plus the
   `/~author` reached from a "Post not found" page). The "Post not found" page
   renders no author link, and the permalink page's own author handle may or may
   not be a link to the profile — this was not verified against
   `web/src/posts/render.rs`. If the handle **is** a link, the second and third
   of these become `converted` and `PREDICTED_TOTAL` falls by 2. Task 7 should
   check `render.rs:203` before writing the declarations.

The remaining `kept:declared` rows are not uncertain: persistence re-reads
(`profile`, `admin-site`, `backup`, `email`, the second `/posts/N/edit`), the
named cold-render subjects (permalink render, boot marks, flash/CLS, the
not-found edit routes), the emailed-link arrivals, and the pre-paint/marker
boots in `authed-flash.spec.ts` are all doing a job the spec explicitly
protects.

## Amendment 3 (2026-08-11) — the persistence-reload premise was wrong, and five loads go

This file is archived; the sections above are left as written. This records what
a PR-time review found and what changed in response.

**The false premise.** Thirteen `kept:declared` rows above justify a reload with
some form of "a fresh load reads the persisted value back through
`profile::get` / `site::get` / `backup::get_settings`". The implied claim — that
only a document load re-reads from the server — is untrue. Each page's
`Resource` is created **at component mount** (e.g.
`web/src/profile/component.rs:15`,
`Resource::new(move || update_action.version().get(), |_| get())`), so an
**in-app re-entry of the route remounts the page and refetches too**; and the
resource already refetches on `update_action.version()`, so the value asserted
straight after a save may itself be a server re-read. The premise came from the
spec, was inherited by every stage that followed, and survived two soundness
reviews because nobody tested it.

**The rule that decides each row is unchanged** ([#896](https://github.com/jaunder-org/jaunder/issues/896)): a test never invents an
affordance the app does not have. So the outcome differs by route, and it was
checked against the source rather than assumed:

- **`/admin/site` and `/admin/backups` — a real affordance exists.**
  `web/src/sidebar/component.rs:115-133` renders "Configure Backups" and "Site
  Settings" nav links for an operator. The five round-trip reloads there are now
  in-app re-entries (leave to the sibling admin route, come back), and their
  declarations are deleted. `SiteSettingsPage`/`BackupSettingsPage` create their
  `Resource` at mount, so the return trip repopulates the form from the server —
  which is what the assertions need.
- **`/profile` — no affordance exists.** It is in neither `NAV_ITEMS`
  (`web/src/sidebar/markup.rs`) nor the sidebar footer, which carries only the
  avatar and a "Sign out" link, and nothing else in `web/src` links to it. Its
  seven loads **stay**, and their reasons are rewritten to the true one: there is
  no in-app control that re-enters the route, so a document load is the only way
  to remount the page and re-read the value.
- **The two `/admin/site` banner rows are untouched.** "Painted from the
  boot-time site config" is a claim about a cold boot and is true as written.
- **`/profile/email`'s row is left as written.** Its route has no affordance
  either, so the load stays regardless; only its wording shares the weak shape.

**Proved, not assumed.** Each converted test was checked to still catch the
regression it exists to catch, by breaking persistence at the server fn
(`site::update_identity` and `backup::update_settings` returning `Ok(())`
without writing) and confirming the converted assertion goes red: `backup
destination round-trips and clears via omission` failed with
`Expected "/srv/jaunder/backups" / Received ""`, and both converted `admin-site`
tests failed the same way. Both breaks were then reverted.

**Count.** Five document loads per run go away (three `admin-site`, two
`backup`), and the declaration census drops from 38 to **33** call sites across
the product specs (32 exact `allowSecondBoot` plus the one
`allowEngineDependentBoot`).

**`docs/observability.md` is deliberately not updated.** Its measured numbers
describe the branch as measured before this change; editing them to match would
turn a measurement into an assertion. The divergence is five fewer loads per run
than the arms it reports.
