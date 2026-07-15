import { test, expect } from "./fixtures";
import { goto, login, waitForSelector } from "./helpers";
import { SEL } from "./selectors";

// #453: the schedule field is client-validated (ValidatedInput<BackupSchedule>, ADR-0065) —
// submit is gated disable-until-valid and a malformed cron shows an inline error, so a bad
// value never reaches the typed `#[server]` arg. The field is prefilled with the persisted
// schedule (a valid default), so — unlike the empty email field — it starts valid.
test("backup schedule field gates submit until a valid cron is entered", async ({
  page,
}) => {
  await login(page, "testoperator", "testpassword123");
  await goto(page, "/admin/backups");

  await waitForSelector(page, 'input[name="schedule"]');
  const scheduleInput = page.locator('input[name="schedule"]');

  // Prefilled with the valid default ("0 0 0 * * *"): submit starts enabled, no error shown.
  await expect(page.locator(SEL.submit)).toBeEnabled();
  await expect(page.locator(SEL.error)).not.toBeVisible();

  // A malformed cron, once the field is touched (blur), shows the inline client-local error
  // and disables submit.
  await scheduleInput.fill("not a cron");
  await scheduleInput.blur();
  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();

  // A valid six-field cron clears the error and re-enables submit.
  await scheduleInput.fill("0 30 2 * * *");
  await expect(page.locator(SEL.error)).not.toBeVisible();
  await expect(page.locator(SEL.submit)).toBeEnabled();
});
