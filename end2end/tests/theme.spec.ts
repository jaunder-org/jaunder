import { test, expect } from "./fixtures";
import { goto, signInAsNewUser } from "./helpers";
import { allowSecondBoot } from "./bootBudget";
import { seedPostsViaTool, seedUserViaTool } from "./seed";
import { expectVisual } from "./visual";
import { expectAccessible } from "./accessibility";

// Regression for #22: the reactive data-theme binding on the plain `.j-root`
// element must survive the CSR mount. A leaked Leptos `attr:` directive prefix
// produced a literal `attr:data-theme` attribute, so `.j-root[data-theme=...]`
// stopped matching and no theme token overrides applied after the client booted.
test(
  "issue #22: .j-root keeps a real data-theme after CSR mount",
  { tag: ["@visual", "@accessibility"] },
  async ({ page }) => {
    await seedUserViaTool("visualauthor", "visualpassword123");
    await seedPostsViaTool("visualauthor", 1, "Visual Timeline Post");
    await goto(page, "/"); // public projector home; goto() waits for the CSR mount

    const probe = await page.evaluate(() => {
      const root = document.querySelector(".j-root");
      if (!root) return { found: false } as const;
      return {
        found: true as const,
        dataTheme: root.getAttribute("data-theme"),
        attrNames: Array.from(root.attributes).map((a) => a.name),
        accentInk: getComputedStyle(root)
          .getPropertyValue("--accent-ink")
          .trim(),
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

    const post = page
      .locator("article.j-post")
      .filter({ hasText: "Visual Timeline Post 0" });
    await expect(post).toBeVisible();
    await expect(post).toContainText("Body for Visual Timeline Post 0");
    await expect(post).toContainText("visualauthor");
    await expectVisual(page, "public-timeline.png", {
      mask: [page.locator(".j-post-time")],
    });
    await expectAccessible(page);
  },
);

const ROOT = ".j-root";

test("theme selector applies built-ins immediately and persists the selection", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");
  const theme = page.getByRole("group", { name: "Theme" });

  await expect(theme).toBeVisible();
  await expect(theme.getByRole("button", { name: "Terminal" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await expect(theme.getByRole("button", { name: "Studio" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(theme.getByRole("button", { name: "Reader" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );

  for (const [themeId, label] of [
    ["terminal", "Terminal"],
    ["studio", "Studio"],
    ["reader", "Reader"],
  ]) {
    const button = theme.getByRole("button", { name: label });

    await expect(button).toHaveAttribute("aria-pressed", "false");
    await button.click();
    await expect(page.locator(ROOT)).toHaveAttribute("data-theme", themeId);
    await expect(button).toHaveAttribute("aria-pressed", "true");
  }

  allowSecondBoot(
    page,
    "reloading proves the browser-local theme selection survives a fresh CSR mount",
  );
  await goto(page, "/profile");
  await expect(page.locator(ROOT)).toHaveAttribute("data-theme", "reader");
  await expect(theme.getByRole("button", { name: "Reader" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("theme selector preserves an unknown stored identifier until selection", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await page.addInitScript(() => {
    if (sessionStorage.getItem("theme-selector-seeded") !== "true") {
      localStorage.setItem("jaunder_theme", "custom-dark");
      sessionStorage.setItem("theme-selector-seeded", "true");
    }
  });
  await goto(page, "/profile");

  const theme = page.getByRole("group", { name: "Theme" });
  await expect(page.locator(ROOT)).toHaveAttribute("data-theme", "custom-dark");
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("jaunder_theme")))
    .toBe("custom-dark");

  for (const label of ["Terminal", "Studio", "Reader"]) {
    await expect(theme.getByRole("button", { name: label })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  }

  await theme.getByRole("button", { name: "Terminal" }).click();
  await expect(page.locator(ROOT)).toHaveAttribute("data-theme", "terminal");
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("jaunder_theme")))
    .toBe("terminal");
  await expect(theme.getByRole("button", { name: "Terminal" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
