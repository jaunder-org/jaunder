# Spec — Issue #22: `.j-root` loses `data-theme` after CSR hydration

**Issue:** [#22](https://github.com/jaunder-org/jaunder/issues/22) · type: bug ·
milestone: Web: canonical Leptos CSR convergence · blocks #21 (theme picker)

## Problem

After the CSR client hydrates, `.j-root` carries no usable `data-theme`
attribute, so `.j-root[data-theme="studio"]` in `jaunder-themes.css` never
matches and **no theme's token overrides apply once the client boots**. The
server-rendered first paint is correct; hydration is what breaks it.

### Root cause (diagnosed and browser-verified)

`web/src/pages/mod.rs:56` binds the attribute using the Leptos directive prefix
on a **plain HTML element**:

```rust
<div class="j-root" attr:data-theme=move || theme.get()>
```

On a plain element `attr:` is not consumed as a directive — the hydrated DOM
ends up with an attribute literally **named** `attr:data-theme`. Observed in
chromium:

| post-hydration `.j-root`     | value                          |
| ---------------------------- | ------------------------------ |
| attribute names              | `["class", "attr:data-theme"]` |
| `getAttribute("data-theme")` | `null`                         |

The original visible symptom (invisible filled-red Delete buttons, `--err`) is
currently **masked**: `:root` in `server/assets/jaunder.css` now mirrors the
state tokens (`--err: #dc2626`), so `var(--err)` still resolves via the
fallback. The theme system is silently non-functional post-hydration rather than
visibly broken. The three SSR-era hypotheses in the issue body are obsolete
(they predate the #180/#192 CSR cutover).

## Scope

1. **Fix** — drop the `attr:` prefix at `web/src/pages/mod.rs:56` so it is a
   plain dynamic attribute:
   `<div class="j-root" data-theme=move || theme.get()>`. Add a short inline
   comment at the site noting the prefix must not be used on a plain element
   (prevents regression).
2. **Regression test** — add a permanent Playwright e2e spec asserting the
   post-hydration DOM state. This is the coverage gap that let the bug regress
   silently through the CSR migration: today the only theme-adjacent e2e
   (`end2end/tests/static-assets.spec.ts`) merely checks the CSS files return
   200; nothing inspects the hydrated `.j-root`.

### Out of scope (explicit)

- The three `attr:class` uses on `<ActionForm>` (`pages/site.rs:62`,
  `pages/backup.rs:58`, `pages/auth.rs:73,160`) — those are the **correct**
  component attribute-spread syntax and are left untouched.
- A lint/xtask gate banning `attr:` on plain elements. Only one instance exists;
  a bespoke AST gate is disproportionate. Noted as a possible future follow-up,
  not built here.
- Any change to the `:root` fallback pack — it already mirrors the state tokens
  and its defense-in-depth role is unaffected.

## Regression test design

- **File:** new `end2end/tests/theme.spec.ts` (theme is its own concern; keep it
  out of `static-assets.spec.ts`).
- **Page:** the public projector home `/` — the simplest route that reliably
  renders a hydrated `.j-root` with an empty DB (matches how the fix was
  verified).
- **Wait:** for `body[data-hydrated]` (set by `mount_csr`) so we assert the CSR
  DOM, not the server string.
- **Assertions (post-hydration):**
  1. `document.querySelector('.j-root').getAttribute('data-theme') === 'studio'`
     — the core regression (was `null`).
  2. `.j-root` has **no** attribute whose name starts with `attr:` — pins the
     specific failure mode (the literal-prefix leak) so a reintroduced `attr:`
     prefix fails loudly.
  3. A studio-only token override actually applies — assert
     `getComputedStyle(.j-root).getPropertyValue('--accent-ink')` equals the
     studio value (`#3a2fc9`) and differs from the `:root` default (`#5b4df0`).
     This proves the `[data-theme="studio"]` selector matched, not just that an
     attribute exists. (Confirm the exact token/values against the CSS during
     implementation; pick a token that genuinely differs between `:root` and
     `.j-root[data-theme="studio"]`.)
- **Backend-agnostic:** theme is client-side; the spec runs unchanged across the
  `{sqlite,postgres}×{chromium,firefox}` e2e matrix.

## Verification

- `cargo xtask e2e-local theme.spec.ts` passes (new test green with the fix).
- Sanity: reverting the one-line fix makes the new test fail (the test actually
  bites).
- Full local gate `cargo xtask validate` green before ship.

## Acceptance criteria

- [ ] Post-hydration `.j-root` has `data-theme="studio"` and no `attr:`-prefixed
      attribute.
- [ ] The studio theme's token overrides demonstrably apply after hydration.
- [ ] New e2e spec fails without the fix, passes with it.
- [ ] `cargo xtask validate` green.
