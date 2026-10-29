/** Ancillary visual evidence. Browser reads are not raster-time state. */
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";
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
> & {
  startedUnixMs?: number;
  attachments?: TestInfo["attachments"];
  outputDir?: string;
};
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
    attachments: info.attachments,
    outputDir: info.outputDir,
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

async function collect(page: Page, campaign = false): Promise<unknown> {
  return page.evaluate((campaign) => {
    const limit = campaign ? 6 : 16;
    const triggers = campaign
      ? document.querySelectorAll("button.j-post-action-trigger")
      : [];
    const nodes = campaign
      ? Array.from(triggers)
          .slice(0, 2)
          .flatMap((button) => {
            const parent = button.parentElement;
            return [button, parent, parent?.parentElement].filter(
              (node): node is Element => !!node,
            );
          })
      : Array.from(
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
    const relevantFamilies = campaign
      ? nodes.map((node) => getComputedStyle(node).fontFamily.toLowerCase())
      : [];
    const relevantFonts = campaign
      ? Array.from(document.fonts).filter((font) =>
          relevantFamilies.some((family) =>
            family.includes(font.family.replace(/["']/g, "").toLowerCase()),
          ),
        )
      : [];
    return {
      ...(campaign
        ? {
            status: "available",
            sampled_buttons: Math.min(2, triggers.length),
            omitted_buttons: Math.max(0, triggers.length - 2),
            missing_buttons: Math.max(0, 2 - triggers.length),
            relevant_fonts: relevantFonts.slice(0, 16).map((font) => ({
              family: font.family.slice(0, 256),
              status: font.status,
              style: font.style.slice(0, 256),
              weight: font.weight.slice(0, 256),
              stretch: font.stretch.slice(0, 256),
              unicode_range: font.unicodeRange.slice(0, 256),
            })),
            omitted_fonts: Math.max(0, relevantFonts.length - 16),
            fonts_check: nodes.map((node, index) => {
              const style = getComputedStyle(node);
              const query = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
              return {
                node: index,
                query: query.slice(0, 256),
                available: document.fonts.check(query, "Actions"),
              };
            }),
          }
        : {}),
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
      user_agent: navigator.userAgent.slice(0, campaign ? 256 : 512),
      screenshot_style_active: Array.from(
        document.querySelectorAll("style"),
      ).some((s) => s.textContent?.includes('--font-body: "DejaVu Sans"')),
      fonts_status: document.fonts.status,
      declared_fonts: campaign
        ? "omitted-see-relevant_fonts"
        : Array.from(document.fonts)
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
                : campaign
                  ? "button-ancestor"
                  : "root",
          rect: element.getBoundingClientRect().toJSON(),
          computed: Object.fromEntries(
            properties.map((p) => [p, style.getPropertyValue(p).slice(0, 160)]),
          ),
          hover: element.matches(":hover"),
          focus: element.matches(":focus"),
          focus_visible: element.matches(":focus-visible"),
          expanded: campaign
            ? (element.getAttribute("aria-expanded")?.slice(0, 256) ?? null)
            : element.getAttribute("aria-expanded"),
          root_theme: element.matches(".j-root")
            ? (element
                .getAttribute("data-theme")
                ?.slice(0, campaign ? 256 : Infinity) ?? null)
            : null,
        };
      }),
    };
  }, campaign);
}

function remainingBudget(info: VisualInfo): number {
  if (info.timeout === 0) return Infinity;
  if (info.startedUnixMs === undefined) return 0;
  return (
    info.timeout - (Date.now() - info.startedUnixMs) - originalErrorReserveMs
  );
}

function diagnosticDeadline(budget: number) {
  let timer: ReturnType<typeof setTimeout>;
  const promise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error("diagnostic deadline")), budget);
  });
  return { promise, clear: () => clearTimeout(timer) };
}

/** #1791 only: bounded pre/post reads perturb timing, not screenshot policy. */
async function runTimelineObservation(
  page: Page,
  name: string,
  assertion: () => Promise<void>,
  infoForObservation: () => VisualInfo,
): Promise<void> {
  let spentMs = 0;
  const stamp = () => ({
    unix_ms: Date.now(),
    monotonic_ms: nodePerformance.now(),
  });
  const samples: unknown[] = [];
  // Reuse the lifecycle clock and deadline race. Reads that outlive a deadline
  // remain owned by normal page teardown; no cancellation or readiness loop.
  const phase = async (
    limit: number,
    work: (info: VisualInfo, allowance: () => number) => Promise<unknown>,
  ): Promise<unknown> => {
    const began = nodePerformance.now();
    let deadline: ReturnType<typeof diagnosticDeadline> | undefined;
    try {
      const info = infoForObservation();
      const budget =
        info.startedUnixMs === undefined
          ? 0
          : Math.min(limit, 500 - spentMs, remainingBudget(info));
      if (budget <= 0)
        return {
          status: "skipped",
          reason: "missing-clock-or-exhausted-budget",
        };
      deadline = diagnosticDeadline(budget);
      const expires = nodePerformance.now() + budget;
      const allowance = () =>
        Math.min(expires - nodePerformance.now(), remainingBudget(info));
      return await Promise.race([work(info, allowance), deadline.promise]);
    } catch {
      return {
        status: "unavailable",
        reason: "collection-attachment-or-deadline",
      };
    } finally {
      deadline?.clear();
      spentMs += nodePerformance.now() - began;
    }
  };
  const sample = async (label: string) => {
    const began = stamp();
    const observation = await phase(200, () => collect(page, true));
    samples.push({
      sequence: samples.length + 1,
      phase: label,
      began,
      ended: stamp(),
      observation,
    });
  };
  const began = stamp();
  // All preparation is ancillary, including synchronous metadata/serialization.
  try {
    await sample("pre-assertion");
  } catch {
    reportUnavailable();
  }
  const assertionBegan = stamp();
  let outcome = "passed";
  try {
    await assertion();
  } catch (originalError) {
    outcome = "failed";
    throw originalError;
  } finally {
    const assertionEnded = stamp();
    try {
      await sample("post-assertion");
      const attached = await phase(100, async (info, allowance) => {
        const record = {
          schema_version: 1,
          screenshot: name,
          outcome,
          began,
          assertion_window: { began: assertionBegan, ended: assertionEnded },
          capture_instant_geometry: "unknown",
          rendered_font_face: "unknown",
          pointer_coordinates: "unknown-no-reliable-existing-tracker",
          typography_caveat:
            "Pre/post reads bracket the assertion, not raster capture; stylePath applies screenshot-only font overrides inside the assertion.",
          runtime: {
            platform: process.platform,
            arch: process.arch,
            browser_name:
              page.context().browser()?.browserType().name() ?? "unknown",
            browser_version: page.context().browser()?.version() ?? "unknown",
          },
          diagnostic_work_ms_before_attachment: spentMs,
          attachment_status: "submitted-completion-unobservable-in-own-body",
          string_limit_characters: 256,
          strings_may_be_truncated: true,
          omissions: [] as string[],
          samples,
        };
        let geometryOnly = false;
        const serialize = () =>
          Buffer.from(
            JSON.stringify(record, (key, value) => {
              if (
                geometryOnly &&
                ["computed", "relevant_fonts", "fonts_check"].includes(key)
              )
                return { status: "omitted", reason: "serialized-size-cap" };
              return typeof value === "string" ? value.slice(0, 256) : value;
            }),
          );
        let body = serialize();
        if (body.length > 16_384) {
          geometryOnly = true;
          record.omissions.push(
            "computed-styles-and-font-details-omitted-for-16KiB-cap",
          );
          body = serialize();
        }
        if (body.length > 16_384) {
          record.omissions.push("sample-details-omitted-for-16KiB-cap");
          record.samples = samples.map((sample) => {
            const value = sample as {
              sequence: number;
              phase: string;
              began: unknown;
              ended: unknown;
            };
            return {
              ...value,
              observation: {
                status: "unavailable",
                reason: "serialized-size-cap",
              },
            };
          });
          body = serialize();
        }
        if (body.length > 16_384) throw new Error("diagnostic size cap");
        // Serialization is part of this phase; do not start a sink after it
        // has consumed the remaining allowance or original-error reserve.
        if (allowance() <= 0) throw new Error("diagnostic budget exhausted");
        await info.attach(`visual-timeline-${info.retry}-${++sequence}`, {
          body,
          contentType: "application/json",
        });
        return { status: "attached" };
      });
      if ((attached as { status?: string })?.status !== "attached")
        reportUnavailable();
    } catch {
      reportUnavailable();
    }
  }
}

function reportUnavailable(): void {
  try {
    console.warn(
      "visual diagnostics unavailable (collection, attachment or deadline)",
    );
  } catch {
    // A broken ancillary reporter must not alter either screenshot outcome.
  }
}

/** Internal visual-helper seam; callbacks permit deterministic failure controls. */
async function runOriginalVisualAssertion(
  page: Page,
  name: string,
  assertion: () => Promise<void>,
  infoForFailure: () => VisualInfo,
): Promise<void> {
  if (name === "home-timeline.png") {
    await runTimelineObservation(page, name, assertion, infoForFailure);
    return;
  }
  const began = { unix_ms: Date.now(), monotonic_ms: nodePerformance.now() };
  try {
    await assertion();
  } catch (originalError) {
    const failed = { unix_ms: Date.now(), monotonic_ms: nodePerformance.now() };
    let deadline: ReturnType<typeof diagnosticDeadline> | undefined;
    try {
      const info = infoForFailure();
      const budget = Math.min(diagnosticLimitMs, remainingBudget(info));
      if (budget <= 0) throw new Error("diagnostic budget exhausted");
      deadline = diagnosticDeadline(budget);
      // A single deadline covers collection AND attachment; it never extends
      // the test budget. A timed-out browser read is left to normal page teardown.
      const observation = await Promise.race([collect(page), deadline.promise]);
      const producer = await Promise.race([producerInput(), deadline.promise]);
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
        deadline.promise,
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
      deadline?.clear();
    }
    throw originalError;
  }
}

/** Existing helper seam: the producer callback substitutes only the file boundary in tests. */
export async function runVisualAssertion(
  page: Page,
  name: string,
  assertion: () => Promise<void>,
  infoForFailure: () => VisualInfo = currentInfo,
  producerForProbe: () => Promise<unknown> = producerInput,
): Promise<void> {
  let failed = false;
  let originalError: unknown;
  try {
    await runOriginalVisualAssertion(page, name, assertion, infoForFailure);
  } catch (error) {
    failed = true;
    originalError = error;
  }
  // Original matcher and existing diagnostics finish before any new probe work.
  if (name === "home-timeline.png") {
    try {
      await runCornerProbe(page, infoForFailure(), producerForProbe, failed);
    } catch {
      reportUnavailable();
    }
  }
  if (failed) throw originalError;
}

/** #1791 only. Admission limits work, not RPC lifetime; normal teardown owns RPCs.
 * Main-document style injection and screenshot style (which pierces frames/shadows)
 * are intentionally coupled. Explicit zero transforms also change the paint path.
 */
async function runCornerProbe(
  page: Page,
  info: VisualInfo,
  producer: () => Promise<unknown>,
  failed: boolean,
): Promise<void> {
  if (info.project.name !== "chromium-visual" || info.retry !== 0) return;
  const began = nodePerformance.now();
  const expires = began + 750;
  const remaining = () =>
    Math.min(expires - nodePerformance.now(), remainingBudget(info));
  const prefix = `visual-corner-probe-${info.retry}-${++sequence}`;
  const record: Record<string, unknown> = {
    schema_version: 1,
    status: "skipped",
    reason: "not-admitted",
    original_outcome: failed ? "failed" : "passed",
    began_unix_ms: Date.now(),
    limits: {
      budget_ms: 750,
      cleanup_allowance_ms: 100,
      original_reserve_ms: 100,
      png_count: 3,
      png_bytes: 262144,
      json_bytes: 32768,
      string_characters: 256,
      controls: 2,
    },
    caveat:
      "Coupled geometry/paint experiment; locator actionability, scroll and clipping plus duplicated CSS are confounds. Rect brackets are not raster-time telemetry or original comparison buffers.",
    css_scope:
      "Main-document target only; screenshot style also pierces frames/shadows, unlike main-frame style tag.",
  };
  const attachRecord = async (suffix = "") => {
    const body = Buffer.from(
      JSON.stringify(record, (_, value) =>
        typeof value === "string" ? value.slice(0, 256) : value,
      ),
    );
    if (body.length > 32768) throw new Error("probe JSON size cap");
    if (remaining() <= 0)
      throw new Error("probe metadata attachment unavailable: budget");
    await info.attach(`${prefix}${suffix}`, {
      body,
      contentType: "application/json",
    });
  };
  // Reserve 100ms for the original result in addition to the full 750ms probe.
  if (info.startedUnixMs === undefined || remainingBudget(info) < 750) {
    record.reason = "missing-clock-or-less-than-850ms-remaining";
    if (remaining() > 0) await attachRecord();
    else reportUnavailable();
    return;
  }
  // Leave 100ms of this probe's allowance for independent cleanup and metadata.
  const admit = () => {
    if (remaining() <= 100)
      throw new Error("probe work unavailable: cleanup allowance");
  };
  let style: Awaited<ReturnType<Page["addStyleTag"]>> | undefined;
  let target: ReturnType<Page["locator"]> | undefined;
  let originalStyle: string | null = null;
  let savedStyle = false;
  let initialScroll: { x: number; y: number } | undefined;
  const phases: Array<Record<string, unknown>> = [];
  record.phases = phases;
  const cleanupErrors: string[] = [];
  record.cleanup_errors = cleanupErrors;
  try {
    admit();
    const input = (await producer()) as { lane?: string } | null;
    if (input?.lane !== "sqlite-chromium-unsplit") {
      record.reason = "unknown-or-ineligible-producer-lane";
      return;
    }
    record.producer_lane = input.lane;
    // Freeze original public attachment identities BEFORE any screenshot call.
    // No probe API receives a path: original actual/expected/diff, context and
    // teardown-owned trace paths cannot be reused by these ancillary captures.
    record.original_evidence = {
      attachments: (info.attachments ?? []).map(
        ({ name, path, contentType }) => ({ name, path, contentType }),
      ),
      context_path: info.outputDir
        ? join(info.outputDir, "error-context.md")
        : "unavailable",
      trace_path: info.outputDir
        ? join(info.outputDir, "trace.zip")
        : "unavailable",
      trace_status: "normal-teardown-owned-may-not-yet-exist",
    };
    if (failed) {
      record.reason = "original-evidence-saved-before-probe";
      admit();
      await attachRecord("-original-evidence");
    }
    admit();
    const cssBytes = await readFile(join(__dirname, "visual.css"));
    const css = cssBytes.toString("utf8");
    record.css_sha256 = createHash("sha256").update(cssBytes).digest("hex");
    record.browser = {
      name: page.context().browser()?.browserType().name(),
      version: page.context().browser()?.version(),
    };
    admit();
    initialScroll = await page.evaluate(() => ({ x: scrollX, y: scrollY }));
    admit();
    style = await page.addStyleTag({ content: css });
    // Portal controls are siblings of the theme surface, not article children.
    const controls = page.locator(
      "#j-trusted-post-actions button.j-post-action-trigger",
    );
    admit();
    const count = Math.min(2, await controls.count());
    const candidates: Array<{
      index: number;
      state: Awaited<ReturnType<typeof geometry>>;
    }> = [];
    for (let index = 0; index < count; index++) {
      admit();
      const state = await geometry(
        controls.nth(index),
        Math.min(150, remaining() - 100),
      );
      if (
        state.main_document &&
        state.rect.width > 0 &&
        state.rect.height > 0 &&
        state.rect.right > 0 &&
        state.rect.bottom > 0 &&
        state.rect.left < state.viewport.width &&
        state.rect.top < state.viewport.height
      )
        candidates.push({ index, state });
    }
    record.controls = candidates;
    if (candidates.length !== 1) {
      record.reason = "none-or-ambiguous-in-viewport-controls";
      return;
    }
    const selected = candidates[0];
    record.selected_index = selected.index;
    if (selected.state.transform !== "none") {
      record.reason = "unsupported-existing-transform";
      return;
    }
    target = controls.nth(selected.index);
    admit();
    originalStyle = await target.getAttribute("style", {
      timeout: Math.min(150, remaining() - 100),
    });
    savedStyle = true;
    record.original_style = originalStyle;
    record.status = "unavailable";
    record.reason = "incomplete-phases";
    for (const [phase, dy] of [
      ["zero-before", 0],
      [
        "integer-snap",
        Math.round(selected.state.rect.y) - selected.state.rect.y,
      ],
      ["zero-after", 0],
    ] as const) {
      const result: Record<string, unknown> = {
        phase,
        dy,
        status: "unavailable",
        began_unix_ms: Date.now(),
      };
      phases.push(result);
      try {
        admit();
        await target.evaluate(
          (el, offset) => {
            (el as HTMLElement).style.transform = `translateY(${offset}px)`;
          },
          dy,
          { timeout: Math.min(150, remaining() - 100) },
        );
        admit();
        result.before = await geometry(
          target,
          Math.min(150, remaining() - 100),
        );
        const timeout = Math.min(150, remaining() - 100);
        if (timeout <= 0) {
          result.reason = "non-positive-capture-timeout";
          continue;
        }
        const png = await target.screenshot({
          type: "png",
          scale: "css",
          animations: "disabled",
          caret: "hide",
          style: css,
          timeout,
        });
        admit();
        result.after = await geometry(target, Math.min(150, remaining() - 100));
        if (png.length > 262144) throw new Error("probe PNG size cap");
        if (
          png.length < 24 ||
          png.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a"
        )
          throw new Error("invalid PNG");
        result.png = {
          width: png.readUInt32BE(16),
          height: png.readUInt32BE(20),
          bytes: png.length,
          sha256: createHash("sha256").update(png).digest("hex"),
        };
        admit();
        const attachment = `${prefix}-${phase}`;
        await info.attach(attachment, { body: png, contentType: "image/png" });
        result.attachment = attachment;
        result.status = "available";
      } catch (error) {
        result.reason = String(error).slice(0, 256);
      } finally {
        result.ended_unix_ms = Date.now();
      }
    }
    if (phases.length === 3 && phases.every((p) => p.status === "available")) {
      const control = phases[0].before;
      const returned = phases[2].after;
      const stable = (a: unknown, b: unknown) => {
        const pick = (v: unknown) => {
          const s = v as Awaited<ReturnType<typeof geometry>>;
          return { rect: s.rect, scroll: s.scroll };
        };
        return JSON.stringify(pick(a)) === JSON.stringify(pick(b));
      };
      const brackets = phases.every((p) => stable(p.before, p.after));
      record.status =
        brackets && stable(control, returned) ? "available" : "INVALID";
      record.reason =
        brackets && stable(control, returned)
          ? "geometry-and-scroll-returned-raw-PNG-review-required"
          : "geometry-or-scroll-did-not-return";
    }
  } catch (error) {
    record.status = "unavailable";
    record.reason = String(error).slice(0, 256);
  } finally {
    // Each attempt is independent. Admission is best effort, not cancellation;
    // a late RPC may exhaust the allowance, making restoration INVALID.
    const cleanup = async (label: string, work: () => Promise<unknown>) => {
      try {
        if (remaining() <= 0) throw new Error("cleanup budget unavailable");
        await work();
      } catch (error) {
        cleanupErrors.push(`${label}: ${String(error)}`.slice(0, 256));
      }
    };
    if (target && savedStyle)
      await cleanup("inline-style", async () => {
        await target!.evaluate(
          (el, value) => {
            if (value === null) el.removeAttribute("style");
            else el.setAttribute("style", value);
          },
          originalStyle,
          { timeout: Math.min(150, remaining()) },
        );
        if (remaining() <= 0)
          throw new Error("style verification budget unavailable");
        if (
          (await target!.getAttribute("style", {
            timeout: Math.min(150, remaining()),
          })) !== originalStyle
        )
          throw new Error("inline style did not return");
      });
    if (initialScroll)
      await cleanup("scroll", async () => {
        await page.evaluate(
          ({ x, y }) => scrollTo({ left: x, top: y, behavior: "instant" }),
          initialScroll!,
        );
        if (remaining() <= 0)
          throw new Error("scroll verification budget unavailable");
        const returned = await page.evaluate(() => ({
          x: scrollX,
          y: scrollY,
        }));
        if (returned.x !== initialScroll!.x || returned.y !== initialScroll!.y)
          throw new Error("scroll did not return");
      });
    if (style)
      await cleanup("stylesheet", () =>
        style!.evaluate((el) => (el as Element).remove()),
      );
    if (cleanupErrors.length) {
      record.status = "INVALID";
      record.reason = "cleanup-failed";
    }
    record.elapsed_ms = nodePerformance.now() - began;
    try {
      await attachRecord();
    } catch {
      reportUnavailable();
    }
  }
}

async function geometry(target: ReturnType<Page["locator"]>, timeout: number) {
  return target.evaluate(
    (el) => {
      const computed = getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      return {
        rect: {
          x: rect.x,
          y: rect.y,
          left: rect.left,
          top: rect.top,
          right: rect.right,
          bottom: rect.bottom,
          width: rect.width,
          height: rect.height,
        },
        scroll: { x: scrollX, y: scrollY },
        viewport: {
          width: innerWidth,
          height: innerHeight,
          dpr: devicePixelRatio,
        },
        transform: computed.transform.slice(0, 256),
        radius: {
          top_right: computed.borderTopRightRadius.slice(0, 256),
          bottom_right: computed.borderBottomRightRadius.slice(0, 256),
        },
        font_status: document.fonts.status,
        main_document:
          el.ownerDocument === document &&
          el.getRootNode() === document &&
          window === window.top,
        observed_unix_ms: performance.timeOrigin + performance.now(),
      };
    },
    undefined,
    { timeout },
  );
}

let sequence = 0;

/** #1791: session facts plus configured intent, never OS-process attestation. */
export async function recordVisualIdentity(page: Page): Promise<void> {
  const began = nodePerformance.now();
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const info = currentInfo();
    const remaining =
      info.startedUnixMs === undefined
        ? 0
        : info.timeout === 0
          ? 100
          : info.timeout -
            (Date.now() - info.startedUnixMs) -
            originalErrorReserveMs;
    const expires = began + Math.min(100, remaining);
    if (expires <= nodePerformance.now())
      throw new Error("missing clock or exhausted budget");
    const browser = page.context().browser();
    const type = browser?.browserType();
    const use = info.project.use as {
      headless?: boolean;
      channel?: string;
      launchOptions?: {
        executablePath?: string;
        headless?: boolean;
        channel?: string;
      };
      connectOptions?: unknown;
    };
    const version = browser?.version();
    const record = {
      schema_version: 1,
      status: browser && type && version ? "available" : "unavailable",
      observed_unix_ms: Date.now(),
      observed_monotonic_ms: nodePerformance.now(),
      project: info.project.name,
      actual_session: {
        browser_version: version ?? "unknown",
        browser_type: type?.name() ?? "unknown",
      },
      configured_intent: {
        api_default_executable_path: type?.executablePath() ?? "unknown",
        playwright_browsers_path:
          process.env.PLAYWRIGHT_BROWSERS_PATH ?? "unknown",
        executable_override: use.launchOptions?.executablePath ?? "unset",
        channel: use.channel ?? use.launchOptions?.channel ?? "unset",
        headless: use.headless ?? use.launchOptions?.headless ?? true,
        headless_basis:
          "configured value or Playwright default; not process observation",
        pwdebug: process.env.PWDEBUG ?? "unset",
        connection_configured: !!use.connectOptions,
        selection_caveat:
          "API default path does not account for headless-shell selection; compare separately retained preflight metadata.",
        thumbnail_browser: "JAUNDER_THEME_THUMBNAIL_BROWSER is unrelated",
      },
      actual_os_executable: "unproven",
      interpretation:
        "Qualified session-version plus configured-intent comparison only; does not repair historical identity. Calls perturb timing.",
      string_limit_characters: 256,
      strings_may_be_truncated: true,
    };
    const body = Buffer.from(
      JSON.stringify(record, (_, value) =>
        typeof value === "string" ? value.slice(0, 256) : value,
      ),
    );
    if (body.length > 16_384) throw new Error("identity size cap");
    const budget = expires - nodePerformance.now();
    if (budget <= 0) throw new Error("identity deadline");
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error("identity deadline")), budget);
    });
    await Promise.race([
      info.attach(`visual-identity-${info.retry}-${++sequence}`, {
        body,
        contentType: "application/json",
      }),
      deadline,
    ]);
  } catch {
    // Missing identity stops later campaign admission externally, not this test.
    try {
      console.warn(
        "visual identity unavailable (clock, collection, serialization, attachment or deadline); stop campaign admission",
      );
    } catch {
      // Ancillary reporting must not alter either screenshot outcome.
    }
  } finally {
    if (timer) clearTimeout(timer);
  }
}
