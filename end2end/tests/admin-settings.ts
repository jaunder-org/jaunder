/**
 * Internal: re-entry leaves an admin settings page for the Settings hub and
 * returns through its row, so remounting recreates the settings page Resource
 * and refetches server state. Per ADR-0111, both hops stay in-app behind
 * readiness barriers; reloading or collapsing this to one hop would not
 * exercise the lifecycle under test.
 */
import type { Page } from "@playwright/test";
import { openHubDestination } from "./hubs";

const ADMIN_SETTINGS_TARGETS = {
  site: { url: "/admin/site", ready: 'input[name="title"]' },
  backups: { url: "/admin/backups", ready: 'input[name="destination_path"]' },
  smtp: { url: "/admin/smtp", ready: 'input[name="host"]' },
} as const;

type AdminSettingsTarget = keyof typeof ADMIN_SETTINGS_TARGETS;

/** Remount a settings page through the Settings hub. */
export async function reenterAdminSettings(
  page: Page,
  target: AdminSettingsTarget,
): Promise<void> {
  await openHubDestination(page, "/settings", ADMIN_SETTINGS_TARGETS[target]);
}
