// Start from the classic shell's early WASM fetch and mark the actual instantiate
// path, API duration, and module shape for startup diagnostics and coverage.
import init from "./csr.js";
import wasmUrl from "./csr_bg.wasm?url";

// Coverage reads the instrumentable wasm-bindgen exports from this entry.
export * from "./csr.js";

const experimentArm = import.meta.env.VITE_WASM_EXPERIMENT_ARM;

const moduleShape = (module) => {
  if (!(module instanceof WebAssembly.Module)) return null;
  const imports = WebAssembly.Module.imports(module);
  const exports = WebAssembly.Module.exports(module);
  const countKind = (items, kind) =>
    items.filter((item) => item.kind === kind).length;
  return {
    imports: imports.length,
    importedFunctions: countKind(imports, "function"),
    importedTables: countKind(imports, "table"),
    importedMemories: countKind(imports, "memory"),
    exports: exports.length,
    exportedFunctions: countKind(exports, "function"),
    exportedTables: countKind(exports, "table"),
    exportedMemories: countKind(exports, "memory"),
    customSections: WebAssembly.Module.customSections(module, "jaunder.shape")
      .length,
  };
};

export async function initMeasured(moduleOrPath) {
  performance.mark("jaunder.wasm.init_start");
  let successfulPath = null;
  let apiMs = null;
  let shape = null;
  const originalStreaming = WebAssembly.instantiateStreaming;
  const originalInstantiate = WebAssembly.instantiate;
  const measure = (original, path) =>
    async function (...args) {
      const startedAt = performance.now();
      const result = await original.apply(this, args);
      successfulPath = path;
      apiMs = performance.now() - startedAt;
      shape = moduleShape(
        result?.module ??
          (result instanceof WebAssembly.Module ? result : null),
      );
      return result;
    };
  if (typeof originalStreaming === "function") {
    WebAssembly.instantiateStreaming = measure(originalStreaming, "streaming");
  }
  if (typeof originalInstantiate === "function") {
    WebAssembly.instantiate = measure(originalInstantiate, "buffered");
  }
  try {
    const result = await init(moduleOrPath);
    if (successfulPath !== null && apiMs !== null) {
      performance.mark("jaunder.wasm.init_done", {
        detail: {
          path: successfulPath,
          apiMs,
          experimentArm,
          moduleShape: shape,
        },
      });
    }
    return result;
  } finally {
    WebAssembly.instantiateStreaming = originalStreaming;
    WebAssembly.instantiate = originalInstantiate;
  }
}

performance.mark("jaunder.module.before_init");
const wasmInput = window.__jaunderWasmFetch ?? fetch(wasmUrl);
window.__jaunderWasmUrl = wasmUrl;
window.__jaunderWasmFetch ??= wasmInput;
initMeasured(wasmInput);
