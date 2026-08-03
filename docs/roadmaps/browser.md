# Tiling-WM browser roadmap

Status: current as of 2026-08. This is the plan of record for Hwatu as a
keyboard-driven, window-per-page browser for tiling window managers. Portfolio
policy and cross-product priority live in the [roadmap index](../roadmap.md).

## Outcome

A tiling-WM user can use Hwatu as a credible primary browser without importing
the tab-manager model or product churn of a conventional browser. Browser work
consumes the shared runtime and may expose reusable capabilities through the
platform contract, but browser-shell policy must not leak into agent
verification.

Adopted 2026-08 after a three-track research pass: a code audit of
what exists vs what a daily driver needs, user-testimony research on
why people adopt and abandon minimal keyboard browsers (qutebrowser,
vimb, luakit, nyxt), and a WebKitGTK 2.52 capability audit of what
the engine gives us for free.

The evidence pattern: people adopt this category for keyboard-first
UX, minimal chrome, and config-as-dotfile, and abandon it over (in
order) ad quality, broken sites, password friction, and video. The
window-per-page model itself has vocal demand and no polished
incumbent. So the plan is: fix what is silently broken first, then
ship the category-defining features, then close the abandonment
drivers.

## Priorities

### D0: silently broken or one-line-cheap (engine already does it)

Each of these is small because WebKitGTK 2.52 already implements the
hard part; hwatud just never connected the signal or flipped the
setting.

H1. **File uploads.** `run-file-chooser` is unhandled, so `<input
    type=file>` does nothing today. Any upload flow (attaching a file
    to an email, a GitHub issue, a form) is dead. Connect the signal
    to a GTK file dialog (~80 lines). Highest urgency in this tier:
    it is invisible until it bites, then it forces a browser switch.
H2. **WebRTC calls.** `enable-webrtc` defaults OFF; one
    `set_enable_webrtc(true)` plus documenting the `gst-plugins-bad`
    runtime dependency turns on Meet/Discord/Zoom-web calls. The
    permission prompt plumbing already exists. Verify live on
    meet.google.com; per-site UA (`siteua.rs`) covers UA-sniffing
    services.
H3. **Hardware video decode.** Zero code: VA-API decode arrives with
    `gst-plugin-va` (in gst-plugins-bad) + the vendor driver.
    Document it in install/doctor; consider a `hwatu doctor` probe.
    Big battery and smoothness win on laptops, and the same package
    install as H2.
H4. **Web notifications.** Permission prompt exists but
    `show-notification` is unhandled and the Arch build does not link
    libnotify, so grants are silent. Forward to
    `org.freedesktop.Notifications` over D-Bus and route `clicked` to
    window focus (~100 lines).
H5. **Persistent per-site decisions.** Permission grants
    (`prompts.rs` Memory) are daemon-lifetime RAM; every restart
    re-asks mic/cam/notification questions. Persist the
    (host, kind) -> bool map to disk. Add per-site zoom persistence
    on the same store.
H6. **Spell check.** Enchant backend is present; two lines
    (`set_spell_checking_enabled` + languages) plus a config key.
H7. **Printing.** Connect the `print` signal to a
    `WebKitPrintOperation` (~30 lines) and bind Ctrl+P. Print-to-PDF
    comes free.
H8. **PDF viewing.** WebKitGTK ships pdf.js enabled by default;
    verify `decide-policy` does not intercept `application/pdf`
    into a download before the viewer sees it.

### D1: category-defining features (what the audience switches for)

Ordered by user-testimony criticality crossed with implementation
cost. These reverse specific entries in the old not-planned list;
the reversal is deliberate.

H9. **Global history + URL completion.** The single most-missed
    feature in the audit: the bar is a bare GTK Entry and nothing
    records visited URLs, so every navigation is retyped. Store
    (url, title, visit_count, last_visit) in SQLite next to the
    cookie store; fuzzy-complete in the bar and the command palette.
    "Press o and have access to the world" is the retention feature
    of this category.
H10. **Link hints.** Keyboard navigation to links is table stakes
    for the audience (qutebrowser `f`). All the pieces exist: the JS
    injection infra (automation.rs), the interactables enumeration
    (snapshot machinery), and the bar for hint input. Variants can
    wait; plain follow + open-in-new-window + yank-href cover most
    use.
H11. **Password manager integration.** The most-cited "almost
    perfect but..." gap in competitor testimony. First-class fill
    from `pass`, KeePassXC, and Bitwarden CLIs: a fill action that
    shells out, matches by host, and types username/password (TOTP
    next). No sync, no storage of our own, ever.
H12. **Undo close window.** Ctrl+W with no undo loses the page and
    its history; with WM-as-tab-bar this is a daily event. Keep an
    N-deep closed-window stack (URL + serialized history blob, the
    discard machinery already serializes it) and bind Ctrl+Shift+T.
H13. **Quickmarks + search keywords.** Named shortcuts (`:open
    foodrecipes`) and per-engine keywords (`w foo` searches
    Wikipedia) in the existing search.conf/palette machinery. Cheap,
    universally used.

### D2: abandonment drivers (why people go back to Firefox)

H14. **Cosmetic filtering.** Network-level EasyList blocking exists,
    but the #1 stated reason users leave this category is ad quality:
    element hiding is what makes YouTube/news sites bearable.
    Compile EasyList cosmetic rules to per-site injected CSS (the
    content-extension engine handles the network tier already).
H15. **Forced dark mode.** Chromium-derived darkmode is a praised
    qutebrowser feature with no WebKitGTK equivalent; ship a
    prefers-color-scheme override plus an injected-CSS darkener with
    a per-site toggle, persisted on the H5 store.
H16. **Cookie/site-data management.** Persistence exists; clearing
    does not (no verb, no keybind, only hand-deleting cookies.sqlite).
    Add `hwatu clear-site-data [--host H]` and a palette action.
H17. **mpv hand-off.** The loved mitigation for video gaps: a
    keybind that hands the current URL (or hinted link) to `mpv`.
    Trivial with H10's hint machinery.
H18. **Edit-in-$EDITOR.** Celebrated in every browser of this class:
    edit any textarea in the user's editor and paste back on save.
H19. **Session restore to WM workspaces.** Crash-restore exists;
    extend session entries with enough identity (stable per-window
    app_id/title conventions, documented) that WM rules can re-place
    restored windows, and restore on clean quit too (opt-in).

### D3: native-parity shortform scrolling

Adopted 2026-08-01 after a three-agent research pass comparing native
Reels/Shorts/TikTok clients against the mobile-web feeds in hwatu;
full findings in
[research-shortform-native-parity.md](../research-shortform-native-parity.md).
The headline: native apps do NOT crossfade between reels — the
seamlessness is commit-time playback handoff plus in-memory adjacent
preload, both replicable from injected user scripts because hwatu
already owns the gesture (smoothwheel's snap pager and synthetic IG
swipe). Ordered by seamlessness per unit effort:

H20. **Commit-time playback handoff.** Native clients start the
    incoming video the moment the gesture commits, under the still-
    running transition, and hard-cut the outgoing audio (tens-of-ms
    ramp against clicks). Web feeds wait for IntersectionObserver
    after settle; hwatu's IG swipe path adds a 650ms absorb on top,
    so audio audibly overlaps. **Shipped 2026-08-01** (smoothwheel
    `handoffPlayback`): at synthetic pointerup / snap-animation
    start, `play()` the incoming card's video and ramp-out+pause the
    outgoing one (volume 1→0 over ~40ms). Idempotent with the site's
    own observer; fail-open. No crossfade: native hard-cuts too.
H21. **Touchpad guard on Instagram Reels.** Precise two-finger
    deltas bypassed the swipeFeed protection (only discrete wheel
    was claimed) and silently desynced IG's gesture-state feed.
    **Shipped 2026-08-01** (smoothwheel `preciseFeedScroll`): precise
    deltas on the IG feed are claimed, accumulated to a flick's worth
    (120px within 300ms), and paged via the same synthetic swipe.
H22. **reelwarm adjacent prefetch.** Native keeps N±1 in memory
    (Media3 PreloadManager). Browser equivalent: user script Range-
    fetches ~1MB (moov + first GOP) of the N±1 cards' video URLs
    into WebKit's network-process cache (IG mobile and TikTok are
    progressive MP4 — warmable), and forces `preload="metadata"` on
    the N+1 element only so GStreamer prerolls one decoded frame.
    Never ±2+: >3-4 concurrent pipelines is the documented deadlock
    zone.
H23. **Settle-curve velocity + extent bounce.** Snap settle is a
    fixed 350ms ease-in-out from zero velocity; native settles start
    at gesture velocity (reuse `easeWithSlope`, ~300ms ease-out).
    Ticks at feed extents are silently eaten; add an iOS-style
    rubber-band transform bounce (c=0.55).
H24. **Shortform verb batch.** Like, share, save, profile, and
    keyboard seek (`,`/`.` = ±2s — IG mobile web has no scrubber at
    all), plus Esc closing the comment sheet. All via the existing
    aria-label matching in smoothwheel.
H25. **MPRIS bridge.** `org.mpris.MediaPlayer2.hwatu` driving the
    shortform play/pause via the existing JS-eval IPC (~150 lines);
    makes playerctl and headset keys work.
H26. **Quality + decode audit.** H3 hardware decode multiplies every
    latency number here. Then: codec advertisement (canPlayType for
    vp9/av1), honest devicePixelRatio (ABR picks rungs by element
    size × dpr), desktop-UA option for youtube shorts (quality menu),
    and verifying IG's ladder under the iPhone UA live.
H27. **Long-session hygiene.** Focused windows never discard, so a
    2h reel session accumulates; add memory telemetry (observe.rs),
    then `WebKitMemoryPressureSettings` and a soft reload-to-current-
    reel-URL past an RSS threshold (shortform URLs carry position,
    so the reload is near-seamless). Same property enables resume:
    persist {url, currentTime} on discard.

Non-gap worth stating: focusshield already beats native on background
audio (WM focus loss never pauses playback), and the compositing path
is measured solid (142.9fps). No crossfade will be added — native
does not have one either.

### Engine-bound gaps (documented honestly, not planned)

- **Widevine/DRM:** WebKitGTK has ClearKey only; no distro ships
  OpenCDM for the GTK port. Netflix/Spotify-web will not play.
  Mitigation: document it; keep a fallback-browser keybind.
- **WebAuthn/passkeys:** not exposed by the GTK port at all. Track
  upstream; sites that hard-require passkeys need the fallback
  browser. This will grow into a real problem and the roadmap should
  re-check upstream status quarterly.
- **Anti-bot walls (Akamai/Cloudflare):** a growing category killer
  for WebKitGTK browsers generally. Per-site UA helps; beyond that
  it is upstream's fight, and `challenge` hand-off is the honest
  answer.

### Still not planned (churn magnets with no constituency here)

- Tabs (the WM tiles), sync, extensions/WebExtensions as a platform
  (specific needs get native features or the userscript escape
  hatch), a password store of our own (we integrate, never store),
  Widevine workarounds.

The old blanket non-goals list claimed link hints, history+completion,
bookmarks, password integration, and per-site settings would never
happen; the daily-driver decision reverses those specific entries,
and this section is the plan of record for them now.
