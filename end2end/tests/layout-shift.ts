/**
 * Reusable empirical layout-shift (CLS) probe for the projector-paint → wasm-mount
 * transition (#202). Deterministic by construction: it gates only on the wasm-route
 * release, `body[data-mounted]`, and `document.fonts.ready` — never a timer — so it
 * is safe under `fullyParallel` `workers>1` (#182).
 *
 * Page-agnostic: a per-page CLS check supplies a {@link MountShiftProbe} (its `url`,
 * the `targets` to measure, an optional post-mount assertion, and a `tolerancePx`)
 * and calls {@link expectNoShiftAcrossMount}. The first concrete use is the
 * authed-owner own-post action column (`authed-cls.spec.ts`).
 */
import { expect, type Page, type Locator } from "@playwright/test";
import { BASE_URL } from "./helpers";
import { waitForMount } from "./mount";

export interface MountShiftProbe {
  /**
   * Path to probe (e.g. `"/"`), prepended with `BASE_URL` like the harness `goto`.
   * Loaded with the wasm held, so first paint is the projector's.
   */
  url: string;
  /**
   * Elements whose top-left must not move. Resolved on `page` after `goto` (they
   * must exist in the projector first paint). Each is named so a failure says which
   * element shifted. Use author/content-scoped locators — on a multi-item page the
   * measured element must be the same one before and after mount.
   */
  targets: (page: Page) => { name: string; locator: Locator }[];
  /**
   * Optional: assert the mount actually decorated the measured content (so a green
   * result can't be a no-op) — e.g. the owner action column appeared. Runs after
   * the mount, before the after-sample.
   */
  afterMount?: (page: Page) => Promise<void>;
  /**
   * Max allowed |Δ| per axis, in px. `0` (default) = exact equality. A caller
   * loosens with a comment citing the observed value + browser ("start exact,
   * loosen on evidence"); passing it per-probe keeps other pages strict.
   */
  tolerancePx?: number;
}

/**
 * Asserts each probe target's top-left does not move across the projector-paint →
 * wasm-mount transition on `probe.url`.
 */
export async function expectNoShiftAcrossMount(
  page: Page,
  probe: MountShiftProbe,
): Promise<void> {
  const tol = probe.tolerancePx ?? 0;
  const WASM = "**/pkg/jaunder*.wasm";

  // Hold the wasm so `init()` can't complete → the projector first paint stays
  // frozen while we sample. NOTE: `page.route` also disables Playwright's HTTP cache
  // for this URL, forcing a fresh, holdable request even though the wasm was already
  // warmed by an earlier navigation. Do not remove — it is what makes the pre-mount
  // sample deterministic rather than a race with a cached, instant mount.
  let releaseWasm!: () => void;
  const held = new Promise<void>((resolve) => (releaseWasm = resolve));
  await page.route(WASM, async (route) => {
    await held;
    await route.continue();
  });

  try {
    await page.goto(`${BASE_URL}${probe.url}`, {
      waitUntil: "domcontentloaded",
    });
    // Settle font/text metrics before measuring so a late web-font load can't
    // masquerade as a shift. Return nothing — `FontFaceSet` is not serializable.
    await page.evaluate(async () => {
      await document.fonts.ready;
    });

    const targets = probe.targets(page);
    const before = await Promise.all(
      targets.map((t) => t.locator.boundingBox()),
    );

    releaseWasm();
    await waitForMount(page);
    if (probe.afterMount) await probe.afterMount(page);

    const after = await Promise.all(
      targets.map((t) => t.locator.boundingBox()),
    );

    targets.forEach((t, i) => {
      const b = before[i];
      const a = after[i];
      expect(b, `${t.name} missing at first paint`).not.toBeNull();
      expect(a, `${t.name} missing after mount`).not.toBeNull();
      // toBeLessThanOrEqual(0) is exact equality for the default; the same path
      // serves a loosened tolerance with no separate branch.
      expect(Math.abs(a!.x - b!.x), `${t.name} x shift`).toBeLessThanOrEqual(
        tol,
      );
      expect(Math.abs(a!.y - b!.y), `${t.name} y shift`).toBeLessThanOrEqual(
        tol,
      );
    });
  } finally {
    await page.unroute(WASM);
  }
}
