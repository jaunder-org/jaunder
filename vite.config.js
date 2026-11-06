import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { defineConfig } from "vite";
import { compression, defineAlgorithm } from "vite-plugin-compression2";

const projectRoot = fileURLToPath(new URL(".", import.meta.url));
const inputRoot = resolve(
  process.env.CSR_VITE_INPUT_DIR ?? resolve(projectRoot, "csr"),
);
const systemArtifacts = process.env.CSR_SYSTEM_ARTIFACTS_DIR;

// Resolve the classic fetch URL from Vite's completed build bundle and the
// separately generated stylesheet digest before Vite emits index.html.
function csrHtml() {
  return {
    name: "jaunder-csr-html",
    transformIndexHtml(html, context) {
      if (!context.bundle) return html;
      const wasm = Object.values(context.bundle).find(
        (output) => output.type === "asset" && output.name === "csr_bg.wasm",
      );
      if (!wasm) throw new Error("Vite did not emit csr_bg.wasm");
      if (!systemArtifacts)
        throw new Error("CSR_SYSTEM_ARTIFACTS_DIR is required");
      const inventory = readFileSync(
        resolve(systemArtifacts, "inventory.txt"),
        "utf8",
      );
      const application = inventory
        .split(/\r?\n/)
        .find((line) => line.startsWith("application\t"));
      const digest = application?.match(/\tdigest=([0-9a-f]{64})(?:\t|$)/)?.[1];
      if (!digest)
        throw new Error(
          "system artifact inventory has no application stylesheet digest",
        );
      return html
        .replace("{{WASM_URL}}", `/${wasm.fileName}`)
        .replace("{{APPLICATION_STYLESHEET_URL}}", `/theme/${digest}`);
    },
  };
}

export default defineConfig({
  root: inputRoot,
  // The application stylesheet is a separate system artifact; other static
  // project assets remain copied from the existing public/ tree.
  publicDir: resolve(
    process.env.CSR_VITE_PUBLIC_DIR ?? resolve(projectRoot, "public"),
  ),
  base: "/",
  define: {
    "import.meta.env.VITE_WASM_EXPERIMENT_ARM": JSON.stringify(
      process.env.CSR_WASM_EXPERIMENT_ARM || null,
    ),
  },
  plugins: [
    csrHtml(),
    compression({
      include: /\.(js|wasm)$/,
      algorithms: [
        defineAlgorithm("gzip", { level: 9, mtime: 0 }),
        defineAlgorithm("brotliCompress", { params: { 1: 11, 2: 22 } }),
      ],
      threshold: 0,
      skipIfLargerOrEqual: false,
      deleteOriginFile: false,
    }),
  ],
  build: {
    outDir: resolve(
      process.env.CSR_VITE_OUT_DIR ?? resolve(projectRoot, "target/site"),
    ),
    emptyOutDir: true,
    manifest: true,
    assetsInlineLimit: 0,
    rolldownOptions: {
      // HTML scripts are side-effect imports. Make the measured initializer and
      // wasm-bindgen diagnostics a real JS entry, not an HTML-only re-export.
      input: [resolve(inputRoot, "index.html"), resolve(inputRoot, "entry.js")],
      preserveEntrySignatures: "strict",
      output: {
        entryFileNames: "assets/[name]-[hash].js",
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
      },
    },
  },
});
