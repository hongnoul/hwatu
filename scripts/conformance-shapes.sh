#!/usr/bin/env bash
# Print normalized replies for a fixed command sequence on
# tests/conformance/snap.html. scripts/test-conformance.sh compares the
# output with tests/conformance/shapes.golden, so every engine must
# answer each command with the same JSON. Timings and window ids are
# normalized.
# Usage: scripts/conformance-shapes.sh <hwatu-binary> <fixture-base-url>
set -u
H=$1; B=$2
norm() { sed -E "s#$B#BASE#g; s/after [0-9]+ ms/after N ms/g; s/\"(elapsed_ms|id)\":[0-9]+/\"\1\":N/g; s/^window [0-9]+ -> (.*) \([0-9]+ ms\)$/window N -> \1/"; }
run() { echo "== $*" | sed -E "s/--id [0-9]+/--id N/; s/\\\"id\\\":[0-9]+/\\\"id\\\":N/g"; "$H" "$@" 2>&1 | grep -v "daemon build .* != client build" | norm; }
id=$("$H" "$B/snap.html" | grep -o 'window [0-9]*' | head -1 | cut -d' ' -f2)
"$H" wait-load --id "$id" >/dev/null
run snapshot --id "$id"
run content --id "$id" --format text
run content --id "$id" --format links
run content --id "$id" --format title
run content --id "$id" p
run click --id "$id" button --contains Add 1
run click --id "$id" --ref 3
run type --id "$id" '#q' hello
run type --id "$id" '#q' x --no-clear
run press --id "$id" Tab
run scroll --id "$id" --by 1
run scroll --id "$id" '#s'
run expect --id "$id" '#add1'
run expect --id "$id" '#nope' --absent
run expect --id "$id" '#add1' --text Add
run eval --id "$id" '1+1'
run fill --id "$id" --field '#q=z'
run auth-context --id "$id"
run challenge --id "$id"
run snapshot --id "$id" --diff
"$H" eval --id "$id" 'document.querySelector("h1").textContent = "Shop"; return 1' >/dev/null
run snapshot --id "$id" --diff
run snapshot --id "$id" --budget 300
"$H" goto --id "$id" "$B/snap.html" >/dev/null
run batch "[{\"cmd\":\"type\",\"id\":$id,\"selector\":\"#q\",\"text\":\"b\"},{\"cmd\":\"expect\",\"id\":$id,\"selector\":\"#add1\"},{\"cmd\":\"eval\",\"id\":$id,\"js\":\"return 7\"}]"
run batch "[{\"cmd\":\"eval\",\"id\":$id,\"js\":\"return 1\"},{\"cmd\":\"expect\",\"id\":$id,\"selector\":\"#nope\",\"timeout_ms\":300},{\"cmd\":\"eval\",\"id\":$id,\"js\":\"return 2\"}]"
# Eval results that JSON cannot express directly.
for js in 'document.body' 'return () => 1' 'return Symbol("x")' 'throw new Error("boom")' 'return 1n' 'return new Map([[1,2]])' 'return undefined' 'return new Date(0)' 'const a={}; a.self=a; return a'; do
  run eval --id "$id" "$js"
done
"$H" close "$id" >/dev/null
