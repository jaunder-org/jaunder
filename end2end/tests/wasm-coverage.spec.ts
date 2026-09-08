import { expect, test } from "./fixtures";
import { goto } from "./helpers";
import { captureWasmCoverage } from "./wasm-coverage";

// This file is selected explicitly by the diagnostic Nix producers. It remains in
// the shared configuration so it exercises the normal browser and fixture setup.
test("diagnostic coverage captures a real mounted CSR flow", async ({
  page,
  firstNav,
}, testInfo) => {
  await goto(page, "/", { timeout: firstNav });
  await expect(page.locator("body[data-mounted]")).toBeVisible();
  await captureWasmCoverage(page, testInfo.project.name);
});
