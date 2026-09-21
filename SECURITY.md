# Security Policy

## Supported versions

Only the current release line receives security fixes.

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |
| < 0.1   | No        |

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting (the repository's
**Security** tab → *Report a vulnerability*) instead of opening a public
issue, so the details stay private while a fix is prepared. If private
reporting is unavailable for any reason, open an issue that says only
"security concern — details shared privately" and we will arrange a
channel.

Include in the report:

- The rutter version (`rutter --version`) and your OS.
- How the engine was acquired (downloaded, cached, or
  `--engine-executable`).
- The tool sequence or command line that triggers the problem, and the
  full error text including the `hint:` line.

Redact any URLs you cannot share and note that in the report.

## Scope

In scope — the surfaces rutter itself implements:

- The dashboard's loopback web server: per-launch token, loopback
  `Host` validation, WebSocket replay/stream protocol
  ([`docs/dashboard.md`](docs/dashboard.md)).
- The engine download path: pinned Chrome for Testing host, https-only
  artifact URLs, fetched/extracted byte caps
  ([`docs/engine-supervision.md`](docs/engine-supervision.md)).
- The MCP HTTP transport: browser-originated request rejection
  (Origin validation) and the loopback-only bind default
  ([`docs/tool-catalog.md`](docs/tool-catalog.md)).
- Session storage state on disk: atomic writes, owner-only permissions
  on Unix ([`docs/sessions.md`](docs/sessions.md)).

Out of scope, by design ([`docs/architecture.md`](docs/architecture.md)):

- The browser engine itself (Chrome for Testing) and any page content
  it loads: rutter drives a real browser, so visited sites are
  untrusted input. The policy/approval layer is rutter's control, not
  sandboxing.
- Hardening `--allow-remote` or `--http` non-loopback binds for
  internet exposure: the HTTP transport carries no authentication and
  is meant for loopback use; the CLI warns on every non-loopback bind.
  Reports here are handled as documentation issues unless a loopback
  default is bypassed.

## Disclosure

We prefer coordinated disclosure: the report stays private until a fix
is released, and the fix notes credit the reporter unless they opt out.
