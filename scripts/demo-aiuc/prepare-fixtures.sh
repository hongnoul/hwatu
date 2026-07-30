#!/usr/bin/env bash
# Capture the current AIUC homepage HTML for a repeatable local demo take.
# The generated fixtures are intentionally gitignored because they contain
# third-party page content and retain external asset references.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SOURCE=${AIUC_SOURCE_HTML:-}
URL=${AIUC_URL:-https://aiuc.com/}

rm -rf "$HERE/reference" "$HERE/app"
mkdir -p "$HERE/reference" "$HERE/app"
if [ -n "$SOURCE" ]; then
  cp "$SOURCE" "$HERE/reference/index.html"
else
  curl -fsSL --retry 3 --max-time 30 "$URL" -o "$HERE/reference/index.html"
fi
cp "$HERE/reference/index.html" "$HERE/app/index.html"

python3 - "$HERE/reference/index.html" "$HERE/evidence" <<'PY'
import hashlib, json, pathlib, re, sys
from collections import Counter
from urllib.parse import urlparse

page = pathlib.Path(sys.argv[1])
evidence = pathlib.Path(sys.argv[2])
evidence.mkdir(parents=True, exist_ok=True)
raw = page.read_bytes()
text = raw.decode(errors="replace")
urls = re.findall(r'https?://[^"\'\)\\\s<]+', text)
hosts = Counter(urlparse(url).netloc for url in urls)
manifest = {
    "source": "AIUC homepage HTML",
    "sha256": hashlib.sha256(raw).hexdigest(),
    "bytes": len(raw),
    "absolute_url_occurrences": len(urls),
    "external_hosts": dict(hosts.most_common()),
    "self_contained": False,
    "caveat": "HTML is pinned per take; external assets remain live dependencies.",
}
(evidence / "fixture-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps(manifest, indent=2))
PY

