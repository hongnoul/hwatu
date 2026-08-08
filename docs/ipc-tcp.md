# IPC over TCP: architecture specification

Status: implemented.

Scope: expand the `hwatu` ↔ `hwatud` IPC from a same-machine Unix-domain
socket to optionally run over TCP, so a client embedded next to an LLM agent
(inside a headless Docker container on a workstation) can drive a daemon that
runs on a different host (a laptop with a display and WebKitGTK).

Design rule for this work: **minimal, dependency-free**. No new crates for the
client, no new runtime services, no TLS in the binary, no framing rewrite. The
wire protocol stays newline-delimited JSON; TCP is a second transport for the
same protocol, plus a small set of additive, optional fields for moving file
payloads that can no longer be shared by path.

---

## 1. Problem

Today the daemon and client must share a filesystem and a user:

- The client resolves the socket via `hwatu_ipc::socket_path()`
  (`$HWATU_SOCKET` → `$XDG_RUNTIME_DIR/hwatu.sock` →
  `/tmp/hwatu-$UID.sock`), so client and daemon are always same-host,
  same-UID.
- The client's `connect_or_spawn()` falls back to **spawning** the daemon
  when the socket is absent — correct on a desktop, meaningless in a
  container where no daemon (and no WebKit session) exists.
- Several requests carry **host filesystem paths** that the daemon reads or
  writes (`screenshot`, `check --baseline/--heatmap`, `diff --baseline`,
  `upload`). Over TCP these paths live on the *daemon's* host, not the
  agent's container.

Desired scenario:

```
┌─ workstation ──────────────────────────┐        ┌─ user laptop ────────────────┐
│ agent (LLM) + hwatu client (container) │  TCP   │ hwatud daemon (webkitgtk)    │
│   tool calls ──► hwatu check/shot/...   │ ─────► │   windows + display          │
└─────────────────────────────────────────┘  (or   └──────────────────────────────┘
                                             ssh -L / socat tunnel)
```

Two reachability modes, both must work:

1. **Direct**: the container can reach the laptop's address
   (`HWATU_ENDPOINT=tcp://192.168.x.x:8741`).
2. **Tunneled**: the user establishes a channel (`ssh -L`, `socat`) and the
   client points at a loopback port (`HWATU_ENDPOINT=tcp://127.0.0.1:8741`).

---

## 2. Constraints and non-goals

- **No new dependencies.** Client stays std-only for transport (`TcpStream`,
  `connect_timeout` are std). The daemon uses gio, which already has
  `InetSocketAddress` and accepts it in the existing `SocketListener`. Base64
  is a small hand-rolled helper in the `hwatu-ipc` crate (see §7).
- **Wire protocol unchanged** except additive optional fields, all with the
  existing `#[serde(default)]` back-compat discipline. Old clients against new
  daemons and new clients against old daemons keep their documented behavior
  (unknown JSON fields are ignored; missing fields take defaults).
- **Unix socket behavior is untouched.** Same path resolution, same no-auth
  policy, same spawn fallback when `HWATU_ENDPOINT` is not set.
- **No TLS, no encryption in the binary.** std has none, and adding it is the
  opposite of minimal. Confidentiality over untrusted networks is the
  operator's job (`ssh -L`, WireGuard, …). The token authenticates; it is not
  a transport.
- Non-goals: UDP, custom framing, daemon discovery (mDNS), per-client
  authorization beyond one shared token, path remapping/virtual filesystems,
  streaming file transfer.

---

## 3. Current architecture (choke points)

The whole transport surface is two files plus one function, which is what
keeps this change small:

| Layer | Where | Notes |
|---|---|---|
| Endpoint resolution | `crates/ipc/src/lib.rs` → `socket_path()` | client and daemon both call it |
| Client transport | `crates/hwatu/src/main.rs` → `connect_or_spawn()` | sole connect point; also used by `mcp.rs`, `clone.rs`, `watch`, `expect_watch` |
| Daemon transport | `crates/hwatud/src/ipc_server.rs` → `start()` | `gio::SocketListener` + `UnixSocketAddress`, GLib main loop, async line reader, sequential reply, `Subscribe` hand-off to `events.rs` |
| File-bearing fields | `Request::{Screenshot,Check,Diff,Upload}` | paths interpreted daemon-side |
| Daemon flags | `crates/hwatud/src/main.rs` → `parse_security_args()` | extend for `--listen` / `--token` |

---

## 4. Design overview

Five additive changes:

1. **Endpoint abstraction** in `hwatu-ipc`: `endpoint()` returns Unix or TCP;
   `HWATU_ENDPOINT` env selects TCP. `HWATU_SOCKET` keeps its meaning.
2. **Client**: `connect_or_spawn()` branches on the endpoint; TCP never
   spawns a daemon.
3. **Daemon**: `hwatud --listen [host:]port` binds an additional TCP listener
   on the same `gio::SocketListener`; `--token`/`HWATU_TOKEN` enables a
   one-line auth handshake on TCP connections.
4. **Inline file payloads**: new optional fields carry screenshots, baselines,
   and uploads as base64 instead of daemon-host paths, because remote clients
   cannot read or write laptop filesystem paths.
5. **Bounds for a network listener**: max frame size, max TCP connections,
   `TCP_NODELAY`.

Everything else — request/response semantics, subscribe/events, batching,
`ping` version handshake, one-shot-per-connection default — is identical over
TCP.

---

## 5. Endpoint resolution (`hwatu-ipc`)

New API in `crates/ipc/src/lib.rs`:

```rust
pub enum Endpoint {
    Unix(PathBuf),          // as resolved today by socket_path()
    Tcp(std::net::SocketAddr),
}

/// Resolution order:
/// 1. HWATU_ENDPOINT  (new; tcp://host:port, host:port, unix:///path)
/// 2. HWATU_SOCKET    (existing; unix path only — unchanged)
/// 3. default         (socket_path(), unchanged)
pub fn endpoint() -> Endpoint;
```

`HWATU_ENDPOINT` grammar (all parsed with std only — `SocketAddr`/`ToSocketAddrs`):

| Value | Endpoint |
|---|---|
| `tcp://127.0.0.1:8741` | TCP loopback |
| `tcp://[::1]:8741` | TCP IPv6 loopback |
| `tcp://10.0.0.5:8741` | TCP remote |
| `127.0.0.1:8741` / `host:8741` | TCP (scheme-less convenience) |
| `unix:///run/user/1000/hwatu.sock` | explicit Unix path |
| *(anything else)* | Unix path, existing `socket_path()` semantics |

`socket_path()` stays exactly as is (including the `HWATU_SOCKET` override)
for the Unix case; `endpoint()` is a thin wrapper.

`HWATU_ENDPOINT` beats `HWATU_SOCKET` when both are set; documenting that
they are mutually exclusive is enough.

---

## 6. Client changes (`crates/hwatu`)

`connect_or_spawn()` (plus `watch`/`expect_watch`, which call it):

```rust
match hwatu_ipc::endpoint() {
    Endpoint::Unix(path) => /* existing behavior, unchanged */,
    Endpoint::Tcp(addr)  => {
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        s.set_nodelay(true)?;
        // token handshake, see §8
        Ok(s)
    },
}
```

Rules:

- **Never spawn a daemon for a TCP endpoint.** There is nothing local to
  spawn; the daemon lives on the laptop. Also: spawning a local `hwatud`
  because a tunnel is down would create a *second* daemon (two engines, two
  cookie jars, session confusion) — the worst possible failure mode. TCP
  connect failure is a hard error with a hint: `cannot reach daemon at
  tcp://…: is the daemon listening (hwatud --listen …) and is the tunnel up?`.
- `connect_timeout` (5 s) replaces the 10 s spawn-poll loop; no spawn path
  exists for TCP.
- **Paths are daemon-host paths.** `resolve_path()`/`normalize_request_paths()`
  keep running (they are harmless no-ops for absolute paths), but the spec
  contract changes for remote use: every path field refers to the daemon's
  filesystem. Agents that need files on *their* side use the inline payload
  fields (§7). This is documented in `docs/agents.md`, not enforced in code.
- `mcp.rs` gains TCP-aware screenshot handling (§8.4).
- `clone.rs`, `watch`, `expect_watch` need no changes — they all
  route through `connect_or_spawn()`.

---

## 7. Daemon changes (`crates/hwatud`)

### 7.1 `--listen [host:]port` (env `HWATU_LISTEN`)

Extended `parse_security_args()`:

```
usage: hwatud [--no-eval] [--ephemeral-profile] [--listen [host:]port] [--token <secret>]
```

- `--listen 8741` → bind `127.0.0.1:8741` (loopback default).
- `--listen 0.0.0.0:8741` / `--listen [::]:8741` → explicit non-loopback.
- Default port constant: `HWATU_TCP_PORT = 8741`.
- The Unix socket is always bound too (unchanged). `ipc_server::start()`
  calls `listener.add_address()` for the Unix address **and** the optional
  inet address on the same `gio::SocketListener`; `accept_next` needs no
  change — one accept loop, two addresses.
- Startup log line, e.g.
  `hwatud: listening on 127.0.0.1:8741 (token required: no)`.

### 7.2 Bounds for a network listener

Today the daemon relies on kernel peer-auth and same-user trust. A TCP
listener is reachable by anyone on the network, so:

- **Max frame** `MAX_FRAME_BYTES = 32 MiB` (covers the 8 MiB `render` cap and
  ~24 MiB of inline base64 payloads, §7.4). Enforced in the line-reader
  callback after `read_line_utf8_async`: oversize line → `Response::err` +
  close. One check, no behavior change for legitimate clients.
- **Max TCP connections** `MAX_TCP_CONNS = 64`. Counter incremented in
  `accept_next` for inet connections, decremented on close; over the cap,
  accept and immediately close. Prevents fd/queue exhaustion from a scan.
  (Unix socket connections stay uncapped — same-user trust as today.)
- **`TCP_NODELAY`** on accepted inet connections
  (`conn.socket().set_option(Tcp, 1 /* TCP_NODELAY */, 1)`); the daemon's
  ~35 ms one-roundtrip check promise dies under Nagle's 40 ms delay
  otherwise. Note the constant is 1 on Linux; the codebase already avoids a
  libc dependency, so hardcode with a comment.

### 7.3 Auth handshake (TCP connections only)

A TCP connection is authenticated once, before any request:

```
client →  {"auth":"<token>"}\n
daemon →  {"status":"ok"}\n                     (then normal protocol)
daemon →  {"status":"err","message":"…"}\n      (then close)
```

- New `Handshake` type in `hwatu-ipc` (the only new wire message; it never
  touches the Unix path):
  ```rust
  #[derive(Serialize, Deserialize)]
  pub struct AuthRequest { pub auth: String }
  #[derive(Serialize, Deserialize)]
  #[serde(tag = "status", rename_all = "snake_case")]
  pub enum AuthReply { Ok, Err { message: String } }
  ```
- **Token source**: daemon `--token <secret>` or `HWATU_TOKEN` env; client
  `HWATU_TOKEN` env only. Same env var name both sides.
- **Policy**:
  - Unix socket connections: no handshake, today's behavior exactly (kernel
    peer identity is the auth).
  - TCP + token configured: handshake required; mismatch → `err` + close.
  - TCP loopback bind + no token: daemon starts, prints a warning that any
    local process can connect (weaker than the 0700 unix socket).
  - **TCP non-loopback bind + no token: refuse to start.** `eval` is remote
    code execution, screenshots are data exfiltration, `upload`/click/type
    can drive the user's authenticated browser. This mirrors `SECURITY.md`
    and is the one hard startup gate added.
- Comparison is constant-time (compare a fixed-size digest of both sides,
  e.g. FNV/xxhash-style hash then `==`; no crypto dependency needed for
  timing safety against a remote peer).
- Client behavior: with a TCP endpoint, always send the handshake first
  (`{"auth": HWATU_TOKEN or ""}`), wait for the reply line, fail with the
  daemon's message on `err`. Empty token ⇒ the daemon's "auth required" error
  tells the operator to set `HWATU_TOKEN` on both sides.
- After a successful handshake the connection is a normal protocol
  connection: request/response loop, or `subscribe` hand-off to `events.rs`
  — both unchanged. The events disconnect-watcher (`read_bytes_async`) and
  write pump work identically over gio TCP connections.

### 7.4 `Quit` policy note

A remote client can send `quit` and kill the user's daemon. No code change:
this matches the local trust model (any socket client can already do it) and
gating it would add policy complexity for an edge case. Documented in
`docs/agents.md` and this spec only.

---

## 8. Protocol additions: inline file payloads

The hard blocker for remote use is *files*, not the transport: the daemon
returns screenshot paths the container cannot read, and reads baselines/uploads
the container cannot provide. Minimal fix: optional base64 fields, all
`#[serde(default)]`, absent on the wire unless used — so old peers are
unaffected in both directions.

### 8.1 Field changes

| Type | New field(s) | Semantics |
|---|---|---|
| `Request::Screenshot` | `data: bool` (default false) | capture to memory and return base64 PNG in the reply instead of writing `path` |
| `Response::Ok` | `data: Option<String>` | base64 payload (PNG for screenshots) |
| `Request::Check` | `shot_data: bool` | include `"shot_data":"<base64>"` in the reply `value` next to `"shot"` |
| `Request::Check` | `baseline_data: Option<String>` | base64 baseline PNG; mutually exclusive with `baseline`; diffed exactly like a path baseline |
| `Request::Diff` | `baseline_data: Option<String>` | same, for the standalone diff |
| `Request::Upload` | `data: Option<String>` | file bytes from the client instead of reading daemon-host `path`; `data` wins if both are set |

Rules:

- **Outputs** (`screenshot`, `check --shot`, heatmaps) gain a base64 *reply*
  so the container gets pixels without any daemon-host path. `path`/`shot`
  fields keep their meaning for local use.
- **Inputs** (`baseline`, `upload`) gain a base64 *request* so container
  files never have to be copied to the laptop first. `render`/`base` are
  already inline — no change.
- All new request fields serialize with `skip_serializing_if`/`default`
  exactly like the existing `viewports`/`render` pattern, so legacy fixtures
  in `crates/ipc/tests/wire_conformance.rs` stay green and old daemons
  silently ignore the new fields (serde's default ignore-unknown-fields).
- Size cap: `INLINE_MAX_BYTES = 24 MiB` decoded per payload
  (≈ 32 MiB base64 on the wire — inside `MAX_FRAME_BYTES`). Enforced on both
  sides; the client checks before sending, the daemon checks because clients
  are not trusted (same pattern as `RENDER_MAX_BYTES`).

### 8.2 CLI surface (`crates/hwatu/src/main.rs`)

| Command | New flag | Behavior |
|---|---|---|
| `hwatu shot --stdout` | `--stdout` | `data: true`; prints base64 PNG to stdout (agent: `hwatu shot --stdout \| base64 -d > x.png`), `path` reply ignored |
| `hwatu check <url> --shot-data` | `--shot-data` | value includes `shot_data` |
| `hwatu check <url> --baseline-data <file>` | `--baseline-data <file>` | reads the local file, sends base64 |
| `hwatu diff --id N --baseline-data <file>` | `--baseline-data <file>` | ditto |
| `hwatu upload <selector> --file <path>` | `--file <path>` | reads the local file, sends base64 |

`--baseline` and `--baseline-data` are mutually exclusive (CLI error, mirroring
the existing `--baseline`/`--baseline-dir` check). `clone.rs` keeps using
paths (local feature) but inherits `shot --stdout` for free via the CLI.

### 8.3 Base64 without dependencies

Small internal module in `hwatu-ipc` (`encode`/`decode`, standard alphabet
with padding, ~40 lines, unit tests). The client stays dependency-free by
construction; the alternative (adding the `base64` crate to `hwatu-ipc` only)
is acceptable if a maintainer prefers a reviewed implementation — it does not
leak into the client crate either way. Default is hand-rolled.

### 8.4 MCP server: automatic path translation over TCP

The `hwatu mcp` process (the MCP server) runs on the client side (container)
and speaks the IPC protocol to the daemon. When the endpoint is TCP, it
automatically bridges the filesystem gap so agents see local file paths,
identical to the Unix socket flow. No agent-side changes needed.

**Request side** — `build_request()` sets inline data flags when over TCP:

| Tool | Field set | Effect |
|------|-----------|--------|
| screenshot | `data: true` | daemon returns base64 PNG in `Response::Ok::data` |
| check | `shot_data: true` | daemon includes `shot_data` base64 in reply `value` |
| render | `shot_data: true` | same as check |
| upload | `data: "<base64>"` | MCP reads local `path`, sends base64; `path` kept for reference |

Explicit agent arguments (`data`, `shot_data`) still honored (OR'd with
TCP detection). Over Unix socket, flags default to `false` — unchanged.
The agent never sees `data` or `shot_data` — the MCP layer handles them.
**Upload** — the agent provides `path` (a file on the client host).
Over TCP, the MCP layer reads the file, sends it as `data` (base64)
in the request, so the daemon never needs access to the client's
filesystem. Over Unix socket, `path` is sent as-is (daemon reads it
directly). The agent always provides `path`; the transport switch is
transparent.

**Response side** — `hwatu mcp` transforms the daemon's reply before
forwarding to the agent:

1. **Screenshot** — `Response::Ok::data` (base64) is decoded, written to
   `/tmp/hwatu-mcp-<pid>-<seq>.png` on the client host, and `data` is
   replaced with `path` in the response. The agent sees a local file path.
2. **Check/render with shot** — `shot_data` (base64) inside the `value`
   object is decoded, written to a temp file, `shot` is replaced with the
   local path, and `shot_data` is removed. Multi-viewport sweeps are
   handled recursively.

The agent receives the same path-based response whether the daemon is
local (Unix socket) or remote (TCP). The temp files are bounded
(one per screenshot request) and orphaned on `hwatu mcp` process exit.

---

## 9. Security model

Deltas to `SECURITY.md`'s threat model; local-socket assumptions (§
"Local-socket assumptions and limitations") are unchanged.

| Listener | Peer auth | Notes |
|---|---|---|
| Unix socket (always) | kernel: same UID | status quo, unchanged |
| TCP loopback, no token | none | **warning at startup**; any local process/user can connect (weaker than the 0700 unix socket) |
| TCP loopback, token | bearer token | recommended for tunneled use |
| TCP non-loopback | bearer token **required** | startup refuses to bind without `--token`/`HWATU_TOKEN` |

- The token defends against casual port scans and a misconfigured firewall.
  It is **not** a defense against a determined on-path attacker: the protocol
  is plaintext. Any path crossing an untrusted network must ride inside an
  operator channel (`ssh -L`, WireGuard, VPN) that provides confidentiality.
  The daemon's error messages and this document say so.
- The token gates the *entire* protocol surface, including `eval` — which is
  code execution in the user's browser session. Operators choosing remote
  access should additionally consider `--no-eval` for least privilege, and
  `--ephemeral-profile` to avoid exposing persisted cookies. Both already
  exist; this spec only documents the recommendation.
- A token leak is total compromise of the browser session; advise rotating it
  (`--token` is per-daemon-invocation, so rotation = restart).

---

## 10. Tunneled paths — zero daemon code

Two ways to reach the daemon from the container; both require only the client
side of this work:

```
ssh -L 8741:localhost:8741 user@laptop
HWATU_ENDPOINT=tcp://127.0.0.1:8741  # in the container
```

or, when the daemon runs without `--listen` (unix socket only), forward the
socket with socat on the laptop:

```
socat TCP-LISTEN:8741,reuseaddr,fork UNIX-CONNECT:$XDG_RUNTIME_DIR/hwatu.sock
```

In the socat case the daemon needs *no* change at all beyond the client's TCP
support — useful for `systemd --user` daemons that predate `--listen`.

---

## 11. Backward compatibility and wire invariants

Invariants that must hold over both transports (extend the conformance suite
to assert them):

1. Newline-delimited JSON; one `Response` per `Request`, strictly in order.
2. Legacy one-shot clients: connect, send one request, read one response,
   disconnect — unchanged.
3. `Subscribe` hands the connection to the event stream; no further requests
   accepted; monotonic `seq`; drop-not-queue backpressure.
4. All pre-existing request/response fixtures round-trip byte-identically
   (`wire_conformance.rs` golden tests stay green — new fields are optional
   and absent by default).
5. `ping` version handshake works identically (an old daemon behind a tunnel
   reports its stale build; the CLI already prints the restart hint).
6. Unix behavior is bit-for-bit unchanged when `HWATU_ENDPOINT` is unset.

Compile-time note: adding `data` to `Response::Ok` requires the client's
destructure in `main.rs` to name (or `..`) the new field — a same-release
change, not a wire concern.

---

## 12. Testing plan

**Unit / conformance (`hwatu-ipc`)**
- `endpoint()` parsing: all `HWATU_ENDPOINT` grammar rows, precedence over
  `HWATU_SOCKET`, defaults.
- New fields: golden round-trips for `screenshot{data}`, `check{shot_data,
  baseline_data}`, `diff{baseline_data}`, `upload{data}`; legacy fixtures
  still parse; new fields absent when defaults.
- Base64 encode/decode: vectors, padding, rejects invalid input.
- Handshake types round-trip.

**Daemon (`hwatud`)**
- `--listen` binds loopback default; `0.0.0.0`/`[::]` bind; unix socket still
  bound alongside; non-loopback without token refuses startup; loopback
  without token warns.
- Auth: correct token passes; wrong token → `err` + close; no token sent →
  "auth required"; unix connections skip handshake.
- Limits: oversize frame → error + close; >64 TCP conns → immediate close.
- `subscribe` over TCP: events stream, disconnect unregisters.

**Client (`hwatu`)**
- TCP endpoint: connects, no daemon spawn attempt (assert no `hwatud` child
  is created); tunnel-down → clean error with hint.
- `shot --stdout` decodes to a valid PNG; `check --baseline-data` diffs
  correctly; `upload --file` injects bytes.

**Manual matrix**
1. Same host, loopback TCP, with/without token.
2. Container ↔ laptop over `ssh -L`; `hwatu check`, `shot --stdout`,
   `baseline-data`, `upload --file`, `watch` (subscribe).
3. Laptop socat → unix socket (old-daemon path, new client).
4. `--no-eval` + remote: eval rejected with the same structured error as
   local.
5. Old binary daemon (from a release tarball) behind a tunnel vs new client:
   new fields ignored, `ping` shows the build mismatch.

---

## 13. Rollout

1. ~~`hwatu-ipc`: endpoint abstraction, handshake types, inline fields, base64,
   tests.~~ ✅
2. ~~Daemon: `--listen`/`--token`, bounds, `TCP_NODELAY`.~~ ✅
3. ~~Client: `connect_or_spawn` branch, CLI flags.~~ ✅
4. ~~MCP server: TCP-aware screenshot handling (§8.4).~~ ✅
5. ~~Docs: this spec, `docs/agents.md`, `SECURITY.md`.~~ ✅
6. Single release; no staged wire migration needed (all additions optional).

---

## 14. Deferred (explicit non-goals)

- TLS/encryption in the binary (operator tunnel provides it).
- Per-client ACLs, multiple tokens, key rotation while running.
- Streaming/batched file transfer between client and daemon; inline base64
  covers the real payloads (screenshots, baselines, uploads). A generic
  `fetch`/`push` primitive is a follow-up if heatmaps or full-page captures
  grow past `INLINE_MAX_BYTES`.
- Path remapping (agent-side virtual paths → daemon host); out of scope until
  someone actually needs it.
- Daemon discovery, auto-reconnect, connection pooling.

---

## 15. Open questions

1. Default TCP port `8741` — bikeshed freely; needs an IANA-unregistered,
   collision-resistant pick and a constant in `hwatu-ipc`.
2. Should `--listen` imply a warning when `--token` is absent even on
   loopback, or is the current warning-only stance right? (Proposal: warn.)
3. Do we want `HWATU_TOKEN` honored on Unix sockets as an *optional* extra
   gate, or strictly transport-scoped as specified? (Proposal: Unix stays
   kernel-auth-only, zero behavior change.)
4. **ANSWERED** — The CLI keeps `--stdout` explicit (scriptability). The MCP
   server defaults to data mode over TCP automatically (§8.4), because it
   translates the base64 back to a local temp file before the agent sees it.
   The agent never needs to know about `data` or `shot_data`.
