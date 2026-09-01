import { existsSync, readFileSync } from "node:fs";
import { capturePathViaTool } from "./capture";
import { test, expect } from "./fixtures";
import { goto, waitForSelector } from "./helpers";
import { pollUntil } from "./polling";
import { applySeededSession } from "./seed";

const LOCAL_WARNING = "jaunder swallowed browser error";
const TELEMETRY_PATH = "/api/client-telemetry";
const INTAKE_WARNING = "client error swallowed after reporting";

function captureLines(path: string): string[] {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((line) => line.length > 0);
}

test("capture records normalized browser diagnostics in phase and delivery order", async ({
  page,
  browserTrace,
}) => {
  const warning = page.waitForEvent(
    "console",
    (message) =>
      message.type() === "warning" && message.text() === "captured warning",
  );
  const error = page.waitForEvent(
    "console",
    (message) =>
      message.type() === "error" && message.text() === "captured error",
  );
  const excludedLog = page.waitForEvent(
    "console",
    (message) => message.type() === "log" && message.text() === "excluded log",
  );
  const pageError = page.waitForEvent("pageerror");
  await page.evaluate(() => {
    console.warn("captured warning");
    console.error("captured error");
    console.log("excluded log");
    setTimeout(() => {
      throw new TypeError("captured page error");
    }, 0);
  });
  const [warningMessage, errorMessage, logMessage, browserError] =
    await Promise.all([warning, error, excludedLog, pageError]);
  expect(logMessage.type()).toBe("log");

  const diagnostics = await pollUntil(
    "wait.browser_diagnostics",
    () => {
      const records = browserTrace()?.browserDiagnostics ?? [];
      const captured = records.filter(
        (record) =>
          (record.kind === "console" &&
            (record.text === "captured warning" ||
              record.text === "captured error")) ||
          (record.kind === "pageerror" &&
            record.message === "captured page error"),
      );
      return captured.length === 3 ? captured : undefined;
    },
    {
      intervalMs: 10,
      timeoutMs: 5_000,
      describe: "three normalized browser diagnostics",
    },
  );

  expect(diagnostics).toEqual([
    {
      kind: "console",
      type: "warning",
      text: "captured warning",
      location: {
        url: warningMessage.location().url,
        line: warningMessage.location().lineNumber,
        column: warningMessage.location().columnNumber,
      },
      sequence: expect.any(Number),
      emittedMs: expect.any(Number),
    },
    {
      kind: "console",
      type: "error",
      text: "captured error",
      location: {
        url: errorMessage.location().url,
        line: errorMessage.location().lineNumber,
        column: errorMessage.location().columnNumber,
      },
      sequence: expect.any(Number),
      emittedMs: expect.any(Number),
    },
    {
      kind: "pageerror",
      name: browserError.name,
      message: browserError.message,
      ...(browserError.stack === undefined
        ? {}
        : { stack: browserError.stack }),
      sequence: expect.any(Number),
      emittedMs: expect.any(Number),
    },
  ]);
  expect(diagnostics[0]?.sequence).toBeLessThan(diagnostics[1]?.sequence ?? 0);
  expect(diagnostics[1]?.sequence).toBeLessThan(diagnostics[2]?.sequence ?? 0);
  expect(
    browserTrace()?.browserDiagnostics.some(
      (record) => record.kind === "console" && record.text === "excluded log",
    ),
  ).toBe(false);
});

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
      const warning = sink?.browserDiagnostics.find(
        (record) =>
          record.kind === "console" &&
          record.type === "warning" &&
          record.text === LOCAL_WARNING,
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
    browserTrace()?.browserDiagnostics.filter(
      (record) =>
        record.kind === "console" &&
        record.type === "warning" &&
        record.text === LOCAL_WARNING,
    ) ?? [];
  expect(matchingLocalWarnings).toHaveLength(1);

  // The keepalive contract is request-start-before-teardown. Server acceptance was
  // observed above only to prove the real authenticated path; nothing below asserts
  // delivery after page termination.
  await page.close();
});

test("session marker read failure reports before authenticated recovery", async ({
  page,
  user,
  firstNav,
}) => {
  await page.context().addInitScript(
    ({ markerKey }) => {
      const nativeGetItem = Storage.prototype.getItem;
      Storage.prototype.getItem = function (key: string): string | null {
        if (key === markerKey) {
          throw new DOMException(
            "injected marker text must remain browser-local",
            "SecurityError",
          );
        }
        return nativeGetItem.call(this, key);
      };
    },
    { markerKey: user.markerKey },
  );

  const warningPromise = page.waitForEvent("console", {
    predicate: (message) =>
      message.type() === "warning" && message.text() === LOCAL_WARNING,
  });
  await applySeededSession(page.context(), user);
  await goto(page, "/app", { timeout: firstNav });
  await waitForSelector(page, "a[href='/logout']");
  const warning = await warningPromise;

  expect(warning.text()).toBe(LOCAL_WARNING);
  expect(warning.text()).not.toContain("injected marker text");
  await expect(page.locator("a[href='/logout']")).toBeVisible();
});
