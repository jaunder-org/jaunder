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
