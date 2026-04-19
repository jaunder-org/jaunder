import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";

type HydrationRecorder = (payload: { href: string }) => void;

type GlobalWithHydrationRecorder = typeof globalThis & {
  __jaunderRecordHydration?: HydrationRecorder;
};

/** Wait for Leptos WASM hydration and explicitly mark completion for OTEL capture. */
export async function waitForHydration(page: Page): Promise<void> {
  await withTimedAction(page, "wait.hydration", () =>
    page.waitForSelector("body[data-hydrated]"),
  );

  await page.evaluate(() => {
    const globalScope = globalThis as GlobalWithHydrationRecorder;
    const recorder = globalScope.__jaunderRecordHydration;
    if (typeof recorder === "function") {
      recorder({ href: location.href });
    }
  });
}
