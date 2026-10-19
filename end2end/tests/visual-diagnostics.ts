/** Failure-only visual evidence. Post-failure reads are not capture-time state. */
import { readFile } from "node:fs/promises";
import { performance as nodePerformance } from "node:perf_hooks";
import {
  test,
  type BrowserContextOptions,
  type Page,
  type TestInfo,
} from "@playwright/test";

const diagnosticLimitMs = 1_000;
const originalErrorReserveMs = 100;

type VisualInfo = Pick<
  TestInfo,
  "timeout" | "testId" | "retry" | "project" | "config" | "attach"
> & { startedUnixMs?: number };
const testStarts = new WeakMap<TestInfo, number>();

/** Reuse the harness's existing lifecycle clock, without a browser observation. */
export function markVisualTestStart(info: TestInfo, unixMs: number): void {
  testStarts.set(info, unixMs);
}

function currentInfo(): VisualInfo {
  const info = test.info();
  return {
    timeout: info.timeout,
    testId: info.testId,
    retry: info.retry,
    project: info.project,
    config: info.config,
    attach: info.attach.bind(info),
    startedUnixMs: testStarts.get(info),
  };
}

async function producerInput(): Promise<unknown> {
  const path = process.env.JAUNDER_E2E_PRODUCER_INPUT;
  if (!path) return { available: false };
  try {
    const body = await readFile(path, "utf8");
    if (body.length > 65_536) return { available: false, reason: "oversized" };
    const value = JSON.parse(body);
    if (value?.schema_version !== 1 || typeof value?.lane !== "string") {
      return { available: false, reason: "invalid-schema" };
    }
    // Host and guest schemas are controlled producers, not environment dumps.
    return {
      schema_version: value.schema_version,
      lane: value.lane,
      revision: value.revision,
      dirty_revision: value.dirty_revision,
      requesting_checkout: value.requesting_checkout,
      input_attestation: value.input_attestation,
    };
  } catch {
    return { available: false, reason: "unreadable" };
  }
}

async function collect(page: Page): Promise<unknown> {
  return page.evaluate(() => {
    const limit = 16;
    const nodes = Array.from(
      document.querySelectorAll(
        ".j-root, .j-post-actions-slot, .j-post-action-control, .j-post-action-trigger",
      ),
    );
    const properties = [
      "position",
      "position-anchor",
      "anchor-name",
      "font-family",
      "font-size",
      "font-weight",
      "line-height",
      "border-right-width",
      "border-right-color",
      "border-top-right-radius",
      "border-bottom-right-radius",
      "transform",
      "zoom",
    ];
    return {
      observed_unix_ms: performance.timeOrigin + performance.now(),
      observed_performance_ms: performance.now(),
      viewport: {
        width: innerWidth,
        height: innerHeight,
        dpr: devicePixelRatio,
      },
      scroll: { x: scrollX, y: scrollY },
      ready_state: document.readyState,
      csr_mounted: document.body.hasAttribute("data-mounted"),
      color_scheme_preference: matchMedia("(prefers-color-scheme: dark)")
        .matches
        ? "dark"
        : "light",
      reduced_motion_preference: matchMedia("(prefers-reduced-motion: reduce)")
        .matches,
      user_agent: navigator.userAgent.slice(0, 512),
      screenshot_style_active: Array.from(
        document.querySelectorAll("style"),
      ).some((s) => s.textContent?.includes('--font-body: "DejaVu Sans"')),
      fonts_status: document.fonts.status,
      declared_fonts: Array.from(document.fonts)
        .slice(0, 8)
        .map((f) => ({
          family: f.family.slice(0, 80),
          status: f.status,
          weight: f.weight,
        })),
      active_element_tag: document.activeElement?.tagName ?? null,
      omitted_nodes: Math.max(0, nodes.length - limit),
      nodes: nodes.slice(0, limit).map((element) => {
        const style = getComputedStyle(element);
        return {
          tag: element.tagName,
          role: element.matches(".j-post-action-trigger")
            ? "actions-trigger"
            : element.matches(".j-post-actions-slot")
              ? "actions-slot"
              : element.matches(".j-post-action-control")
                ? "actions-control"
                : "root",
          rect: element.getBoundingClientRect().toJSON(),
          computed: Object.fromEntries(
            properties.map((p) => [p, style.getPropertyValue(p).slice(0, 160)]),
          ),
          hover: element.matches(":hover"),
          focus: element.matches(":focus"),
          focus_visible: element.matches(":focus-visible"),
          expanded: element.getAttribute("aria-expanded"),
          root_theme: element.matches(".j-root")
            ? element.getAttribute("data-theme")
            : null,
        };
      }),
    };
  });
}

/** Internal visual-helper seam; callbacks permit deterministic failure controls. */
export async function runVisualAssertion(
  page: Page,
  name: string,
  assertion: () => Promise<void>,
  infoForFailure: () => VisualInfo = currentInfo,
): Promise<void> {
  const began = { unix_ms: Date.now(), monotonic_ms: nodePerformance.now() };
  try {
    await assertion();
  } catch (originalError) {
    const failed = { unix_ms: Date.now(), monotonic_ms: nodePerformance.now() };
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const info = infoForFailure();
      const remaining =
        info.timeout === 0
          ? diagnosticLimitMs
          : info.startedUnixMs === undefined
            ? 0
            : info.timeout -
              (Date.now() - info.startedUnixMs) -
              originalErrorReserveMs;
      const budget = Math.min(diagnosticLimitMs, remaining);
      if (budget <= 0) throw new Error("diagnostic budget exhausted");
      const deadline = new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error("diagnostic deadline")),
          budget,
        );
      });
      // A single deadline covers collection AND attachment; it never extends
      // the test budget. A timed-out browser read is left to normal page teardown.
      const observation = await Promise.race([collect(page), deadline]);
      const producer = await Promise.race([producerInput(), deadline]);
      const use = info.project.use as BrowserContextOptions & {
        browserName?: string;
      };
      const body = Buffer.from(
        JSON.stringify(
          {
            schema_version: 1,
            observation_phase: "post-assertion-failure",
            assertion_window: { began, failed },
            observed_after_failure: true,
            capture_instant_geometry: "unobserved",
            rendered_font_face: "unmeasured",
            initial_pointer_coordinates: "unobserved",
            test_id: info.testId,
            retry: info.retry,
            project: info.project.name,
            screenshot: name,
            producer_input: producer,
            runtime: {
              platform: process.platform,
              arch: process.arch,
              configured_workers: info.config.workers,
              configured_retries: info.project.retries,
              effective_timeout_ms: info.timeout,
              fontconfig_store_path: process.env.FONTCONFIG_FILE?.startsWith(
                "/nix/store/",
              )
                ? process.env.FONTCONFIG_FILE.slice(0, 512)
                : "unavailable-or-non-store",
              browsers_store_path:
                process.env.PLAYWRIGHT_BROWSERS_PATH?.startsWith("/nix/store/")
                  ? process.env.PLAYWRIGHT_BROWSERS_PATH.slice(0, 512)
                  : "unavailable-or-non-store",
              playwright_version: require("@playwright/test/package.json")
                .version,
              browser_name:
                page.context().browser()?.browserType().name() ??
                use.browserName ??
                "unavailable",
              browser_version:
                page.context().browser()?.version() ?? "unavailable",
              viewport: use.viewport,
              device_scale_factor: use.deviceScaleFactor,
              color_scheme: use.colorScheme,
              reduced_motion: use.reducedMotion,
            },
            observation,
          },
          null,
          2,
        ),
      );
      // Playwright gives each attachment a unique artifact path; include retry
      // and a per-process sequence so two failed assertions cannot collide.
      const attachment = `visual-failure-${info.retry}-${++sequence}`;
      await Promise.race([
        info.attach(attachment, { body, contentType: "application/json" }),
        deadline,
      ]);
    } catch {
      // Diagnostics are ancillary: neither collection nor a broken attachment
      // sink may replace the identical original screenshot matcher rejection.
      try {
        console.warn(
          "visual diagnostics unavailable (collection, attachment or deadline)",
        );
      } catch {
        // Even a broken warning sink is ancillary to the original matcher error.
      }
    } finally {
      if (timer) clearTimeout(timer);
    }
    throw originalError;
  }
}

let sequence = 0;
