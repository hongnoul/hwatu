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
func errResponse(_ message: String) -> JSON { ["status": "err", "message": message] }

func encodeFrame(_ obj: JSON) -> Data {
    var data = (try? JSONSerialization.data(withJSONObject: obj)) ?? Data("{\"status\":\"err\",\"message\":\"serialize failed\"}".utf8)
    data.append(0x0A)
    return data
}

// MARK: - Window

final class Window: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    let id: UInt64
    let webView: WKWebView
    let host: NSWindow
    var mode: String
    var appId: String?
    var loadState: String = "idle" // idle | loading | committed | settled | failed
    var lastError: String?
    var consoleBuf: [JSON] = []

    init(id: UInt64, mode: String, appId: String?, profile: String?) {
        self.id = id
        self.mode = mode
        self.appId = appId
        let cfg = WKWebViewConfiguration()
        cfg.websiteDataStore = WKWebsiteDataStore.default()
        cfg.preferences.javaScriptCanOpenWindowsAutomatically = false
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
        if mode != "headless" {
            host.makeKeyAndOrderFront(nil)
        } else {
            // WebKit suspends rAF, compositing, and render-gated JS when the
            // hosting window is occluded or unmapped, which stalls pages
            // whose frameworks wait on render ticks (e.g. Okta sign-in).
            // Keep the window technically visible to the window server but
            // imperceptible and non-interactive: ~0 alpha, click-through,
            // hidden from Mission Control / window cycling, never key.
            host.alphaValue = 0.01
            host.ignoresMouseEvents = true
            host.collectionBehavior = [.transient, .ignoresCycle, .canJoinAllSpaces]
            host.level = NSWindow.Level(Int(CGWindowLevelForKey(.desktopIconWindow)) - 1)
            host.setFrameOrigin(NSPoint(x: 0, y: 0))
            host.orderFrontRegardless()
        }
    }

    func userContentController(_ c: WKUserContentController, didReceive message: WKScriptMessage) {
        if let body = message.body as? JSON {
            consoleBuf.append(["level": body["level"] ?? "log", "text": body["text"] ?? ""])
            if consoleBuf.count > 500 { consoleBuf.removeFirst(consoleBuf.count - 500) }
        }
    }

    func webView(_ w: WKWebView, didStartProvisionalNavigation n: WKNavigation!) { loadState = "loading" }
    func webView(_ w: WKWebView, didCommit n: WKNavigation!) { loadState = "committed" }
    func webView(_ w: WKWebView, didFinish n: WKNavigation!) { loadState = "settled" }
    func webView(_ w: WKWebView, didFail n: WKNavigation!, withError e: Error) {
        loadState = "failed"; lastError = e.localizedDescription
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
        return true
    }
}

let daemon = Daemon()

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
    if let n = any as? NSNumber { return n }
    if let s = any as? String { return s }
    if let a = any as? [Any] { return a.map { jsonValue(from: $0) } }
    if let d = any as? [String: Any] { return d.mapValues { jsonValue(from: $0) } }
    return String(describing: any)
}

/// Evaluate a JS *function body* (so `return` works); Promises are awaited.
func runJS(_ w: Window, _ js: String, timeoutMs: Int, done: @escaping (JSON) -> Void) {
    var finished = false
    let finish: (JSON) -> Void = { r in
        if !finished { finished = true; done(r) }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(timeoutMs)) {
        finish(errResponse("eval timed out after \(timeoutMs) ms"))
    }
    w.webView.callAsyncJavaScript(js, arguments: [:], in: nil, in: .page) { result in
        switch result {
        case .success(let value):
            finish(okResponse(["value": jsonValue(from: value)]))
        case .failure(let error):
            let ns = error as NSError
            let detail = (ns.userInfo["WKJavaScriptExceptionMessage"] as? String) ?? ns.localizedDescription
            finish(errResponse("eval: \(detail)"))
        }
    }
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
                        done(okResponse())
                    } else if Date() > deadline {
                        done(errResponse("wait-load timed out after \(timeoutMs) ms (state: \(w.loadState))"))
                    } else {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: check)
                    }
                }
                return
            }
            done(okResponse())
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

let findElementJS = """
function __hwatuFind(selector, nth, contains, ref) {
  if (ref !== null && ref !== undefined) {
    const el = (window.__hwatuRefs || [])[ref];
    if (!el) throw new Error('ref ' + ref + ' not found; take a snapshot first');
    return { el, matches: 1 };
  }
  let els = Array.from(document.querySelectorAll(selector));
  if (contains) els = els.filter(e => (e.innerText || e.value || '').includes(contains));
  if (!els.length) throw new Error('no element matches ' + selector + (contains ? ' containing ' + JSON.stringify(contains) : ''));
  const el = els[nth || 0];
  if (!el) throw new Error('nth ' + nth + ' out of range (' + els.length + ' matches)');
  return { el, matches: els.length };
}
"""

let setValueJS = """
function __hwatuSetValue(el, text, clear) {
  const tag = el.tagName.toLowerCase();
  if (tag === 'select') {
    let opt = Array.from(el.options).find(o => o.value === text) ||
              Array.from(el.options).find(o => o.label.trim() === text || o.text.trim() === text);
    if (!opt) throw new Error('no option ' + JSON.stringify(text));
    el.value = opt.value;
    el.dispatchEvent(new Event('input', {bubbles: true}));
    el.dispatchEvent(new Event('change', {bubbles: true}));
    return;
  }
  if (el.isContentEditable) {
    if (clear) el.textContent = '';
    el.focus();
    document.execCommand('insertText', false, text);
    return;
  }
  const proto = tag === 'textarea' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
  el.focus();
  const next = clear ? text : (el.value + text);
  setter.call(el, next);
  el.dispatchEvent(new Event('input', {bubbles: true}));
  el.dispatchEvent(new Event('change', {bubbles: true}));
}
"""

// MARK: - Request handling

func handle(_ req: JSON) -> JSON {
    guard let cmd = jstr(req, "cmd") else { return errResponse("missing cmd") }
    switch cmd {
    case "ping":
        return okResponse(["value": ["build": "mac-wkwebview", "version": "0.7.1-mac", "platform": "macos"]])

    case "quit":
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { exit(0) }
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

    case "focus":
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

    case "unfocus":
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
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let format = jstr(req, "format") ?? "text"
            let selector = jstr(req, "selector")
            let maxChars = jint(req, "max_chars") ?? 65536
            let selJSON = selector.flatMap { try? String(data: JSONSerialization.data(withJSONObject: [$0]), encoding: .utf8) } ?? "[null]"
            let js = """
            \(findElementJS)
            const sel = \(selJSON)[0];
            const nth = \(jint(req, "nth") ?? 0);
            const contains = \(jstr(req, "contains").flatMap { try? String(data: JSONSerialization.data(withJSONObject: [$0]), encoding: .utf8) } ?? "[null]")[0];
            const root = sel ? __hwatuFind(sel, nth, contains, null).el : document.documentElement;
            const format = \((try? String(data: JSONSerialization.data(withJSONObject: [format]), encoding: .utf8)) ?? "[\"text\"]")[0];
            const budget = \(maxChars);
            if (format === 'title') return { title: document.title, url: location.href };
            if (format === 'links') {
              const seen = new Set(); const links = [];
              for (const a of root.querySelectorAll('a[href]')) {
                const url = a.href; if (seen.has(url)) continue; seen.add(url);
                links.push({ url, text: (a.innerText || '').trim().slice(0, 200) });
              }
              return { links: links.slice(0, 500), url: location.href };
            }
            if (format === 'html') {
              const html = root.outerHTML;
              return { html: html.slice(0, budget), truncated: html.length > budget, url: location.href };
            }
            const text = (root.innerText || root.textContent || '').replace(/\\n{3,}/g, '\\n\\n');
            return { text: text.slice(0, budget), truncated: text.length > budget, title: document.title, url: location.href };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "snapshot":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let budget = jint(req, "budget") ?? 0
            let textBudget = budget > 0 ? max(budget / 2, 1000) : 20000
            let js = """
            const refs = [];
            window.__hwatuRefs = refs;
            const interactables = [];
            const selectorList = 'a[href], button, input, textarea, select, [role=button], [onclick], [contenteditable=true], summary';
            for (const el of document.querySelectorAll(selectorList)) {
              const r = el.getBoundingClientRect();
              const style = getComputedStyle(el);
              if (style.display === 'none' || style.visibility === 'hidden') continue;
              const ref = refs.length;
              refs.push(el);
              const entry = {
                ref,
                tag: el.tagName.toLowerCase(),
                text: ((el.innerText || el.value || el.placeholder || el.getAttribute('aria-label') || '')+'').trim().slice(0, 120),
              };
              if (el.type) entry.type = el.type;
              if (el.name) entry.name = el.name;
              if (el.href) entry.href = (el.href+'').slice(0, 300);
              if (el.checked !== undefined && (el.type === 'checkbox' || el.type === 'radio')) entry.checked = el.checked;
              if (\(jbool(req, "rect") ? "true" : "false")) entry.rect = [r.x, r.y, r.width, r.height].map(Math.round);
              interactables.push(entry);
              if (interactables.length >= 400) break;
            }
            const text = (document.body ? document.body.innerText : '').replace(/\\n{3,}/g, '\\n\\n').slice(0, \(textBudget));
            return { url: location.href, title: document.title, text, interactables };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "click":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let args = targetArgsJS(req)
            let js = """
            \(findElementJS)
            \(args)
            const { el, matches } = __hwatuFind(__sel, __nth, __contains, __ref);
            el.scrollIntoView({block: 'center'});
            const r = el.getBoundingClientRect();
            const opts = {bubbles: true, cancelable: true, view: window, clientX: r.x + r.width/2, clientY: r.y + r.height/2, button: 0};
            el.dispatchEvent(new PointerEvent('pointerdown', opts));
            el.dispatchEvent(new MouseEvent('mousedown', opts));
            if (el.focus) el.focus();
            el.dispatchEvent(new PointerEvent('pointerup', opts));
            el.dispatchEvent(new MouseEvent('mouseup', opts));
            el.dispatchEvent(new MouseEvent('click', opts));
            if ((el.type === 'checkbox' || el.type === 'radio') && typeof el.click === 'function') { /* click event above toggles via default action */ }
            return { matches, tag: el.tagName.toLowerCase(), text: ((el.innerText || el.value || '')+'').trim().slice(0, 120) };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "type":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let text = jstr(req, "text") else { done(errResponse("type: missing text")); return }
            let textJSON = (try? String(data: JSONSerialization.data(withJSONObject: [text]), encoding: .utf8)) ?? "[\"\"]"
            let clear = jbool(req, "clear", true)
            let enter = jbool(req, "enter")
            let js = """
            \(findElementJS)
            \(setValueJS)
            \(targetArgsJS(req))
            const { el, matches } = __hwatuFind(__sel, __nth, __contains, __ref);
            const text = \(textJSON)[0];
            __hwatuSetValue(el, text, \(clear ? "true" : "false"));
            if (\(enter ? "true" : "false")) {
              const kd = new KeyboardEvent('keydown', {key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true, cancelable: true});
              const handled = !el.dispatchEvent(kd);
              el.dispatchEvent(new KeyboardEvent('keyup', {key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true}));
              if (!handled && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
            }
            return { matches, tag: el.tagName.toLowerCase(), value: (el.value || '').slice(0, 200) };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 15000, done: done)
        }

    case "fill_form":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            guard let fields = req["fields"] as? [JSON] else { done(errResponse("fill_form: missing fields")); return }
            let fieldsJSON = (try? String(data: JSONSerialization.data(withJSONObject: fields), encoding: .utf8)) ?? "[]"
            let js = """
            \(findElementJS)
            \(setValueJS)
            const fields = \(fieldsJSON);
            const results = [];
            let lastEl = null;
            for (const f of fields) {
              const { el } = __hwatuFind(f.selector, f.nth || 0, f.contains || null, f.ref === undefined ? null : f.ref);
              lastEl = el;
              if (f.checked !== undefined && f.checked !== null) {
                if (el.checked !== f.checked) { el.click ? el.click() : (el.checked = f.checked); }
                if (el.checked !== f.checked) { el.checked = f.checked; el.dispatchEvent(new Event('change', {bubbles: true})); }
                results.push({ selector: f.selector, checked: el.checked });
              } else {
                __hwatuSetValue(el, f.value === undefined || f.value === null ? '' : String(f.value), true);
                results.push({ selector: f.selector, value: (el.value || '').slice(0, 200) });
              }
            }
            if (\(jbool(req, "submit") ? "true" : "false") && lastEl && lastEl.form) {
              lastEl.form.requestSubmit ? lastEl.form.requestSubmit() : lastEl.form.submit();
            }
            return { filled: results.length, results };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 20000, done: done)
        }

    case "press":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            let key = jstr(req, "key") ?? "enter"
            let keyName: String
            let keyCode: Int
            switch key {
            case "tab": keyName = "Tab"; keyCode = 9
            case "escape": keyName = "Escape"; keyCode = 27
            case "arrow_left", "arrowleft": keyName = "ArrowLeft"; keyCode = 37
            case "arrow_right", "arrowright": keyName = "ArrowRight"; keyCode = 39
            default: keyName = "Enter"; keyCode = 13
            }
            let js = """
            const el = document.activeElement || document.body;
            const opts = {key: '\(keyName)', code: '\(keyName)', keyCode: \(keyCode), bubbles: true, cancelable: true};
            const handled = !el.dispatchEvent(new KeyboardEvent('keydown', opts));
            el.dispatchEvent(new KeyboardEvent('keyup', opts));
            if ('\(keyName)' === 'Enter' && !handled) {
              if (el.form) { el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit(); }
              else if (el.click) el.click();
            }
            if ('\(keyName)' === 'Tab') {
              const tabbable = Array.from(document.querySelectorAll('a[href], button, input, textarea, select, [tabindex]'))
                .filter(e => !e.disabled && e.tabIndex >= 0 && e.offsetParent !== null);
              const i = tabbable.indexOf(el);
              const next = tabbable[(i + 1) % tabbable.length];
              if (next) next.focus();
            }
            return { key: '\(keyName)', target: el.tagName.toLowerCase() };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 10000, done: done)
        }

    case "scroll":
        return onMain { done in
            guard let w = daemon.resolve(jint(req, "id")) else { done(errResponse("no window open")); return }
            var body = "window.scrollBy(0, window.innerHeight);"
            if let sel = jstr(req, "selector") {
                let selJSON = (try? String(data: JSONSerialization.data(withJSONObject: [sel]), encoding: .utf8)) ?? "[\"\"]"
                body = """
                \(findElementJS)
                const { el } = __hwatuFind(\(selJSON)[0], \(jint(req, "nth") ?? 0), \(jstr(req, "contains").flatMap { try? String(data: JSONSerialization.data(withJSONObject: [$0]), encoding: .utf8) } ?? "[null]")[0], null);
                el.scrollIntoView({block: 'center'});
                """
            } else if let y = (req["to_y"] as? NSNumber)?.doubleValue {
                body = "window.scrollTo(0, \(y));"
            } else if let pages = (req["by_pages"] as? NSNumber)?.doubleValue {
                body = "window.scrollBy(0, window.innerHeight * \(pages));"
            }
            let js = """
            \(body)
            return { y: window.scrollY, max_y: Math.max(0, document.documentElement.scrollHeight - window.innerHeight), at_bottom: window.scrollY + window.innerHeight >= document.documentElement.scrollHeight - 2 };
            """
            runJS(w, js, timeoutMs: jint(req, "timeout_ms") ?? 10000, done: done)
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

    default:
        return errResponse("command \(cmd) is not supported by the macOS daemon yet (hwatud-mac); supported: ping, open, list, close, focus, unfocus, navigate, eval, wait_load, screenshot, resize, get_content, snapshot, click, type, press, fill_form, scroll, console, quit")
    }
}

/// Emit JS `__sel/__nth/__contains/__ref` bindings from a request's targeting fields.
func targetArgsJS(_ req: JSON) -> String {
    let sel = jstr(req, "selector").flatMap { try? String(data: JSONSerialization.data(withJSONObject: [$0]), encoding: .utf8) } ?? "[null]"
    let contains = jstr(req, "contains").flatMap { try? String(data: JSONSerialization.data(withJSONObject: [$0]), encoding: .utf8) } ?? "[null]"
    let ref = jint(req, "ref").map(String.init) ?? "null"
    return """
    const __sel = \(sel)[0];
    const __nth = \(jint(req, "nth") ?? 0);
    const __contains = \(contains)[0];
    const __ref = \(ref);
    """
}

// MARK: - Socket server

func serve() {
    let path = socketPath()
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
