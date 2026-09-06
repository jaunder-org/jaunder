import { expect, test } from "./fixtures";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { goto, BASE_URL } from "./helpers";

// This is deliberately separate from the permanent coverage capture: it measures
// the same mounted CSR flow without making a profiler export part of the baseline.
test("diagnostic wasm measurement records the mounted CSR flow", async ({
  page,
  firstNav,
}, testInfo) => {
  const root = process.env.JAUNDER_WASM_COVERAGE_OUT;
  const cacheBuster = process.env.JAUNDER_WASM_COVERAGE_CACHE_BUSTER;
  const mode = process.env.JAUNDER_WASM_COVERAGE_MODE;
  if (!root || !cacheBuster || !mode)
    throw new Error("measurement producer environment is incomplete");
  const started = performance.now();
  await goto(page, "/", { timeout: firstNav });
  await expect(page.locator("body[data-mounted]")).toBeVisible();
  const focusedFlowMilliseconds = Math.max(
    1,
    Math.round(performance.now() - started),
  );
  const wasm = await (
    await page.request.get(`${BASE_URL}/pkg/jaunder.wasm`)
  ).body();
  await writeFile(
    join(root, "measurement.json"),
    `${JSON.stringify(
      {
        version: "wasm-coverage-measurement-v1",
        browser: testInfo.project.name,
        mode,
        cache_buster: cacheBuster,
        focused_flow_milliseconds: focusedFlowMilliseconds,
        served_wasm_bytes: wasm.byteLength,
      },
      null,
      2,
    )}\n`,
  );
});
