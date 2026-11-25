import { test, expect } from "./fixtures";
import { click, goto, signInAs } from "./helpers";
import { navigateInApp } from "./navigate";

type ShellProbe = Window & { __insertedText?: string[] };
type ProbedElement = Element & { __shellProbe?: boolean };

// The app shell keeps one Sidebar and one route outlet for the whole session
// (#2013): moving between public and private layouts must neither rebuild the
// chrome nor mount the page being left a second time inside the new layout.
test("the app shell survives navigation and never re-mounts the page being left", async ({
  page,
  user,
}) => {
  await signInAs(page, user.username);
  await goto(page, "/feeds");
  await expect(
    page.locator("main h1", { hasText: "Syndication feeds for Local" }),
  ).toBeVisible();

  // Tag the live Sidebar node and record the text of every element inserted
  // under the shell from here on.
  await page.evaluate(() => {
    (document.querySelector(".j-sidebar") as ProbedElement).__shellProbe = true;
    const inserted: string[] = [];
    (window as ShellProbe).__insertedText = inserted;
    new MutationObserver((records) => {
      for (const record of records) {
        for (const node of record.addedNodes) {
          if (node instanceof Element) inserted.push(node.textContent ?? "");
        }
      }
    }).observe(document.querySelector(".j-root")!, {
      childList: true,
      subtree: true,
    });
  });
  const sidebarSurvived = () =>
    page.evaluate(
      () =>
        (document.querySelector(".j-sidebar") as ProbedElement | null)
          ?.__shellProbe === true,
    );

  // Public → private.
  await navigateInApp(page, () => click(page, '.j-nav a[href="/app"]'), {
    url: "/app",
    ready: '.j-topbar h1:has-text("Home")',
  });
  const inserted = await page.evaluate(
    () => (window as ShellProbe).__insertedText ?? [],
  );
  expect(inserted.filter((text) => /syndication feed/i.test(text))).toEqual([]);
  expect(await sidebarSurvived()).toBe(true);

  // Private → private.
  await navigateInApp(page, () => click(page, '.j-nav a[href="/drafts"]'), {
    url: "/drafts",
    ready: '.j-topbar h1:has-text("Drafts")',
  });
  expect(await sidebarSurvived()).toBe(true);
});
