# macOS WKWebView feasibility probes

Raw probes behind the macOS section of [`docs/roadmaps/native-platforms.md`](../../../docs/roadmaps/native-platforms.md).
They are deliberately standalone: no Xcode project, no `.app` bundle, no
signature. Each is a single `swiftc` file so the result cannot be confused
with something that only works inside an app bundle.

Run one with:

```sh
swiftc -O p.swift -o /tmp/p && /tmp/p
```

Several probes intentionally never exit (they wait on a callback that a
throttled offscreen page never fires). Run those with a timeout:

```sh
swiftc -O raf2.swift -o /tmp/raf2 && ( /tmp/raf2 & p=$!; sleep 10; kill $p )
```

| Probe | Question |
| --- | --- |
| `p.swift` | Can an unbundled CLI load a page offscreen with `.prohibited` activation policy and snapshot real pixels? |
| `w.swift` | Do 8 views on a shared `WKProcessPool` load in parallel, and what are warm eval/snapshot/nav costs? |
| `deep.swift` | Does JS-dispatched clicking work offscreen? Are cookies visible? |
| `raf2.swift` | Does `requestAnimationFrame` tick in an offscreen window (`orderBack` vs `orderFrontRegardless`)? |
| `raf3.swift` | Does a transparent non-activating desktop-level panel restore rAF ticking? (No.) |
| `seek.swift` | Does WAAPI pause + `currentTime` seek actually render the sought instant offscreen? (Yes.) |
| `cook.swift` | Do cookies written by one process persist to a second via `WKWebsiteDataStore.default()`? Run `cook write` then `cook read`. |

Measured results, and what they imply for the roadmap, are in the research
note. The single load-bearing finding: **rAF is throttled to 0-1 fps
offscreen, but WAAPI seek renders correctly**, so hwatu's deterministic
verification model survives while frame-watching approaches do not.
