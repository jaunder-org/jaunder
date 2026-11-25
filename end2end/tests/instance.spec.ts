import { nonBrowserTest as test, expect } from "./fixtures";
import { bindWorkerInstance, parseInstances } from "./instance";

const instances = JSON.stringify([
  {
    baseUrl: "http://localhost:4100",
    db: "sqlite:/run/a/jaunder.db",
    storagePath: "/run/a",
    captureDir: "/run/a/capture",
  },
  { baseUrl: "http://localhost:4200" },
]);

test("the runner process binds no instance and writes no environment", () => {
  const env = { JAUNDER_E2E_INSTANCES: instances, JAUNDER_DB: "ambient" };
  expect(bindWorkerInstance(env)).toBeUndefined();
  expect(env).toEqual({
    JAUNDER_E2E_INSTANCES: instances,
    JAUNDER_DB: "ambient",
  });
});

test("a worker binds its own entry and replaces the instance variables", () => {
  const first = {
    JAUNDER_E2E_INSTANCES: instances,
    TEST_PARALLEL_INDEX: "0",
    JAUNDER_DB: "ambient",
  };
  expect(bindWorkerInstance(first)).toBe("http://localhost:4100");
  expect(first).toEqual({
    JAUNDER_E2E_INSTANCES: instances,
    TEST_PARALLEL_INDEX: "0",
    JAUNDER_DB: "sqlite:/run/a/jaunder.db",
    JAUNDER_STORAGE_PATH: "/run/a",
    JAUNDER_CAPTURE_DIR: "/run/a/capture",
  });

  const second = {
    JAUNDER_E2E_INSTANCES: instances,
    TEST_PARALLEL_INDEX: "1",
    JAUNDER_DB: "ambient",
    JAUNDER_CAPTURE_DIR: "ambient",
  };
  expect(bindWorkerInstance(second)).toBe("http://localhost:4200");
  expect(second).toEqual({
    JAUNDER_E2E_INSTANCES: instances,
    TEST_PARALLEL_INDEX: "1",
  });
});

test("a worker without its own instance fails instead of sharing one", () => {
  expect(() => bindWorkerInstance({ TEST_PARALLEL_INDEX: "0" })).toThrow(
    "JAUNDER_E2E_INSTANCES is not set",
  );
  expect(() =>
    bindWorkerInstance({
      JAUNDER_E2E_INSTANCES: instances,
      TEST_PARALLEL_INDEX: "2",
    }),
  ).toThrow("none for parallel index 2");
  expect(() => parseInstances("[]")).toThrow("non-empty JSON array");
  expect(() => parseInstances('[{"db":"sqlite:x"}]')).toThrow(
    "must have a string baseUrl",
  );
});
