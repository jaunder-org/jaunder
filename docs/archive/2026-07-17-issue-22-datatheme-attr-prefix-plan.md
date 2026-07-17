# Plan — Issue #22: `.j-root` `data-theme` `attr:`-prefix leak

Spec:
[`2026-07-17-issue-22-datatheme-attr-prefix-spec.md`](2026-07-17-issue-22-datatheme-attr-prefix-spec.md)

## Review header

- **Goal:** After CSR hydration, `.j-root` carries a real `data-theme="studio"`
  (not a literal `attr:data-theme`), so the studio theme's token overrides
  apply. Add the e2e coverage that was missing.
- **Scope in:** one-line fix at `web/src/pages/mod.rs:56` + an explanatory
  comment; new `end2end/tests/theme.spec.ts`.
- **Scope out:** `<ActionForm attr:class=...>` (correct spread — untouched); any
  lint/gate for `attr:` on plain elements; any `:root` CSS change. (Per spec.)
- **Tasks:** 1) RED — add the e2e spec, prove it fails on current code. 2) GREEN
  — drop the `attr:` prefix, prove it passes. 3) Gate + commit.
- **Key risks/decisions:**
  - The token assertion pins studio's `--accent-ink: #3a2fc9` vs the `:root`
    default `#5b4df0` — a genuine difference, so it proves
    `[data-theme="studio"]` matched, not just that an attribute exists.
    Custom-property reads come back as a raw string (possibly space-padded) —
    `.trim()` before comparing.
  - **leptosfmt** relocates comments inside a `view!` macro — put the
    explanatory comment on the line **above `view! {`**, not inline on the
    `<div>`.
  - **Nix ignores untracked files** — `git add` the new spec before
    `cargo xtask validate`, or the Nix e2e check won't see it.
  - e2e is the only layer that catches this (server string is already correct
    via the existing `render_shell` unit test); a Rust unit test cannot observe
    the hydrated DOM.
- **For agentic workers:** execute with `jaunder-iterate` (optionally delegating
  a task via `jaunder-dispatch`). This is a ~2-file change; inline execution is
  fine.

## Global constraints

- No `Co-Authored-By` trailer on commits.
- Fix and test land in **one** commit (bug + its regression test belong
  together); verify RED locally first rather than committing a broken
  intermediate.
- Follow `CONTRIBUTING.md`; TypeScript e2e follows the `end2end/` conventions
  (use `goto` from `./helpers`, `test`/`expect` from `./fixtures`).

---

## Task 1 — RED: add the post-hydration regression e2e spec

**Files:**

- Create `end2end/tests/theme.spec.ts`:

```ts
import { test, expect } from "./fixtures";
import { goto } from "./helpers";

// Regression for #22: the reactive data-theme binding on the plain `.j-root`
// element must survive CSR hydration. A leaked Leptos `attr:` directive prefix
// produced a literal `attr:data-theme` attribute, so `.j-root[data-theme=...]`
// stopped matching and no theme token overrides applied after the client booted.
test("issue #22: .j-root keeps a real data-theme after CSR hydration", async ({
  page,
}) => {
  await goto(page, "/"); // public projector home; goto() waits for hydration

  const probe = await page.evaluate(() => {
    const root = document.querySelector(".j-root");
    if (!root) return { found: false } as const;
    return {
      found: true as const,
      dataTheme: root.getAttribute("data-theme"),
      attrNames: Array.from(root.attributes).map((a) => a.name),
      accentInk: getComputedStyle(root).getPropertyValue("--accent-ink").trim(),
    };
  });

  expect(probe.found).toBe(true);
  if (!probe.found) return; // narrow the type for the assertions below

  // 1. Core regression: the attribute is real and named `data-theme`.
  expect(probe.dataTheme).toBe("studio");

  // 2. Pin the specific failure mode: no leaked `attr:`-prefixed attribute name.
  expect(probe.attrNames.some((n) => n.startsWith("attr:"))).toBe(false);

  // 3. Prove the [data-theme="studio"] selector actually matched: studio's
  //    --accent-ink (#3a2fc9) differs from the :root default (#5b4df0).
  expect(probe.accentInk).toBe("#3a2fc9");
});
```

**Run (expect FAIL on current code):**

```
cargo xtask e2e-local theme.spec.ts
```

Expected: fails at assertion 1 (`dataTheme` is `null` pre-fix), confirming the
test bites. Read the parked `.xtask/run/<id>.out` for the
`dataTheme`/`attrNames` probe values.

## Task 2 — GREEN: drop the `attr:` prefix

**Files:**

- `web/src/pages/mod.rs` (~line 55–56 in `AppShell`): change the binding and add
  the comment **above** the `view!` macro (leptosfmt-safe):

```rust
    // `data-theme` must be a plain dynamic attribute, NOT `attr:data-theme`:
    // the Leptos `attr:` directive prefix is only for spreading onto a
    // component; on a plain element it leaks a literal `attr:data-theme`
    // attribute into the hydrated DOM and the theme selector stops matching (#22).
    view! {
        <div class="j-root" data-theme=move || theme.get()>
```

(Only the `<div class="j-root" ...>` line changes: `attr:data-theme=` →
`data-theme=`.)

**Run (expect PASS):**

```
cargo xtask e2e-local theme.spec.ts
```

Expected: 1 passed — `dataTheme="studio"`, no `attr:`-prefixed name,
`--accent-ink` `#3a2fc9`.

## Task 3 — Gate + commit

- `git add end2end/tests/theme.spec.ts web/src/pages/mod.rs` (Nix must see the
  new spec).
- Run the full local gate (static + clippy + coverage + full e2e matrix):

```
cargo xtask validate
```

Expected: green. (`jaunder-commit`: the pre-commit hook reruns
`cargo xtask check`; run it clean first.)

- Commit fix + test together:

```
fix(web): render .j-root data-theme as a plain attribute so it survives hydration (#22)
```

## Done when

- [ ] `theme.spec.ts` fails on pre-fix code, passes after the fix (verified
      locally).
- [ ] `web/src/pages/mod.rs` uses `data-theme=move || theme.get()` with the
      leptosfmt-safe comment above `view!`.
- [ ] `cargo xtask validate` green.
- [ ] One commit referencing #22; no `Co-Authored-By` trailer.
