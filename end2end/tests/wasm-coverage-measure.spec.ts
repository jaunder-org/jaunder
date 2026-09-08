import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "./fixtures";
import { allowSecondBoot } from "./bootBudget";
import { goto, BASE_URL } from "./helpers";

// This is deliberately separate from the permanent coverage capture: it measures
// the same mounted CSR flow without making a profiler export part of the baseline.
test("diagnostic wasm measurement records the mounted CSR flow", async ({
  page,
  firstNav,
}, testInfo) => {
  const root = process.env.JAUNDER_WASM_COVERAGE_OUT;
  const csr = process.env.JAUNDER_WASM_COVERAGE_CSR;
  const cacheBuster = process.env.JAUNDER_WASM_COVERAGE_CACHE_BUSTER;
  const mode = process.env.JAUNDER_WASM_COVERAGE_MODE;
  if (!root || !csr || !cacheBuster || !mode)
    throw new Error("measurement producer environment is incomplete");
  // Each retained Nix realization owns this unmeasured boot, so timing never
  // inherits warm state from a discarded realization.
  await goto(page, "/", { timeout: firstNav });
  await expect(page.locator("body[data-mounted]")).toBeVisible();
  allowSecondBoot(
    page,
    "measurement timing follows an unmeasured warm-up in this realization",
  );
  const started = performance.now();
  await goto(page, "/", { timeout: firstNav });
  const focusedFlowMilliseconds = Math.max(
    1,
    Math.round(performance.now() - started),
  );
  const manifest = JSON.parse(
    await readFile(join(csr, "pkg", "manifest.json"), "utf8"),
  ) as { assets?: Array<{ role?: string; path?: string }> };
  const wasmPath = manifest.assets?.find(
    (asset) => asset.role === "wasm",
  )?.path;
  if (!wasmPath) throw new Error("CSR manifest does not select a wasm module");
  const wasm = await (await page.request.get(`${BASE_URL}/${wasmPath}`)).body();
  await writeFile(
    join(root, "measurement.json"),
    `${JSON.stringify(
      {
        version: "wasm-coverage-measurement-v1",
        browser: testInfo.project.name,
        mode,
        cache_buster: cacheBuster,
        focused_flow_milliseconds: focusedFlowMilliseconds,
        served_wasm_path: wasmPath,
        served_wasm_bytes: wasm.byteLength,
      },
      null,
      2,
    )}\n`,
  );
});
