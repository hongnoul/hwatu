#!/usr/bin/env bash
set -uo pipefail

# Screen the phrases in .astrophile/geo-prompts.txt with Agent Reach's
# selected search backend. Search output is untrusted public data, so this
# script only writes a bounded artifact. It never evaluates or commits it.

ROOT=${HWATU_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
PROMPTS=${HWATU_SCREEN_PROMPTS:-$ROOT/.astrophile/geo-prompts.txt}
OUTPUT=${HWATU_SCREEN_OUTPUT:-$ROOT/.astrophile/public-signal-screen.jsonl}
MAX_RESULTS=${HWATU_SCREEN_RESULTS:-5}
MAX_CHARS=${HWATU_SCREEN_MAX_CHARS:-25000}
MCPORTER=${MCPORTER:-mcporter}
GH=${GH:-gh}

if ! command -v "$MCPORTER" >/dev/null 2>&1; then
    printf 'screen-public-signals: mcporter is not installed\n' >&2
    exit 2
fi
if [[ ! -r "$PROMPTS" ]]; then
    printf 'screen-public-signals: cannot read prompts: %s\n' "$PROMPTS" >&2
    exit 2
fi
if ! [[ "$MAX_RESULTS" =~ ^[1-9][0-9]*$ && "$MAX_CHARS" =~ ^[1-9][0-9]*$ ]]; then
    printf 'screen-public-signals: result and character limits must be positive integers\n' >&2
    exit 2
fi

mkdir -p "$(dirname "$OUTPUT")"
tmp=$(mktemp "${OUTPUT}.tmp.XXXXXX")
trap 'rm -f "$tmp"' EXIT

checked=0
found=0
failed=0
while IFS= read -r query || [[ -n "$query" ]]; do
    [[ -z "$query" || "$query" == \#* ]] && continue
    checked=$((checked + 1))

    args=$(python3 - "$query" "$MAX_RESULTS" <<'PY'
import json
import sys
print(json.dumps({"query": sys.argv[1], "numResults": int(sys.argv[2])}))
PY
)
    backend=exa
    if raw=$(
        "$MCPORTER" call exa.web_search_exa \
            --args "$args" --output json --timeout 30000 2>&1
    ); then
        status=ok
    else
        exa_error=$raw
        backend=github
        if command -v "$GH" >/dev/null 2>&1 && raw=$(
            "$GH" api -X GET search/repositories \
                -f "q=$query in:name,description,readme" \
                -f "per_page=$MAX_RESULTS" 2>&1
        ); then
            status=ok_fallback
        else
            status=error
            raw=$(printf 'Exa: %s\nGitHub: %s' "$exa_error" "$raw")
            failed=$((failed + 1))
        fi
    fi

    record=$(python3 - "$query" "$status" "$backend" "$MAX_CHARS" "$raw" <<'PY'
import json
import sys

query, status, backend, max_chars, raw = sys.argv[1:]
raw = raw[: int(max_chars)]
mentions_hwatu = "hwatu" in raw.casefold()
print(json.dumps({
    "query": query,
    "status": status,
    "backend": backend,
    "mentions_hwatu": mentions_hwatu,
    "result": raw,
}, ensure_ascii=False))
PY
)
    printf '%s\n' "$record" >>"$tmp"
    [[ "$record" == *'"mentions_hwatu": true'* ]] && found=$((found + 1))
done <"$PROMPTS"

mv "$tmp" "$OUTPUT"
trap - EXIT
printf 'screen-public-signals: checked=%d mentions=%d errors=%d output=%s\n' \
    "$checked" "$found" "$failed" "$OUTPUT"

# Search outages and free-tier rate limits are observations, not workflow
# failures. A structurally invalid invocation still exits above.
exit 0
