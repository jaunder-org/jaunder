# #873 — e2e round-trip coverage for the format toggle on all three composer surfaces

## Summary

`FormatToggle` (`web/src/posts/component.rs:81`) renders on three surfaces:

| Surface          | Route             | Where the toggle sits                             |
| ---------------- | ----------------- | ------------------------------------------------- |
| compact composer | `/app`            | `.j-composer-toolbar` (`component.rs:577`)        |
| full composer    | `/posts/new`      | the options aside, via `ComposeOptions` (`:1271`) |
| post editor      | `/posts/:id/edit` | the same `ComposeOptions` (`:1271`)               |

Two gaps exist, and the second is wider than the issue as filed states:

1. **No coverage on the two full-page shapes.**
   `end2end/tests/posts.spec.ts:715` ("inline composer: format toggle switches
   active button") is the sole toggle test and drives only `.j-composer`. A
   regression on `/posts/new` or the editor ships green.
2. **The toggle's _effect_ is untested on every surface, `/app` included.**
   `:715` asserts only that the `is-selected` class moves. `posts.spec.ts:130`
   does click Org on `/posts/new`, but asserts nothing about the format — only
   the draft summary and slug. So a toggle that flips its own highlight while
   staying disconnected from the format actually submitted would pass the whole
   suite.

This spec closes both: each of the three surfaces gets a **round-trip** test —
pick a format, save, follow the permalink, assert the post really rendered in
that format.

## Why a round-trip, and what discriminates the two formats

Asserting the CSS class proves the button is a radio group. It does not prove
the signal reaches `SavedPost.format`. The rendered output does.

`common/src/render.rs` pins both halves of the discriminator, in the two
`render_preserves_*` fixtures:

- **Markdown** — `*emphasis*` → `<em>emphasis</em>` (asserted at
  `render.rs:1647`, body at `:1638`)
- **Org** — `*bold*` → `<b>bold</b>` (asserted at `render.rs:1666`, body at
  `:1660`)

Distinct element names, so the assertion is two-sided: the expected tag is
present **and** the other is absent. One-sided ("some bold exists") would pass
on a page that rendered both, or neither.

`*emphasis*` is safe as a whole body. An Org headline needs `* ` (asterisk
**space**): `canonicalize_org_body` strips a title-source heading only on
`t.starts_with("* ")` (`render.rs:803`) and `extract_org_title` matches likewise
(`:732`). So `*emphasis*` parses as inline bold, is not consumed as a title, and
does not trip the title-only `InvalidPostBody` rejection (`:760`).

## Decisions

**D1 — Round-trip, not class-only, on all three surfaces.** Each test asserts
the class flip _and_ the rendered result. The class assertion stays because it
localizes the failure: class-passed + render-failed points at the submit wiring,
both-failed at the toggle.

**D2 — The edit test starts from an _Org_ post and switches it to Markdown.**
This is what makes the prefill assertion meaningful. `ComposeState`'s initial
`format` is `Markdown` (`compose_state.rs:54`) and `seed_from` (`:104`,
`self.format.set(fetched.format)`) runs inside a `Suspend` block
(`component.rs:1050-1053`). So "open a Markdown post, assert Markdown is active"
would pass before the fetch resolves, and would pass identically if `seed_from`
never touched `format` at all — it cannot distinguish prefill from the hardcoded
default. Opening an **Org** post and asserting **Org** is active can only
succeed once the fetched format has actually landed. Switching that post to
Markdown then also exercises the helper's Markdown branch, which no other test
would (see D7).

**D3 — The post under edit is a draft, not a published post.** `EditSaveOutcome`
(`component.rs:1300-1327`) renders `.j-save-summary` with
`[data-test="permalink-link"]` **only** when `updated.published_at.is_none()`; a
published post falls to `Ok(_) => "Redirecting…"` (`:1324`) and navigates away,
leaving no permalink to read. A draft is reachable at its permalink —
`posts.spec.ts:178-184` already relies on exactly that.

**D4 — Reaching `/posts/:id/edit` uses the established two-hop route.** Neither
the save summary nor the permalink exposes a post id. The existing idiom
(`posts.spec.ts:178-189`) is: follow the permalink, wait for
`.j-post-acts a:has-text("Edit")`, and regex the id out of its `href`
(`/\/posts\/(\d+)\/edit/`). The edit test reuses it rather than inventing a
route. This is the most fragile step in the test and is called out so a later
failure there is recognized as infrastructure, not a format bug.

**D5 — Upgrade the existing `/app` test (`:715`) rather than deferring it.** The
compact composer has the same untested wiring. Its success flash is itself the
permalink anchor (`component.rs:717-718` — `<p class="success"><a href=url>`),
so the round-trip is reachable there too. The test keeps its existing class
assertions; the round-trip is appended.

_Amended during implementation:_ this decision originally said the test keeps
its **name** too. It was renamed —
`"inline composer: format toggle switches active button"` →
`"…: format toggle round-trips to the rendered post"` — because the old name
describes only the half of the test that existed before, and a name that
undersells its assertions misleads whoever next reads a failure. The assertions
were kept, which is what AC5 requires.

**D6 — Read each permalink `href` from the page the save just produced,
immediately.** Two reasons, one per surface shape:

- The compact composer's flash is **time-bounded and input-cleared** — a 30 s
  `set_timeout` (`component.rs:696`) plus an `on_input` reset (`:710`). The
  `href` must be captured right after
  `waitForSelector(page, ".j-composer p.success a")`, not after further
  interaction, or a cold Postgres/Firefox run can read a null href that looks
  like a product bug.
- On the full-page shapes the summary is simply the authoritative source, and
  re-reading it is cheaper than reasoning about when a slug could change. Note
  the edit path does **not** re-derive the slug — `EditPostPage` seeds
  `slug_field` from the stored slug (`component.rs:1054`) and `EditPostForm`
  passes `slug_field.parsed()` as the `slug_override` (`:1097`), so the save
  supplies it explicitly. (Nor would derivation drift here: `derive_post_naming`
  (`render.rs:618-659`) finds no title in `*emphasis*` in either format and
  falls to `first_meaningful_line` both times.) The rule is a cheap invariant,
  not a fix for an observed drift.

**D7 — One shared assertion helper in `end2end/tests/posts.ts`, three
independent tests.** The helper is post-specific, so it belongs beside
`composePost` and `createPostViaApi` in `posts.ts:22,56` — **not** in
`helpers.ts`, which holds auth/navigation/email plumbing (`goto`, `click`,
`login`, `registerViaUi`). Three independent tests rather than one parametrized
loop: `/posts/new` and the editor emit byte-identical summary markup
(`component.rs:767-779` and `:1311-1319`), but the compact composer's flash is a
bare `<a>` with no `data-test` hook — reachable only as
`.j-composer p.success a` — and the editor additionally carries the prefill and
two-hop-id steps. A table would be mostly per-surface branches.

**D8 — No ADR.** This adds coverage using established conventions
(`.j-seg button` idiom at `posts.spec.ts:130`, `.j-post-body` at `:164`).
Nothing here is a decision a future reader would reverse-engineer.

## Acceptance criteria

**AC1 — the helper exists, is two-sided, and its format argument is
load-bearing.** `end2end/tests/posts.ts` exports a helper that, given a
permalink href and an expected format (`"org" | "markdown"`), navigates there
and asserts within `.j-post-body` that the expected format's element (`b` for
Org, `em` for Markdown) is present with the probe text **and** that the other
format's element has count 0. Verifiable, and not merely by thought experiment:
AC4 calls it with `"markdown"` while AC2/AC5 call it with `"org"`, so a helper
that ignored its argument, or whose Markdown branch was broken, fails the suite.

**AC2 — `/posts/new` round-trip.** A test navigates to `/posts/new`, fills
`SEL.postBody` with the emphasis probe, asserts the `.j-seg` Markdown button
carries `is-selected` and Org does not, clicks Org, asserts the two have
swapped, publishes via `SEL.publishButton("true")`, reads the href from
`[data-test="permalink-link"]` inside `SEL.saveSummary`, and asserts via AC1's
helper that the post rendered as **Org**.

**AC3 — edit page prefill reflects the stored format.** A test creates a
**draft** whose format is **Org** (D2, D3), reaches `/posts/:id/edit` by the
two-hop route of D4, and asserts the `.j-seg` **Org** button carries
`is-selected` and Markdown does not — i.e. the toggle shows the fetched post's
format, which the Markdown default could not produce.

**AC4 — edit page change persists.** Continuing AC3's test: click Markdown,
assert the active button moved, save the draft via `SEL.publishButton("false")`,
read the href fresh from the editor's own `.j-save-summary`
`[data-test="permalink-link"]`, and assert via AC1's helper that the post now
renders as **Markdown**.

**AC5 — `/app` compact composer round-trip.** `posts.spec.ts:715` keeps its
existing default-state and class-flip assertions, and additionally fills the
body with the probe, publishes with Org selected, waits for and immediately
captures the href from `.j-composer p.success a` (D6), and asserts via AC1's
helper that the post rendered as **Org**.

**AC6 — the gate is green.** `cargo xtask validate` passes, including the full
e2e matrix (`{sqlite,postgres}×{chromium,firefox}`) — these tests must be stable
on all four combos, not just the local default.

## Non-goals

- **Changing any product code.** This is test-only. If a test fails because the
  wiring is genuinely broken, that is a separate issue, not a fix folded in
  here.
- **Covering `PostFormat::Html`.** It is renderer-internal and deliberately
  filtered out of the toggle (`component.rs:78`, #445).
- **Asserting the stored format via the API or database.** The rendered
  permalink is the user-observable contract and is what these tests assert.
- **Adding a `data-test` hook to the compact composer's flash anchor.** That is
  product markup; the existing `.j-composer p.success a` selector is sufficient
  here.
- **Refactoring the three surfaces to share more markup.** #871 and #872 own
  that.

## Risks

- **Runtime.** Three publish/save-and-navigate round-trips are added, each on 4
  backend×browser combos, and the edit test is the longest (create → permalink →
  edit → save → permalink). The two full-page tests likely need `test.slow()`,
  as `:139` and `:167` already use for the same shape.
- **The two-hop id lookup (D4)** depends on `.j-post-acts a:has-text("Edit")`
  being present on a draft's permalink page. It is the most markup-coupled step;
  it is also already load-bearing for `posts.spec.ts:167`, so a break there
  breaks an existing test too.
- **Compact-composer flash expiry (D6)** is the main flakiness vector; mitigated
  by capturing the href immediately after the wait.
- **Org/Markdown emphasis assumption** is pinned by unit tests at
  `render.rs:1647` and `:1666`; if that behavior changes, these tests fail
  loudly rather than passing silently.
