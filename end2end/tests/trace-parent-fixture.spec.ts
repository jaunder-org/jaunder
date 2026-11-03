import { expect, type BrowserContext, type Page } from "@playwright/test";
import { test as tracedTest } from "./fixtures";
import { goto } from "./helpers";
import { navigateInApp } from "./navigate";
import { traceContextFromEnvironment } from "./otel";

type RequestObservation = {
  phase: "page-fixture-setup" | "test-body" | "helper-context";
  contextId: number;
  pageId: number;
  method: string;
  path: string;
  matchesTestParent: boolean;
  probeHeaderPreserved: boolean;
};

let phase: RequestObservation["phase"] = "page-fixture-setup";
let observations: RequestObservation[] = [];
let nextContextId = 0;

function observeGetSessionRequests(
  context: BrowserContext,
  contextId: number,
  pageId: number,
  testSpanId: string,
): void {
  const traceId = traceContextFromEnvironment().traceId;
  context.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname !== "/api/auth/get_session") return;
    const headers = request.headers();
    const probeHeader =
      phase === "helper-context" ? "x-e2e-helper-probe" : "x-e2e-default-probe";
    const probeValue =
      phase === "helper-context"
        ? "helper-header-preserved"
        : "default-header-preserved";
    observations.push({
      phase,
      contextId,
      pageId,
      method: request.method(),
      path: url.pathname,
      matchesTestParent:
        headers["traceparent"] === `00-${traceId}-${testSpanId}-01`,
      probeHeaderPreserved: headers[probeHeader] === probeValue,
    });
  });
}

const setupNavigatesBeforeTest = tracedTest.extend({
  page: async (
    { context, testSpanId }: { context: BrowserContext; testSpanId: string },
    use: (page: Page) => Promise<void>,
  ) => {
    phase = "page-fixture-setup";
    observations = [];
    const contextId = ++nextContextId;
    const pageId = contextId;
    observeGetSessionRequests(context, contextId, pageId, testSpanId);

    // This is a supported page-fixture setup navigation, before the base fixture
    // yields page to the composed page-dependent auto fixtures.
    const page = await context.newPage();
    const setupResponse = page.waitForResponse((response) => {
      return new URL(response.url()).pathname === "/api/auth/get_session";
    });
    await goto(page, "/");
    await setupResponse;
    await use(page);
  },
});

const displacedTraceparent =
  "00-11111111111111111111111111111111-2222222222222222-01";
setupNavigatesBeforeTest.use({
  extraHTTPHeaders: {
    "x-e2e-default-probe": "default-header-preserved",
    TraceParent: displacedTraceparent,
  },
});

setupNavigatesBeforeTest(
  "page fixture setup and test-body requests carry their owning test parent",
  async ({ page, testSpanId, tracedContext }) => {
    phase = "test-body";
    const bodyResponse = page.waitForResponse((response) => {
      return new URL(response.url()).pathname === "/api/auth/get_session";
    });
    await navigateInApp(
      page,
      () => page.locator('a[href="/register"]').click(),
      { url: "/register", ready: 'input[name="username"]' },
    );
    await bodyResponse;

    const setupRequests = observations.filter(
      (observation) => observation.phase === "page-fixture-setup",
    );
    const bodyRequests = observations.filter(
      (observation) => observation.phase === "test-body",
    );
    expect(testSpanId).toMatch(/^[0-9a-f]{16}$/);
    expect(setupRequests.length).toBeGreaterThan(0);
    expect(bodyRequests.length).toBeGreaterThan(0);
    // In-memory comparison only: never serialize traceparent/header values.
    expect(
      setupRequests.every((observation) => observation.matchesTestParent),
    ).toBe(true);
    expect(
      bodyRequests.every((observation) => observation.matchesTestParent),
    ).toBe(true);
    expect(
      [...setupRequests, ...bodyRequests].every(
        (observation) => observation.probeHeaderPreserved,
      ),
    ).toBe(true);

    phase = "helper-context";
    const helperContext = await tracedContext({
      extraHTTPHeaders: {
        "x-e2e-helper-probe": "helper-header-preserved",
        TraceParent: displacedTraceparent,
      },
    });
    try {
      const helperContextId = ++nextContextId;
      observeGetSessionRequests(
        helperContext,
        helperContextId,
        helperContextId,
        testSpanId,
      );
      const helperPage = await helperContext.newPage();
      const helperResponse = helperPage.waitForResponse((response) => {
        return new URL(response.url()).pathname === "/api/auth/get_session";
      });
      await goto(helperPage, "/");
      await helperResponse;
      const helperRequests = observations.filter(
        (observation) => observation.phase === "helper-context",
      );
      expect(helperRequests.length).toBeGreaterThan(0);
      expect(
        helperRequests.every((observation) => observation.matchesTestParent),
      ).toBe(true);
      expect(
        helperRequests.every((observation) => observation.probeHeaderPreserved),
      ).toBe(true);
    } finally {
      await helperContext.close();
    }
  },
);
