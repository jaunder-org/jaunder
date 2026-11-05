import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const [outputDirectory] = process.argv.slice(2);
if (!outputDirectory) throw new Error('usage: node scripts/finalize-csr-html.mjs <vite-output>');
const root = resolve(outputDirectory);
const manifest = JSON.parse(
  await readFile(resolve(root, '.vite/manifest.json'), 'utf8'),
);
const wasm = manifest['csr_bg.wasm']?.file;
const systemArtifacts = process.env.CSR_SYSTEM_ARTIFACTS_DIR;
if (!systemArtifacts) throw new Error('CSR_SYSTEM_ARTIFACTS_DIR is required');
const inventory = await readFile(resolve(systemArtifacts, 'inventory.txt'), 'utf8');
const applicationRecord = inventory
  .split(/\r?\n/)
  .find((line) => line.startsWith('application\t'));
const applicationDigest = applicationRecord?.match(/\tdigest=([0-9a-f]{64})(?:\t|$)/)?.[1];
const applicationStylesheet = applicationDigest
  ? `/theme/${applicationDigest}`
  : null;
if (!wasm || wasm.startsWith('/') || wasm.split('/').includes('..')) {
  throw new Error('Vite manifest has no safe csr_bg.wasm output path');
}
if (!applicationStylesheet) {
  throw new Error('system artifact inventory has no application stylesheet digest');
}
const indexPath = resolve(root, 'index.html');
let html = await readFile(indexPath, 'utf8');
for (const [marker, value] of [
  ['{{WASM_URL}}', `/${wasm}`],
  ['{{APPLICATION_STYLESHEET_URL}}', applicationStylesheet],
]) {
  if (html.split(marker).length !== 2) {
    throw new Error(`CSR shell must contain exactly one ${marker} marker`);
  }
  html = html.replace(marker, value);
}
if (/rel="preload"[^>]+\.wasm|href="[^"]+\.wasm"[^>]+rel="preload"/.test(html)) {
  throw new Error('CSR shell must not preload the WASM asset (ADR-0121)');
}
await writeFile(indexPath, html);
