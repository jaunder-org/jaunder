/**
 * In-app movement through the Settings and Account hubs (#1733).
 *
 * Configuration and account pages are reached from the page-header corner
 * controls, not the sidebar: control → hub page → row. Both hops are router
 * pushes behind `navigateInApp` readiness barriers, exactly as a user moves.
 */
import type { Page } from "@playwright/test";
import { click } from "./helpers";
import { navigateInApp, type InAppDestination } from "./navigate";
import { SEL } from "./selectors";

/** A configuration hub, by its page path. */
export type HubPath = "/settings" | "/account";

/** The Account hub's Sign out link; it exists nowhere else in the app chrome. */
export const SIGN_OUT_LINK = 'a.j-btn[href="/logout"]';

/** Move from the current page to a hub through its corner control. `ready`
 *  must be unique to the hub page (e.g. one of its rows). */
export async function openHub(
  page: Page,
  hub: HubPath,
  ready: string,
): Promise<void> {
  await navigateInApp(page, () => click(page, SEL.hubControl(hub)), {
    url: hub,
    ready,
  });
}

/** Reach `destination` the way a user does: the hub's corner control, then
 *  the hub row linking to `destination.url`. */
export async function openHubDestination(
  page: Page,
  hub: HubPath,
  destination: InAppDestination,
): Promise<void> {
  const row = SEL.hubRow(destination.url);
  await openHub(page, hub, row);
  await navigateInApp(page, () => click(page, row), destination);
}

/** Sign out through the UI: Account control → Account hub → Sign out. Callers
 *  wait for their own signed-out evidence. */
export async function signOutViaAccount(page: Page): Promise<void> {
  await openHub(page, "/account", SIGN_OUT_LINK);
  await click(page, SIGN_OUT_LINK);
}
