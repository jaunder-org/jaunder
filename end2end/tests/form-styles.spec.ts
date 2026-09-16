import type { Locator, Page } from "@playwright/test";
import { allowSecondBoot } from "./bootBudget";
import { test, expect } from "./fixtures";
import { goto, signInAs, waitForSelector } from "./helpers";
import { navigateInApp } from "./navigate";

type FormControl = {
  name: string;
  label: string;
  standardChrome: boolean;
};

type FormPresentation = {
  label: Record<string, string>;
  control?: Record<string, string>;
  error?: Record<string, string>;
  focusBorderColor?: string;
};

async function expectStandardCard(
  page: Page,
  heading: string,
  controls: FormControl[],
  options: { stacked?: boolean; focus?: boolean } = {},
): Promise<FormPresentation> {
  const card = page.locator(".j-card").filter({
    has: page.getByRole("heading", { name: heading, exact: true }),
  });
  await expect(card).toHaveCount(1);

  const body = card.locator(".j-form-body");
  const actions = card.locator(".j-form-actions");
  await expect(body).toHaveCount(1);
  await expect(actions).toHaveCount(1);

  const fields: Locator[] = [];
  const labels: Locator[] = [];
  const styledControls: Locator[] = [];
  for (const { name, label: expectedLabel, standardChrome } of controls) {
    const control = card.locator(`[name="${name}"]`);
    const field = control.locator("..");
    const label = field.locator(".j-form-label");
    await expect(control).toHaveCount(1);
    await expect(field).toHaveCount(1);
    await expect(field).toHaveClass(/(?:^|\s)j-form-field(?:\s|$)/);
    await expect(label).toHaveCount(1);
    await expect(label).toHaveText(expectedLabel);
    fields.push(field);
    labels.push(label);

    if (standardChrome) {
      await expect(control).toHaveClass(/(?:^|\s)j-form-input(?:\s|$)/);
      styledControls.push(control);
    }
  }

  const layout = await card.evaluate((element) => {
    const computedBody = getComputedStyle(
      element.querySelector<HTMLElement>(".j-form-body")!,
    );
    const computedActions = getComputedStyle(
      element.querySelector<HTMLElement>(".j-form-actions")!,
    );
    return {
      bodyPadding: Number.parseFloat(computedBody.paddingTop),
      actionPadding: Number.parseFloat(computedActions.paddingTop),
      actionDivider: Number.parseFloat(computedActions.borderTopWidth),
      documentFits:
        document.documentElement.scrollWidth <=
        document.documentElement.clientWidth,
    };
  });
  expect(layout.bodyPadding).toBeGreaterThan(0);
  expect(layout.actionPadding).toBeGreaterThan(0);
  expect(layout.actionDivider).toBeGreaterThan(0);
  expect(layout.documentFits).toBe(true);

  if (options.stacked !== false) {
    const fieldBoxes = await Promise.all(
      fields.map((field) => field.boundingBox()),
    );
    for (let index = 1; index < fieldBoxes.length; index += 1) {
      const previous = fieldBoxes[index - 1];
      const current = fieldBoxes[index];
      expect(previous).not.toBeNull();
      expect(current).not.toBeNull();
      expect(previous!.y + previous!.height).toBeLessThanOrEqual(current!.y);
    }
  }

  const labelStyles = await Promise.all(
    labels.map((label) =>
      label.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          fontFamily: style.fontFamily,
          fontSize: style.fontSize,
          color: style.color,
        };
      }),
    ),
  );
  for (const style of labelStyles.slice(1)) {
    expect(style).toEqual(labelStyles[0]);
  }

  if (styledControls.length > 0) {
    await styledControls[0].blur();
  }
  const controlStyles = await Promise.all(
    styledControls.map((control) =>
      control.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          minHeight: style.minHeight,
          padding: style.padding,
          border: style.border,
          borderRadius: style.borderRadius,
          backgroundColor: style.backgroundColor,
          color: style.color,
        };
      }),
    ),
  );
  for (const style of controlStyles.slice(1)) {
    expect(style).toEqual(controlStyles[0]);
  }

  let focusBorderColor: string | undefined;
  if (options.focus && styledControls.length > 0) {
    const target = styledControls[0];
    await target.blur();
    const restingBorderColor = await target.evaluate(
      (element) => getComputedStyle(element).borderColor,
    );
    await target.focus();
    focusBorderColor = await target.evaluate(
      (element) => getComputedStyle(element).borderColor,
    );
    expect(focusBorderColor).not.toBe(restingBorderColor);
  }

  return {
    label: labelStyles[0],
    control: controlStyles[0],
    focusBorderColor,
  };
}

function expectSamePresentation(
  actual: FormPresentation,
  expected: FormPresentation,
) {
  expect(actual.label).toEqual(expected.label);
  expect(actual.control).toEqual(expected.control);
  if (expected.error !== undefined) {
    expect(actual.error).toEqual(expected.error);
  }
  if (expected.focusBorderColor !== undefined) {
    expect(actual.focusBorderColor).toBe(expected.focusBorderColor);
  }
}

async function expectValidationError(
  page: Page,
  heading: string,
  controlName: string,
  invalidValue: string,
) {
  const card = page.locator(".j-card").filter({
    has: page.getByRole("heading", { name: heading, exact: true }),
  });
  const control = card.locator(`[name="${controlName}"]`);
  await control.fill(invalidValue);
  await control.blur();
  const error = card.locator(".error");
  await expect(error).toBeVisible();
  const style = await error.evaluate((element) => {
    const computed = getComputedStyle(element);
    return {
      padding: computed.padding,
      border: computed.border,
      borderRadius: computed.borderRadius,
      backgroundColor: computed.backgroundColor,
      color: computed.color,
    };
  });
  await control.fill("");
  return style;
}

async function expectSiteForms(page: Page) {
  await waitForSelector(page, 'input[name="title"]');
  await waitForSelector(page, 'input[name="uploads_enabled"]');
  const presentation = await expectStandardCard(page, "Site Settings", [
    { name: "title", label: "Site title", standardChrome: true },
    { name: "base_url", label: "Base URL", standardChrome: true },
  ]);
  await expectStandardCard(page, "Media Uploads", [
    {
      name: "uploads_enabled",
      label: "Enable new media uploads",
      standardChrome: false,
    },
  ]);
  return presentation;
}

async function expectSmtpForm(page: Page) {
  await waitForSelector(page, 'input[name="enabled"]');
  return expectStandardCard(page, "SMTP Relay", [
    { name: "enabled", label: "Enable SMTP relay", standardChrome: false },
    { name: "host", label: "Relay host", standardChrome: true },
    { name: "port", label: "Port", standardChrome: true },
    { name: "tls_mode", label: "TLS mode", standardChrome: true },
    { name: "sender", label: "Sender mailbox", standardChrome: true },
    {
      name: "authentication_enabled",
      label: "Use authentication",
      standardChrome: false,
    },
    { name: "username", label: "Username", standardChrome: true },
    { name: "password", label: "Password", standardChrome: true },
  ]);
}

async function expectBackupForm(page: Page, columns: 1 | 2) {
  await waitForSelector(page, 'input[name="destination_path"]');
  const presentation = await expectStandardCard(
    page,
    "Scheduled Backups",
    [
      {
        name: "destination_path",
        label: "Destination path",
        standardChrome: true,
      },
      { name: "schedule", label: "Schedule", standardChrome: true },
      {
        name: "retention_count",
        label: "Retention count",
        standardChrome: true,
      },
      { name: "mode", label: "Mode", standardChrome: true },
    ],
    { stacked: false },
  );

  const retention = await page
    .locator('input[name="retention_count"]')
    .boundingBox();
  const mode = await page.locator('select[name="mode"]').boundingBox();
  expect(retention).not.toBeNull();
  expect(mode).not.toBeNull();
  if (columns === 2) {
    expect(Math.abs(retention!.y - mode!.y)).toBeLessThanOrEqual(1);
  } else {
    expect(retention!.y + retention!.height).toBeLessThanOrEqual(mode!.y);
  }
  return presentation;
}

async function expectWebsubForm(page: Page) {
  await waitForSelector(page, 'input[name="hub_url"]');
  const presentation = await expectStandardCard(
    page,
    "WebSub hub",
    [{ name: "hub_url", label: "Hub URL", standardChrome: true }],
    { focus: true },
  );
  presentation.error = await expectValidationError(
    page,
    "WebSub hub",
    "hub_url",
    "not a URL",
  );
  return presentation;
}

async function expectAppPasswordForm(page: Page) {
  await waitForSelector(page, 'input[name="label"]');
  const presentation = await expectStandardCard(
    page,
    "App passwords",
    [{ name: "label", label: "Label", standardChrome: true }],
    { focus: true },
  );
  presentation.error = await expectValidationError(
    page,
    "App passwords",
    "label",
    " ",
  );
  return presentation;
}

test("operator settings forms share responsive field presentation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1080, height: 800 });
  await signInAs(page, "testoperator");
  await goto(page, "/admin/site");
  const desktopSite = await expectSiteForms(page);

  await navigateInApp(
    page,
    () => page.click('a.j-nav-item[href="/admin/smtp"]'),
    { url: "/admin/smtp", ready: 'input[name="enabled"]' },
  );
  expectSamePresentation(await expectSmtpForm(page), desktopSite);

  await page.setViewportSize({ width: 600, height: 800 });
  expectSamePresentation(await expectSmtpForm(page), desktopSite);

  await navigateInApp(
    page,
    () => page.click('a.j-nav-item[href="/admin/site"]'),
    { url: "/admin/site", ready: 'input[name="title"]' },
  );
  expectSamePresentation(await expectSiteForms(page), desktopSite);
});

test("specialized form layouts retain shared presentation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1080, height: 800 });
  await signInAs(page, "testoperator");
  await goto(page, "/admin/backups");
  const desktopBackup = await expectBackupForm(page, 2);

  await navigateInApp(
    page,
    () => page.click('a.j-nav-item[href="/admin/websub"]'),
    { url: "/admin/websub", ready: 'input[name="hub_url"]' },
  );
  const desktopWebsub = await expectWebsubForm(page);
  expectSamePresentation(desktopWebsub, desktopBackup);

  allowSecondBoot(
    page,
    "the Sessions form's cold rendering is part of the cross-form presentation proof",
  );
  await goto(page, "/sessions");
  const desktopAppPassword = await expectAppPasswordForm(page);
  expectSamePresentation(desktopAppPassword, desktopWebsub);

  await page.setViewportSize({ width: 600, height: 800 });
  expectSamePresentation(await expectAppPasswordForm(page), desktopAppPassword);

  await navigateInApp(
    page,
    () => page.click('a.j-nav-item[href="/admin/backups"]'),
    { url: "/admin/backups", ready: 'input[name="destination_path"]' },
  );
  expectSamePresentation(await expectBackupForm(page, 1), desktopBackup);

  await navigateInApp(
    page,
    () => page.click('a.j-nav-item[href="/admin/websub"]'),
    { url: "/admin/websub", ready: 'input[name="hub_url"]' },
  );
  expectSamePresentation(await expectWebsubForm(page), desktopWebsub);
});
