import type { Page } from "@playwright/test";

export type ActionRecord = {
  testKey: string;
  name: string;
  startedMs: number;
  endedMs: number;
  durationMs: number;
  ok: boolean;
  pageUrl?: string;
  error?: string;
};

let currentTestKey: string | null = null;
const actionRecords: ActionRecord[] = [];

export function setCurrentActionTestKey(testKey: string | null): void {
  currentTestKey = testKey;
}

export async function withTimedAction<T>(
  page: Page,
  name: string,
  action: () => Promise<T>,
): Promise<T> {
  const testKey = currentTestKey;
  const startedMs = Date.now();
  try {
    const result = await action();
    if (testKey !== null) {
      const endedMs = Date.now();
      actionRecords.push({
        testKey,
        name,
        startedMs,
        endedMs,
        durationMs: endedMs - startedMs,
        ok: true,
        pageUrl: page.url(),
      });
    }
    return result;
  } catch (error) {
    if (testKey !== null) {
      const endedMs = Date.now();
      actionRecords.push({
        testKey,
        name,
        startedMs,
        endedMs,
        durationMs: endedMs - startedMs,
        ok: false,
        pageUrl: page.url(),
        error: error instanceof Error ? error.message : String(error),
      });
    }
    throw error;
  }
}

export function drainActionsForTest(testKey: string): ActionRecord[] {
  const mine: ActionRecord[] = [];
  let writeIndex = 0;
  for (let readIndex = 0; readIndex < actionRecords.length; readIndex += 1) {
    const record = actionRecords[readIndex];
    if (record.testKey === testKey) {
      mine.push(record);
      continue;
    }
    actionRecords[writeIndex] = record;
    writeIndex += 1;
  }
  actionRecords.length = writeIndex;
  return mine;
}
