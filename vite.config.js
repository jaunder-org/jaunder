import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { defineConfig } from "vite";
import { compression, defineAlgorithm } from "vite-plugin-compression2";

const projectRoot = fileURLToPath(new URL(".", import.meta.url));
const inputRoot = resolve(
  process.env.CSR_VITE_INPUT_DIR ?? resolve(projectRoot, "csr"),
);

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
