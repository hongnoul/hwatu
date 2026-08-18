#!/usr/bin/env bash
# End-to-end contract test for the harness-neutral verification router.
# Exercises the exact same versioned job through the CLI and MCP surfaces,
# compares semantic evidence, and checks owned server/daemon cleanup.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${HWATU_TEST_BIN:-$root/target/debug}"
work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-verify-router.XXXXXX")"
port="${HWATU_VERIFY_TEST_PORT:-48767}"
export XDG_RUNTIME_DIR="$work/run" XDG_STATE_HOME="$work/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$work/job"

cleanup() {
  "$bin/hwatu" quit >/dev/null 2>&1 || true
  if [ "${HWATU_VERIFY_TEST_KEEP:-0}" = 1 ]; then
    echo "kept fixture: $work"
  else
    rm -rf "$work"
  fi
}
trap cleanup EXIT

cat > "$work/job/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Harness-neutral chore</title>
<style>
  * { box-sizing: border-box; }
  body { margin: 0; font: 16px system-ui; color: #171717; background: #f6f4ef; }
  main { width: min(100% - 32px, 980px); margin: 40px auto; border: 1px solid #222; padding: 24px; }
  @media (min-width: 768px) { main { padding: 48px; } }
</style>
<main data-chore="router"><h1>Harness-neutral verification chore</h1><p>One spec, every agent.</p></main>
HTML

cat > "$work/job/verify.json" <<JSON
{
  "version": 1,
  "name": "harness-neutral-chore",
  "cwd": ".",
  "url": "http://127.0.0.1:$port/index.html",
  "tier": "micro",
  "preflight": {
    "argv": ["python3", "-c", "from pathlib import Path; assert 'data-chore=' in Path('index.html').read_text()"]
  },
  "server": {
    "argv": ["python3", "-m", "http.server", "$port", "--bind", "127.0.0.1"]
  },
  "ready_timeout_ms": 5000,
  "viewports": ["390x844", "768x1024", "1440x1000"],
  "assertion_js": "const el = document.querySelector('[data-chore=router]'); return { ok: !!el && /One spec/.test(document.body.innerText), marker: el?.dataset.chore ?? null };",
  "source_files": ["index.html"],
  "artifacts_dir": "artifacts",
  "report_path": "artifacts/report.json"
}
JSON

"$bin/hwatu" verify "$work/job/verify.json" > "$work/cli.json"
python3 - "$work/cli.json" <<'PY'
import json, pathlib, sys
r = json.load(open(sys.argv[1]))
assert r["passed"] is True, r
assert r["schema_version"] == 1
assert r["job"] == "harness-neutral-chore"
assert r["spec_fingerprint"].startswith("fnv1a64:")
assert len(r["check"]["viewports"]) == 3
assert all(pathlib.Path(v["shot"]).is_file() for v in r["check"]["viewports"])
assert all(v["eval"]["assertion"]["marker"] == "router" for v in r["check"]["viewports"])
PY
cp "$work/job/artifacts/report.json" "$work/cli-report.json"

cat > "$work/mcp-input.jsonl" <<JSON
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"verify_ui","arguments":{"spec_path":"$work/job/verify.json"}}}
JSON
"$bin/hwatu" mcp < "$work/mcp-input.jsonl" > "$work/mcp-output.jsonl"
python3 - "$work/mcp-output.jsonl" "$work/mcp-report.json" <<'PY'
import json, sys
lines = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
reply = next(x for x in lines if x.get("id") == 2)
assert reply["result"]["isError"] is False, reply
report = json.loads(reply["result"]["content"][0]["text"])
assert report["passed"] is True, report
json.dump(report, open(sys.argv[2], "w"), indent=2)
PY

python3 - "$work/cli-report.json" "$work/mcp-report.json" <<'PY'
import json, sys
cli, mcp = (json.load(open(path)) for path in sys.argv[1:])
def semantic(r):
    return {
        "schema_version": r["schema_version"],
        "job": r["job"],
        "tier": r["tier"],
        "passed": r["passed"],
        "url": r["url"],
        "source_fingerprint": r["source_fingerprint"],
        "sizes": [v["size"] for v in r["check"]["viewports"]],
        "assertions": [v["eval"]["assertion"] for v in r["check"]["viewports"]],
    }
assert semantic(cli) == semantic(mcp), (semantic(cli), semantic(mcp))
PY

# A failed contract is a failed CLI exit and an MCP tool error, while both
# surfaces retain the complete JSON evidence report.
python3 - "$work/job/verify.json" "$work/job/fail.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
r["name"] = "harness-neutral-failure"
r["assertion_js"] = "return { ok: false, reason: 'intentional' };"
r["report_path"] = "artifacts/fail-report.json"
json.dump(r, open(sys.argv[2], "w"))
PY
set +e
"$bin/hwatu" verify "$work/job/fail.json" > "$work/fail-cli.json"
fail_exit=$?
set -e
[ "$fail_exit" -eq 1 ]
cat > "$work/fail-mcp.jsonl" <<JSON
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"verify_ui","arguments":{"spec_path":"$work/job/fail.json"}}}
JSON
"$bin/hwatu" mcp < "$work/fail-mcp.jsonl" > "$work/fail-mcp-output.jsonl"
python3 - "$work/fail-cli.json" "$work/fail-mcp-output.jsonl" <<'PY'
import json, sys
cli = json.load(open(sys.argv[1]))
reply = next(json.loads(line) for line in open(sys.argv[2]) if '"id":2' in line)
assert cli["passed"] is False
assert reply["result"]["isError"] is True, reply
mcp = json.loads(reply["result"]["content"][0]["text"])
assert mcp["passed"] is False
assert mcp["spec_fingerprint"] == cli["spec_fingerprint"]
PY

# Losing a tracked source during the pass produces a failed report instead of
# dropping all evidence through an early error return.
printf 'tracked\n' > "$work/job/tracked.txt"
python3 - "$work/job/verify.json" "$work/job/stale.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
r["name"] = "harness-neutral-stale"
r["source_files"] = ["tracked.txt"]
r["preflight"] = {"argv": ["python3", "-c", "from pathlib import Path; Path('tracked.txt').unlink()"]}
r["report_path"] = "artifacts/stale-report.json"
json.dump(r, open(sys.argv[2], "w"))
PY
set +e
"$bin/hwatu" verify "$work/job/stale.json" > "$work/stale-cli.json"
stale_exit=$?
set -e
[ "$stale_exit" -eq 1 ]
python3 - "$work/stale-cli.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
assert r["passed"] is False
assert r["source_fingerprint_after"] is None
assert any("became unreadable" in finding for finding in r["findings"])
PY

# SIGTERM cancellation is converted into orderly unwinding, so the server
# process group is gone before the router exits.
cancel_port=$((port + 1))
cat > "$work/job/cancel.json" <<JSON
{
  "version": 1,
  "name": "harness-neutral-cancel",
  "url": "http://127.0.0.1:$cancel_port/",
  "server": { "argv": ["python3", "-c", "import os,time; open('cancel-server.pid','w').write(str(os.getpid())); time.sleep(30)"] },
  "ready_timeout_ms": 30000,
  "source_files": ["index.html"],
  "report_path": "artifacts/cancel-report.json"
}
JSON
"$bin/hwatu" verify "$work/job/cancel.json" > "$work/cancel-cli.json" 2> "$work/cancel-cli.err" &
router_pid=$!
for _ in $(seq 1 100); do
  [ -s "$work/job/cancel-server.pid" ] && break
  sleep .02
done
[ -s "$work/job/cancel-server.pid" ]
server_pid=$(cat "$work/job/cancel-server.pid")
kill -TERM "$router_pid"
set +e
wait "$router_pid"
cancel_exit=$?
set -e
[ "$cancel_exit" -ne 0 ]
if kill -0 "$server_pid" 2>/dev/null; then
  echo "verification cancellation leaked server pid $server_pid" >&2
  exit 1
fi

# The router owns and terminates the server it starts.
python3 - "$port" <<'PY'
import socket, sys
s = socket.socket()
s.settimeout(.25)
assert s.connect_ex(("127.0.0.1", int(sys.argv[1]))) != 0, "verification server leaked"
PY

echo "verify router: parity, failures, staleness, and cancellation cleanup passed"
