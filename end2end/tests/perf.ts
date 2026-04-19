import type { TestInfo } from "@playwright/test";
import {
  buildSpan,
  exportSpans,
  makeEvent,
  otlpAttribute,
  traceContextFromEnvironment,
} from "./otel";

type MarkEntry = {
  name: string;
  tsMs: number;
};

/**
 * Lightweight semantic flow probe for Playwright tests.
 *
 * This emits an OTEL span (`e2e.flow.*`) with mark-to-mark phase timings.
 * It complements the automatic coarse-grained `e2e.test` span in fixtures.ts.
 */
export function createPerfProbe(testInfo: TestInfo, flow: string) {
  const startMs = Date.now();
  const marks: MarkEntry[] = [{ name: "start", tsMs: startMs }];
  const traceContext = traceContextFromEnvironment();

  return {
    mark(name: string) {
      marks.push({ name, tsMs: Date.now() });
    },
    async log(extra: Record<string, unknown> = {}) {
      const sorted = [...marks].sort((a, b) => a.tsMs - b.tsMs);
      const phases = sorted.slice(1).map((entry, index) => {
        const previous = sorted[index];
        return {
          from: previous.name,
          to: entry.name,
          elapsed_ms: entry.tsMs - previous.tsMs,
        };
      });
      const endMs = Date.now();

      const attributes = [
        otlpAttribute("e2e.file", testInfo.file),
        otlpAttribute("e2e.test", testInfo.title),
        otlpAttribute("e2e.flow", flow),
        otlpAttribute("e2e.total_ms", endMs - startMs),
        otlpAttribute("e2e.trace_id", process.env.JAUNDER_E2E_TRACE_ID ?? null),
        otlpAttribute(
          "e2e.traceparent",
          process.env.JAUNDER_E2E_TRACEPARENT ?? null,
        ),
        otlpAttribute("e2e.phases_json", JSON.stringify(phases)),
        otlpAttribute("e2e.extra_json", JSON.stringify(extra)),
      ].filter(
        (attribute): attribute is NonNullable<typeof attribute> =>
          attribute !== null,
      );

      const events = sorted.map((mark) =>
        makeEvent(
          `mark:${mark.name}`,
          mark.tsMs,
          [otlpAttribute("e2e.mark_name", mark.name)].filter(
            (attribute): attribute is NonNullable<typeof attribute> =>
              attribute !== null,
          ),
        ),
      );

      const span = buildSpan({
        traceContext,
        name: `e2e.flow.${flow}`,
        kind: "client",
        startMs,
        endMs,
        attributes,
        events,
      });

      try {
        await exportSpans([span]);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        console.warn(`[e2e-otel] flow export failed: ${message}`);
      }
    },
  };
}
