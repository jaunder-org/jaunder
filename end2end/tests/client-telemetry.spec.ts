import { existsSync, readFileSync } from "node:fs";
import { capturePathViaTool } from "./capture";
import { test, expect } from "./fixtures";
import { waitForSelector } from "./helpers";
import { pollUntil } from "./polling";

const LOCAL_WARNING = "jaunder swallowed browser error";
const TELEMETRY_PATH = "/api/client-telemetry";
const INTAKE_WARNING = "client error swallowed after reporting";

function captureLines(path: string): string[] {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((line) => line.length > 0);
}

test("audited browser failure warns before authenticated keepalive delivery", async ({
  page,
  registeredPage,
  browserTrace,
}) => {
  const diagnosticPath = capturePathViaTool("diag");
  const diagnosticBaseline = captureLines(diagnosticPath).length;

  // Fault only the cosmetic theme write. Seeded session-marker writes still work,
  // so the resulting telemetry request exercises cookie authentication rather than
  // an anonymous rejection. The exception text is intentionally arbitrary: the
  // closed event must not transport it.
  await page.context().addInitScript(() => {
    const nativeSetItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key: string, value: string): void {
      if (key === "jaunder_theme") {
        throw new DOMException(
          "injected text must remain browser-local",
          "QuotaExceededError",
        );
      }
      nativeSetItem.call(this, key, value);
    };
  });

  await registeredPage("/app");
  await waitForSelector(page, "a[href='/logout']");

  const observed = await pollUntil(
    "wait.client_telemetry_start",
    () => {
      const sink = browserTrace();
      const warning = sink?.consoleWarnings.find(
        (record) => record.text === LOCAL_WARNING,
      );
      const request = sink?.requestStarts.find(
        (record) =>
          record.method === "POST" &&
          new URL(record.url).pathname === TELEMETRY_PATH,
      );
      return warning && request ? { warning, request } : undefined;
    },
    {
      intervalMs: 100,
      timeoutMs: 5_000,
      describe: "the local warning and client-telemetry request start",
    },
  );

  expect(observed.warning.sequence).toBeLessThan(observed.request.sequence);

  // The failed persistence write is cosmetic: the mounted caller-visible theme
  // remains the same studio theme the app selected before persistence failed.
  const root = page.locator(".j-root");
  await expect(root).toBeVisible();
  await expect(root).toHaveAttribute("data-theme", "studio");

  const intakeWarning = await pollUntil(
    "wait.client_telemetry_diag",
    () => {
      const matching = captureLines(diagnosticPath)
        .slice(diagnosticBaseline)
        .filter(
          (line) =>
            line.includes(INTAKE_WARNING) &&
            line.includes('"error.context":"client.theme_storage.write"'),
        );
      return matching.length === 1 ? matching[0] : undefined;
    },
    {
      intervalMs: 100,
      timeoutMs: 5_000,
      describe: "one captured server warning for the failed theme write",
    },
  );

  for (const field of [
    '"error.kind":"storage"',
    '"error.class":"transient"',
    '"error.disposition":"swallowed"',
    '"telemetry.origin":"client"',
    '"error.source_kind":"storage_operation"',
  ]) {
    expect(intakeWarning).toContain(field);
  }
  expect(intakeWarning).not.toContain(
    "injected text must remain browser-local",
  );

  const matchingLocalWarnings =
    browserTrace()?.consoleWarnings.filter(
      (record) => record.text === LOCAL_WARNING,
    ) ?? [];
  expect(matchingLocalWarnings).toHaveLength(1);

  // The keepalive contract is request-start-before-teardown. Server acceptance was
  // observed above only to prove the real authenticated path; nothing below asserts
  // delivery after page termination.
  await page.close();
});
