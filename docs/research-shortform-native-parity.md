# Research: native short-form parity (Reels / Shorts / TikTok)

Date: 2026-08-01. Method: three parallel research agents (gesture and
scroll mechanics, media pipeline, UI/session/performance) comparing
native Android/iOS clients (including Meta's published Media3
PreloadManager engineering writeup) against the mobile-web versions of
the same feeds running in hwatu (WebKitGTK 2.52.5 + GStreamer 1.28.5),
grounded in the current code (`smoothwheel.rs`, `mediashim.rs`,
`window.rs`, `siteua.rs`, `focusshield.rs`, `session.rs`).

Feeds referenced: instagram.com Reels (mobile-UA spoof per
`siteua.rs`), m.youtube.com Shorts, tiktok.com.

> Status note: this is a point-in-time snapshot. A1/A2 (commit-time
> handoff, roadmap H20) and B1 (IG touchpad guard, H21) shipped on
> main the same day (`handoffPlayback` / `preciseFeedScroll` in
> smoothwheel); the "today" columns below describe the pre-fix state.

## The headline finding

Native apps do **not** crossfade video or audio between reels. Audio
overlap between two reels is universally perceived as a bug. The
perceived seamlessness comes from three other mechanisms:

1. **Gesture-coupled rendering**: both video surfaces translate with
   the finger; the incoming video's first frame is already decoded
   and visible as it slides in.
2. **Commit-time playback handoff**: the incoming video starts
   playing the moment the gesture crosses the paging threshold
   (~50%), under the still-running transition animation, hiding
   200-350 ms of start latency. The outgoing audio is hard-cut at
   commit (at most a tens-of-ms ramp to avoid a click).
3. **Adjacent preload**: Media3 `DefaultPreloadManager` keeps the
   next/previous videos ready *in memory* (buffered samples for the
   codec, not just disk cache), so time-to-first-frame on swipe is
   near zero. Meta reports tiered readiness: N±1 gets ~5s/~3s of
   samples, N±2 track selection, N±4 prepared source.

Mobile-web feeds instead start the incoming video only after the
scroll settles and an IntersectionObserver fires. The dead gap
(old audio stops → silence → settle → load → play) is what makes the
browser version feel discrete. In hwatu the Instagram path is
currently *worse* than plain web: the synthetic swipe (~160 ms) plus
the 650 ms `SWIPE_ABSORB` window run before IG's observer even fires,
so outgoing audio audibly overlaps the transition.

## Findings by area

Severity is impact on a seamless scrolling session.

### A. Playback handoff and preload (media pipeline)

| # | Aspect | Native | hwatu/web today | Sev | Suggestion |
|---|--------|--------|-----------------|-----|------------|
| A1 | Start point | Play at gesture commit, under the animation | Play after settle via site IntersectionObserver; IG path adds swipe+absorb delay | HIGH | In smoothwheel's shortform layer, at synthetic pointerup (IG) or snap-animation start (CSS-snap feeds), find the incoming card's `<video>` and `play()` immediately; pause the outgoing one. Idempotent with the site's later observer call; fail-open. ~50 lines. |
| A2 | Audio cut | Hard cut at commit, tens-of-ms ramp against clicks | Outgoing audio keeps playing 300-700 ms into the transition | MED | Same hook: ramp outgoing `volume` 1→0 over ~40 ms (4 rAF steps) then `pause()`; incoming starts at 0 and ramps to 1 after `playing`. Coalesce play/pause to one per frame per element (GStreamer `changePipelineState` main-thread wedge, see `mediashim.rs` history). No true crossfade: native doesn't do one. |
| A3 | Adjacent preload | N±1 samples in memory (Media3 PreloadManager) | Site-controlled only; at best first bytes in HTTP cache | HIGH | "reelwarm" user script: extract N±1 cards' video URLs (`src`, `<source>`, or PerformanceObserver resource entries) and `fetch` with `Range: bytes=0-1048575` to pull moov + first GOP into WebKit's network-process disk cache. Media loads go through the same cache, so preroll becomes a cache hit. No pipeline objects created (avoids the multi-pipeline deadlock class in `mediashim.rs`). |
| A4 | First-frame pre-decode | Cover frame always ready; never black | IG recycles elements and re-srcs on swipe, losing WebKit's `preload="metadata"` preroll | MED | Force `preload="metadata"` on the N+1 card only. Never ±2+: each preroll is a live GStreamer pipeline and >3-4 concurrent pipelines is the documented deadlock/perf zone. `requestVideoFrameCallback` can verify frame arrival. |
| A5 | Loop wrap | Gapless repeat-one from memory | WebKit uses SEGMENT seeks when seamless seeking is on; older path was a flushing seek-to-0 hiccup every loop | LOW-MED | Empirically check 2.52.5 with `GST_DEBUG=webkitmediaplayer:6`. If flushing, not fixable from a user script; track upstream. |
| A6 | Autoplay gating | N/A | hwatu already overrides WebKit's ALLOW_WITHOUT_SOUND to full Allow | NONE | Already ahead; keep. |
| A7 | Stream format per site | — | IG (iPhone UA): progressive MP4 fallback if `ManagedMediaSource` absent — Range warming targets this perfectly. TikTok: progressive MP4, signed Range URLs — warmable. YT Shorts: MSE (SABR/UMP) under desktop UA; keep youtube out of `HWATU_MOBILE_UA_SITES` (iOS UA would push it to native HLS via gst hlsdemux, shakier than MSE) | — | Verify `typeof ManagedMediaSource` under the IG UA live before building. |

### B. Gesture and scroll mechanics

| # | Aspect | Native | hwatu today | Sev | Suggestion |
|---|--------|--------|-------------|-----|------------|
| B1 | Precise touchpad on IG Reels | Finger-tracked paging | Two-finger touchpad scroll bypasses the swipeFeed protection (only discrete wheel ticks are claimed), silently breaking IG's gesture-state feed | HIGH | Extend the IG feed guard to claim precise deltas too: accumulate them and translate to synthetic swipes (or at minimum absorb them so the feed doesn't desync). |
| B2 | Settle curve | Settle starts at gesture velocity, decelerates | Fixed 350 ms ease-in-out from zero velocity | MED-HIGH | Reuse `easeWithSlope` with a synthetic initial slope; ~300 ms ease-out. |
| B3 | Feed extents | Rubber-band bounce | Ticks silently consumed, zero feedback | MED | Transform-based bounce using the iOS rubber-band formula (c=0.55) on the feed container. |
| B4 | Synthetic IG swipe feel | Eased, velocity-shaped | Constant-velocity pointer spacing, fixed 650 ms absorb | LOW-MED | Ease-out pointer spacing; release the absorb early on URL/mutation change instead of the fixed timer. |
| B5 | Wheel glide, retarget, keyboard scheme | — | Chromium-curve animator, velocity-preserving retarget, absorb window, unified shortform keys | NONE | Already matches or beats desktop-web comparators. |

### C. UI chrome, session, system

| # | Aspect | Native | hwatu/web today | Sev | Suggestion |
|---|--------|--------|-----------------|-----|------------|
| C1 | Hardware video decode | Mandatory | Software unless `gst-plugin-va` + driver installed (roadmap H3) | HIGH (power), MED (smoothness) | Ship H3: doctor probe, docs, AUR optdepends. Software decode inflates every TTFF number in section A. |
| C2 | Stream quality | Top ladder (AV1/HEVC 1080p+) | IG serves a reduced H.264/HLS ladder to the iPhone-Safari UA; YT web ladder depends on MSE+VP9/AV1 advertisement; soft upscale on big monitors | MED-HIGH | Audit: (i) gst `canPlayType`/MediaCapabilities answers for vp9/av1, (ii) desktop-Chrome UA on youtube.com/shorts gets the desktop ladder + quality menu, (iii) honest devicePixelRatio (ABR picks rungs by element size × dpr), (iv) mpv+yt-dlp hand-off (H17) for the true top rung. Verify ladders live first. |
| C3 | Missing interaction verbs | Double-tap like, share, scrub, save | Only play/pause, 2x hold, comments, mute | MED | One batch via the existing aria-label matching (`nearestButton()`): like, share, save, profile, plus keyboard seek (`,`/`.` = ±2 s on `activeShortformVideo()`) — fixes IG mobile web having no scrubber at all. Bind Esc as a second comment-sheet close. |
| C4 | Media keys | Lock-screen/headset controls | No MediaSession→MPRIS bridge | MED | Minimal `org.mpris.MediaPlayer2.hwatu` D-Bus object driving `toggleShortformPlayback()` via the existing JS-eval IPC (~150 lines); makes `playerctl` work. |
| C5 | Resume position | Reopens mid-feed with per-video resume | session.json restores URL only; shortform URLs carry the current reel, so last-URL ≈ resume | MED | Persist `{url, currentTime}` on discard, seek on restore via user script. |
| C6 | Picture-in-picture | System PiP tile | W3C PiP not implemented in the GTK port | MED-HIGH | hwatu-level instead of engine-level: `hwatu open --pip` always-on-top 9:16 window + WM float rule, or the H17 mpv hand-off. |
| C7 | Long-session memory | Recycler views, 3-5 pooled surfaces | Discard (120 s) only fires on unfocused windows; a focused 2 h session never discards | MED | (i) `WebKitMemoryPressureSettings` on the WebContext; (ii) soft-reload-to-current-reel-URL past an RSS threshold (URL carries position → near-seamless); (iii) telemetry via observe.rs first to quantify. |
| C8 | Compositing during transitions | 60-120 fps | Largely solved (Chromium-curve animator, snap suspend, blurshield, DMABuf off = 142.9 vs 58.8 fps) | LOW | Re-test DMABuf + PropagateDamagingInformation each WebKitGTK release (upstream 305560/305758 landed after 2.52). |
| C9 | Background audio on focus loss | Pauses by policy | hwatu is BETTER: focusshield pins `visibilityState`, autoplay Allow avoids the muted fallback | NO GAP | Document as a differentiator. |
| C10 | Fullscreen immersion | Edge-to-edge | Already near-native (no chrome by design, F11) | LOW | Optional per-site kiosk flag using the existing `shortformSite()` classifier. |
| C11 | Haptics | Haptic ticks | No actuator on desktop | NONE | Not addressable; the fixed snap cadence is the perceptual substitute. |

## Recommended order (seamlessness per unit effort)

1. **Commit-time playback handoff** (A1+A2): play incoming / ramp-out
   outgoing at gesture commit in smoothwheel. Biggest perceived win,
   ~50 lines, reuses existing plumbing.
2. **Touchpad guard on IG** (B1): the one currently *broken* path.
3. **reelwarm prefetch** (A3+A4): Range-warm N±1, `preload="metadata"`
   on N+1.
4. **Hardware decode** (C1 = existing H3): multiplies every latency
   number above.
5. **Settle-curve velocity** (B2) and extent bounce (B3).
6. **Interaction verb batch** (C3), MPRIS (C4).
7. **Quality audit** (C2), resume timestamp (C5), PiP window (C6),
   memory telemetry (C7).

Caveats: native-behavior claims are from Meta's published writeup,
Android Media3 docs, and training knowledge; IG/YT web bitrate
ladders and the `ManagedMediaSource`/loop-seek behaviors should be
verified live before building on them.
