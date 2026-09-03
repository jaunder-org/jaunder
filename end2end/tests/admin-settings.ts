/**
 * Internal: re-entry leaves and returns through a second admin route so remounting
 * recreates the settings page Resource and refetches server state. Per ADR-0111,
 * both hops stay in-app behind readiness barriers; reloading or collapsing this
 * to one hop would not exercise the lifecycle under test.
 */
import type { Page } from "@playwright/test";
import { navigateInApp } from "./navigate";

const ADMIN_SETTINGS_TARGETS = {
  site: {
    link: 'a.j-nav-item[href="/admin/site"]',
    url: "/admin/site",
    ready: 'input[name="title"]',
    intermediate: "backups",
  },
  backups: {
    link: 'a.j-nav-item[href="/admin/backups"]',
    url: "/admin/backups",
    ready: 'input[name="destination_path"]',
    intermediate: "site",
  },
} as const;

type AdminSettingsTarget = keyof typeof ADMIN_SETTINGS_TARGETS;
type AdminSettingsDestination =
  (typeof ADMIN_SETTINGS_TARGETS)[AdminSettingsTarget];

async function navigateToAdminSettings(
  page: Page,
  destination: AdminSettingsDestination,
): Promise<void> {
  await navigateInApp(page, () => page.click(destination.link), destination);
}

/** Remount a settings page through another admin settings route. */
export async function reenterAdminSettings(
  page: Page,
  target: AdminSettingsTarget,
): Promise<void> {
  const destination = ADMIN_SETTINGS_TARGETS[target];
  const intermediate = ADMIN_SETTINGS_TARGETS[destination.intermediate];

  await navigateToAdminSettings(page, intermediate);
  await navigateToAdminSettings(page, destination);
}
