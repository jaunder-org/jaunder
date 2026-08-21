#!/usr/bin/env node
// Reproduce the machine-readable parts of
// 2026-08-21-development-friction-session-analysis.md.
//
// Usage:
//   node docs/archive/2026-08-21-development-friction-session-analysis.mjs \
//     [/home/mdorman/.omp/agent/sessions]
//
// The script intentionally reads only OMP session artifacts whose directory name starts
// with `-src-jaunder`, then summarizes files, JSONL transcripts, log files, and
// `xtask-done:` gate sentinels.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const sessionRoot = process.argv[2] ?? path.join(os.homedir(), ".omp/agent/sessions");
const targetPrefix = "-src-jaunder";

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full, out);
    } else if (entry.isFile()) {
      out.push(full);
    }
  }
  return out;
}

function quantile(sorted, q) {
  if (sorted.length === 0) return null;
  return sorted[Math.floor(sorted.length * q)] ?? sorted.at(-1);
}

function summarize(rows) {
  const byCommand = new Map();
  for (const row of rows) {
    const bucket = byCommand.get(row.command) ?? {
      count: 0,
      failures: 0,
      total_ms: 0,
      min_ms: Number.POSITIVE_INFINITY,
      max_ms: 0,
      durations: [],
    };
    bucket.count += 1;
    bucket.failures += row.ok ? 0 : 1;
    bucket.total_ms += row.duration_ms;
    bucket.min_ms = Math.min(bucket.min_ms, row.duration_ms);
    bucket.max_ms = Math.max(bucket.max_ms, row.duration_ms);
    bucket.durations.push(row.duration_ms);
    byCommand.set(row.command, bucket);
  }

  return Object.fromEntries(
    [...byCommand.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([command, bucket]) => {
        bucket.durations.sort((a, b) => a - b);
        return [
          command,
          {
            count: bucket.count,
            failures: bucket.failures,
            total_ms: bucket.total_ms,
            min_ms: bucket.min_ms,
            max_ms: bucket.max_ms,
            avg_ms: Math.round(bucket.total_ms / bucket.count),
            median_ms: quantile(bucket.durations, 0.5),
            p90_ms: quantile(bucket.durations, 0.9),
          },
        ];
      }),
  );
}

function commandName(rawCommand, text) {
  if (rawCommand === "check" && /--no-test/.test(text)) return "check --no-test";
  if (rawCommand === "validate" && /--no-e2e/.test(text)) return "validate --no-e2e";
  return rawCommand;
}

function parseXtaskRows(file, text, relativeFile) {
  const rows = [];
  const sentinel = /xtask-done: command=([^\s]+) ok=(true|false) exit=([^\s]+) duration_ms=(\d+)/g;
  for (const match of text.matchAll(sentinel)) {
    rows.push({
      file: relativeFile,
      command: commandName(match[1], text),
      raw_command: match[1],
      ok: match[2] === "true",
      exit: match[3],
      duration_ms: Number(match[4]),
      fail_lines: [...text.matchAll(/\[FAIL\][^\n]*/g)].map((m) => m[0]).slice(0, 8),
    });
  }
  return rows;
}

const sessionDirs = fs
  .readdirSync(sessionRoot, { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && entry.name.startsWith(targetPrefix))
  .map((entry) => path.join(sessionRoot, entry.name))
  .sort();

const files = sessionDirs.flatMap((dir) => walk(dir));
const logFiles = files.filter((file) => file.endsWith(".log"));
const jsonlFiles = files.filter((file) => file.endsWith(".jsonl"));
const totalBytes = files.reduce((sum, file) => sum + fs.statSync(file).size, 0);

const xtaskRows = [];
for (const file of logFiles) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch {
    continue;
  }
  const relativeFile = path.relative(sessionRoot, file);
  xtaskRows.push(...parseXtaskRows(file, text, relativeFile));
}

const totalGateMs = xtaskRows.reduce((sum, row) => sum + row.duration_ms, 0);
const failedGateMs = xtaskRows.filter((row) => !row.ok).reduce((sum, row) => sum + row.duration_ms, 0);
const greenGateMs = totalGateMs - failedGateMs;

const report = {
  session_root: sessionRoot,
  corpus: {
    session_directories: sessionDirs.map((dir) => path.relative(sessionRoot, dir)),
    session_directory_count: sessionDirs.length,
    file_count: files.length,
    jsonl_transcript_count: jsonlFiles.length,
    log_file_count: logFiles.length,
    bash_log_count: logFiles.filter((file) => /\.bash(?:-original)?\.log$/.test(file)).length,
    total_bytes: totalBytes,
  },
  gate_time: {
    xtask_done_count: xtaskRows.length,
    total_ms: totalGateMs,
    green_ms: greenGateMs,
    failed_ms: failedGateMs,
    green_percent: totalGateMs === 0 ? null : Number(((greenGateMs / totalGateMs) * 100).toFixed(1)),
  },
  commands: summarize(xtaskRows),
  failures: xtaskRows
    .filter((row) => !row.ok)
    .map(({ file, command, duration_ms, fail_lines }) => ({ file, command, duration_ms, fail_lines })),
};

console.log(JSON.stringify(report, null, 2));
