import { expect, nonBrowserTest } from "./fixtures";
import { BootBudgetAccounting } from "./bootBudget";

const ENTRY = "http://127.0.0.1:3000/";
const LOGIN = "http://127.0.0.1:3000/login";

nonBrowserTest(
  "an exact allowance covers one further document load only",
  () => {
    const budget = new BootBudgetAccounting();
    budget.recordDocumentLoad(ENTRY);
    budget.allowSecondBoot("one extra load");
    budget.recordDocumentLoad(LOGIN);
    budget.recordDocumentLoad("http://127.0.0.1:3000/register");

    expect(budget.bootCount()).toBe(3);
    expect(budget.pendingReasons()).toEqual([]);
    expect(() => budget.throwIfViolated()).toThrow(
      /second document load[\s\S]*\/register[\s\S]*allowSecondBoot/,
    );
    expect(budget.takeFailures()).toEqual([]);
  },
);

nonBrowserTest(
  "a scoped allowance matches its pathname before an exact allowance",
  () => {
    const budget = new BootBudgetAccounting();
    budget.recordDocumentLoad(ENTRY);
    budget.allowSecondBoot("the load that always happens");
    budget.allowEngineDependentBoot("/register", "the engine-dependent load");
    budget.recordDocumentLoad("http://other.test/register?run=salted");
    budget.recordDocumentLoad(LOGIN);

    expect(budget.pendingReasons()).toEqual([]);
    expect(budget.takeFailures()).toEqual([]);
  },
);

nonBrowserTest("a scoped allowance is inert for another pathname", () => {
  const budget = new BootBudgetAccounting();
  budget.recordDocumentLoad(ENTRY);
  budget.allowEngineDependentBoot(
    "/register",
    "the load this engine did not produce",
  );
  budget.recordDocumentLoad(LOGIN);

  expect(budget.takeFailures()).toEqual([
    expect.stringContaining("undeclared second load"),
  ]);
});

nonBrowserTest("both allowance forms require a non-empty reason", () => {
  const budget = new BootBudgetAccounting();

  expect(() => budget.allowSecondBoot("   ")).toThrow(
    "allowSecondBoot needs a non-empty reason",
  );
  expect(() => budget.allowEngineDependentBoot("/register", "   ")).toThrow(
    "allowEngineDependentBoot needs a non-empty reason",
  );
});

nonBrowserTest(
  "failure collection reports route-bearing orphan reasons and clears them",
  () => {
    const budget = new BootBudgetAccounting();
    budget.recordDocumentLoad(ENTRY);
    budget.allowSecondBoot("a second load that never happens");

    expect(budget.takeFailures()).toEqual([
      `${ENTRY}: a second load that never happens`,
    ]);
    expect(budget.pendingReasons()).toEqual([]);
    expect(budget.takeFailures()).toEqual([]);
  },
);

nonBrowserTest(
  "an unconsumed engine-dependent allowance is not an orphan",
  () => {
    const budget = new BootBudgetAccounting();
    budget.recordDocumentLoad(ENTRY);
    budget.allowEngineDependentBoot(
      "/register",
      "a load only some engines produce",
    );

    expect(budget.takeFailures()).toEqual([]);
  },
);
