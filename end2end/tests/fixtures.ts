/**
 * E2E composed fixture surface.
 *
 * Owns the explicit registration order, composed `test`/`expect` exports, and
 * compatibility re-exports. Performance/OTel behavior lives in
 * `performance.ts`, timeout policy in `timeout-policy.ts`, and identity/page
 * provisioning in `provisioning.ts`.
 */

import { expect, test as base } from "@playwright/test";
import {
  applyTestTraceparent,
  autoPerfSpanFixture,
  bootTimingFixture,
  browserTraceFixture,
  lifecycleStartFixture,
  testSpanIdFixture,
  tracedContextFixture,
  type NewTracedContext,
} from "./performance";
import {
  mailboxFixture,
  registeredPageFixture,
  userFixture,
  verifiedUserFixture,
  type Mailbox,
  type RegisteredPage,
  type TestUser,
} from "./provisioning";
import {
  autoDurationBudgetFixture,
  autoTestTimeoutFixture,
  firstNavFixture,
  setTestInfoAccessor,
} from "./timeout-policy";
import type { DocumentTiming } from "./capture-trace";
import type { CaptureSink } from "./capture-trace";

const test = base.extend<{
  _lifecycleStart: number;
  _autoTestTimeout: void;
  _autoDurationBudget: void;
  _autoPerfSpan: void;
  testSpanId: string;
  tracedContext: NewTracedContext;
  bootTiming: () => Promise<DocumentTiming | undefined>;
  browserTrace: () => CaptureSink | undefined;
  firstNav: number;
  registeredPage: RegisteredPage;
  user: TestUser;
  mailbox: Mailbox;
  verifiedUser: TestUser;
}>({
  // Registration order is load-bearing: auto fixtures set up in insertion order,
  // so `_lifecycleStart` must precede the timeout and telemetry fixtures.
  _lifecycleStart: lifecycleStartFixture,
  testSpanId: testSpanIdFixture,
  bootTiming: bootTimingFixture,
  browserTrace: browserTraceFixture,
  tracedContext: tracedContextFixture,
  _autoTestTimeout: autoTestTimeoutFixture,
  _autoDurationBudget: autoDurationBudgetFixture,
  firstNav: firstNavFixture,
  registeredPage: registeredPageFixture,
  user: userFixture,
  mailbox: mailboxFixture,
  verifiedUser: verifiedUserFixture,
  _autoPerfSpan: autoPerfSpanFixture,
});

/**
 * The project fixture surface without browser-backed performance setup.
 *
 * Tests retain timeout and lifecycle policy while deliberately overriding the
 * only automatic fixture that requests `page`. A test may still request a real
 * page explicitly when the contract requires one without automatic arming.
 */
const testWithoutAutoPerf = test.extend({
  _autoPerfSpan: async ({}, use) => {
    await use();
  },
});
setTestInfoAccessor(() => test.info());

export { expect, test, testWithoutAutoPerf };
export {
  applyTestTraceparent,
  browserDiagnosticSpanProjectionFor,
  navigationBridgeFieldsFrom,
  navigationSummariesFrom,
  navigationTopTelemetryFrom,
  stylesheetModuleDiagnosticsFrom,
  tracedContextCapture,
} from "./performance";
export type {
  NavigationBridgeFields,
  NavigationSummary,
  NavigationTopTelemetry,
  NewTracedContext,
  StylesheetModuleDiagnostics,
} from "./performance";
export type { Mailbox, RegisteredPage, TestUser } from "./provisioning";
export {
  DEFAULT_TEST_BUDGET_MS,
  setTestBudget,
  slowBrowserFirstNavigationTimeoutMs,
  slowBrowserTimeoutMs,
} from "./timeout-policy";
