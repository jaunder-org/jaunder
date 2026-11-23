/**
 * Worker-scoped E2E server instances (#2076).
 *
 * The harness starts one Jaunder instance per Playwright worker and describes
 * them in `JAUNDER_E2E_INSTANCES`: a JSON array indexed by Playwright's
 * `parallelIndex`, each entry `{ baseUrl, db?, storagePath?, captureDir? }`.
 * A worker binds to its own entry when this module loads, before any spec or
 * helper reads the environment. It overwrites `JAUNDER_DB`,
 * `JAUNDER_STORAGE_PATH` and `JAUNDER_CAPTURE_DIR` (deleting any the entry
 * omits), so child tools such as seeding, capture-path and mail keep inheriting
 * them unchanged.
 *
 * The runner process (spec loading, `--list`, census, reporters) has no
 * `TEST_PARALLEL_INDEX`: it binds nothing and writes no environment, and its
 * `BASE_URL` throws when used. This is the only module that may read instance
 * variables at load; everything else reads them inside a test.
 *
 * Fixtures and helpers import this module first.
 */

export type E2eInstance = {
  baseUrl: string;
  db?: string;
  storagePath?: string;
  captureDir?: string;
};

const INSTANCES_ENV = "JAUNDER_E2E_INSTANCES";
const instanceEnv = {
  db: "JAUNDER_DB",
  storagePath: "JAUNDER_STORAGE_PATH",
  captureDir: "JAUNDER_CAPTURE_DIR",
} as const;

/** Parse and validate the harness's instance map. */
export function parseInstances(raw: string): E2eInstance[] {
  const value: unknown = JSON.parse(raw);
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error(`${INSTANCES_ENV} must be a non-empty JSON array`);
  }
  return value.map((entry: unknown, index) => {
    const fields =
      typeof entry === "object" && entry !== null
        ? (entry as Record<string, unknown>)
        : undefined;
    if (
      fields === undefined ||
      typeof fields.baseUrl !== "string" ||
      Object.keys(instanceEnv).some(
        (key) => fields[key] !== undefined && typeof fields[key] !== "string",
      )
    ) {
      throw new Error(
        `${INSTANCES_ENV}[${index}] must have a string baseUrl and optional string db, storagePath and captureDir`,
      );
    }
    return fields as E2eInstance;
  });
}

/**
 * Bind a Playwright worker's environment to its instance and return the
 * instance's base URL. Returns `undefined`, touching nothing, outside a worker.
 */
export function bindWorkerInstance(env: NodeJS.ProcessEnv): string | undefined {
  const index = env.TEST_PARALLEL_INDEX;
  if (index === undefined) return undefined;
  const raw = env[INSTANCES_ENV];
  if (raw === undefined) {
    throw new Error(
      `${INSTANCES_ENV} is not set; every Playwright invocation must describe its server instances`,
    );
  }
  const instances = parseInstances(raw);
  const instance = /^\d+$/.test(index) ? instances[Number(index)] : undefined;
  if (instance === undefined) {
    throw new Error(
      `${INSTANCES_ENV} has ${instances.length} entries; none for parallel index ${index}`,
    );
  }
  for (const [key, name] of Object.entries(instanceEnv)) {
    const value = instance[key as keyof typeof instanceEnv];
    if (value === undefined) delete env[name];
    else env[name] = value;
  }
  return instance.baseUrl;
}

// Any coercion (template literal, concatenation, URL construction) throws, so a
// runner-side use fails loudly instead of silently reaching some instance.
const runnerBaseUrl = {
  [Symbol.toPrimitive](): never {
    throw new Error(
      "BASE_URL is worker-scoped: the Playwright runner process has no E2E instance",
    );
  },
} as unknown as string;

/** This worker's server origin; throws if used in the runner process. */
export const BASE_URL: string =
  bindWorkerInstance(process.env) ?? runnerBaseUrl;
