/**
 * E2E identity and page-provisioning infrastructure.
 *
 * Owns per-test users, recipient-scoped mailbox cursors, seeded authentication,
 * and the one-shot registered page. Seeding never drives the UI, and a
 * registered page accepts one document load; `fixtures.ts` explicitly composes
 * these fixtures into the suite's test surface.
 */

import type { Browser, BrowserContext, Page } from "@playwright/test";
import { readEmailLines, type CapturedEmail } from "./mail";
import {
  generateUsername,
  goto,
  setAndVerifyEmail,
  TEST_PASSWORD,
} from "./helpers";
import { pollUntil } from "./polling";
import { applySeededSession, seedUserViaTool } from "./seed";
import type { NewTracedContext } from "./performance";

export type TestUser = {
  username: string;
  password: string;
  email: string;
  token: string;
  setCookie: string;
  marker: string;
  markerKey: string;
  isOperator: boolean;
};

export type Mailbox = {
  waitForNewEmail(timeoutMs?: number): Promise<CapturedEmail>;
};

export type RegisteredPage = (entry: string) => Promise<Page>;

type Use<T> = (value: T) => Promise<void>;

export const registeredPageFixture = async (
  { page, firstNav }: { page: Page; firstNav: number },
  use: Use<RegisteredPage>,
): Promise<void> => {
  const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
  await applySeededSession(page.context(), record);
  let bootedAt: string | undefined;
  await use(async (entry: string): Promise<Page> => {
    if (bootedAt !== undefined) {
      throw new Error(
        `registeredPage() called twice: already booted at ${bootedAt}. ` +
          `A page boots once (#867); move within the app with navigateInApp, ` +
          `or declare a second load with allowSecondBoot.`,
      );
    }
    bootedAt = entry;
    await goto(page, entry, { timeout: firstNav });
    return page;
  });
};

export const userFixture = async ({}, use: Use<TestUser>): Promise<void> => {
  const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
  await use({
    username: record.username,
    password: TEST_PASSWORD,
    email: `${record.username}@example.com`,
    token: record.token,
    setCookie: record.setCookie,
    marker: record.marker,
    markerKey: record.markerKey,
    isOperator: record.isOperator,
  });
};

export const mailboxFixture = async (
  { user }: { user: TestUser },
  use: Use<Mailbox>,
): Promise<void> => {
  const matching = () =>
    readEmailLines()
      .map((line) => JSON.parse(line) as CapturedEmail)
      .filter((mail) => mail.to.includes(user.email));
  let cursor = matching().length;
  const waitForNewEmail = async (timeoutMs = 5_000): Promise<CapturedEmail> =>
    pollUntil(
      "wait.mail",
      () => {
        const mails = matching();
        if (mails.length <= cursor) return undefined;
        const next = mails[cursor];
        cursor += 1;
        return next;
      },
      {
        intervalMs: 100,
        timeoutMs,
        describe: `an email to ${user.email}`,
      },
    );
  await use({ waitForNewEmail });
};

export const verifiedUserFixture = async (
  {
    tracedContext,
    user,
    mailbox,
  }: {
    tracedContext: NewTracedContext;
    user: TestUser;
    mailbox: Mailbox;
  },
  use: Use<TestUser>,
): Promise<void> => {
  const context = await tracedContext();
  const page = await context.newPage();
  await applySeededSession(context, user);
  await setAndVerifyEmail(page, user.email, mailbox);
  await context.close();
  await use(user);
};
