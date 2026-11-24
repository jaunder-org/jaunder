#!/usr/bin/env node
// Reduce a full e2e OTLP capture to the committed server-fn-coverage fixture
// `otel-traces-seed.jsonl`, which sits next to this script (#681).
//
//     tools/host-proof/src/server_fn_coverage/testdata/reduce-otel-capture.mjs \
//       <otel-traces.jsonl> <out.jsonl>
//
// It lives beside the fixture it generates rather than under `scripts/`: the
// host-only `tools/host-proof` sources are outside the application and e2e
// source closures, so editing it cannot bust the e2e derivations' input hashes
// (spec AC16).
//
// The fixture is the evidence behind `docs/coverage/server-fns.json` and the
// allowlist, so spec AC11 — "every allowlist entry names a fn absent from the
// hit set of the committed capture" — is only checkable if a reader can confirm
// the reduction preserved that hit set. A 25 MB capture cannot be committed and
// an uncommitted reduction cannot be audited: an entry's absence would then be
// indistinguishable from the reduction having dropped the key that would have
// covered it. Hence this script.
//
// WHAT IT KEEPS, AND WHY
//
//   1. Every `e2e.test` span. Attribution resolves a hit to a test by walking
//      `parentSpanId` upward until it lands on one of these, so dropping any
//      would silently turn covered fns into orphans.
//   2. Every span that either extractor signal could possibly match:
//        - `uri` contains `/api/` — the superset of the URI signal, which
//          requires that prefix and then matches the declared endpoint;
//        - `code.module.name` starts with `web::`, or legacy `code.namespace`
//          does when the current attribute is absent — the superset of the
//          span-name signal, which only ever accepts a span whose module is a
//          `web` crate module (`extract.rs` strips that crate prefix before
//          comparing).
//      Supersets on purpose: see THE TRAP below.
//   3. The transitive `parentSpanId` ancestors of everything kept by (2). A hit
//      is attributed through its chain, not directly — a `#[server]` fn's
//      instrument span reaches its test in two hops, via the request span — so
//      an orphaned chain is what a missing ancestor looks like.
//   4. One hit-chain per (span name + URI path, test title). The full capture
//      repeats the same fn/test pair up to hundreds of times; the extractor
//      de-duplicates test titles per fn, so extra repeats change no output.
//      This is the whole size win.
//   5. At most 2 orphan examples per (span name + URI path) key — enough to
//      exercise the orphan path, not enough to carry the run's real orphan
//      counts. Per-fn orphan *counts* are therefore NOT preserved by this
//      reduction; the committed snapshot's counts come from the full capture.
//   6. The first 8 non-`/api/` spans that carry a `uri` (static assets, feeds).
//      These are the negative case: traffic that must be attributed to no fn.
//   7. Of each kept span, only the fields the gate reads —
//      `traceId`/`spanId`/`parentSpanId`/`name`/`kind` and the attributes
//      `uri`, `code.module.name`, `code.namespace`, `e2e.test`, `e2e.project`,
//      `method`. Everything else (timestamps, status, the storage-layer attribute
//      payloads) is dropped. `resource`/`scope` objects and the original span
//      batching are preserved so the fixture stays a realistic OTLP document.
//
// THE TRAP: DO NOT KEY THIS ON A REIMPLEMENTATION OF FN IDENTIFICATION
//
// An earlier reduction decided what to keep by re-deriving the `#[server]`
// inventory here and matching span names and URIs against it. That silently
// dropped `upload_media`, declared as
//
//     #[server(input = MultipartFormData, endpoint = "/upload_media")]
//
// because the reimplementation read `endpoint` as the attribute's FIRST
// argument. The failure is invisible: the fixture still parses, the tests still
// pass, and the fn merely looks uncovered — which is exactly the claim AC11
// rests on. So the rules above are deliberately stated in terms of a span's own
// `name`, `uri`, and module attributes. They depend on nothing about how fns are
// enumerated or how endpoints are declared, which is why over-keeping (rule 2's
// supersets include `web::` spans that are not server fns at all) is the right
// trade: a superset cannot lose a hit.
//
// Re-running it needs a capture from a passing SQLite/Chromium lane, e.g.
// `dev e2e-producer sqlite-chromium-1-of-3` —
// `.xtask/diagnostics/e2e-<lane>/capture-<lane>.tar.gz`, member
// `capture/otel-traces.jsonl`; reduce each SQLite/Chromium lane's capture to
// cover the whole population. Byte-identical output requires the same capture;
// a fresh run produces different span ids, so regenerating the fixture means
// regenerating the snapshot from the same capture too.

import { readFileSync, writeFileSync } from "node:fs";

/** Ancestor-walk bound, matching the extractor's own cycle guard. */
const MAX_DEPTH = 32;
/** Attributes `parse_spans` and `extract` read; the rest are dropped. */
const CURRENT_MODULE_ATTR = "code.module.name";
const LEGACY_MODULE_ATTR = "code.namespace";
const KEEP_ATTRS = new Set([
  "uri",
  CURRENT_MODULE_ATTR,
  LEGACY_MODULE_ATTR,
  "e2e.test",
  "e2e.project",
  "method",
]);
/** Span fields the gate reads. */
const KEEP_FIELDS = ["traceId", "spanId", "parentSpanId", "name", "kind"];
const TEST_SPAN_NAME = "e2e.test";
const API_PREFIX = "/api/";
const WEB_MODULE_PREFIX = "web::";
const ORPHAN_EXAMPLES_PER_KEY = 2;
const NON_API_EXAMPLES = 8;

const attr = (span, key) => {
  const found = (span.attributes || []).find((a) => a.key === key);
  return (found && found.value && found.value.stringValue) || "";
};

/** Current module attribute first, then the legacy namespace fallback. */
const spanModule = (span) =>
  attr(span, CURRENT_MODULE_ATTR) || attr(span, LEGACY_MODULE_ATTR);

/** A `uri` with its query string and fragment stripped, as the extractor does. */
const uriPath = (uri) => uri.split(/[?#]/, 1)[0];

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error("usage: reduce-otel-capture.mjs <otel-traces.jsonl> <out.jsonl>");
  process.exit(2);
}

// Parse once, remembering where each span sat so the batching can be rebuilt.
const lines = readFileSync(input, "utf8").split("\n");
const records = [];
const spans = [];
const byId = new Map();
lines.forEach((line, lineIndex) => {
  if (!line.trim()) return;
  const record = JSON.parse(line);
  records[lineIndex] = record;
  for (const resourceSpan of record.resourceSpans || [])
    for (const scopeSpan of resourceSpan.scopeSpans || [])
      for (const span of scopeSpan.spans || []) {
        spans.push(span);
        if (span.spanId) byId.set(span.spanId, span);
      }
});
if (!spans.length) {
  console.error(`${input}: no spans — refusing to write an empty fixture`);
  process.exit(1);
}

const testTitles = new Map(
  spans
    .filter((s) => s.name === TEST_SPAN_NAME && s.spanId)
    .map((s) => [s.spanId, attr(s, TEST_SPAN_NAME)]),
);

/** The title of the test a span's chain reaches, or `null` when it reaches none. */
const attribute = (start) => {
  let current = start;
  for (let hop = 0; hop < MAX_DEPTH; hop++) {
    if (!current.parentSpanId) return null;
    const title = testTitles.get(current.parentSpanId);
    if (title !== undefined) return title;
    const parent = byId.get(current.parentSpanId);
    if (!parent) return null;
    current = parent;
  }
  return null;
};

const keep = new Set();
const keepWithAncestors = (span) => {
  let current = span;
  for (let hop = 0; hop < MAX_DEPTH; hop++) {
    keep.add(current);
    const parent = byId.get(current.parentSpanId);
    if (!parent || keep.has(parent)) return;
    current = parent;
  }
};

const seenChains = new Set();
const orphansPerKey = new Map();
let nonApiKept = 0;
for (const span of spans) {
  if (span.name === TEST_SPAN_NAME) {
    keep.add(span);
    continue;
  }
  const uri = attr(span, "uri");
  const isApi = uri.includes(API_PREFIX);
  const isWebModule = spanModule(span).startsWith(WEB_MODULE_PREFIX);
  if (!isApi && !isWebModule) {
    // The negative case: real traffic that resolves to no server fn.
    if (uri && nonApiKept < NON_API_EXAMPLES) {
      keep.add(span);
      nonApiKept++;
    }
    continue;
  }

  const key = `${span.name}${uriPath(uri)}`;
  const title = attribute(span);
  if (title === null) {
    const seen = orphansPerKey.get(key) || 0;
    if (seen >= ORPHAN_EXAMPLES_PER_KEY) continue;
    orphansPerKey.set(key, seen + 1);
  } else {
    const chain = `${key}${title}`;
    if (seenChains.has(chain)) continue;
    seenChains.add(chain);
  }
  keepWithAncestors(span);
}

const reduceSpan = (span) => {
  const out = {};
  for (const field of KEEP_FIELDS) if (field in span) out[field] = span[field];
  out.attributes = (span.attributes || []).filter((a) => KEEP_ATTRS.has(a.key));
  return out;
};

const out = [];
for (const record of records) {
  if (!record) continue;
  const resourceSpans = [];
  for (const resourceSpan of record.resourceSpans || []) {
    const scopeSpans = [];
    for (const scopeSpan of resourceSpan.scopeSpans || []) {
      const kept = (scopeSpan.spans || []).filter((s) => keep.has(s));
      if (kept.length) scopeSpans.push({ ...scopeSpan, spans: kept.map(reduceSpan) });
    }
    if (scopeSpans.length) resourceSpans.push({ ...resourceSpan, scopeSpans });
  }
  if (resourceSpans.length) out.push(JSON.stringify({ resourceSpans }));
}
writeFileSync(output, `${out.join("\n")}\n`);

console.error(
  `${spans.length} spans in, ${keep.size} kept over ${out.length} line(s); ` +
    `${testTitles.size} test span(s), ${seenChains.size} distinct hit-chain(s), ` +
    `${nonApiKept} non-${API_PREFIX} example(s)`,
);
