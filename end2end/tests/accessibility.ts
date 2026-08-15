import AxeBuilder from "@axe-core/playwright";
import { expect, type Page } from "@playwright/test";
import type { Result } from "axe-core";

const WCAG_TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

function formatViolations(violations: Result[]): string {
  const lines = ["Accessibility violations:"];

  for (const violation of [...violations].sort((left, right) =>
    left.id.localeCompare(right.id),
  )) {
    lines.push(
      `- ${violation.id} (impact: ${violation.impact ?? "null"})`,
      `  ${violation.help}`,
      `  ${violation.helpUrl}`,
    );

    for (const node of [...violation.nodes].sort((left, right) =>
      JSON.stringify(left.target).localeCompare(JSON.stringify(right.target)),
    )) {
      lines.push(`  - target: ${JSON.stringify(node.target)}`);
    }
  }

  return lines.join("\n");
}

/** Assert the complete mounted document has no machine-checkable WCAG 2.2 A/AA violations. */
export async function expectAccessible(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page }).withTags(WCAG_TAGS).analyze();

  expect(results.violations, formatViolations(results.violations)).toEqual([]);
}
