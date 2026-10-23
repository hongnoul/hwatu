// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Justin Hong
//
// hwatud-mac: macOS daemon for the hwatu CLI, backed by WKWebView.
//
// The Linux daemon (crates/hwatud) is WebKitGTK + Wayland and cannot run on
// macOS. This Swift daemon speaks the same newline-delimited-JSON protocol on
// the same Unix socket path ($XDG_RUNTIME_DIR/hwatu.sock, falling back to
// /tmp/hwatu-$UID.sock), so the stock `hwatu` client works unmodified: it
// spawns `hwatud` from PATH when the socket is dead, and this binary installs
// as ~/.local/bin/hwatud.
//
// Implemented requests: ping, open, list, close, focus, unfocus, navigate,
// eval, wait_load, screenshot, get_content, snapshot, click, type, press,
// fill_form, scroll, console (stub), quit. Everything else answers a clean
// error naming the unsupported command.

import AppKit
import Foundation
import JavaScriptCore
import WebKit

// MARK: - Socket path (mirror of crates/ipc socket_path())

func socketPath() -> String {
    if let p = ProcessInfo.processInfo.environment["HWATU_SOCKET"], !p.trimmingCharacters(in: .whitespaces).isEmpty {
        return p
    }
    if let dir = ProcessInfo.processInfo.environment["XDG_RUNTIME_DIR"], !dir.isEmpty {
        return dir + "/hwatu.sock"
    }
    return "/tmp/hwatu-\(geteuid()).sock"
}

let MAX_FRAME: Int = 32 * 1024 * 1024

/// Persistent cookies and storage for this daemon. The default socket
/// uses the user's default store, so sessions persist across runs. A
/// daemon on another socket (HWATU_SOCKET, XDG_RUNTIME_DIR: tests,
/// isolated agents) gets its own persistent store derived from the
/// socket path, as the GTK daemon's data directory follows XDG.
let sessionStore: WKWebsiteDataStore = {
    let path = socketPath()
    if path == "/tmp/hwatu-\(geteuid()).sock" { return WKWebsiteDataStore.default() }
    if #available(macOS 14.0, *) {
        // Stable UUID from the path (FNV-1a over the bytes, twice).
        var a: UInt64 = 0xcbf29ce484222325, b: UInt64 = 0x84222325cbf29ce4
        for byte in path.utf8 {
            a = (a ^ UInt64(byte)) &* 0x100000001b3
            b = (b ^ UInt64(byte)) &* 0x100000001b3 &+ 1
        }
        var bytes = [UInt8](repeating: 0, count: 16)
        for i in 0..<8 { bytes[i] = UInt8((a >> (8 * UInt64(i))) & 0xff); bytes[8 + i] = UInt8((b >> (8 * UInt64(i))) & 0xff) }
        let uuid = NSUUID(uuidBytes: bytes) as UUID
        return WKWebsiteDataStore(forIdentifier: uuid)
    }
    return WKWebsiteDataStore.default()
}()

// MARK: - JSON helpers

typealias JSON = [String: Any]

func jstr(_ d: JSON, _ k: String) -> String? { d[k] as? String }
func jint(_ d: JSON, _ k: String) -> Int? {
    if let n = d[k] as? NSNumber { return n.intValue }
    return nil
}
func jbool(_ d: JSON, _ k: String, _ dflt: Bool = false) -> Bool {
    (d[k] as? NSNumber)?.boolValue ?? (d[k] as? Bool ?? dflt)
}

func okResponse(_ extra: JSON = [:]) -> JSON {
    var r: JSON = ["status": "ok"]
    for (k, v) in extra { r[k] = v }
    return r
}
/// True when `k` is present and not JSON null (clients send explicit nulls).
func has(_ d: JSON, _ k: String) -> Bool { if let v = d[k] { return !(v is NSNull) } else { return false } }

func errResponse(_ message: String) -> JSON { ["status": "err", "message": message] }

func encodeFrame(_ obj: JSON) -> Data {
    // dataWithJSONObject raises an Objective-C exception (not a Swift
    // error) on invalid input such as NaN, which `try?` cannot catch.
    let fallback = Data("{\"status\":\"err\",\"message\":\"response is not valid JSON (non-finite number or unsupported type)\"}".utf8)
    var data = JSONSerialization.isValidJSONObject(obj)
        ? ((try? JSONSerialization.data(withJSONObject: obj)) ?? fallback)
        : fallback
    data.append(0x0A)
    return data
}

// MARK: - Window

/// Records completed resource loads for `net`. WKWebView has no public
/// per-request API; this uses WebKit's `_WKResourceLoadDelegate`, guarded
/// by `responds(to:)` so a WebKit without it degrades to "net unavailable"
/// instead of crashing. Observation only.
final class NetLog: NSObject {
    weak var window: Window?
    static let setter = NSSelectorFromString("_setResourceLoadDelegate:")
    private(set) var available = false

    func attach(_ wv: WKWebView) {
        guard wv.responds(to: NetLog.setter) else { return }
        wv.perform(NetLog.setter, with: self)
        available = true
    }

    private var started: [UInt64: Date] = [:]

    @objc(webView:resourceLoad:didSendRequest:)
    func sent(_ wv: WKWebView, info: NSObject, request: URLRequest) {
        guard let k = info.value(forKey: "resourceLoadID") as? UInt64 else { return }
        started[k] = Date()
        if started.count > 1000 { started.removeAll() }
    }

    @objc(webView:resourceLoad:didCompleteWithError:response:)
    func complete(_ wv: WKWebView, info: NSObject, error: NSError?, response: URLResponse?) {
        guard let w = window else { return }
        // Navigating away cancels in-flight loads; that is noise.
        if let e = error, e.domain == NSURLErrorDomain, e.code == NSURLErrorCancelled {
            if let k = info.value(forKey: "resourceLoadID") as? UInt64 { started.removeValue(forKey: k) }
            return
        }
        let url = (response?.url ?? (info.value(forKey: "originalURL") as? URL))?.absoluteString ?? ""
        let method = (info.value(forKey: "originalHTTPMethod") as? String) ?? "GET"
        let mime = response?.mimeType ?? ""
        var e: JSON = [
            "method": method,
            "url": url,
            "type": NetLog.classify(mime: mime),
        ]
        let now = Date()
        let key = info.value(forKey: "resourceLoadID") as? UInt64
        if let k = key, let sent = started.removeValue(forKey: k) {
            e["start_ms"] = max(0, Int(sent.timeIntervalSince(w.netEpoch) * 1000))
            e["duration_ms"] = max(0, Int(now.timeIntervalSince(sent) * 1000))
        } else {
            e["start_ms"] = max(0, Int(now.timeIntervalSince(w.netEpoch) * 1000))
        }
        if let http = response as? HTTPURLResponse { e["status"] = http.statusCode }
        if let err = error { e["error"] = err.localizedDescription }
        if let page = wv.url?.absoluteString { e["page"] = page }
        w.netBuf.append(e)
        if w.netBuf.count > 500 { w.netBuf.removeFirst(w.netBuf.count - 500) }
    }

    /// Same coarse classes as crates/hwatud/src/net.rs classify(), from
    /// the response MIME type. WebKit's raw request-type values are
    /// undocumented and not used.
    static func classify(mime: String) -> String {
        let m = mime.lowercased()
        if m.contains("javascript") || m.contains("ecmascript") { return "script" }
        if m.hasPrefix("text/css") { return "stylesheet" }
        if m.hasPrefix("image/") { return "image" }
        if m.hasPrefix("font/") || m.contains("font") { return "font" }
        if m.hasPrefix("audio/") || m.hasPrefix("video/") { return "media" }
        if m.contains("json") || m.contains("xml") { return "data" }
        if m.hasPrefix("text/html") { return "document" }
        return "other"
    }
}

final class Window: NSObject, WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler {
    let id: UInt64
    let webView: WKWebView
    let host: NSWindow
    var mode: String
    var appId: String?
    var loadState: String = "idle" // idle | loading | committed | settled | failed
    /// One-shot callbacks for the next document commit / load finish
    /// (navigation during an eval; see runJS NavPolicy).
    var commitHooks: [() -> Void] = []
    var finishHooks: [() -> Void] = []
    var lastError: String?
    var consoleBuf: [JSON] = []
    /// Last `snapshot --diff` value; cleared when a new document commits.
    var snapshotBaseline: Any?
    /// `net` ring buffer (same cap and entry shape as the GTK daemon).
    var netBuf: [JSON] = []
    var netEpoch = Date()
    let netLog = NetLog()
    /// File the next file chooser receives (`upload`). Without one, file
    /// choosers are cancelled: a headless window never shows a panel.
    var pendingUpload: URL?
    /// Inline-data uploads staged on disk. The page's File objects read
    /// from these lazily, so they live until the window closes (as on Linux).
    var stagedUploads: [(dir: URL, size: Int)] = []

    func removeStagedUploads() {
        for s in stagedUploads { try? FileManager.default.removeItem(at: s.dir) }
        stagedUploads.removeAll()
    }

    init(id: UInt64, mode: String, appId: String?, profile: String?) {
        self.id = id
        self.mode = mode
        self.appId = appId
        let cfg = WKWebViewConfiguration()
        cfg.websiteDataStore = sessionStore
        cfg.preferences.javaScriptCanOpenWindowsAutomatically = false
        // Headless windows are never key; do not let WebKit deprioritize
        // them as inactive pages.
        if #available(macOS 14.0, *) {
            cfg.preferences.inactiveSchedulingPolicy = .none
        }
        let controller = WKUserContentController()
        // Console capture: forward console.* and errors to the daemon.
        let js = """
        (function(){
          const send = (level, text) => {
            try { window.webkit.messageHandlers.hwatuConsole.postMessage({level, text: String(text).slice(0, 4096)}); } catch(e) {}
          };
          for (const level of ['log','info','warn','error','debug']) {
            const orig = console[level];
            console[level] = function(...args) {
              send(level, args.map(a => { try { return typeof a === 'string' ? a : JSON.stringify(a); } catch(e) { return String(a); } }).join(' '));
              return orig.apply(console, args);
            };
          }
          window.addEventListener('error', e => send('error', e.message + ' @ ' + e.filename + ':' + e.lineno));
          window.addEventListener('unhandledrejection', e => send('error', 'unhandled rejection: ' + e.reason));
        })();
        """
        controller.addUserScript(WKUserScript(source: js, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        // Trusted-input target resolver in every frame, including
        // cross-origin ones (CAPTCHA widgets): shared page-scripts/trusted-resolve.js.
        controller.addUserScript(WKUserScript(source: trustedResolveJS, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        if mode == "headless" {
            // WKWebView throttles requestAnimationFrame (and suspends CSS
            // animation ticks) when its window is not visible, which stalls
            // JS frameworks that gate rendering on rAF (e.g. the Okta
            // sign-in widget). Drive rAF off timers so headless pages run
            // their full render loops.
            let rafShim = """
            (function(){
              const nativeRAF = window.requestAnimationFrame.bind(window);
              const nativeCancel = window.cancelAnimationFrame.bind(window);
              let nextId = 1;
              const pending = new Map(); // id -> {rafId, timerId}
              window.requestAnimationFrame = function(cb) {
                const id = nextId++;
                const fire = () => {
                  const entry = pending.get(id);
                  if (!entry) return;
                  pending.delete(id);
                  nativeCancel(entry.rafId);
                  clearTimeout(entry.timerId);
                  cb(performance.now());
                };
                const rafId = nativeRAF(fire);
                const timerId = setTimeout(fire, 32);
                pending.set(id, {rafId, timerId});
                return id;
              };
              window.cancelAnimationFrame = function(id) {
                const entry = pending.get(id);
                if (!entry) return;
                pending.delete(id);
                nativeCancel(entry.rafId);
                clearTimeout(entry.timerId);
              };
            })();
            """
            controller.addUserScript(WKUserScript(source: rafShim, injectionTime: .atDocumentStart, forMainFrameOnly: false))
            // Headless windows report document.visibilityState === "hidden",
            // and visibility-aware pages (Okta sign-in, lazy loaders, video
            // players) defer work indefinitely. Spoof the Page Visibility
            // API so pages believe they are visible.
            let visShim = """
            (function(){
              const spoof = (proto, prop, value) => {
                try { Object.defineProperty(proto, prop, { get: () => value, configurable: true }); } catch(e) {}
              };
              spoof(Document.prototype, 'visibilityState', 'visible');
              spoof(Document.prototype, 'hidden', false);
              spoof(Document.prototype, 'webkitVisibilityState', 'visible');
              spoof(Document.prototype, 'webkitHidden', false);
              document.addEventListener('visibilitychange', e => e.stopImmediatePropagation(), true);
              document.addEventListener('webkitvisibilitychange', e => e.stopImmediatePropagation(), true);
              spoof(Document.prototype, 'hasFocus', () => true);
            })();
            """
            controller.addUserScript(WKUserScript(source: visShim, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        }
        cfg.userContentController = controller
        webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 1280, height: 800), configuration: cfg)
        if mode == "headless" {
            // A headless window is always "occluded" while the display
            // sleeps or another app covers it, and WebKit then throttles
            // page timers to about 2 Hz (measured: 4 ticks of a 100 ms
            // interval in 2 s with the display asleep, 20 with this off).
            // WebKit's private switch is guarded so a WebKit without it
            // keeps working, only slower.
            let sel = NSSelectorFromString("_setWindowOcclusionDetectionEnabled:")
            if webView.responds(to: sel) { webView.perform(sel, with: NSNumber(value: false)) }
        }
        webView.customUserAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15 hwatu"
        // A real (but never-presented) window keeps WebKit rendering so
        // snapshots are not blank. It is never ordered front for headless.
        host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 800),
                        styleMask: [.titled, .closable, .resizable],
                        backing: .buffered, defer: false)
        host.isReleasedWhenClosed = false
        host.contentView = webView
        super.init()
        controller.add(self, name: "hwatuConsole")
        webView.navigationDelegate = self
        webView.uiDelegate = self
        netLog.window = self
        netLog.attach(webView)
        if mode != "headless" {
            host.makeKeyAndOrderFront(nil)
        } else {
            // WebKit suspends rAF, compositing, and render-gated JS when the
            // hosting window is occluded or unmapped, which stalls pages
            // whose frameworks wait on render ticks (e.g. Okta sign-in).
            // Keep the window technically visible to the window server but
            // imperceptible and non-interactive: ~0 alpha, click-through,
            // hidden from Mission Control / window cycling, never key.
            // 0.0001: stacked headless windows composite their alphas
            // (1% each produced a visible "ghost"; 15 windows at 0.001
            // composite to 1.5%). At 0.0001, 15 stacked windows composite
            // to 0.15%; timers run at full rate and snapshots render the
            // page (measured 2026-10-22, macOS 26.6).
            //
            // Normal window level, not below the desktop icons: macOS treats
            // a below-desktop window as occluded and throttles its page
            // timers to about 1 s (measured: setInterval(100) produced 9
            // ticks in 4 s below desktop, 39 at normal level), which stalls
            // expect and wait-load quiescent.
            host.alphaValue = 0.0001
            host.ignoresMouseEvents = true
            host.collectionBehavior = [.transient, .ignoresCycle, .canJoinAllSpaces]
            host.hasShadow = false
            host.setFrameOrigin(NSPoint(x: 0, y: 0))
            host.orderFrontRegardless()
        }
    }

    func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters,
                 initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping ([URL]?) -> Void) {
        let file = pendingUpload
        pendingUpload = nil
        completionHandler(file.map { [$0] })
    }

    func userContentController(_ c: WKUserContentController, didReceive message: WKScriptMessage) {
        if let body = message.body as? JSON {
            consoleBuf.append(["level": body["level"] ?? "log", "text": body["text"] ?? ""])
            if consoleBuf.count > 500 { consoleBuf.removeFirst(consoleBuf.count - 500) }
        }
    }

    func webView(_ w: WKWebView, didStartProvisionalNavigation n: WKNavigation!) {
        loadState = "loading"
        netEpoch = Date()
    }
    func webView(_ w: WKWebView, didCommit n: WKNavigation!) {
        loadState = "committed"
        snapshotBaseline = nil
        let hooks = commitHooks; commitHooks.removeAll()
        hooks.forEach { $0() }
    }
    func webView(_ w: WKWebView, didFinish n: WKNavigation!) {
        loadState = "settled"
        let hooks = finishHooks; finishHooks.removeAll()
        hooks.forEach { $0() }
    }
    func webView(_ w: WKWebView, didFail n: WKNavigation!, withError e: Error) {
        loadState = "failed"; lastError = e.localizedDescription
    }
    // VISION: no password storage. HTTP auth challenges (Basic/Digest,
    // NTLM) are not answered from or saved to the keychain: password
    // challenges are cancelled so the page reports 401 and the agent asks
    // the user. Server trust and client certificates keep default handling.
    func webView(_ w: WKWebView, didReceive challenge: URLAuthenticationChallenge,
                 completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        switch challenge.protectionSpace.authenticationMethod {
        case NSURLAuthenticationMethodHTTPBasic, NSURLAuthenticationMethodHTTPDigest,
             NSURLAuthenticationMethodNTLM, NSURLAuthenticationMethodDefault:
            completionHandler(.rejectProtectionSpace, nil)
        default:
            completionHandler(.performDefaultHandling, nil)
        }
    }

    func webView(_ w: WKWebView, didFailProvisionalNavigation n: WKNavigation!, withError e: Error) {
        let ns = e as NSError
        // NSURLErrorCancelled fires on redirects; not a real failure.
        if ns.code == NSURLErrorCancelled { return }
        loadState = "failed"; lastError = e.localizedDescription
    }

    func info() -> JSON {
        var d: JSON = [
            "id": id,
            "url": webView.url?.absoluteString ?? "",
            "title": webView.title ?? "",
            "focused": false,
            "suspended": false,
        ]
        if mode != "normal" { d["mode"] = mode }
        if let a = appId { d["app_id"] = a }
        return d
    }
}

// MARK: - Daemon state (main-thread only)

final class Daemon {
    var windows: [UInt64: Window] = [:]
    var nextId: UInt64 = 1
    var order: [UInt64] = []

    func resolve(_ id: Int?) -> Window? {
        if let id = id { return windows[UInt64(id)] }
        // id: None targets the focused window, else the most recent one.
        if let last = order.last { return windows[last] }
        return nil
    }

    func open(url: String?, mode: String, appId: String?, profile: String?) -> Window {
        let w = Window(id: nextId, mode: mode, appId: appId, profile: profile)
        nextId += 1
        windows[w.id] = w
        order.append(w.id)
        if let u = url, let parsed = URL(string: u) {
            w.loadState = "loading"
            w.webView.load(URLRequest(url: parsed))
        }
        return w
    }

    func close(_ id: UInt64) -> Bool {
        guard let w = windows.removeValue(forKey: id) else { return false }
        order.removeAll { $0 == id }
        w.host.close()
        w.removeStagedUploads()
        return true
    }
}

let daemon = Daemon()
// Same limits as the GTK daemon (hwatu-ipc INLINE_MAX_BYTES, window.rs).
let inlineMaxBytes = 16 * 1024 * 1024
let maxRetainedUploads = 16
let maxRetainedUploadBytes = 64 * 1024 * 1024

/// Keeps the socket watchdog's DispatchSourceTimer alive for the
/// process lifetime (a local would be deallocated when serve() returns).
var watchdogTimer: DispatchSourceTimer?

// MARK: - Main-thread bridging

/// Run `body` on the main thread; `body` must eventually call the completion
/// with a response. The calling (connection) thread blocks on a semaphore.
/// The main thread itself is never blocked, so WebKit callbacks keep flowing.
func onMain(_ body: @escaping (@escaping (JSON) -> Void) -> Void) -> JSON {
    let sem = DispatchSemaphore(value: 0)
    var result: JSON = errResponse("internal: no response produced")
    DispatchQueue.main.async {
        body { r in
            result = r
            sem.signal()
        }
    }
    sem.wait()
    return result
}

// MARK: - JS plumbing

func jsonValue(from any: Any?) -> Any {
    guard let any = any else { return NSNull() }
    if any is NSNull { return NSNull() }
    if let n = any as? NSNumber {
        // JSON has no Infinity/NaN; NSJSONSerialization throws on them
        // (uncatchable from Swift) and would kill the daemon. Map them to
        // null, as the GTK daemon's serde_json does.
        if CFGetTypeID(n) != CFBooleanGetTypeID(), !n.doubleValue.isFinite { return NSNull() }
        return n
    }
    if let s = any as? String { return s }
    if let a = any as? [Any] { return a.map { jsonValue(from: $0) } }
    if let d = any as? [String: Any] { return d.mapValues { jsonValue(from: $0) } }
    return String(describing: any)
}

// Trusted input: AppKit events delivered straight to the WKWebView. WebKit
// turns them into page events with isTrusted === true. They are not posted
// to the window server, so the user's cursor, keyboard focus and frontmost
// app do not change, and they work in headless (click-through) windows.
func mouseEvent(_ w: Window, _ type: NSEvent.EventType, _ p: NSPoint) -> NSEvent? {
    NSEvent.mouseEvent(with: type, location: p, modifierFlags: [],
                       timestamp: ProcessInfo.processInfo.systemUptime,
                       windowNumber: w.host.windowNumber, context: nil, eventNumber: 0,
                       clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0)
}

func keyEvent(_ w: Window, _ type: NSEvent.EventType, _ chars: String, _ keyCode: UInt16) -> NSEvent? {
    NSEvent.keyEvent(with: type, location: .zero, modifierFlags: [],
                     timestamp: ProcessInfo.processInfo.systemUptime,
                     windowNumber: w.host.windowNumber, context: nil, characters: chars,
                     charactersIgnoringModifiers: chars, isARepeat: false, keyCode: keyCode)
}

/// Resolve a trusted-input target with the shared resolver: the top
/// document first, then child frames (cross-origin included) through
/// postMessage. Returns `matched`, `frame`, `iframe` and the centre in
/// top-viewport CSS pixels.
func targetCenterJS(_ req: JSON) -> String {
    let sel = jstr(req, "selector").map(jsonStringLiteral) ?? "null"
    let contains = jstr(req, "contains").map(jsonStringLiteral) ?? "null"
    let ref = jint(req, "ref").map(String.init) ?? "null"
    if sel == "null" && ref == "null" {
        return "throw new Error('pass exactly one of a CSS selector or --ref <n> (from `hwatu snapshot`)');"
    }
    return """
    const resolver = window.__hwatuTrustedResolve;
    if (!resolver) throw new Error('trusted input resolver is not installed; reload the page');
    return await resolver(\(sel), \(jint(req, "nth") ?? 0), \(contains), \(ref));
    """
}

func nativeClick(_ w: Window, cssX: Double, cssY: Double) {
    // WKWebView is a flipped view, so its local coordinates match CSS
    // viewport coordinates (top-left origin). Page zoom is 1.
    let local = NSPoint(x: cssX, y: w.webView.isFlipped ? cssY : Double(w.webView.bounds.height) - cssY)
    let p = w.webView.convert(local, to: nil)
    if let down = mouseEvent(w, .leftMouseDown, p), let up = mouseEvent(w, .leftMouseUp, p) {
        w.webView.mouseDown(with: down)
        w.webView.mouseUp(with: up)
    }
}

/// Reply shape shared with the GTK daemon: `{clicked|typed: matched,
/// trusted: true, url, input: {backend}}`.
func trustedReply(_ w: Window, _ action: String, _ target: JSON, extra: JSON = [:]) -> JSON {
    var v: JSON = [action: target["matched"] ?? NSNull(), "trusted": true,
                   "frame": target["frame"] ?? "top", "iframe": target["iframe"] ?? NSNull(),
                   "url": w.webView.url?.absoluteString ?? "",
                   "input": ["backend": "appkit-nsevent"]]
    for (k, x) in extra { v[k] = x }
    return okResponse(["value": v])
}

func trustedClick(_ w: Window, _ req: JSON, _ done: @escaping (JSON) -> Void) {
    runJS(w, targetCenterJS(req), timeoutMs: jint(req, "timeout_ms") ?? 15000, nav: .success) { r in
        guard let v = r["value"] as? JSON, let x = v["x"] as? Double, let y = v["y"] as? Double else { done(r); return }
        nativeClick(w, cssX: x, cssY: y)
        done(trustedReply(w, "clicked", v))
    }
}

func trustedType(_ w: Window, _ req: JSON, _ text: String, _ done: @escaping (JSON) -> Void) {
    let clear = jbool(req, "clear", true)
    let enter = jbool(req, "enter")
    // Click to focus (trusted), optionally clear the old value, then send
    // one key event per character.
    let js = targetCenterJS(req)
    runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000) { r in
        guard let v = r["value"] as? JSON, let x = v["x"] as? Double, let y = v["y"] as? Double else { done(r); return }
        nativeClick(w, cssX: x, cssY: y)
        let send = {
            for ch in text {
                let s = String(ch)
                if let d = keyEvent(w, .keyDown, s, 0), let u = keyEvent(w, .keyUp, s, 0) {
                    w.webView.keyDown(with: d); w.webView.keyUp(with: u)
                }
            }
            if enter, let d = keyEvent(w, .keyDown, "\r", 36), let u = keyEvent(w, .keyUp, "\r", 36) {
                w.webView.keyDown(with: d); w.webView.keyUp(with: u)
            }
            // Read back after WebKit has processed the queued key events.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                runJS(w, "const el = document.activeElement; return { value: el && 'value' in el ? String(el.value).slice(0, 200) : null };", timeoutMs: 5000) { after in
                    let value = (after["value"] as? JSON)?["value"] ?? NSNull()
                    done(trustedReply(w, "typed", v, extra: ["value": value]))
                }
            }
        }
        w.host.makeFirstResponder(w.webView)
        if clear {
            runJS(w, "const el = document.activeElement; if (el && 'select' in el) el.select(); else document.execCommand('selectAll'); return true;", timeoutMs: 5000) { _ in
                // Typed text replaces the selection.
                send()
            }
        } else {
            send()
        }
    }
}

/// Dispatch a pointer-event click at the target's center. Called after the
/// irreversible-action check, or directly when the request carries confirm.
func clickNow(_ w: Window, _ req: JSON, _ done: @escaping (JSON) -> Void) {
    if jbool(req, "trusted") { trustedClick(w, req, done); return }
    // Shared page-scripts/target.js + click.js: same events, same reply.
    let js = """
    \(nativeTimeJS)
    \(targetPreludeJS(req))
    \(clickJS)
    """
    runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, nav: .success, done: done)
}

/// Evaluate JS with Linux-parity eval semantics (docs/agents.md "Eval
/// semantics", mirrors crates/hwatud plan_eval_source): an expression
/// runs as `return (expr)`; a body with explicit `return`/`await` runs
/// as-is; a return-less body routes through indirect eval so the page
/// answers the completion value of the last statement. Daemon-side
/// syntax errors answer `eval failed: SyntaxError: <message>` without
/// ever reaching the page. User code executes exactly once, in the page.
/// What a cross-document navigation during a script means (same as the
/// GTK daemon's NavPolicy): an interruption for eval, the point of the
/// action for click/type/fill/challenge.
enum NavPolicy { case error, success }

func runJS(_ w: Window, _ js: String, timeoutMs: Int, nav: NavPolicy = .error, done: @escaping (JSON) -> Void) {
    let source: String
    switch planEvalSource(js) {
    case .success(let s): source = s
    case .failure(let err):
        done(errResponse("eval failed: SyntaxError: \(err.message)"))
        return
    }
    var finished = false
    let finish: (JSON) -> Void = { r in
        if !finished { finished = true; done(r) }
    }
    // A committed navigation destroys the script's JS context; its
    // completion never arrives. Answer for it.
    w.commitHooks.append { [weak w] in
        guard let w = w, !finished else { return }
        let uri = w.webView.url?.absoluteString ?? ""
        switch nav {
        case .error:
            finish(errResponse("eval interrupted: the page navigated to \(uri) while the script was running, destroying its JS context. Wait for the load (`hwatu wait-load`) and run the script against the new document."))
        case .success:
            w.finishHooks.append { [weak w] in
                finish(okResponse(["value": ["navigated": true, "url": w?.webView.url?.absoluteString ?? uri] as JSON]))
            }
        }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(timeoutMs)) {
        finish(errResponse("eval timed out after \(timeoutMs) ms"))
    }
    // Serialize in the page with JSON.stringify, as WebKitGTK's
    // jsc_value_to_json does: Dates become ISO strings, cycles and other
    // unserializable objects fall back to String(value), and DOM nodes,
    // functions and window are refused. WKWebView's own bridging would
    // otherwise recurse forever on a cycle and crash the daemon.
    let wrapped = """
    const __hwatuValue = await (async () => {
    \(source)
    })();
    if (__hwatuValue === undefined || __hwatuValue === null) return 'null';
    const __t = typeof __hwatuValue;
    if (__t === 'function' || __t === 'symbol' || __t === 'bigint' ||
        (__t === 'object' && (__hwatuValue === window || __hwatuValue instanceof Node)))
      return __t === 'symbol' || __t === 'bigint' ? 'null' : { __hwatuUnsupported: true };
    try {
      const __s = JSON.stringify(__hwatuValue);
      return __s === undefined ? 'null' : __s;
    } catch (_) {
      return JSON.stringify(String(__hwatuValue));
    }
    """
    w.webView.callAsyncJavaScript(wrapped, arguments: [:], in: nil, in: .page) { result in
        switch result {
        case .success(let value):
            if let d = value as? [String: Any], d["__hwatuUnsupported"] != nil {
                finish(errResponse("eval failed: Unsupported result type; the script's completion value is not JSON-serializable (DOM node, function, symbol, or window). Return plain data instead, e.g. el.textContent rather than el, or end a statement-style body with an explicit `return`"))
                return
            }
            guard let text = value as? String, let data = text.data(using: .utf8),
                  let parsed = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]) else {
                finish(okResponse(["value": jsonValue(from: value)])); return
            }
            finish(okResponse(["value": jsonValue(from: parsed)]))
        case .failure(let error):
            let ns = error as NSError
            let detail = (ns.userInfo["WKJavaScriptExceptionMessage"] as? String) ?? ns.localizedDescription
            // Same prefix and hint as the GTK daemon so agents match one string.
            let hint = detail.contains("Unsupported result type")
                ? "; the script's completion value is not JSON-serializable (DOM node, function, symbol, or window). Return plain data instead, e.g. el.textContent rather than el, or end a statement-style body with an explicit `return`"
                : ""
            finish(errResponse("eval failed: \(detail)\(hint)"))
        }
    }
}

/// The shape `callAsyncJavaScript` compiles a body into (an async
/// function wrapper), so daemon-side parse verdicts match what the
/// page's compile would say about the same body.
func wrapAsyncBody(_ body: String) -> String {
    "async function __hwatu_probe() {\n\(body)\n}"
}

struct EvalSyntaxError: Error { let message: String }

/// Parse-only check: evaluating a function *definition* never calls it,
/// so the probe compiles the candidate body without executing user code.
/// Must run on the main thread (all planEvalSource callers do).
func jsSyntaxMessage(_ code: String) -> String? {
    struct Holder { static let ctx = JSContext()! }
    let ctx = Holder.ctx
    ctx.exception = nil
    ctx.evaluateScript(code + "\n0;")
    if let ex = ctx.exception {
        ctx.exception = nil
        return ex.toString()
    }
    return nil
}

func jsonStringLiteral(_ s: String) -> String {
    let arr = (try? String(data: JSONSerialization.data(withJSONObject: [s]), encoding: .utf8)) ?? "[\"\"]"
    // Strip the array brackets: indirect eval needs the bare string.
    return String(arr.dropFirst().dropLast())
}

func planEvalSource(_ js: String) -> Result<String, EvalSyntaxError> {
    // Trailing semicolons are meaningless on an expression but would
    // break the `return ( ... )` wrapping; strip them for the
    // expression form (mirrors Linux).
    var trimmed = js.trimmingCharacters(in: .whitespacesAndNewlines)
    while trimmed.hasSuffix(";") {
        trimmed = String(trimmed.dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    let expr = "return (\n\(trimmed)\n);"
    if jsSyntaxMessage(wrapAsyncBody(expr)) == nil {
        return .success(expr)
    }
    let body: String
    if js.contains("return") || js.contains("await") {
        body = js
    } else {
        body = "return (0, eval)(\(jsonStringLiteral(js)));"
    }
    if let msg = jsSyntaxMessage(wrapAsyncBody(body)) {
        return .failure(EvalSyntaxError(message: msg))
    }
    return .success(body)
}

/// Poll until the load reaches `until` (committed|dom|settled) or timeout.
func waitLoad(_ w: Window, until: String, timeoutMs: Int, done: @escaping (JSON) -> Void) {
    let deadline = Date().addingTimeInterval(Double(timeoutMs) / 1000.0)
    func check() {
        if w.loadState == "failed" {
            done(errResponse("load failed: \(w.lastError ?? "unknown error")"))
            return
        }
        let reached: Bool
        switch until {
        case "committed": reached = ["committed", "settled"].contains(w.loadState)
        case "dom":
            reached = w.loadState == "settled" || w.loadState == "committed"
        default: reached = w.loadState == "settled" && !w.webView.isLoading
        }
        if reached {
            if until == "dom", w.loadState != "settled" {
                // committed but maybe DOM not parsed: verify readyState.
                w.webView.evaluateJavaScript("document.readyState") { v, _ in
                    let rs = v as? String ?? ""
                    if rs == "interactive" || rs == "complete" {
                        done(okResponse(["window": w.info()]))
                    } else if Date() > deadline {
                        done(errResponse("wait-load timed out after \(timeoutMs) ms (state: \(w.loadState))"))
                    } else {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: check)
                    }
                }
                return
            }
            if until == "quiescent" {
                // Settled, then no DOM mutations for 300 ms, capped at 5 s.
                // Same constants as the GTK daemon.
                let js = """
                return await new Promise((resolve) => {
                  let last = performance.now(); const started = last;
                  const mo = new MutationObserver(() => { last = performance.now(); });
                  mo.observe(document.documentElement || document, { childList: true, subtree: true, attributes: true, characterData: true });
                  const tick = () => {
                    const now = performance.now();
                    if (now - last >= 300) { mo.disconnect(); resolve({ quiescent: true, waited_ms: Math.round(now - started) }); }
                    else if (now - started >= 5000) { mo.disconnect(); resolve({ quiescent: false, waited_ms: Math.round(now - started) }); }
                    else setTimeout(tick, 100);
                  };
                  setTimeout(tick, 100);
                });
                """
                runJS(w, js, timeoutMs: 7000) { _ in done(okResponse(["window": w.info()])) }
                return
            }
            done(okResponse(["window": w.info()]))
            return
        }
        if Date() > deadline {
            done(errResponse("wait-load timed out after \(timeoutMs) ms (state: \(w.loadState))"))
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: check)
    }
    check()
}

// MARK: - Shared JS snippets (element targeting mirrors the GTK daemon)

// MARK: - Request handling

func handle(_ req: JSON) -> JSON {
    guard let cmd = jstr(req, "cmd") else { return errResponse("missing cmd") }
    switch cmd {
    case "ping":
        let webkit = Bundle(identifier: "com.apple.WebKit")?.infoDictionary?["CFBundleShortVersionString"] as? String ?? "unknown"
        return okResponse(["value": [
            "build": "mac-wkwebview",
            "version": "0.7.1-mac",
            "platform": "macos",
            "engine": ["name": "wkwebview", "version": webkit],
            "capabilities": [
                "show": true,
                "trusted_input": true,
                "trusted_input_headless": true,
                "structured_stops": ["irreversible_action", "login_required", "challenge"],
                "transport": ["unix"],
            ],
        ]])

    case "quit":
        // Remove the socket now so the next client spawns a fresh daemon
        // instead of connecting to this one during its exit delay (it would
        // read EOF). The delay only lets this reply reach the client.
        unlink(socketPath())
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { exit(0) }
        return okResponse()

    case "open":
        return onMain { done in
            var mode = jstr(req, "mode") ?? "normal"
            if mode.isEmpty { mode = "normal" }
            let w = daemon.open(url: jstr(req, "url"), mode: mode,
                                appId: jstr(req, "app_id"), profile: jstr(req, "profile"))
            done(okResponse(["window": w.info()]))
        }

    case "list":
        return onMain { done in
            done(okResponse(["windows": daemon.order.compactMap { daemon.windows[$0]?.info() }]))
        }

    case "close":
        return onMain { done in
            guard let id = jint(req, "id") else { done(errResponse("close: missing id")); return }
            done(daemon.close(UInt64(id)) ? okResponse() : errResponse("no window \(id)"))
        }

    case "show", "focus":
        return onMain { done in
            guard let id = jint(req, "id"), let w = daemon.windows[UInt64(id)] else {
                done(errResponse("no such window")); return
            }
            w.mode = "normal"
            NSApp.setActivationPolicy(.regular)
            w.host.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            done(okResponse())
        }

    case "hide", "unfocus":
        return onMain { done in
            guard let id = jint(req, "id"), let w = daemon.windows[UInt64(id)] else {
                done(errResponse("no such window")); return
            }
            w.mode = "headless"
            w.host.orderOut(nil)
            done(okResponse())
        }

    case "navigate":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let urlStr = jstr(req, "url"), let url = URL(string: urlStr) else {
                done(errResponse("navigate: bad url")); return
            }
            w.loadState = "loading"
            w.lastError = nil
            w.webView.load(URLRequest(url: url))
            let wait = jbool(req, "wait", true)
            if !wait { done(okResponse(["window": w.info()])); return }
            let until = jstr(req, "until") ?? "settled"
            let timeout = jint(req, "timeout_ms") ?? 40000
            waitLoad(w, until: until, timeoutMs: timeout) { r in
                if (r["status"] as? String) == "ok" {
                    done(okResponse(["window": w.info()]))
                } else { done(r) }
            }
        }

    case "wait_load":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            waitLoad(w, until: jstr(req, "until") ?? "settled",
                     timeoutMs: jint(req, "timeout_ms") ?? 40000, done: done)
        }

    case "eval":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let js = jstr(req, "js") else { done(errResponse("eval: missing js")); return }
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 30000, done: done)
        }

    case "screenshot":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let full = jbool(req, "full")
            let path = jstr(req, "path") ?? NSTemporaryDirectory() + "hwatu-shot-\(w.id)-\(Int(Date().timeIntervalSince1970)).png"
            func capture() {
                let cfg = WKSnapshotConfiguration()
                cfg.afterScreenUpdates = true
                w.webView.takeSnapshot(with: cfg) { image, error in
                    guard let image = image else {
                        done(errResponse("screenshot: \(error?.localizedDescription ?? "no image")")); return
                    }
                    guard let tiff = image.tiffRepresentation,
                          let rep = NSBitmapImageRep(data: tiff),
                          let png = rep.representation(using: .png, properties: [:]) else {
                        done(errResponse("screenshot: PNG encode failed")); return
                    }
                    do {
                        try png.write(to: URL(fileURLWithPath: path))
                        done(okResponse(["path": path]))
                    } catch { done(errResponse("screenshot: \(error.localizedDescription)")) }
                }
            }
            if full {
                w.webView.evaluateJavaScript("Math.min(document.documentElement.scrollHeight, 20000)") { v, _ in
                    let h = (v as? NSNumber)?.doubleValue ?? 800
                    let frame = NSRect(x: 0, y: 0, width: w.webView.frame.width, height: max(h, 400))
                    w.host.setContentSize(frame.size)
                    w.webView.frame = frame
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.4, execute: capture)
                }
            } else { capture() }
        }

    case "resize":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let width = jint(req, "width") ?? 1280
            let height = jint(req, "height") ?? 800
            let size = NSSize(width: width, height: height)
            w.host.setContentSize(size)
            w.webView.frame = NSRect(origin: .zero, size: size)
            done(okResponse())
        }

    case "get_content":
        // Shared page-scripts/content.js and target.js (same reply shape as
        // the GTK daemon).
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let format = jstr(req, "format") ?? "text"
            guard ["text", "html", "title", "links"].contains(format) else {
                done(errResponse("content: unknown format \(format)")); return
            }
            let maxChars = min(max(jint(req, "max_chars") ?? 65536, 1), 4 * 1024 * 1024)
            let scope: String
            if jstr(req, "selector") != nil {
                var scoped = req
                scoped.removeValue(forKey: "ref")
                scope = targetPreludeJS(scoped) + "\nconst root = el;"
            } else {
                scope = "const root = document.documentElement;"
            }
            let js = """
            const MAX = \(maxChars);
            const FORMAT = '\(format)';
            \(scope)
            \(contentJS)
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "snapshot":
        // Shared page-scripts/snapshot.js. Budgeting, injection quarantine
        // and --diff run in the client (hwatu-snapshot), as for every engine
        // whose daemon returns the raw snapshot.
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let js = snapshotJS.replacingOccurrences(of: "__HWATU_INCLUDE_RECTS__", with: jbool(req, "rect") ? "true" : "false")
            let diff = jbool(req, "diff")
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000) { r in
                guard let value = r["value"], (r["status"] as? String) == "ok" else { done(r); return }
                // The client finishes the reply: it reads `raw_snapshot`,
                // diffs against `previous` when asked, then budgets and
                // quarantines. `previous` resets on navigation commit.
                var raw: JSON = ["snapshot": value]
                if diff {
                    raw["previous"] = w.snapshotBaseline ?? NSNull()
                    w.snapshotBaseline = value
                }
                done(okResponse(["value": ["hwatu_raw_snapshot": raw]]))
            }
        }

    case "click":
        if let at = req["at"] as? [NSNumber], at.count == 2 {
            // Point click (`click --trusted --at x,y`): top-viewport CSS px.
            // The guard classifies the element at the point when it is in
            // the top document; inside a cross-origin frame it is unreadable.
            let x = at[0].doubleValue, y = at[1].doubleValue
            guard x.isFinite, y.isFinite, x >= 0, y >= 0 else { return errResponse("click --at needs non-negative x,y") }
            return onMain { done in
                guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
                let probe = """
                const el = document.elementFromPoint(\(x), \(y));
                if (!el) throw new Error('no element at \(x),\(y) (outside the viewport?)');
                const inFrame = el.tagName === 'IFRAME' || el.tagName === 'FRAME';
                return { irreversible: inFrame ? null : \(irreversibleJS.trimmingCharacters(in: .whitespacesAndNewlines)), tag: el.tagName.toLowerCase() };
                """
                runJS(w, probe, timeoutMs: jint(req, "timeout_ms") ?? 15000) { r in
                    guard let v = r["value"] as? JSON else { done(r); return }
                    if !jbool(req, "confirm"), let hit = v["irreversible"] as? JSON {
                        let kind = hit["kind"] as? String ?? "irreversible"
                        let label = hit["label"] as? String ?? ""
                        done([
                            "status": "err",
                            "message": "confirmation required: \(kind) control \"\(label)\"; ask the user, then retry with confirm",
                            "stop": ["reason": "irreversible_action", "kind": kind, "label": label, "retry": ["confirm": true]],
                        ])
                        return
                    }
                    nativeClick(w, cssX: x, cssY: y)
                    done(trustedReply(w, "clicked", ["matched": ["at": [x, y], "tag": v["tag"] ?? NSNull()], "frame": "point"]))
                }
            }
        }
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            // Structured stop: unless confirmed, classify the target with the
            // shared page-scripts/irreversible.js before clicking.
            if !jbool(req, "confirm") {
                // Same targeting as the click itself (target.js), so the
                // guard classifies exactly the element that would be clicked.
                let probe = """
                \(targetPreludeJS(req))
                return { irreversible: \(irreversibleJS) };
                """
                runJS(w, probe, timeoutMs: jint(req, "timeout_ms") ?? 15000) { r in
                    if let v = r["value"] as? JSON, let hit = v["irreversible"] as? JSON {
                        let kind = hit["kind"] as? String ?? "irreversible"
                        let label = hit["label"] as? String ?? ""
                        done([
                            "status": "err",
                            "message": "confirmation required: \(kind) control \"\(label)\"; ask the user, then retry with confirm",
                            "stop": ["reason": "irreversible_action", "kind": kind, "label": label, "retry": ["confirm": true]],
                        ])
                        return
                    }
                    clickNow(w, req, done)
                }
                return
            }
            clickNow(w, req, done)
        }

    case "type":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let text = jstr(req, "text") else { done(errResponse("type: missing text")); return }
            if jbool(req, "trusted") { trustedType(w, req, text, done); return }
            // Shared page-scripts/target.js + type.js: same setter path and
            // reply as the GTK daemon.
            let js = """
            \(nativeTimeJS)
            \(targetPreludeJS(req))
            const text = \(jsonStringLiteral(text));
            const clear = \(jbool(req, "clear", true));
            const enter = \(jbool(req, "enter"));
            \(typeJS)
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, nav: .success, done: done)
        }

    case "fill_form":
        // Shared page-scripts/fill.js (same field rules and reply as GTK).
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let fields = req["fields"] as? [JSON], !fields.isEmpty else {
                done(errResponse("fill_form needs at least one field")); return
            }
            var specs: [JSON] = []
            for (i, f) in fields.enumerated() {
                if has(f, "selector") == has(f, "ref") {
                    done(errResponse("fill_form field \(i): pass exactly one of selector or ref")); return
                }
                if has(f, "value") == has(f, "checked") {
                    done(errResponse("fill_form field \(i): pass exactly one of value or checked")); return
                }
                let nth: Any = has(f, "nth") ? f["nth"]! : 0
                specs.append(["selector": f["selector"] ?? NSNull(), "nth": nth,
                              "contains": f["contains"] ?? NSNull(), "ref": f["ref"] ?? NSNull(),
                              "value": f["value"] ?? NSNull(), "checked": f["checked"] ?? NSNull()])
            }
            let fieldsJSON = (try? String(data: JSONSerialization.data(withJSONObject: specs), encoding: .utf8)) ?? "[]"
            let js = """
            const FIELDS = \(fieldsJSON);
            const SUBMIT = \(jbool(req, "submit"));
            \(fillJS)
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, nav: jbool(req, "submit") ? .success : .error, done: done)
        }

    case "press":
        // Shared page-scripts/press.js (focus, activation and dialog
        // semantics match the GTK daemon).
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let names = ["tab": "Tab", "enter": "Enter", "escape": "Escape",
                         "arrowleft": "ArrowLeft", "arrowright": "ArrowRight",
                         "arrow_left": "ArrowLeft", "arrow_right": "ArrowRight"]
            let raw = (jstr(req, "key") ?? "enter").lowercased()
            guard let key = names[raw] else { done(errResponse("press: unsupported key \(raw)")); return }
            let js = nativeTimeJS + "\n" + pressJS.replacingOccurrences(of: "__HWATU_KEY__", with: jsonStringLiteral(key))
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, nav: .success, done: done)
        }

    case "scroll":
        // Shared page-scripts/scroll.js.
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let modes = [has(req, "selector"), has(req, "to_y"), has(req, "by_pages")].filter { $0 }.count
            if modes > 1 { done(errResponse("scroll takes exactly one of selector, to_y, by_pages")); return }
            let num: (String) -> String = { k in (req[k] as? NSNumber).map { "\($0.doubleValue)" } ?? "null" }
            let js = """
            \(nativeTimeJS)
            const selector = \(jstr(req, "selector").map(jsonStringLiteral) ?? "null");
            const nth = \(jint(req, "nth") ?? 0);
            const contains = \(jstr(req, "contains").map(jsonStringLiteral) ?? "null");
            const toY = \(num("to_y"));
            const byPages = \(num("by_pages"));
            \(scrollJS)
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "console":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            var entries = w.consoleBuf
            if let limit = jint(req, "limit"), entries.count > limit {
                entries = Array(entries.suffix(limit))
            }
            if jbool(req, "clear") { w.consoleBuf.removeAll() }
            done(okResponse(["value": entries]))
        }

    case "net":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard w.netLog.available else {
                done(errResponse("net is unavailable: this WebKit has no resource-load delegate")); return
            }
            var entries = w.netBuf
            if let limit = jint(req, "limit"), entries.count > limit { entries = Array(entries.suffix(limit)) }
            if jbool(req, "clear") { w.netBuf.removeAll() }
            done(okResponse(["value": entries]))
        }

    case "expect":
        // Shared page-scripts/expect.js + visibility.js. The page loop owns
        // the deadline; the eval gets a margin so the structured failure,
        // not a generic timeout, reaches the caller.
        if jbool(req, "watch") {
            return errResponse("expect --watch is not supported by the macOS daemon yet (hwatud-mac)")
        }
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let sel = jstr(req, "selector") else { done(errResponse("expect: missing selector")); return }
            let deadline = jint(req, "timeout_ms") ?? 5000
            let js = """
            \(nativeTimeJS)
            \(visibilityJS)
            const selector = \(jsonStringLiteral(sel));
            const nth = \(jint(req, "nth") ?? 0);
            const contains = \(jstr(req, "contains").map(jsonStringLiteral) ?? "null");
            const wantText = \(jstr(req, "text").map(jsonStringLiteral) ?? "null");
            const absent = \(jbool(req, "absent"));
            const wantVisible = \(jbool(req, "visible"));
            const deadline = \(deadline);
            \(expectJS)
            """
            runJS(w, js, timeoutMs: deadline + 2000, done: done)
        }

    case "auth_context":
        // Login-wall structured stop, from the shared page-scripts/login-wall.js.
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            // Shared page-scripts/auth-context.js (same inventory as GTK).
            let js = authContextJS.replacingOccurrences(of: "__LOGIN_WALL__", with: loginWallJS.trimmingCharacters(in: .whitespacesAndNewlines))
            runJS(w, js, timeoutMs: 10000) { r in
                guard var v = r["value"] as? JSON else { done(r); return }
                // No named profiles on this daemon: every window uses the
                // default WKWebsiteDataStore.
                v["profile"] = NSNull()
                let wall = v.removeValue(forKey: "login_wall")
                if let wall = wall as? JSON {
                    let needs = ((wall["otp_fields"] as? NSNumber)?.intValue ?? 0) > 0 ? "one_time_code" : "credentials"
                    v["login_required"] = [
                        "reason": "login_required", "needs": needs, "url": v["url"] ?? "",
                        "evidence": wall,
                        "retry": ["command": "type", "after": "the user provides the value; enter it once, the session persists"],
                    ] as JSON
                } else {
                    v["login_required"] = NSNull()
                }
                done(okResponse(["value": v]))
            }
        }

    case "challenge":
        // Detect CAPTCHA / anti-bot UI with the shared page-scripts/challenge.js.
        // With wait, a challenge still present at the deadline returns a
        // structured `challenge` stop (same shape as the GTK daemon).
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let wait = jbool(req, "wait")
            let timeout = jint(req, "timeout_ms") ?? 30000
            let js = """
            \(challengeJS)
            const started = Date.now();
            let current = detectHwatuChallenge();
            if (\(wait)) {
              while (current.status === 'challenge' && Date.now() - started < \(timeout)) {
                await new Promise(r => setTimeout(r, 500));
                current = detectHwatuChallenge();
              }
              current.status = current.status === 'challenge' ? 'manual_required' : 'cleared';
              current.manual_required = current.status === 'manual_required';
            }
            current.elapsed_ms = Date.now() - started;
            return current;
            """
            runJS(w, js, timeoutMs: timeout + 5000, nav: .success) { r in
                if let v = r["value"] as? JSON, (v["status"] as? String) == "manual_required" {
                    done([
                        "status": "err",
                        "message": "challenge still present; the agent could not clear it",
                        "stop": [
                            "reason": "challenge",
                            "challenge_type": v["challenge_type"] ?? NSNull(),
                            "url": v["url"] ?? NSNull(),
                            "retry": ["command": "show", "after": "the user clears the challenge"],
                        ] as JSON,
                    ])
                    return
                }
                done(r)
            }
        }

    case "motion":
        // Declared motion inventory from the shared page-scripts/motion.js.
        if jbool(req, "observe") {
            return errResponse("motion --observe is not supported by the macOS daemon yet (hwatud-mac). Use motion without --observe for declared CSS/WAAPI motion.")
        }
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            runJS(w, motionJS, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "upload":
        // Arm the file the next chooser receives, then click the input and
        // wait for its change event (same flow as the GTK daemon).
        guard let selector = jstr(req, "selector"), let path = jstr(req, "path") else {
            return errResponse("upload: missing selector or path")
        }
        var file: URL
        var stagedDir: (dir: URL, size: Int)?
        if let data = jstr(req, "data") {
            guard let bytes = Data(base64Encoded: data) else { return errResponse("upload data is not valid base64") }
            if bytes.count > inlineMaxBytes {
                return errResponse("upload is \(bytes.count) bytes; the inline limit is \(inlineMaxBytes) bytes")
            }
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("hwatu-upload-\(UUID().uuidString)")
            do {
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                        attributes: [.posixPermissions: 0o700])
                file = dir.appendingPathComponent((path as NSString).lastPathComponent)
                try bytes.write(to: file, options: .atomic)
                stagedDir = (dir, bytes.count)
            } catch {
                return errResponse("cannot stage upload: \(error.localizedDescription)")
            }
        } else {
            var isDir: ObjCBool = false
            guard FileManager.default.fileExists(atPath: path, isDirectory: &isDir) else {
                return errResponse("cannot read \(path): No such file or directory")
            }
            if isDir.boolValue { return errResponse("cannot upload \(path): not a regular file") }
            file = URL(fileURLWithPath: path)
        }
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else {
                if let s = stagedDir { try? FileManager.default.removeItem(at: s.dir) }
                done(errResponse("no window open")); return
            }
            if let s = stagedDir {
                let bytes = w.stagedUploads.reduce(0) { $0 + $1.size }
                if w.stagedUploads.count >= maxRetainedUploads || bytes + s.size > maxRetainedUploadBytes {
                    try? FileManager.default.removeItem(at: s.dir)
                    done(errResponse("too many staged uploads are still retained by this window")); return
                }
                w.stagedUploads.append(s)
            }
            w.pendingUpload = file
            let js = """
            const selector = \(jsonStringLiteral(selector));
            const el = document.querySelector(selector);
            if (!el) throw new Error('no element matches selector: ' + selector);
            if (!(el instanceof HTMLInputElement) || el.type !== 'file')
              throw new Error('element is not an <input type=file>: ' + selector);
            el.value = '';
            return await new Promise((resolve, reject) => {
              el.addEventListener('change', () => {
                const f = el.files && el.files[0];
                if (!f) return reject(new Error('file chooser completed without a file'));
                resolve({ uploaded: f.name, size: f.size, type: f.type });
              }, { once: true });
              el.click();
            });
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000) { r in
                w.pendingUpload = nil
                done(r)
            }
        }

    case "batch":
        // Run steps in order on this connection thread; each step does its own
        // main-thread hop. Stop at the first error, report per-step results.
        guard let actions = req["actions"] as? [JSON] else { return errResponse("batch: missing actions") }
        var steps: [JSON] = []
        var failedAt: Int? = nil
        for (i, a) in actions.enumerated() {
            let name = jstr(a, "cmd") ?? "?"
            if failedAt != nil {
                steps.append(["index": i, "action": name, "status": "skipped", "skipped_reason": "not run after step \(failedAt!) failed"])
                continue
            }
            if name == "batch" {
                failedAt = i
                steps.append(["index": i, "action": name, "status": "error", "error": "nested batch is not allowed"])
                continue
            }
            let r = handle(a)
            var step: JSON = ["index": i, "action": name, "response": r]
            if (r["status"] as? String) == "err" {
                failedAt = i
                step["status"] = "error"
                step["error"] = r["message"] ?? ""
            } else {
                step["status"] = "ok"
            }
            steps.append(step)
        }
        var batch: JSON = ["complete": failedAt == nil, "executed": failedAt.map { $0 + 1 } ?? actions.count, "steps": steps]
        if let f = failedAt { batch["failed_at"] = f }
        return okResponse(["value": ["batch": batch]])

    default:
        return errResponse("command \(cmd) is not supported by the macOS daemon yet (hwatud-mac); supported: ping, open, list, close, show, hide, navigate, eval, wait_load, screenshot, resize, get_content, snapshot, click, type, press, fill_form, scroll, console, net, motion, expect, auth_context, challenge, upload, batch, quit")
    }
}

/// The GTK daemon's target prelude: `selector/nth/contains/refIdx` consts
/// then page-scripts/target.js, leaving `el` and `matched` in scope.
func targetPreludeJS(_ req: JSON) -> String {
    let sel = jstr(req, "selector").map(jsonStringLiteral) ?? "null"
    let contains = jstr(req, "contains").map(jsonStringLiteral) ?? "null"
    let ref = jint(req, "ref").map(String.init) ?? "null"
    return """
    const selector = \(sel);
    const nth = \(jint(req, "nth") ?? 0);
    const contains = \(contains);
    const refIdx = \(ref);
    \(targetJS)
    """
}

// MARK: - Socket server

/// True when a live daemon is accepting connections on `path`. A stale
/// socket file (daemon crashed) answers ECONNREFUSED and reads false.
func socketAlive(_ path: String) -> Bool {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return false }
    defer { Darwin.close(fd) }
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    withUnsafeMutableBytes(of: &addr.sun_path) { raw in
        path.withCString { cstr in
            _ = memcpy(raw.baseAddress!, cstr, min(strlen(cstr) + 1, raw.count))
        }
    }
    let size = socklen_t(MemoryLayout<sockaddr_un>.size)
    let rc = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, size) }
    }
    return rc == 0
}

func serve() {
    let path = socketPath()
    // Single-instance gate: never steal a live daemon's socket. The old
    // behavior (unconditional unlink) orphaned the previous daemon with
    // no socket, so it could never be reached or told to quit, and its
    // near-invisible headless windows accumulated into a visible ghost
    // over the desktop. A stale socket file (no listener) is removed.
    if FileManager.default.fileExists(atPath: path), socketAlive(path) {
        FileHandle.standardError.write(Data("hwatud-mac: another daemon is live on \(path); exiting\n".utf8))
        exit(0)
    }
    unlink(path)
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { fatalError("socket() failed") }
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    withUnsafeMutableBytes(of: &addr.sun_path) { raw in
        path.withCString { cstr in
            _ = memcpy(raw.baseAddress!, cstr, min(strlen(cstr) + 1, raw.count))
        }
    }
    let size = socklen_t(MemoryLayout<sockaddr_un>.size)
    let bound = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, size) }
    }
    guard bound == 0 else { fatalError("bind(\(path)) failed: \(String(cString: strerror(errno)))") }
    chmod(path, 0o600)
    guard listen(fd, 16) == 0 else { fatalError("listen failed") }
    FileHandle.standardError.write(Data("hwatud-mac: listening on \(path)\n".utf8))

    // Socket watchdog: if the socket file is removed or replaced (a
    // second daemon raced past the gate, or the user cleaned /tmp),
    // this daemon is unreachable forever. Exit so its windows die with
    // it instead of lingering as ghost overlays.
    var boundStat = stat()
    if stat(path, &boundStat) == 0 {
        let timer = DispatchSource.makeTimerSource(queue: DispatchQueue.global())
        timer.schedule(deadline: .now() + 5, repeating: 5)
        timer.setEventHandler {
            var now = stat()
            if stat(path, &now) != 0 || now.st_ino != boundStat.st_ino || now.st_dev != boundStat.st_dev {
                FileHandle.standardError.write(Data("hwatud-mac: socket \(path) was removed or replaced; exiting\n".utf8))
                exit(0)
            }
        }
        timer.activate()
        watchdogTimer = timer
    }

    Thread.detachNewThread {
        while true {
            let client = accept(fd, nil, nil)
            if client < 0 { continue }
            Thread.detachNewThread { handleConnection(client) }
        }
    }
}

func handleConnection(_ fd: Int32) {
    defer { Darwin.close(fd) }
    var buffer = Data()
    var chunk = [UInt8](repeating: 0, count: 64 * 1024)
    while true {
        // Extract complete frames already buffered.
        while let nl = buffer.firstIndex(of: 0x0A) {
            let frame = buffer.subdata(in: buffer.startIndex..<nl)
            buffer.removeSubrange(buffer.startIndex...nl)
            guard !frame.isEmpty else { continue }
            let response: JSON
            if let obj = (try? JSONSerialization.jsonObject(with: frame)) as? JSON {
                response = handle(obj)
            } else {
                response = errResponse("invalid JSON frame")
            }
            let out = encodeFrame(response)
            var sent = 0
            out.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
                while sent < raw.count {
                    let n = write(fd, raw.baseAddress!.advanced(by: sent), raw.count - sent)
                    if n <= 0 { sent = raw.count; return }
                    sent += n
                }
            }
        }
        let n = read(fd, &chunk, chunk.count)
        if n <= 0 { return }
        if buffer.count + n > MAX_FRAME { return }
        buffer.append(contentsOf: chunk[0..<n])
    }
}

// MARK: - Main

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
serve()
app.run()
