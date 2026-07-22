#!/usr/bin/env node
// bench-vs-playwright.mjs: head-to-head verification-loop benchmark,
// hwatu vs Playwright-driven headless Chromium, on the same fixture
// page and the same wall clock.
//
// Run:  node scripts/bench-vs-playwright.mjs [--runs 10]
// Needs: hwatu/hwatud on PATH, node, and playwright installed
// (npm i playwright && npx playwright install chromium).
//
// Scenarios (all medians over N runs):
//   1. verify pass, cold engine  — start the tool, open the page, wait
//      for load, read the DOM, screenshot, tear down. What a one-shot
//      script (or an agent without a persistent server) pays.
//   2. verify pass, warm engine  — the engine already runs (hwatu
//      daemon / persistent Chromium): open, wait, read, screenshot,
//      close the page. What an agent with a persistent server pays
//      per check.
//   3. page-state payload        — bytes an agent must read to "see"
//      the page without pixels: hwatu snapshot JSON vs Playwright's
//      ARIA snapshot. Proxy for tokens.
//   4. steady-state memory       — PSS of the whole process tree with
//      5 pages open, warm.
//
// Methodology notes:
//   - hwatu is exercised through its CLI (execFile per step), so its
//     numbers INCLUDE process spawn + Unix-socket roundtrip per step.
//     Playwright is exercised in-process over an already-open CDP
//     connection, which flatters it. The bias runs against hwatu.
//   - Memory is PSS from /proc/<pid>/smaps_rollup summed over each
//     tool's full process tree, to avoid double-counting shared libs.

import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { createServer } from "node:http";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const execFileP = promisify(execFile);
const RUNS = (() => {
  const i = process.argv.indexOf("--runs");
  return i > 0 ? parseInt(process.argv[i + 1], 10) : 10;
})();

// ---------------------------------------------------------------- fixture

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

function startFixture() {
  return new Promise((resolve) => {
    const srv = createServer((req, res) => {
      res.writeHead(200, { "Content-Type": "text/html" });
      res.end(FIXTURE);
    });
    srv.listen(0, "127.0.0.1", () =>
      resolve({ srv, url: `http://127.0.0.1:${srv.address().port}/` }));
  });
}

// ---------------------------------------------------------------- helpers

const now = () => performance.now();
const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};
const fmt = (xs) =>
  `median ${median(xs).toFixed(0)} ms  (min ${Math.min(...xs).toFixed(0)}, max ${Math.max(...xs).toFixed(0)}, n=${xs.length})`;

/** Run one benchmark iteration, retrying once on a transient failure
 * (either tool can hiccup under system load); a second failure is
 * real and aborts the run. */
async function attempt(fn) {
  try { return await fn(); }
  catch (e) {
    console.error(`  (retrying after transient failure: ${String(e).split("\n")[0]})`);
    await new Promise((r) => setTimeout(r, 1000));
    return fn();
  }
}

async function hwatu(...args) {
  const { stdout } = await execFileP("hwatu", args);
  return stdout.trim();
}

/** Speak the socket protocol directly (one JSON line per connection),
 * the way jcode's native backend and `hwatu mcp` do: no per-step
 * process spawn. This is the fair peer of in-process Playwright. */
async function hwatuSock(req) {
  const net = await import("node:net");
  const sockPath = process.env.XDG_RUNTIME_DIR
    ? path.join(process.env.XDG_RUNTIME_DIR, "hwatu.sock")
    : `/tmp/hwatu-${process.getuid()}.sock`;
  return new Promise((resolve, reject) => {
    const c = net.createConnection(sockPath, () => c.write(JSON.stringify(req) + "\n"));
    let buf = "";
    c.on("data", (d) => {
      buf += d;
      if (buf.includes("\n")) { c.end(); resolve(JSON.parse(buf)); }
    });
    c.on("error", reject);
  });
}

/** PSS (kB) summed over a pid and all its descendants. */
function treePss(rootPids) {
  const children = new Map();
  for (const pid of fs.readdirSync("/proc").filter((p) => /^\d+$/.test(p))) {
    try {
      const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
      const ppid = parseInt(stat.split(") ")[1].split(" ")[1], 10);
      if (!children.has(ppid)) children.set(ppid, []);
      children.get(ppid).push(parseInt(pid, 10));
    } catch {}
  }
  const seen = new Set();
  const stack = [...rootPids];
  while (stack.length) {
    const pid = stack.pop();
    if (seen.has(pid)) continue;
    seen.add(pid);
    for (const c of children.get(pid) ?? []) stack.push(c);
  }
  let kb = 0;
  for (const pid of seen) {
    try {
      const roll = fs.readFileSync(`/proc/${pid}/smaps_rollup`, "utf8");
      kb += parseInt(roll.match(/^Pss:\s+(\d+)/m)?.[1] ?? "0", 10);
    } catch {}
  }
  return kb;
}

function pidsOf(pattern) {
  try {
    const out = fs.readdirSync("/proc").filter((p) => /^\d+$/.test(p));
    return out.filter((pid) => {
      try {
        return fs.readFileSync(`/proc/${pid}/comm`, "utf8").trim() === pattern;
      } catch { return false; }
    }).map(Number);
  } catch { return []; }
}

/** hwatud pids belonging to THIS benchmark's XDG_RUNTIME_DIR, so a
 * live desktop daemon never contaminates the measurement. */
function benchDaemonPids() {
  const want = `XDG_RUNTIME_DIR=${process.env.XDG_RUNTIME_DIR ?? ""}`;
  return pidsOf("hwatud").filter((pid) => {
    try {
      return fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(want);
    } catch { return false; }
  });
}

// ---------------------------------------------------------------- hwatu side

async function hwatuColdVerify(url, shotPath) {
  // Cold = daemon not running. `hwatu` autostarts it; `quit` tears it down.
  const t0 = now();
  const open = await hwatu("--headless", "--json", url);
  const id = JSON.parse(open).id;
  await hwatu("wait-load", "--id", String(id));
  // On a just-autostarted daemon, wait-load can win a race with the
  // navigation actually starting; do what an agent does with the
  // structured "eval interrupted" error: wait again and retry.
  try {
    await hwatu("eval", "--id", String(id), "document.title");
  } catch {
    await hwatu("wait-load", "--id", String(id));
    await hwatu("eval", "--id", String(id), "document.title");
  }
  await hwatu("shot", "--id", String(id), shotPath);
  await hwatu("close", String(id));
  const dt = now() - t0;
  await hwatu("quit").catch(() => {});
  await new Promise((r) => setTimeout(r, 400)); // let the socket vanish
  return dt;
}

async function hwatuWarmVerify(url, shotPath) {
  const t0 = now();
  const open = await hwatu("--headless", "--json", url);
  const id = JSON.parse(open).id;
  await hwatu("wait-load", "--id", String(id));
  await hwatu("eval", "--id", String(id), "document.title");
  await hwatu("shot", "--id", String(id), shotPath);
  await hwatu("close", String(id));
  return now() - t0;
}

/** Same loop over the raw socket (no CLI process per step). */
async function hwatuWarmVerifySock(url, shotPath) {
  const t0 = now();
  const open = await hwatuSock({ cmd: "open", url, mode: "headless" });
  const id = open.window.id;
  await hwatuSock({ cmd: "wait_load", id });
  await hwatuSock({ cmd: "eval", id, js: "document.title" });
  if (shotPath) await hwatuSock({ cmd: "screenshot", id, path: shotPath });
  await hwatuSock({ cmd: "close", id });
  return now() - t0;
}

// ---------------------------------------------------------------- playwright side

async function pwColdVerify(chromium, url, shotPath) {
  const t0 = now();
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.goto(url, { waitUntil: "load" });
  await page.title();
  await page.screenshot({ path: shotPath });
  await browser.close();
  return now() - t0;
}

async function pwWarmVerify(browser, url, shotPath) {
  const t0 = now();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(url, { waitUntil: "load" });
  await page.title();
  if (shotPath) await page.screenshot({ path: shotPath });
  await context.close();
  return now() - t0;
}

/** Open a page and have it fully loaded, nothing else. */
async function hwatuOpenLoaded(url) {
  const t0 = now();
  const open = await hwatuSock({ cmd: "open", url, mode: "headless" });
  await hwatuSock({ cmd: "wait_load", id: open.window.id });
  const dt = now() - t0;
  await hwatuSock({ cmd: "close", id: open.window.id });
  return dt;
}

async function pwOpenLoaded(browser, url) {
  const t0 = now();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(url, { waitUntil: "load" });
  const dt = now() - t0;
  await context.close();
  return dt;
}

// ---------------------------------------------------------------- main

const { srv, url } = await startFixture();
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "bench-"));
const shot = (n) => path.join(tmp, `${n}.png`);
const results = {};

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  // Not next to this script: resolve from the invoking directory, so
  // `npm i playwright` in any scratch dir works.
  try {
    const { createRequire } = await import("node:module");
    const req = createRequire(path.join(process.cwd(), "noop.js"));
    const mod = await import(req.resolve("playwright"));
    chromium = mod.chromium ?? mod.default?.chromium;
  } catch {
    console.error("playwright not installed; npm i playwright"); process.exit(1);
  }
}

// Make sure no daemon lingers from a previous run.
await hwatu("quit").catch(() => {});
await new Promise((r) => setTimeout(r, 500));

// -- 1. cold verify ----------------------------------------------------
{
  const h = [], p = [];
  for (let i = 0; i < RUNS; i++) h.push(await attempt(() => hwatuColdVerify(url, shot("hc"))));
  for (let i = 0; i < RUNS; i++) p.push(await attempt(() => pwColdVerify(chromium, url, shot("pc"))));
  results.cold = { hwatu: h, playwright: p };
  console.log(`cold verify   hwatu:      ${fmt(h)}`);
  console.log(`cold verify   playwright: ${fmt(p)}`);
}

// -- 2. warm verify ----------------------------------------------------
{
  await hwatu("ping"); // spawn the daemon; first-window init paid here
  await hwatuWarmVerify(url, shot("w0"));
  const browser = await chromium.launch();
  await pwWarmVerify(browser, url, shot("w1"));

  const h = [], hs = [], hd = [], p = [], pd = [], ho = [], po = [];
  for (let i = 0; i < RUNS; i++) ho.push(await attempt(() => hwatuOpenLoaded(url)));
  for (let i = 0; i < RUNS; i++) po.push(await attempt(() => pwOpenLoaded(browser, url)));
  for (let i = 0; i < RUNS; i++) h.push(await attempt(() => hwatuWarmVerify(url, shot("hw"))));
  for (let i = 0; i < RUNS; i++) hs.push(await attempt(() => hwatuWarmVerifySock(url, shot("hs"))));
  for (let i = 0; i < RUNS; i++) hd.push(await attempt(() => hwatuWarmVerifySock(url, null)));
  for (let i = 0; i < RUNS; i++) p.push(await attempt(() => pwWarmVerify(browser, url, shot("pw"))));
  for (let i = 0; i < RUNS; i++) pd.push(await attempt(() => pwWarmVerify(browser, url, null)));
  results.warm = { hwatu_open: ho, playwright_open: po, hwatu_cli: h, hwatu_socket: hs, hwatu_socket_noshot: hd, playwright: p, playwright_noshot: pd };
  console.log(`open loaded   hwatu (socket):     ${fmt(ho)}`);
  console.log(`open loaded   playwright:         ${fmt(po)}`);
  console.log(`warm verify   hwatu (CLI):        ${fmt(h)}`);
  console.log(`warm verify   hwatu (socket):     ${fmt(hs)}`);
  console.log(`warm verify   hwatu (sock,noshot):${fmt(hd)}`);
  console.log(`warm verify   playwright:         ${fmt(p)}`);
  console.log(`warm verify   playwright (noshot):${fmt(pd)}`);

  // -- 3. page-state payload -------------------------------------------
  const open = await hwatu("--headless", "--json", url);
  const id = JSON.parse(open).id;
  await hwatu("wait-load", "--id", String(id));
  const snap = await hwatu("snapshot", "--id", String(id));
  const page = await browser.newPage();
  await page.goto(url, { waitUntil: "load" });
  const aria = await page.locator("body").ariaSnapshot();
  results.payload = { hwatu: snap.length, playwright_aria: aria.length };
  console.log(`page payload  hwatu snapshot: ${snap.length} bytes | playwright aria: ${aria.length} bytes`);
  await page.close();
  await hwatu("close", String(id));

  // -- 4. memory with 5 pages -------------------------------------------
  // Fresh engines on both sides: WebKit caches terminated web
  // processes for reuse, so a daemon that just served ~60 benchmark
  // windows reports several GB of PSS that a real 5-window session
  // never sees. Measure what a user/agent actually gets.
  await browser.close();
  await hwatu("quit").catch(() => {});
  await new Promise((r) => setTimeout(r, 800));
  await hwatu("ping");
  const browser2 = await chromium.launch();
  const ids = [];
  for (let i = 0; i < 5; i++) {
    const o = await hwatu("--headless", "--json", url);
    ids.push(JSON.parse(o).id);
  }
  await hwatu("wait-load", "--id", String(ids[4]));
  const contexts = [];
  for (let i = 0; i < 5; i++) {
    const c = await browser2.newContext();
    const pg = await c.newPage();
    await pg.goto(url, { waitUntil: "load" });
    contexts.push(c);
  }
  await new Promise((r) => setTimeout(r, 3000)); // settle
  const hwatuPss = treePss(benchDaemonPids());
  // Playwright's Browser has no public .process(); find the Chromium
  // tree by cmdline (the ms-playwright install path) and take roots.
  const chromePids = fs.readdirSync("/proc").filter((p) => /^\d+$/.test(p)).filter((pid) => {
    try {
      return fs.readFileSync(`/proc/${pid}/cmdline`, "utf8").includes("ms-playwright");
    } catch { return false; }
  }).map(Number);
  const chromePss = treePss(chromePids);
  results.memory5 = { hwatu_mb: Math.round(hwatuPss / 1024), playwright_mb: Math.round(chromePss / 1024) };
  console.log(`memory (5 pages, PSS)  hwatu tree: ${results.memory5.hwatu_mb} MB | chromium tree: ${results.memory5.playwright_mb} MB`);

  for (const c of contexts) await c.close();
  for (const id of ids) await hwatu("close", String(id)).catch(() => {});
  await browser2.close();
}

fs.writeFileSync(path.join(tmp, "results.json"), JSON.stringify(results, null, 2));
console.log(`\nraw results: ${path.join(tmp, "results.json")}`);
const v = await execFileP("hwatu", ["ping"]).then(r => r.stdout.trim()).catch(() => "?");
console.log(`hwatu: ${v}`);
console.log(`playwright chromium: ${chromium.executablePath()}`);
srv.close();
process.exit(0);
