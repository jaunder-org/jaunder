# Feed alternate-link translation implementation plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks through `jaunder-dispatch` when useful). Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove repeated alternate-link DOM translation while preserving the
feed-discovery tests’ observable assertions and live route-settle wait.

**Architecture:** `feeds.ts` owns one read-only Playwright DOM adapter returning
browser-resolved typed alternate-link records. `feeds.spec.ts` consumes it at
all materialized reads; callers continue to own filtering, counts, requests, and
polling.

**Tech Stack:** TypeScript, Playwright, existing Jaunder e2e fixture and helper
conventions.

## Global constraints

- Implement
  [issue #1037’s specification](../specs/2026-08-20-issue-1037-feed-alternate-links.md),
  especially AC1–AC6.
- Use the local `test` fixture and existing `goto`/`click` wrappers; do not add
  a direct navigation, fixed delay, or network-idle wait.
- `readAlternateLinks` is read-only: no polling, network request, assertion, or
  trace action.
- Keep crawler raw-HTML verification independent of browser-DOM discovery.

## Review header

**Scope:** Add the typed alternate-link reader and cut all four materialized
reads over to it. Do not alter discovery markup, feed behavior, crawler
coverage, or reactive polling discipline.

**Tasks:**

1. Add the narrow typed DOM reader and migrate discovery callers, including the
   live route-settle predicate.
2. Run focused feed coverage and the per-commit repository gate; review the
   complete diff.

**Key risks/decisions:** `HTMLLinkElement.href` provides the browser-resolved
absolute value; preserve `type` to retain MIME assertions. The tag route can
settle after the URL change, so only its existing `expect.poll` proves the
reactive head replacement.

## File structure

- Modify `end2end/tests/feeds.ts`: own `AlternateLink` and
  `readAlternateLinks(page)`.
- Modify `end2end/tests/feeds.spec.ts`: replace direct alternate-link `$$eval`
  calls while leaving assertions and raw crawler parsing local.

---

### Task 1: Centralize alternate-link DOM translation

**Files:**

- Modify: `end2end/tests/feeds.ts`
- Modify: `end2end/tests/feeds.spec.ts:45-146`

**Interfaces:**

- Produces:

  ```ts
  export type AlternateLink = { href: string; type: string };
  export async function readAlternateLinks(
    page: Page,
  ): Promise<AlternateLink[]>;
  ```

  The function evaluates only `head link[rel="alternate"]` and maps each node’s
  `HTMLLinkElement.href` and `HTMLLinkElement.type` in document order.

- Consumes: the existing `Page` type and current discovery assertions.

- [x] **Step 1: Write the focused helper contract test**

  Extend `feeds.spec.ts`’ existing home/user and client-side navigation tests so
  their unchanged MIME, absolute-URL fetch, three-link, and changed-href
  assertions consume `readAlternateLinks`. In the `expect.poll` predicate, await
  the helper and count records whose `href` contains `disco198`.

- [x] **Step 2: Run the focused test to establish the red contract**

  ```bash
  devtool run -- tsc --noEmit -p end2end/tsconfig.json
  ```

  Expected before implementation: TypeScript fails because `readAlternateLinks`
  is not exported.

- [x] **Step 3: Implement the typed reader**

  In `feeds.ts`, export `AlternateLink` and implement:

  ```ts
  export async function readAlternateLinks(
    page: Page,
  ): Promise<AlternateLink[]> {
    return page.$$eval('head link[rel="alternate"]', (elements) =>
      elements.map((element) => {
        const link = element as HTMLLinkElement;
        return { href: link.href, type: link.type };
      }),
    );
  }
  ```

  Replace all four direct alternate-link materializations in `feeds.spec.ts`.
  Map returned records to `href` only at the two callers that compare href
  arrays. Leave stylesheet/EditURI DOM checks and raw crawler HTML parsing
  unchanged.

- [x] **Step 4: Run focused behavioral proof**

  ```bash
  devtool run -- cargo xtask e2e-local feeds.spec.ts
  ```

  Expected: pass; the live client-side tag navigation still converges through
  `expect.poll`, and home/user MIME fetch assertions still validate all three
  Syndication Feed formats.

- [x] **Step 5: Commit the deliverable**

  Tick this task, run `devtool run -- cargo xtask check`, inspect and stage all
  mechanical fixes, then commit a focused message without a `Co-Authored-By`
  trailer.

### Task 2: Review the completed deliverable

**Files:**

- Review: `end2end/tests/feeds.ts`
- Review: `end2end/tests/feeds.spec.ts`

**Interfaces:**

- Consumes: Task 1’s `AlternateLink` and `readAlternateLinks` contract.
- Produces: a review packet suitable for the issue-cycle validation/ship stage.

- [ ] **Step 1: Inspect the branch-side diff**

  ```bash
  devtool run -- git diff origin/main...HEAD -- end2end/tests/feeds.ts end2end/tests/feeds.spec.ts
  ```

  Verify no direct `head link[rel="alternate"]` extraction remains in the
  materialized callers, except the selected helper; crawler raw-HTML checking
  remains separate.

- [ ] **Step 2: Run the implementation review**

  Use `jaunder-review` against `origin/main`, separating standards and
  specification conformance. Resolve every finding before the final ship gate.
