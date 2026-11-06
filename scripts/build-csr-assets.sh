#!/usr/bin/env bash
# Build the shared CSR runtime assets from wasm-bindgen's compiled WASM.
# Callers own compilation/profile selection, Vite project setup, and stylesheet
# inventory production. Arguments: wasm input, Vite project, bindgen input,
# system-artifact inventory directory, and output directory. Diagnostic
# coverage may pass a sixth path to retain the pre-optimizer bindgen WASM.
set -euo pipefail

wasm_input=$1
project=$2
input=$3
system_artifacts=$4
dist=$5
bindgen_snapshot=${6:-}

mkdir -p "$input"
cp "$project/csr/index.html" "$input/index.html"
cp "$project/csr/entry.js" "$input/entry.js"
wasm-bindgen --target web --out-dir "$input" "$wasm_input"
if [[ -n "$bindgen_snapshot" ]]; then
  cp "$input/csr_bg.wasm" "$bindgen_snapshot"
fi
wasm-opt -Oz \
  --enable-bulk-memory \
  --enable-multivalue \
  --enable-mutable-globals \
  --enable-nontrapping-float-to-int \
  --enable-reference-types \
  --enable-sign-ext \
  "$input/csr_bg.wasm" -o "$input/csr_bg.opt.wasm"
mv "$input/csr_bg.opt.wasm" "$input/csr_bg.wasm"

(
  cd "$project"
  CSR_VITE_INPUT_DIR="$input" \
    CSR_VITE_OUT_DIR="$dist" \
    CSR_VITE_PUBLIC_DIR="${CSR_VITE_PUBLIC_DIR:?}" \
    CSR_SYSTEM_ARTIFACTS_DIR="$system_artifacts" \
    ./node_modules/.bin/vite build --config vite.config.js --configLoader native
)
cp -a "$system_artifacts" "$dist/system-artifacts"
