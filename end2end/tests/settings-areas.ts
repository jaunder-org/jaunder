/**
 * In-app movement through the Settings and Account pages (#1733).
 *
 * Configuration and account pages are reached from the page-header corner
 * controls, not the sidebar: control → Settings or Account page → row. Both
 * hops are router pushes behind `navigateInApp` readiness barriers, exactly as
 * a user moves.
 */
import type { Page } from "@playwright/test";
import { click } from "./helpers";
import { navigateInApp, type InAppDestination } from "./navigate";
import { SEL } from "./selectors";

/** A settings area (the Settings or Account page), by its page path. */
export type AreaPath = "/settings" | "/account";

/** Move from the current page to a settings area through its corner control.
 *  `ready` must be unique to that page (e.g. one of its rows). */
export async function openArea(
  page: Page,
  area: AreaPath,
  ready: string,
): Promise<void> {
  await navigateInApp(page, () => click(page, SEL.areaControl(area)), {
    url: area,
    ready,
  });
}

/** Reach `destination` the way a user does: the area's corner control, then
 *  the area row linking to `destination.url`. */
export async function openAreaDestination(
  page: Page,
  area: AreaPath,
  destination: InAppDestination,
): Promise<void> {
  const row = SEL.areaRow(destination.url);
  await openArea(page, area, row);
  await navigateInApp(page, () => click(page, row), destination);
}

/** Sign out through the UI: Account control → Account page → Sign out.
 *  Callers wait for their own signed-out evidence. */
export async function signOutViaAccount(page: Page): Promise<void> {
  await openArea(page, "/account", SEL.signOutLink);
  await click(page, SEL.signOutLink);
}
