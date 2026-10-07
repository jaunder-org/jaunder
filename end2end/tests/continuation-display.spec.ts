import { test, expect } from "./fixtures";
import { goto } from "./helpers";
import { applySeededSession, seedPostsViaTool } from "./seed";

for (const route of ["/", "/app"]) {
  for (const viewport of [
    { width: 390, height: 844 },
    { width: 1280, height: 900 },
  ]) {
    test(`timeline continuation stays on one line on ${route} at ${viewport.width}px`, async ({
      page,
      user,
      firstNav,
    }) => {
      await seedPostsViaTool(user.username, 51, "Continuation display Post");
      if (route === "/app") await applySeededSession(page.context(), user);
      await page.setViewportSize(viewport);
      await goto(page, route, { timeout: firstNav });

      const continuation = page.getByRole("button", {
        name: "Load more",
        exact: true,
      });
      await expect(continuation).toBeVisible();
      await continuation.scrollIntoViewIfNeeded();
      // A text-spacing preference must not force a short action label to wrap.
      for (const spacing of ["normal", "0.12em"]) {
        await page.addStyleTag({
          content: `* { letter-spacing: ${spacing} !important; }`,
        });
        const geometry = await continuation.evaluate((button) => {
          const label = document.createRange();
          label.selectNodeContents(button);
          const lines = Array.from(label.getClientRects());
          const control = button.getBoundingClientRect();
          return {
            lineCount: lines.length,
            fitsControl: lines.every(
              (line) =>
                line.left >= control.left - 1 &&
                line.right <= control.right + 1,
            ),
            fitsViewport:
              control.left >= 0 && control.right <= window.innerWidth,
          };
        });
        expect(geometry.lineCount).toBe(1);
        expect(geometry.fitsControl).toBe(true);
        expect(geometry.fitsViewport).toBe(true);
      }
    });
  }
}
