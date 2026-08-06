# Security policy

## Threat model

hwatu is a local browser daemon for trusted local operators and trusted local automation. The `hwatud` daemon owns WebKitGTK browser windows and listens on a Unix-domain socket resolved by the `hwatu-ipc` crate, normally under the current user's runtime directory. The socket protocol is not an Internet-facing API and is not designed to authenticate mutually-distrusting local processes running as the same user.

Treat any process that can connect to the hwatu socket as able to drive the browser session. By default that includes opening pages, clicking, typing, reading page state, taking screenshots, and running page JavaScript through eval-capable automation. The human can see the same live session the agent drives, including authenticated pages and cookies.

## Eval and prompt-injection risk

The default automation surface includes JavaScript evaluation:

- CLI: `hwatu eval ...`
- CLI one-shots: `hwatu check --eval ...` and `hwatu render --eval ...`
- MCP: the `eval` tool and `eval` parameters embedded in `check`/`render` style tools
- Raw IPC: `Request::Eval`, `Request::Check { eval: ... }`, and batches containing either

If an untrusted or prompt-injected agent reaches these tools while the daemon has authenticated browser state, it can execute same-origin JavaScript in pages the daemon loads. That may expose `document.cookie` when cookies are not `HttpOnly`, web storage, visible DOM content, CSRF-protected form state, and other page-accessible secrets. HttpOnly cookies are not readable via `document.cookie`, but authenticated actions may still be possible through normal browser requests.

## Operator opt-outs

For authenticated sessions or untrusted agent workflows, start the daemon with eval disabled:

```sh
hwatud --no-eval
```

or set:

```sh
HWATUD_NO_EVAL=1 hwatud
```

This policy is enforced in the daemon before dispatch, so direct CLI, MCP, and raw socket requests receive the same rejection. It rejects direct eval, `check`/`render` requests with non-empty eval parameters, and batches containing eval surfaces.

To avoid persisting cookies, credentials, crash-recovery sessions, and discarded-window session blobs, start an ephemeral profile:

```sh
hwatud --ephemeral-profile
```

or set:

```sh
HWATUD_EPHEMERAL_PROFILE=1 hwatud
```

Ephemeral-profile mode uses WebKitGTK's memory-only ephemeral network session, disables persistent credential storage, skips persistent cookie setup, skips crash-session restore/save, and skips normal discarded-window state cleanup/writes. No temporary browser profile is created on disk.

For the strictest local handoff mode, combine both:

```sh
hwatud --no-eval --ephemeral-profile
```

## Local-socket assumptions and limitations

- hwatu does not sandbox, authenticate, or authorize same-user local clients that can reach the daemon socket.
- File permissions and the operating system user boundary are the primary trust boundary.
- A malicious same-user process may still drive non-eval automation such as clicks, typing, screenshots, downloads, or navigation.
- `--no-eval` closes hwatu's explicit eval surfaces. It does not disable JavaScript that websites load themselves.
- `--ephemeral-profile` avoids normal persistent browser/session/profile writes for the daemon. It does not make visited sites private from the network, the compositor, the kernel, or other same-user local observation.
- Secrets visible in page pixels or DOM text may still be exposed by screenshot/snapshot tools even when eval is disabled.

## TCP listener security

When `hwatud --listen` is used, the daemon accepts TCP connections in
addition to the Unix socket. TCP changes the threat model:

- **Token required for non-loopback.** `--listen 0.0.0.0:8741` or
  `--listen [::]:8741` requires `--token` (or `HWATU_TOKEN`). The
  daemon refuses to start without a token on non-loopback binds.
- **Loopback without token is allowed but weaker.** Any local process
  can connect (weaker than the 0700 Unix socket). A warning is printed
  at startup.
- **Token comparison is constant-time** (FNV-1a hash + u64 equality)
  to mitigate timing attacks over the network.
- **Max 64 TCP connections** to prevent fd/queue exhaustion.
- **Max 32 MiB per frame** to prevent memory exhaustion.
- **No TLS/encryption.** Use `ssh -L`, WireGuard, or similar for
  confidentiality. The token authenticates; it is not a transport.

### Attack surface

A successful TCP connection (with valid token) has the same privileges
as a Unix socket client:

- **eval (RCE):** arbitrary JavaScript in any loaded page
- **Screenshots:** data exfiltration of visible content
- **upload/click/type:** drive the user's authenticated browser
- **quit:** kill the daemon (and potentially the user's session)

### Recommendations

- Always use `--token` for TCP, even on loopback
- Prefer `ssh -L` tunnels over direct exposure
- Use `--ephemeral-profile` for untrusted agent workflows
- Use `--no-eval` if the agent only needs navigation/screenshot

## Reporting security issues
## Reporting security issues

Please report suspected vulnerabilities privately when possible. If GitHub private vulnerability reporting is enabled for this repository, use it. Otherwise contact the maintainer listed on the GitHub repository profile, or open a minimal public issue that describes the impact without publishing exploit details. Include your hwatu version, WebKitGTK version, operating system, session type, the daemon startup line, and whether `--no-eval` or `--ephemeral-profile` was in use.
