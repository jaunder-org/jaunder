import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { BASE_URL } from "./helpers";

type Stage = {
  outcome: "passed" | "failed" | "not-run";
  blocker: string | null;
};

type BrowserCoverageStatus = {
  version: "v1";
  requested_browser: string;
  actual_browser: string;
  csr_structural: Stage;
  diagnostic_export: Stage;
  source_mapping: Stage;
  module_signature: string | null;
  toolchain_identity: unknown | null;
  served_module: { path: string; sha256: string } | null;
  artifacts: Record<string, { path: string; sha256: string }>;
};

const digest = (bytes: Uint8Array) =>
  createHash("sha256").update(bytes).digest("hex");

async function retain(
  root: string,
  relative: string,
  bytes: Uint8Array | string,
) {
  const path = join(root, relative);
  await mkdir(join(path, ".."), { recursive: true });
  await writeFile(path, bytes);
  const sha256 =
    typeof bytes === "string"
      ? createHash("sha256").update(bytes).digest("hex")
      : digest(bytes);
  return { path: relative, sha256 };
}

async function diagnosticBundleStatus(): Promise<{
  structural: Stage;
  identity: unknown | null;
  servedModule: { path: string; sha256: string } | null;
  glueModule: string | null;
}> {
  const root = process.env.JAUNDER_WASM_COVERAGE_CSR;
  if (!root) {
    return {
      structural: {
        outcome: "failed",
        blocker: "JAUNDER_WASM_COVERAGE_CSR is not set",
      },
      identity: null,
      servedModule: null,
      glueModule: null,
    };
  }
  try {
    const status = JSON.parse(
      await readFile(join(root, "status.json"), "utf8"),
    ) as { outcome?: string };
    const manifest = JSON.parse(
      await readFile(join(root, "pkg", "manifest.json"), "utf8"),
    ) as {
      assets?: Array<{ role?: string; path?: string; sha256?: string }>;
    };
    const wasm = manifest.assets?.find((asset) => asset.role === "wasm");
    const glue = manifest.assets?.find((asset) => asset.role === "glue");
    const identity = JSON.parse(
      await readFile(join(root, "toolchain-identity.json"), "utf8"),
    );
    return status.outcome === "succeeded" &&
      wasm?.path &&
      wasm.sha256 &&
      glue?.path
      ? {
          structural: { outcome: "passed", blocker: null },
          identity,
          servedModule: { path: wasm.path, sha256: wasm.sha256 },
          glueModule: `/${glue.path}`,
        }
      : {
          structural: {
            outcome: "failed",
            blocker: `diagnostic CSR status or manifest is invalid: ${status.outcome}`,
          },
          identity,
          servedModule: null,
          glueModule: null,
        };
  } catch (error) {
    return {
      structural: { outcome: "failed", blocker: String(error) },
      identity: null,
      servedModule: null,
      glueModule: null,
    };
  }
}

/** Capture the diagnostic wasm exports after a real CSR flow has completed. */
export async function captureWasmCoverage(
  page: Page,
  requestedBrowser: string,
): Promise<void> {
  const root = process.env.JAUNDER_WASM_COVERAGE_OUT;
  if (!root) throw new Error("JAUNDER_WASM_COVERAGE_OUT is not set");

  const diagnostics = join(root, "diagnostics");
  await mkdir(diagnostics, { recursive: true });
  const { structural, identity, servedModule, glueModule } =
    await diagnosticBundleStatus();
  const actualBrowser =
    page.context().browser()?.browserType().name() ?? "unknown";
  const artifacts: BrowserCoverageStatus["artifacts"] = {};
  const module = servedModule
    ? new Uint8Array(
        await (
          await page.request.get(`${BASE_URL}/${servedModule.path}`)
        ).body(),
      )
    : new Uint8Array();
  artifacts.module = await retain(
    root,
    servedModule ? `module/${servedModule.path}` : "module/unavailable.wasm",
    module,
  );
  artifacts.diagnostics = await retain(
    root,
    "diagnostics/capture.log",
    "capture started\n",
  );
  const servedSha256 = servedModule?.sha256;

  const status: BrowserCoverageStatus = {
    version: "v1",
    requested_browser: requestedBrowser,
    actual_browser: actualBrowser,
    csr_structural:
      structural.outcome === "passed" &&
      servedSha256 &&
      digest(module) !== servedSha256
        ? {
            outcome: "failed",
            blocker: `served module SHA-256 mismatch: expected ${servedSha256}, got ${digest(module)}`,
          }
        : structural,
    diagnostic_export: { outcome: "not-run", blocker: null },
    source_mapping: { outcome: "not-run", blocker: null },
    module_signature: null,
    toolchain_identity: null,
    served_module: servedModule,
    artifacts,
  };
  try {
    if (structural.outcome !== "passed")
      throw new Error(structural.blocker ?? "diagnostic CSR unavailable");
    if (status.csr_structural.outcome !== "passed") {
      throw new Error(
        status.csr_structural.blocker ?? "diagnostic CSR unavailable",
      );
    }
    status.toolchain_identity = identity;
    if (process.env.JAUNDER_WASM_COVERAGE_INJECT_FAILURE === "export") {
      throw new Error("injected export failure");
    }
    const capture = await page.evaluate(async (diagnosticModule) => {
      // The manifest selects this diagnostic-only runtime URL, so a static import
      // would incorrectly make host Playwright resolve an unavailable artifact.
      if (!diagnosticModule)
        throw new Error("diagnostic glue module is unavailable");
      const wasm = await import(diagnosticModule);
      const signature: unknown = wasm.jaunderCoverageModuleSignature();
      const profile: unknown = wasm.jaunderCoverageProfile();
      if (typeof signature !== "string") {
        throw new Error(
          "diagnostic export returned a non-string module signature",
        );
      }
      if (!(profile instanceof Uint8Array)) {
        throw new Error("diagnostic export returned a non-byte profile");
      }
      return { signature, profile: [...profile] };
    }, glueModule);
    if (capture.profile.length === 0) {
      throw new Error("diagnostic export returned an empty profile");
    }
    status.module_signature = capture.signature;
    status.diagnostic_export = { outcome: "passed", blocker: null };
    artifacts.profile = await retain(
      root,
      "profiles/browser.profraw",
      Uint8Array.from(capture.profile),
    );
  } catch (error) {
    const blocker = error instanceof Error ? error.message : String(error);
    if (status.diagnostic_export.outcome === "not-run") {
      status.diagnostic_export = { outcome: "failed", blocker };
    } else {
      status.source_mapping = { outcome: "failed", blocker };
    }
    await writeFile(
      join(root, "status.json"),
      `${JSON.stringify(status, null, 2)}\n`,
    );
    throw error;
  }
  await writeFile(
    join(root, "status.json"),
    `${JSON.stringify(status, null, 2)}\n`,
  );
}
