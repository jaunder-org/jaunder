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

// #454: the mode <select> is generated from `BackupMode::VARIANTS` (option text = label,
// value = the snake_case wire token), with the persisted mode pre-selected — no hardcoded
// options, so a new enum variant surfaces automatically.
test("backup mode select is generated from the enum variants", async ({
  page,
}) => {
  await login(page, "testoperator", "testpassword123");
  await goto(page, "/admin/backups");

  await waitForSelector(page, 'select[name="mode"]');
  const modeSelect = page.locator('select[name="mode"]');

  // Options come from the enum, in variant order, with human labels and wire-token values.
  await expect(modeSelect.locator("option")).toHaveText([
    "Directory",
    "Archive",
  ]);
  await expect(modeSelect.locator('option[value="directory"]')).toHaveCount(1);
  await expect(modeSelect.locator('option[value="archive"]')).toHaveCount(1);
  // The persisted default (Directory) is pre-selected.
  await expect(modeSelect).toHaveValue("directory");
});

// #455: the retention field is client-validated (ValidatedInput<RetentionCount>, ADR-0065) —
// RetentionCount enforces a min-1 invariant, so 0 (which would let pruning remove every backup)
// is rejected in the browser and disables Save before the request is sent.
test("backup retention field gates submit until a count of at least 1 is entered", async ({
  page,
}) => {
  await login(page, "testoperator", "testpassword123");
  await goto(page, "/admin/backups");

  await waitForSelector(page, 'input[name="retention_count"]');
  const retentionInput = page.locator('input[name="retention_count"]');

  // Prefilled with the valid default (7): submit starts enabled, no error shown.
  await expect(page.locator(SEL.submit)).toBeEnabled();
  await expect(page.locator(SEL.error)).not.toBeVisible();

  // 0 is not a valid retention count (min 1); once touched (blur) it shows the inline error
  // and disables submit.
  await retentionInput.fill("0");
  await retentionInput.blur();
  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();

  // A valid count clears the error and re-enables submit.
  await retentionInput.fill("3");
  await expect(page.locator(SEL.error)).not.toBeVisible();
  await expect(page.locator(SEL.submit)).toBeEnabled();
});
