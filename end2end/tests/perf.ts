import type { TestInfo } from "@playwright/test";

type MarkEntry = {
  name: string;
  tsMs: number;
};

/**
 * Lightweight per-test timing probe for Playwright flows.
 *
 * Logs a single JSON line so timings are easy to parse in CI output:
 * `[e2e-perf] {...}`
 */
export function createPerfProbe(testInfo: TestInfo, flow: string) {
  const startMs = Date.now();
  const marks: MarkEntry[] = [{ name: "start", tsMs: startMs }];

  return {
    mark(name: string) {
      marks.push({ name, tsMs: Date.now() });
    },
    log(extra: Record<string, unknown> = {}) {
      const sorted = [...marks].sort((a, b) => a.tsMs - b.tsMs);
      const phases = sorted.slice(1).map((entry, index) => {
        const previous = sorted[index];
        return {
          from: previous.name,
          to: entry.name,
          elapsed_ms: entry.tsMs - previous.tsMs,
        };
      });

      const payload = {
        kind: "e2e-perf",
        file: testInfo.file,
        test: testInfo.title,
        flow,
        trace_id: process.env.JAUNDER_E2E_TRACE_ID ?? null,
        traceparent: process.env.JAUNDER_E2E_TRACEPARENT ?? null,
        total_ms: Date.now() - startMs,
        marks: sorted,
        phases,
        ...extra,
      };

      console.log(`[e2e-perf] ${JSON.stringify(payload)}`);
    },
  };
}
