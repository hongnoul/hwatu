#!/usr/bin/env python3
"""materialize.py — turn a capture.json into a self-contained local site.

Reads <outdir>/capture.json (from extract.js), downloads every listed
asset into <outdir>/assets/, rewrites asset URLs in both the HTML and
the inlined CSS to the local copies, injects canvas data URLs back
into their elements, and writes <outdir>/index.html.
"""
import hashlib
import json
import pathlib
import re
import sys
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor

OUT = pathlib.Path(sys.argv[1])
cap = json.loads((OUT / "capture.json").read_text())
if isinstance(cap, dict) and "value" in cap:
    cap = cap["value"]

ASSETS = OUT / "assets"
ASSETS.mkdir(exist_ok=True)

UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"


def local_name(url: str) -> str:
    """Stable local filename: hash + best-effort extension."""
    h = hashlib.sha1(url.encode()).hexdigest()[:16]
    path = urllib.parse.urlparse(url).path
    ext = pathlib.Path(path).suffix[:8]
    if not re.fullmatch(r"\.[A-Za-z0-9]{1,7}", ext or ""):
        ext = ""
    return f"{h}{ext}"


def fetch(url: str) -> tuple[str, bytes | None]:
    try:
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=30) as r:
            return url, r.read()
    except Exception as e:
        print(f"  miss {url}: {e}", file=sys.stderr)
        return url, None


# ---- 1. resolve the ordered stylesheet list -------------------------
# `sheets` preserves document order (cascade order matters). Entries
# are either {base, text} (read via CSSOM) or {href} (cross-origin,
# fetched here). Each sheet's url(...) resolves against its own URL.
all_css: list[tuple[str, str]] = []  # (base_url, text)
for sh in cap.get("sheets", []):
    if "text" in sh:
        all_css.append((sh.get("base", cap["base"]), sh["text"]))
    elif sh.get("href"):
        _, body = fetch(sh["href"])
        if body:
            all_css.append((sh["href"], body.decode("utf-8", "replace")))

# ---- 2. collect asset URLs (page manifest + any url() found in the
#         fetched cross-origin css), then download in parallel --------
assets = set(cap.get("assets", []))
url_re = re.compile(r"url\(\s*(['\"]?)([^'\")]+)\1\s*\)")

def absolutize_css(base: str, text: str) -> str:
    """Rewrite every url(...) in a stylesheet to an absolute URL and
    add it to the asset set, so later local rewriting is uniform."""
    def sub(m: re.Match) -> str:
        u = m.group(2)
        if u.startswith("data:"):
            return m.group(0)
        absu = urllib.parse.urljoin(base, u)
        assets.add(absu)
        return f"url({absu})"
    return url_re.sub(sub, text)

all_css = [absolutize_css(b, t) for b, t in all_css]

mapping: dict[str, str] = {}
with ThreadPoolExecutor(max_workers=16) as ex:
    for url, body in ex.map(fetch, sorted(assets)):
        if body is None:
            continue
        name = local_name(url)
        (ASSETS / name).write_bytes(body)
        mapping[url] = f"assets/{name}"

print(f"fetched {len(mapping)}/{len(assets)} assets", file=sys.stderr)


def rewrite(text: str) -> str:
    """Point absolute asset URLs (and their protocol-relative /
    root-relative spellings) at the local copies."""
    for url, local in sorted(mapping.items(), key=lambda kv: -len(kv[0])):
        text = text.replace(url, local)
        # Root-relative form as it may appear in the original document.
        parsed = urllib.parse.urlparse(url)
        base = urllib.parse.urlparse(cap["base"])
        if parsed.netloc == base.netloc:
            rel = urllib.parse.urlunparse(("", "", parsed.path, parsed.params, parsed.query, ""))
            if rel and rel != "/":
                text = text.replace(f'"{rel}"', f'"{local}"').replace(f"'{rel}'", f"'{local}'")
                # CSS url(/root/relative) without quotes.
                text = text.replace(f"url({rel})", f"url({local})")
    return text


html = rewrite(cap["html"])
css = rewrite("\n".join(all_css))

# ---- 3. canvas freeze: replace each canvas with its captured frame --
for c in cap.get("canvases", []):
    # <canvas ... data-hwatu-canvas="i" ...></canvas> -> <img>
    pat = re.compile(
        rf'<canvas([^>]*data-hwatu-canvas="{c["i"]}"[^>]*)>\s*</canvas>', re.S
    )
    # Pin the rendered CSS box: an <img> otherwise sizes itself by the
    # data-URL's intrinsic aspect ratio, not the canvas's layout.
    size = f' style="width:{c["w"]}px;height:{c["h"]}px"' if c.get("w") else ""
    html = pat.sub(rf'<img\1 src="{c["data"]}"{size}>', html)

# ---- 3.6 scroll restoration + snap disable ---------------------------
# scroll-snap in the clone can land scrollers on a different snap
# point than the captured frame; disable snap and restore recorded
# positions with a minimal inline script (the only JS in the clone).
scrolls = cap.get("scrolls", [])
scroll_js = ""
if scrolls:
    payload = json.dumps([{ "i": s0["i"], "l": s0["left"], "t": s0["top"] } for s0 in scrolls])
    scroll_js = (
        "<script>for (const s of " + payload + ") {"
        "const el = document.querySelector('[data-hwatu-scroll=\"' + s.i + '\"]');"
        "if (el) { el.style.scrollSnapType = 'none'; el.scrollLeft = s.l; el.scrollTop = s.t; }"
        "}</script>"
    )
    if "</body>" in html:
        html = html.replace("</body>", scroll_js + "</body>", 1)
    else:
        html += scroll_js

# ---- 4. inject inlined CSS and write ---------------------------------
style_block = f"<style>\n{css}\n</style>"
if "</head>" in html:
    html = html.replace("</head>", style_block + "\n</head>", 1)
else:
    html = style_block + html

(OUT / "index.html").write_text(html)
print(f"index.html: {len(html)} bytes, css: {len(css)} bytes", file=sys.stderr)
