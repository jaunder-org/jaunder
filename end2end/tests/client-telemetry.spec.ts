import { existsSync, readFileSync } from "node:fs";
import { capturePathViaTool } from "./capture";
import {
  browserDiagnosticSpanProjectionFor,
  expect,
  test,
  tracedContextCapture,
} from "./fixtures";
import { goto, waitForSelector } from "./helpers";
import { navigateInApp } from "./navigate";
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

type OtlpAttribute = {
  key: string;
  value: { stringValue?: string; intValue?: string };
};

type OtlpSpan = {
  traceId: string;
  spanId: string;
  name: string;
  startTimeUnixNano: string;
  endTimeUnixNano: string;
  attributes?: OtlpAttribute[];
};

type OtlpExport = {
  resourceSpans: Array<{ scopeSpans: Array<{ spans: OtlpSpan[] }> }>;
};

function completeOtlpSpans(path: string): OtlpSpan[] {
  return captureLines(path).flatMap((line) => {
    try {
      const exported = JSON.parse(line) as OtlpExport;
      return exported.resourceSpans.flatMap((resource) =>
        resource.scopeSpans.flatMap((scope) => scope.spans),
      );
    } catch {
      // The collector may be appending its final JSON line while we poll.
      return [];
    }
  });
}

function attributeFor(span: OtlpSpan, key: string): OtlpAttribute {
  const attribute = span.attributes?.find((candidate) => candidate.key === key);
  if (attribute === undefined) {
    throw new Error(`${span.name} omitted ${key}`);
  }
  return attribute;
}

function consoleDiagnostics(span: OtlpSpan): unknown[] {
  const value = attributeFor(span, "e2e.console_json").value.stringValue;
  if (value === undefined) {
    throw new Error(`${span.name} did not encode e2e.console_json as a string`);
  }
  return JSON.parse(value) as unknown[];
}

function diagnosticsFrom(span: OtlpSpan): unknown[] | undefined {
  const value = span.attributes?.find(
    (attribute) => attribute.key === "e2e.console_json",
  )?.value.stringValue;
  if (value === undefined) return undefined;
  try {
    const diagnostics = JSON.parse(value) as unknown;
    return Array.isArray(diagnostics) ? diagnostics : undefined;
  } catch {
    return undefined;
  }
}

function expectSpanRange(span: OtlpSpan): void {
  expect(span.traceId).toMatch(/^[0-9a-f]{32}$/);
  expect(span.spanId).toMatch(/^[0-9a-f]{16}$/);
  expect(BigInt(span.endTimeUnixNano)).toBeGreaterThanOrEqual(
    BigInt(span.startTimeUnixNano),
  );
}

test.describe.serial("browser diagnostic OTLP export", () => {
  test("traced contexts separate pretest, test, and teardown diagnostics", async ({
    tracedContext,
  }) => {
    const context = await tracedContext();
    const capture = tracedContextCapture(context);
    if (capture === undefined) {
      throw new Error("tracedContext did not expose its attached capture");
    }
    try {
      const page = await context.newPage();

      capture.setPhase("pretest");
      const pretestConsole = page.waitForEvent(
        "console",
        (message) => message.text() === "traced pretest diagnostic",
      );
      await page.evaluate(() => console.warn("traced pretest diagnostic"));
      await pretestConsole;

      capture.setPhase("test");
      const testConsole = page.waitForEvent(
        "console",
        (message) => message.text() === "traced test diagnostic",
      );
      await page.evaluate(() => console.warn("traced test diagnostic"));
      await testConsole;

      capture.beginTeardown();
      const teardownConsole = page.waitForEvent(
        "console",
        (message) => message.text() === "traced teardown diagnostic",
      );
      await page.evaluate(() => console.warn("traced teardown diagnostic"));
      await teardownConsole;

      const pretest = capture.sinkFor("pretest").browserDiagnostics;
      const testDiagnostics = capture.sinkFor("test").browserDiagnostics;
      expect(pretest).toHaveLength(1);
      expect(pretest[0]).toMatchObject({ text: "traced pretest diagnostic" });
      expect(testDiagnostics).toHaveLength(1);
      expect(testDiagnostics[0]).toMatchObject({
        text: "traced test diagnostic",
      });
      const testProjection = browserDiagnosticSpanProjectionFor(
        "e2e.test",
        capture,
      );
      const pageProjection = browserDiagnosticSpanProjectionFor(
        "e2e.page",
        capture,
      );
      expect(testProjection.spanName).toBe("e2e.test");
      expect(pageProjection.spanName).toBe("e2e.page");
      expect(JSON.stringify(testProjection.attributes)).toContain(
        "traced test diagnostic",
      );
      expect(JSON.stringify(pageProjection.attributes)).toContain(
        "traced test diagnostic",
      );
      expect(JSON.stringify(testProjection.attributes)).not.toContain(
        "traced pretest diagnostic",
      );
      expect(JSON.stringify(pageProjection.attributes)).not.toContain(
        "traced pretest diagnostic",
      );
    } finally {
      await context.close();
    }
  });

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
      (message) =>
        message.type() === "log" && message.text() === "excluded log",
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
    expect(diagnostics[0]?.sequence).toBeLessThan(
      diagnostics[1]?.sequence ?? 0,
    );
    expect(diagnostics[1]?.sequence).toBeLessThan(
      diagnostics[2]?.sequence ?? 0,
    );
    expect(
      browserTrace()?.browserDiagnostics.some(
        (record) => record.kind === "console" && record.text === "excluded log",
      ),
    ).toBe(false);
  });

  test("exports browser diagnostics on their real OTLP span owners", async () => {
    const otelPath = capturePathViaTool("otel");
    const { pageSpan, testSpan } = await pollUntil(
      "wait.browser_diagnostic_otlp_export",
      () => {
        const spans = completeOtlpSpans(otelPath);
        const testSpan = spans.find(
          (span) =>
            span.name === "e2e.test" &&
            diagnosticsFrom(span)?.some(
              (diagnostic) =>
                typeof diagnostic === "object" &&
                diagnostic !== null &&
                "text" in diagnostic &&
                diagnostic.text === "captured warning",
            ),
        );
        const pageSpan = spans.find(
          (span) =>
            span.name === "e2e.page" &&
            diagnosticsFrom(span)?.some(
              (diagnostic) =>
                typeof diagnostic === "object" &&
                diagnostic !== null &&
                "text" in diagnostic &&
                diagnostic.text === "traced test diagnostic",
            ),
        );
        return testSpan === undefined || pageSpan === undefined
          ? undefined
          : { pageSpan, testSpan };
      },
      {
        intervalMs: 10,
        timeoutMs: 5_000,
        describe: "the exported diagnostic owner spans",
      },
    );

    expectSpanRange(pageSpan);
    expectSpanRange(testSpan);
    expect(pageSpan.spanId).not.toBe(testSpan.spanId);
    expect(attributeFor(pageSpan, "e2e.console_dropped").value).toEqual({
      intValue: "0",
    });
    expect(attributeFor(testSpan, "e2e.console_dropped").value).toEqual({
      intValue: "0",
    });
    expect(consoleDiagnostics(pageSpan)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: "console",
          type: "warning",
          text: "traced test diagnostic",
        }),
      ]),
    );
    expect(consoleDiagnostics(testSpan)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: "console",
          type: "warning",
          text: "captured warning",
        }),
        expect.objectContaining({
          kind: "console",
          type: "error",
          text: "captured error",
        }),
        expect.objectContaining({
          kind: "pageerror",
          message: "captured page error",
        }),
      ]),
    );
  });
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

  const settings = page.getByRole("link", { name: "Settings" });
  await navigateInApp(page, () => settings.click(), {
    url: "/profile",
    ready: "div[role='group'][aria-label='Theme']",
  });

  const theme = page.getByRole("group", { name: "Theme" });
  const reader = theme.getByRole("button", { name: "Reader" });
  await reader.click();

  // The failed persistence write is cosmetic: selecting a built-in changes the
  // mounted caller-visible theme before the browser reports the write failure.
  const root = page.locator(".j-root");
  await expect(root).toHaveAttribute("data-theme", "reader");
  await expect(reader).toHaveAttribute("aria-pressed", "true");
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

  await expect(root).toHaveAttribute("data-theme", "reader");
  await expect(reader).toHaveAttribute("aria-pressed", "true");

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
