import { expect, test } from "./fixtures";
import { allowSecondBoot, bootCount } from "./bootBudget";
import { BASE_URL, goto } from "./helpers";
import { navigateInApp } from "./navigate";
import { composePost } from "./posts";
import { SEL } from "./selectors";

test("WordPress-compatible permalink alias redirects only on cold entry", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/posts/new");
  const title = "Cold alias entry";
  // The E2E site's configurable default may be non-public; the alias is
  // intentionally resolved as an anonymous reader.
  await page.selectOption("#audience-base", "public");
  const summary = await composePost(page, {
    body: `# ${title}\n\nThe canonical post remains readable after its HTTP alias redirects.`,
    publish: true,
  });

  const permalinkLink = summary.locator(SEL.permalinkLink);
  const canonicalHref = await permalinkLink.getAttribute("href");
  expect(
    canonicalHref,
    "published Post emitted no canonical permalink",
  ).toBeTruthy();

  const canonicalUrl = new URL(canonicalHref!, BASE_URL);
  expect(canonicalUrl.pathname).toMatch(/^\/~[^/]+\/\d{4}\/\d{2}\/\d{2}\/.+$/);
  const aliasPath = `/${canonicalUrl.pathname.split("/").slice(2).join("/")}`;
  const query = "?empty=&repeat=first&repeat=second&encoded=%2Fkept";

  // #1429: this is an inbound HTTP compatibility entry, so its cold render is
  // deliberate; all later movement stays in the already-mounted CSR document.
  allowSecondBoot(page, "the HTTP alias cold entry is the behavior under test");
  await goto(page, `${aliasPath}${query}`);

  const redirectedUrl = new URL(page.url());
  expect(redirectedUrl.pathname).toBe(canonicalUrl.pathname);
  expect(redirectedUrl.search).toBe(query);
  await expect(page.locator("article.j-post")).toContainText(title);

  const bootsBeforeCsrAlias = bootCount(page);
  await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
  });
  // Seed a same-document history entry, return to the canonical route, then
  // traverse forward so the live router receives a real popstate for the alias.
  await page.evaluate((path) => history.pushState({}, "", path), aliasPath);
  await page.evaluate(() => history.back());
  await page.waitForURL(`${BASE_URL}${canonicalUrl.pathname}${query}`);
  await navigateInApp(page, () => page.evaluate(() => history.forward()), {
    url: aliasPath,
    ready: "text=Page not found.",
  });

  expect(bootCount(page)).toBe(bootsBeforeCsrAlias);
  expect(
    await page.evaluate(
      () =>
        (window as Window & { __jaunderNoReload?: boolean })
          .__jaunderNoReload === true,
    ),
  ).toBe(true);
});
