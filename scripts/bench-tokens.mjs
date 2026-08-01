#!/usr/bin/env node
// bench-tokens.mjs: token/context budget benchmark for browser-verification
// tool outputs. It always reports UTF-8 bytes, and reports one pinned
// tokenizer when the optional `gpt-tokenizer` package is available.
//
// Run self-checks:
//   node scripts/bench-tokens.mjs --self-check
//
// Measure built-in fixture transcripts with the pinned tokenizer:
//   npm install --prefix /tmp/hwatu-tokenizer --no-save gpt-tokenizer
//   NODE_PATH=/tmp/hwatu-tokenizer/node_modules node scripts/bench-tokens.mjs
//
// Measure live hwatu output against the shared benchmark fixture:
//   cargo build --release
//   NODE_PATH=/tmp/hwatu-tokenizer/node_modules PATH=$PWD/target/release:$PATH \
//     node scripts/bench-tokens.mjs --hwatu-live
//
// Add local competitor transcripts without inventing numbers:
//   NODE_PATH=/tmp/hwatu-tokenizer/node_modules PATH=$PWD/target/release:$PATH \
//     node scripts/bench-tokens.mjs \
//       --hwatu-live \
//       --input playwright-mcp=bench-inputs/playwright-mcp.txt \
//       --input chrome-devtools-mcp=bench-inputs/chrome-devtools-mcp.txt

import { execFile } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { readFileSync } from "node:fs";

const execFileP = promisify(execFile);
const TOKENIZER_NAME = "gpt-tokenizer cl100k_base";
const DEFAULT_HWATU_BUDGET_BYTES = 16_384;
const DEFAULT_HWATU_BUDGET_TOKENS = 4_096;

const FIXTURE = `<!doctype html><html><head><meta charset="utf-8">
<title>bench fixture</title>
<style>body{font:14px sans-serif;margin:2rem}.card{border:1px solid #ccc;
border-radius:8px;padding:1rem;margin:.5rem;display:inline-block;width:180px}</style>
</head><body><h1>Component gallery</h1>
${Array.from({ length: 40 }, (_, i) =>
  `<div class="card"><h2>Card ${i}</h2><p>Body text for card ${i}.</p>
   <button data-i="${i}">Action ${i}</button>
   <a href="/detail/${i}">Details ${i}</a></div>`).join("\n")}
<script>console.log("fixture ready")</script></body></html>`;

const BUILTIN_TRANSCRIPTS = {
  "hwatu-check-json-fixture": JSON.stringify({
    ok: true,
    url: "http://127.0.0.1:PORT/",
    title: "bench fixture",
    eval: "bench fixture",
    shot_path: "/tmp/hwatu-token-bench/check.png",
    console: [{ level: "log", text: "fixture ready" }],
    timings_ms: { open: 8, wait: 24, eval: 1, shot: 12, total: 39 },
  }, null, 2),
  "playwright-mcp-input-template": [
    "# Local transcript template, not a measurement",
    "# Replace this with the exact tool output emitted by Playwright MCP for:",
    "# navigate to the fixture, read page state/title, screenshot if used.",
    "# Then pass it with --input playwright-mcp=path/to/transcript.txt.",
  ].join("\n"),
  "chrome-devtools-mcp-input-template": [
    "# Local transcript template, not a measurement",
    "# Replace this with the exact tool output emitted by Chrome DevTools MCP for:",
    "# navigate to the fixture, read page state/title, screenshot if used.",
    "# Then pass it with --input chrome-devtools-mcp=path/to/transcript.txt.",
  ].join("\n"),
};

function parseArgs(argv) {
  const args = {
    selfCheck: false,
    hwatuLive: false,
    json: false,
    inputs: [],
    hwatuMaxBytes: numberFromEnv("HWATU_TOKEN_BENCH_MAX_BYTES", DEFAULT_HWATU_BUDGET_BYTES),
    hwatuMaxTokens: numberFromEnv("HWATU_TOKEN_BENCH_MAX_TOKENS", DEFAULT_HWATU_BUDGET_TOKENS),
  };
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--self-check") args.selfCheck = true;
    else if (arg === "--hwatu-live") args.hwatuLive = true;
    else if (arg === "--json") args.json = true;
    else if (arg === "--hwatu-max-bytes") args.hwatuMaxBytes = parsePositiveInt(argv[++i], arg);
    else if (arg === "--hwatu-max-tokens") args.hwatuMaxTokens = parsePositiveInt(argv[++i], arg);
    else if (arg === "--input") args.inputs.push(parseInput(argv[++i]));
    else if (arg.startsWith("--input=")) args.inputs.push(parseInput(arg.slice("--input=".length)));
    else usage(`unknown argument: ${arg}`);
  }
  return args;
}

function numberFromEnv(name, fallback) {
  if (!process.env[name]) return fallback;
  return parsePositiveInt(process.env[name], name);
}

function parsePositiveInt(raw, name) {
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 1) usage(`${name} must be a positive integer`);
  return n;
}

function parseInput(spec) {
  const idx = spec.indexOf("=");
  if (idx <= 0 || idx === spec.length - 1) usage("--input must be name=path");
  return { name: spec.slice(0, idx), file: spec.slice(idx + 1) };
}

function usage(msg) {
  if (msg) console.error(`error: ${msg}`);
  console.error(`usage: node scripts/bench-tokens.mjs [--self-check] [--hwatu-live] [--json]
       [--hwatu-max-bytes N] [--hwatu-max-tokens N]
       [--input name=path/to/transcript.txt]

Reports UTF-8 bytes for every transcript. Reports ${TOKENIZER_NAME} tokens when
'gpt-tokenizer' is installed and visible via NODE_PATH, for example:
  npm install --prefix /tmp/hwatu-tokenizer --no-save gpt-tokenizer
  NODE_PATH=/tmp/hwatu-tokenizer/node_modules node scripts/bench-tokens.mjs --hwatu-live

Only entries whose name starts with "hwatu" are budget-gated.`);
  process.exit(msg ? 2 : 0);
}

async function loadTokenizer() {
  const req = createRequire(import.meta.url);
  for (const spec of ["gpt-tokenizer/model/gpt-4", "gpt-tokenizer"]) {
    try {
      const resolved = req.resolve(spec);
      const mod = await import(resolved);
      const encode = mod.encode ?? mod.default?.encode;
      if (typeof encode !== "function") throw new Error(`missing encode export from ${spec}`);
      return { name: TOKENIZER_NAME, count: (text) => encode(text).length };
    } catch (e) {
      loadTokenizer.lastError = e;
    }
  }
  return {
    name: TOKENIZER_NAME,
    count: null,
    unavailable: String(loadTokenizer.lastError?.message ?? loadTokenizer.lastError ?? "module not found"),
  };
}

function measure(name, text, tokenizer, source = "fixture") {
  const bytes = Buffer.byteLength(text, "utf8");
  return {
    name,
    source,
    bytes,
    tokens: tokenizer.count ? tokenizer.count(text) : null,
  };
}

function gateHwatu(rows, args) {
  const failures = [];
  for (const row of rows) {
    if (!row.name.startsWith("hwatu")) continue;
    if (row.bytes > args.hwatuMaxBytes) {
      failures.push(`${row.name}: ${row.bytes} bytes > ${args.hwatuMaxBytes}`);
    }
    if (row.tokens != null && row.tokens > args.hwatuMaxTokens) {
      failures.push(`${row.name}: ${row.tokens} ${TOKENIZER_NAME} tokens > ${args.hwatuMaxTokens}`);
    }
  }
  return failures;
}

async function collectHwatuLive() {
  const tmp = await mkdtemp(path.join(tmpdir(), "hwatu-token-bench-"));
  const shotPath = path.join(tmp, "check.png");
  let srv;
  try {
    const fixture = await startFixture();
    srv = fixture.srv;
    const { stdout } = await execFileP("hwatu", [
      "check",
      fixture.url,
      "--eval",
      "document.title",
      "--until",
      "dom",
      `--shot=${shotPath}`,
    ], { maxBuffer: 10 * 1024 * 1024 });
    return stdout.trim();
  } finally {
    if (srv) await new Promise((resolve) => srv.close(resolve));
    await rm(tmp, { recursive: true, force: true });
  }
}

function startFixture() {
  return new Promise((resolve) => {
    const srv = createServer((req, res) => {
      if (req.url === "/favicon.ico") {
        res.writeHead(404);
        res.end();
        return;
      }
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      res.end(FIXTURE);
    });
    srv.listen(0, "127.0.0.1", () => {
      resolve({ srv, url: `http://127.0.0.1:${srv.address().port}/` });
    });
  });
}

async function runSelfChecks(tokenizer) {
  const ascii = measure("hwatu-selfcheck-ascii", "abc", tokenizer);
  const unicode = measure("hwatu-selfcheck-unicode", "λ", tokenizer);
  assert(ascii.bytes === 3, `expected ASCII byte count 3, got ${ascii.bytes}`);
  assert(unicode.bytes === 2, `expected lambda byte count 2, got ${unicode.bytes}`);
  const rows = [measure("hwatu-budget-ok", "ok", tokenizer)];
  assert(gateHwatu(rows, { hwatuMaxBytes: 2, hwatuMaxTokens: 999 }).length === 0,
    "expected hwatu budget pass");
  assert(gateHwatu(rows, { hwatuMaxBytes: 1, hwatuMaxTokens: 999 }).length === 1,
    "expected hwatu byte budget failure");
  assert(gateHwatu([measure("playwright-mcp-big", "x".repeat(20), tokenizer)], {
    hwatuMaxBytes: 1,
    hwatuMaxTokens: 1,
  }).length === 0, "competitor rows must not be budget-gated");
  if (tokenizer.count) {
    assert(ascii.tokens > 0, "tokenizer should return positive token count");
  }
  console.log("self-checks passed");
}

function assert(cond, msg) {
  if (!cond) throw new Error(`self-check failed: ${msg}`);
}

function printMarkdown(rows, tokenizer, args, failures) {
  console.log(`# token/context budget benchmark\n`);
  console.log(`tokenizer: ${tokenizer.count ? tokenizer.name : `${tokenizer.name} unavailable (${tokenizer.unavailable})`}`);
  console.log(`hwatu budget gate: ${args.hwatuMaxBytes} bytes${tokenizer.count ? `, ${args.hwatuMaxTokens} ${tokenizer.name} tokens` : ""}`);
  console.log("");
  console.log("| transcript | source | UTF-8 bytes | pinned tokenizer tokens |");
  console.log("|---|---:|---:|---:|");
  for (const row of rows) {
    console.log(`| ${row.name} | ${row.source} | ${row.bytes} | ${row.tokens ?? "n/a"} |`);
  }
  console.log("");
  console.log(failures.length ? `budget failures:\n- ${failures.join("\n- ")}` : "budget failures: none");
}

const args = parseArgs(process.argv);
const tokenizer = await loadTokenizer();
if (args.selfCheck) await runSelfChecks(tokenizer);

const rows = [];
for (const [name, text] of Object.entries(BUILTIN_TRANSCRIPTS)) {
  rows.push(measure(name, text, tokenizer, "built-in fixture/input"));
}
for (const input of args.inputs) {
  rows.push(measure(input.name, readFileSync(input.file, "utf8"), tokenizer, input.file));
}
if (args.hwatuLive) {
  rows.push(measure("hwatu-live-check-json", await collectHwatuLive(), tokenizer, "live hwatu check against fixture"));
}

const failures = gateHwatu(rows, args);
if (args.json) {
  console.log(JSON.stringify({ tokenizer: tokenizer.name, tokenizer_available: Boolean(tokenizer.count), rows, failures }, null, 2));
} else {
  printMarkdown(rows, tokenizer, args, failures);
}
if (failures.length) process.exit(1);
