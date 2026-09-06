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
  servedModuleSha256: string | null;
}> {
  const root = process.env.JAUNDER_WASM_COVERAGE_CSR;
  if (!root) {
    return {
      structural: {
        outcome: "failed",
        blocker: "JAUNDER_WASM_COVERAGE_CSR is not set",
      },
      identity: null,
      servedModuleSha256: null,
    };
  }
  try {
    const status = JSON.parse(
      await readFile(join(root, "status.json"), "utf8"),
    ) as {
      outcome?: string;
      served_module?: { sha256?: string };
    };
    const identity = JSON.parse(
      await readFile(join(root, "toolchain-identity.json"), "utf8"),
    );
    return status.outcome === "succeeded" && status.served_module?.sha256
      ? {
          structural: { outcome: "passed", blocker: null },
          identity,
          servedModuleSha256: status.served_module.sha256,
        }
      : {
          structural: {
            outcome: "failed",
            blocker: `diagnostic CSR status: ${status.outcome}`,
          },
          identity,
          servedModuleSha256: null,
        };
  } catch (error) {
    return {
      structural: { outcome: "failed", blocker: String(error) },
      identity: null,
      servedModuleSha256: null,
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
  const { structural, identity, servedModuleSha256 } =
    await diagnosticBundleStatus();
  const actualBrowser =
    page.context().browser()?.browserType().name() ?? "unknown";
  const artifacts: BrowserCoverageStatus["artifacts"] = {};
  const module = new Uint8Array(
    await (await page.request.get(`${BASE_URL}/pkg/jaunder.wasm`)).body(),
  );
  artifacts.module = await retain(root, "module/jaunder.wasm", module);
  artifacts.diagnostics = await retain(
    root,
    "diagnostics/capture.log",
    "capture started\n",
  );

  const status: BrowserCoverageStatus = {
    version: "v1",
    requested_browser: requestedBrowser,
    actual_browser: actualBrowser,
    csr_structural:
      structural.outcome === "passed" && digest(module) !== servedModuleSha256
        ? {
            outcome: "failed",
            blocker: `served module SHA-256 mismatch: expected ${servedModuleSha256}, got ${digest(module)}`,
          }
        : structural,
    diagnostic_export: { outcome: "not-run", blocker: null },
    source_mapping: { outcome: "not-run", blocker: null },
    module_signature: null,
    toolchain_identity: null,
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
    const capture = await page.evaluate(async () => {
      // This URL exists only in the diagnostic Nix VM; importing it statically
      // would make the host Playwright typecheck resolve a generated artifact.
      const diagnosticModule = "/pkg/jaunder.js";
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
    });
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
