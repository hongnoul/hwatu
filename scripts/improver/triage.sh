#!/usr/bin/env bash
# Structure triage: mechanical DRY/centralization pass over the whole tree.
# Gate 7 of docs/recursive-improver.md. Prints findings; exits 1 only on
# boundary violations (hard). Duplication findings are reported for the
# agent pass and backlog seeding (soft unless caused by the current diff).
set -uo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

hard=0

echo "== boundary-import lint (portable crates clean of native UI) =="
for crate in crates/ipc crates/hwatu; do
    hits=$(grep -rnE 'use (gtk4?|webkit6|gdk4|glib|cocoa|objc2)(::|;)' "$crate/src" 2>/dev/null || true)
    if [[ -n "$hits" ]]; then
        echo "HARD: native UI import in portable crate $crate:"
        echo "$hits"
        hard=1
    fi
done
[[ $hard -eq 0 ]] && echo "clean"

echo
echo "== cross-product reach-through (browser-shell vs verification modules) =="
# Products must consume platform capabilities, not each other. Heuristic:
# verification-ish modules importing browser-ish ones or vice versa inside hwatud.
# Report-only until the crate split in roadmap.md lands.
echo "(report-only: crate split pending per docs/roadmap.md)"

echo
echo "== duplicate shell preamble across scripts/test-*.sh =="
# Scripts that re-implement the same daemon-bootstrap preamble are DRY debt.
boot_count=$(grep -l 'XDG_RUNTIME_DIR="$work/run"' scripts/test-*.sh 2>/dev/null | wc -l | tr -d ' ')
echo "scripts duplicating private-XDG daemon bootstrap: $boot_count (candidate: extract scripts/lib/daemon-harness.sh)"

echo
echo "== duplicate code blocks (rust, >=${DUP_MIN_LINES:-20} identical lines) =="
python3 - "$root" <<'EOF'
import sys, os, hashlib, collections
root = sys.argv[1]
win = int(os.environ.get("DUP_MIN_LINES", "20"))
index = collections.defaultdict(list)
for dirpath, dirs, files in os.walk(os.path.join(root, "crates")):
    dirs[:] = [d for d in dirs if d != "target"]
    for f in files:
        if not f.endswith(".rs"): continue
        p = os.path.join(dirpath, f)
        lines = [l.strip() for l in open(p, errors="replace")]
        sig = [l for l in lines]
        for i in range(0, max(0, len(sig) - win)):
            block = "\n".join(sig[i:i+win])
            if len(block.strip()) < 200: continue
            h = hashlib.md5(block.encode()).hexdigest()
            index[h].append((os.path.relpath(p, root), i + 1))
seen = set()
found = 0
for h, locs in index.items():
    files = {l[0] for l in locs}
    if len(files) > 1:
        key = tuple(sorted(files))
        if key in seen: continue
        seen.add(key)
        found += 1
        print(f"dup-block across: {', '.join(f'{f}:{n}' for f, n in sorted(set(locs))[:4])}")
if not found:
    print("no cross-file duplicate blocks found")
EOF

echo
echo "== orphan fixtures (fixtures not referenced by any script) =="
orphans=0
for f in scripts/fixtures/*; do
    base=$(basename "$f")
    if ! grep -rq "$base" scripts --include='*.sh' --include='*.mjs' --exclude-dir=fixtures 2>/dev/null; then
        echo "orphan: scripts/fixtures/$base"
        orphans=$((orphans+1))
    fi
done
[[ $orphans -eq 0 ]] && echo "none"

echo
if [[ $hard -eq 1 ]]; then
    echo "triage: HARD boundary violations found"
    exit 1
fi
echo "triage: ok (soft findings above are backlog candidates)"
