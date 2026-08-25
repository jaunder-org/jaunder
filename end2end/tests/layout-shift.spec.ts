import { test, expect } from "./fixtures";
import { waitForStableTargetGeometry } from "./layout-shift";

const TARGET = "#target";

test("target geometry waits for consecutive stable animation frames", async ({
  page,
}) => {
  await page.setContent(
    `<div id="target" style="position: absolute; top: 0; left: 0">target</div>`,
  );
  await page.evaluate((selector) => {
    const target = document.querySelector<HTMLElement>(selector)!;
    let frame = 0;
    const moveUntilSettled = () => {
      frame += 1;
      target.style.top = `${Math.min(frame, 2) * 10}px`;
      if (frame < 3) requestAnimationFrame(moveUntilSettled);
    };
    requestAnimationFrame(moveUntilSettled);
  }, TARGET);

  const [settled] = await waitForStableTargetGeometry(
    page,
    [{ name: "target", locator: page.locator(TARGET) }],
    1_000,
  );

  expect(settled?.y).toBe(20);
});

test("target geometry reports a named failure when it never stabilizes", async ({
  page,
}) => {
  await page.setContent(
    `<div id="target" style="position: absolute; top: 0; left: 0">target</div>`,
  );
  await page.evaluate((selector) => {
    const target = document.querySelector<HTMLElement>(selector)!;
    let shifted = false;
    const keepMoving = () => {
      shifted = !shifted;
      target.style.top = shifted ? "10px" : "20px";
      requestAnimationFrame(keepMoving);
    };
    requestAnimationFrame(keepMoving);
  }, TARGET);

  await expect(
    waitForStableTargetGeometry(
      page,
      [{ name: "target", locator: page.locator(TARGET) }],
      100,
    ),
  ).rejects.toThrow("target geometry did not stabilize");
});
