import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";

/**
 * The body attribute the CSR client sets once `mount_to_body` has run — the
 * suite's "app is mounted and interactive" signal. Counterpart of the literal in
 * `csr/src/lib.rs`'s `mark_ready` inline JS; the two must agree or every e2e test
 * times out. Declared once here so a rename touches one place (#251).
 */
export const MOUNTED_ATTR = "data-mounted";

/** {@link MOUNTED_ATTR} as a body selector, for `waitForSelector`. */
export const MOUNTED_SELECTOR = `body[${MOUNTED_ATTR}]`;

type MountRecorder = (payload: { href: string }) => void;

type GlobalWithMountRecorder = typeof globalThis & {
  __jaunderRecordMount?: MountRecorder;
};

/** Wait for the CSR mount and explicitly mark completion for OTEL capture. */
export async function waitForMount(
  page: Page,
  timeoutMs?: number,
): Promise<void> {
  await withTimedAction(page, "wait.mount", () =>
    page.waitForSelector(MOUNTED_SELECTOR, {
      timeout: timeoutMs,
    }),
  );

  await page.evaluate(() => {
    const globalScope = globalThis as GlobalWithMountRecorder;
    const recorder = globalScope.__jaunderRecordMount;
    if (typeof recorder === "function") {
      recorder({ href: location.href });
    }
  });
}
