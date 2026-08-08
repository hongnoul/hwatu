#!/usr/bin/env bash
# Long-lived daemon memory soak for repeated page operations.
#
# Exercises the same warm hwatud process through repeated goto,
# screenshot, and snapshot rounds against a deterministic local fixture.
# By default it records measurements only. Pass --fail-on-growth to turn
# the memory threshold into a regression gate.
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/soak-daemon-rss.sh [options]

Options:
  --rounds N              operation rounds after warmup (default: 50)
  --warmup N              warmup rounds before baseline (default: 5)
  --growth-threshold PCT  max memory growth percentage for --fail-on-growth (default: 20)
  --fail-on-growth        exit non-zero when growth exceeds threshold
  --out DIR               write artifacts and summary.json here (default: temp dir)
  --url URL               use an explicit URL instead of the built-in local fixture
  --keep                  keep the artifact directory after exit
  --no-build              require existing target/release binaries
  --self-test             run parser and summary-output smoke tests, then exit
  -h, --help              show this help

Environment:
  HWATU_SOAK_ROUNDS, HWATU_SOAK_WARMUP, HWATU_SOAK_GROWTH_THRESHOLD
  HWATU_SOAK_FAIL_ON_GROWTH=1, HWATU_SOAK_OUT, HWATU_SOAK_URL
USAGE
}

rounds="${HWATU_SOAK_ROUNDS:-50}"
warmup="${HWATU_SOAK_WARMUP:-5}"
growth_threshold="${HWATU_SOAK_GROWTH_THRESHOLD:-20}"
fail_on_growth="${HWATU_SOAK_FAIL_ON_GROWTH:-0}"
out_dir="${HWATU_SOAK_OUT:-}"
url="${HWATU_SOAK_URL:-}"
keep=0
build=1
self_test=0

parse_positive_int() {
    local name="$1" value="$2"
    if ! [[ "$value" =~ ^[1-9][0-9]*$ ]]; then
        echo "soak-daemon-rss: $name must be a positive integer, got '$value'" >&2
        exit 2
    fi
}

parse_nonnegative_number() {
    local name="$1" value="$2"
    if ! [[ "$value" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
        echo "soak-daemon-rss: $name must be a non-negative number, got '$value'" >&2
        exit 2
    fi
}

while (($#)); do
    case "$1" in
        --rounds)
            [[ $# -ge 2 ]] || { echo "missing value for --rounds" >&2; exit 2; }
            rounds="$2"; shift 2 ;;
        --rounds=*) rounds="${1#*=}"; shift ;;
        --warmup)
            [[ $# -ge 2 ]] || { echo "missing value for --warmup" >&2; exit 2; }
            warmup="$2"; shift 2 ;;
        --warmup=*) warmup="${1#*=}"; shift ;;
        --growth-threshold)
            [[ $# -ge 2 ]] || { echo "missing value for --growth-threshold" >&2; exit 2; }
            growth_threshold="$2"; shift 2 ;;
        --growth-threshold=*) growth_threshold="${1#*=}"; shift ;;
        --fail-on-growth) fail_on_growth=1; shift ;;
        --out)
            [[ $# -ge 2 ]] || { echo "missing value for --out" >&2; exit 2; }
            out_dir="$2"; shift 2 ;;
        --out=*) out_dir="${1#*=}"; shift ;;
        --url)
            [[ $# -ge 2 ]] || { echo "missing value for --url" >&2; exit 2; }
            url="$2"; shift 2 ;;
        --url=*) url="${1#*=}"; shift ;;
        --keep) keep=1; shift ;;
        --no-build) build=0; shift ;;
        --self-test) self_test=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

parse_positive_int "--rounds" "$rounds"
parse_positive_int "--warmup" "$warmup"
parse_nonnegative_number "--growth-threshold" "$growth_threshold"

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release"
metric_name="pss_kb"
metric_method="process_tree_smaps_rollup_pss"

json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

write_summary() {
    local path="$1" status="$2" rounds_done="$3" baseline="$4" final="$5" peak="$6" threshold="$7" fail_gate="$8" url_value="$9" csv_path="${10}" metric="${11}" method="${12}"
    local growth_kb=$((final - baseline))
    local exceeded=false
    awk -v b="$baseline" -v f="$final" -v t="$threshold" 'BEGIN { exit !(b > 0 && ((f - b) * 100 / b) > t) }' && exceeded=true || exceeded=false
    local growth_pct
    growth_pct=$(awk -v b="$baseline" -v f="$final" 'BEGIN { if (b <= 0) printf "0.00"; else printf "%.2f", (f - b) * 100 / b }')
    cat > "$path" <<JSON
{
  "status": "$status",
  "rounds": $rounds_done,
  "metric": "$metric",
  "metric_method": "$method",
  "baseline_kb": $baseline,
  "final_kb": $final,
  "peak_kb": $peak,
  "growth_kb": $growth_kb,
  "growth_pct": $growth_pct,
  "growth_threshold_pct": $threshold,
  "threshold_exceeded": $exceeded,
  "fail_on_growth": $fail_gate,
  "url": $(json_escape "$url_value"),
  "samples_csv": $(json_escape "$csv_path")
}
JSON
}

process_tree_pids() {
    local root_pid="$1"
    python3 - "$root_pid" <<'PY'
import os, sys
root = sys.argv[1]
children = {}
for pid in os.listdir('/proc'):
    if not pid.isdigit():
        continue
    try:
        with open(f'/proc/{pid}/stat', encoding='utf-8', errors='replace') as fh:
            stat = fh.read()
        ppid = stat.rsplit(')', 1)[1].split()[1]
    except Exception:
        continue
    children.setdefault(ppid, []).append(pid)
stack = [root]
seen = set()
out = []
while stack:
    pid = stack.pop()
    if pid in seen:
        continue
    seen.add(pid)
    if os.path.exists(f'/proc/{pid}'):
        out.append(pid)
        stack.extend(children.get(pid, []))
print(' '.join(out))
PY
}

measure_tree() {
    local pids pid value total=0 have_pss=0 have_rss=0 name method
    pids="$(process_tree_pids "$daemon_pid")"
    for pid in $pids; do
        if [[ -r "/proc/$pid/smaps_rollup" ]]; then
            value="$(awk '/^Pss:/ { print $2; found=1; exit } END { if (!found) print "" }' "/proc/$pid/smaps_rollup")"
            if [[ -n "$value" ]]; then
                total=$((total + value))
                have_pss=1
                continue
            fi
        fi
        if [[ -r "/proc/$pid/status" ]]; then
            value="$(awk '/^VmRSS:/ { print $2; found=1; exit } END { if (!found) print "" }' "/proc/$pid/status")"
            if [[ -n "$value" ]]; then
                total=$((total + value))
                have_rss=1
            fi
        fi
    done
    if ((have_pss == 1 && have_rss == 0)); then
        name="pss_kb"
        method="process_tree_smaps_rollup_pss"
    elif ((have_pss == 1 && have_rss == 1)); then
        name="mixed_pss_rss_kb"
        method="process_tree_smaps_rollup_pss_with_status_rss_fallback"
    else
        name="rss_kb"
        method="process_tree_status_vmrss_fallback"
    fi
    printf '%s %s %s\n' "$total" "$name" "$method"
}

kill_owned_tree() {
    local root_pid="$1" pids
    [[ -n "$root_pid" ]] || return 0
    pids="$(process_tree_pids "$root_pid")"
    [[ -n "$pids" ]] || return 0
    # Only kill the hwatud process we spawned and descendants discovered
    # through /proc parent links. Never use global name or process-group kills.
    kill $pids >"$work/cleanup-owned-tree.log" 2>&1 || true
}

run_self_test() {
    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-rss-soak-test.XXXXXX")"
    trap 'rm -rf "$tmp"' RETURN
    write_summary "$tmp/summary.json" ok 3 1000 1100 1120 20 0 "file:///fixture.html" "$tmp/samples.csv" pss_kb process_tree_smaps_rollup_pss
    python3 - "$tmp/summary.json" <<'PY'
import json, sys
summary = json.load(open(sys.argv[1], encoding="utf-8"))
required = {
    "status", "rounds", "metric", "metric_method", "baseline_kb",
    "final_kb", "peak_kb", "growth_kb", "growth_pct",
    "growth_threshold_pct", "threshold_exceeded", "fail_on_growth",
    "url", "samples_csv",
}
assert required <= set(summary), sorted(required - set(summary))
assert summary["status"] == "ok"
assert summary["rounds"] == 3
assert summary["metric"] == "pss_kb"
assert summary["metric_method"] == "process_tree_smaps_rollup_pss"
assert summary["baseline_kb"] == 1000
assert summary["final_kb"] == 1100
assert summary["peak_kb"] == 1120
assert summary["growth_kb"] == 100
assert summary["growth_pct"] == 10.0
assert summary["threshold_exceeded"] is False
assert summary["fail_on_growth"] == 0
PY
    if "$0" --rounds 0 --no-build >"$tmp/out" 2>"$tmp/err"; then
        echo "self-test: expected --rounds 0 to fail" >&2
        return 1
    fi
    grep -q -- "--rounds" "$tmp/err"
    "$0" --help >"$tmp/help"
    grep -q -- "--fail-on-growth" "$tmp/help"
    echo "self-test: ok"
}

if ((self_test)); then
    run_self_test
    exit 0
fi

if ((build)); then
    if [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
        echo "soak-daemon-rss: building release binaries..." >&2
        cargo build --release --manifest-path "$root/Cargo.toml" >&2
    fi
elif [[ ! -x "$bin/hwatu" || ! -x "$bin/hwatud" ]]; then
    echo "soak-daemon-rss: missing target/release/hwatu or hwatud; rerun without --no-build" >&2
    exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-rss-soak.XXXXXX")"
if [[ -z "$out_dir" ]]; then
    out_dir="$work/out"
fi
mkdir -p "$out_dir"

server_pid=""
daemon_pid=""
wid=""
cleanup() {
    set +e
    if [[ -n "$wid" ]]; then "$bin/hwatu" close "$wid" >"$work/cleanup-close.log" 2>&1; fi
    if [[ -n "$daemon_pid" ]]; then "$bin/hwatu" quit >"$work/cleanup-quit.log" 2>&1; fi
    [[ -n "$daemon_pid" ]] && kill_owned_tree "$daemon_pid"
    [[ -n "$server_pid" ]] && kill "$server_pid" >"$work/cleanup-server.log" 2>&1
    if ((keep == 0)); then rm -rf "$work"; fi
}
trap cleanup EXIT

export XDG_RUNTIME_DIR="$work/run"
export XDG_STATE_HOME="$work/state"
export XDG_CONFIG_HOME="$work/config"
export XDG_DATA_HOME="$work/data"
export XDG_CACHE_HOME="$work/cache"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"

if [[ -z "$url" ]]; then
    fixture_dir="$work/fixture"
    mkdir -p "$fixture_dir"
    cat > "$fixture_dir/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>hwatu deterministic memory soak fixture</title>
<style>
  body { margin: 0; font: 16px system-ui, sans-serif; background: #10141f; color: white; }
  main { display: grid; grid-template-columns: repeat(8, minmax(120px, 1fr)); gap: 12px; padding: 24px; }
  article { min-height: 90px; border-radius: 14px; padding: 14px; background: linear-gradient(135deg, #25304f, #14213d); box-shadow: 0 6px 20px #0007; }
  .spark { height: 6px; border-radius: 99px; background: linear-gradient(90deg, #8ecae6, #ffb703); transform: scaleX(var(--n)); transform-origin: left; }
</style>
<h1>hwatu deterministic memory soak fixture</h1>
<main id="cards"></main>
<script>
const cards = document.querySelector('#cards');
for (let i = 0; i < 160; i++) {
  const el = document.createElement('article');
  el.innerHTML = `<h2>Card ${i}</h2><p>${'agent verification '.repeat(12)}</p><div class="spark" style="--n:${(i % 17 + 1) / 17}"></div>`;
  cards.appendChild(el);
}
document.body.dataset.ready = 'true';
</script>
HTML
    python3 - "$fixture_dir" <<'PY' >"$work/http.log" 2>&1 &
import functools
import http.server
import os
import socketserver
import sys

fixture_dir = sys.argv[1]
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=fixture_dir)
with socketserver.TCPServer(("127.0.0.1", 0), handler) as httpd:
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()
PY
    server_pid=$!
    port=""
    for _ in $(seq 100); do
        if [[ -s "$work/http.log" ]]; then
            port="$(head -n 1 "$work/http.log")"
            [[ "$port" =~ ^[0-9]+$ ]] && break
        fi
        sleep 0.05
    done
    [[ "$port" =~ ^[0-9]+$ ]] || { echo "soak-daemon-rss: fixture server did not start" >&2; exit 1; }
    url="http://127.0.0.1:$port/index.html"
fi

"$bin/hwatud" >"$out_dir/hwatud.log" 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do
    [[ -S "$XDG_RUNTIME_DIR/hwatu.sock" ]] && break
    sleep 0.05
done
"$bin/hwatu" ping >"$work/ping.log" 2>&1

open_out="$($bin/hwatu --headless "$url")"
wid="$(awk '{print $2}' <<<"$open_out")"
[[ "$wid" =~ ^[0-9]+$ ]] || { echo "soak-daemon-rss: could not parse window id from: $open_out" >&2; exit 1; }
"$bin/hwatu" wait-load --id "$wid" --until dom >"$work/wait-load.log" 2>&1

samples="$out_dir/samples.csv"

run_round() {
    local round="$1"
    "$bin/hwatu" goto --id "$wid" --until dom "$url?round=$round" >"$work/goto-$round.log" 2>&1
    "$bin/hwatu" shot --id "$wid" "$out_dir/round-$round.png" >"$work/shot-$round.log" 2>&1
    "$bin/hwatu" snapshot --id "$wid" >"$work/snapshot-$round.log" 2>&1
}

for i in $(seq "$warmup"); do
    run_round "warmup-$i"
done

read -r baseline metric_name metric_method < <(measure_tree)
peak="$baseline"
echo "round,${metric_name}" > "$samples"
echo "0,$baseline" >> "$samples"
echo "soak-daemon-rss: baseline after $warmup warmup rounds: ${baseline} ${metric_name} (${metric_method})"

for i in $(seq "$rounds"); do
    run_round "$i"
    read -r memory sample_metric_name sample_metric_method < <(measure_tree)
    metric_name="$sample_metric_name"
    metric_method="$sample_metric_method"
    ((memory > peak)) && peak="$memory"
    echo "$i,$memory" >> "$samples"
    echo "JCODE_PROGRESS {\"message\":\"round $i/$rounds ${metric_name} ${memory} kB\"}"
done

read -r final metric_name metric_method < <(measure_tree)
summary="$out_dir/summary.json"
write_summary "$summary" ok "$rounds" "$baseline" "$final" "$peak" "$growth_threshold" "$fail_on_growth" "$url" "$samples" "$metric_name" "$metric_method"
growth_pct="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["growth_pct"])' "$summary")"
exceeded="$(python3 -c 'import json,sys; print(str(json.load(open(sys.argv[1]))["threshold_exceeded"]).lower())' "$summary")"

echo "soak-daemon-rss: ${metric_name} ${baseline} -> ${final} kB (peak ${peak} kB, growth ${growth_pct}%)"
echo "soak-daemon-rss: wrote $summary"

if [[ "$exceeded" == "true" && "$fail_on_growth" == "1" ]]; then
    echo "FAIL: memory growth ${growth_pct}% exceeds ${growth_threshold}%" >&2
    exit 1
fi
if [[ "$exceeded" == "true" ]]; then
    echo "WARN: memory growth ${growth_pct}% exceeds ${growth_threshold}%; not failing without --fail-on-growth" >&2
else
    echo "OK: memory growth ${growth_pct}% within ${growth_threshold}% threshold"
fi
