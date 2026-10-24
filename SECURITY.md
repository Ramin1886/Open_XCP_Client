# Security Policy

## Supported versions

Only the latest release on `main` receives fixes.

## Reporting a vulnerability

Please do not open a public issue for security problems. Use GitHub's private vulnerability
reporting ("Report a vulnerability" under the repository's Security tab) or contact the maintainer
listed in `Cargo.toml`'s repository page. You should receive an acknowledgement within a week.

## Scope

`open_xcp_client` processes untrusted data from three sources:

- **The XCP target** over UDP/TCP: frame lengths, DAQ identifiers, `GET_ID` upload sizes, command
  responses. Known gaps are tracked in `docs/TODO.md` (for example T-04 response matching, T-11
  allocation caps).
- **A2L text** uploaded from the target or read from disk: `MATRIX_DIM` products, nesting depth,
  record layouts.
- **Files on disk**: `.sdf` recordings (tail index offsets), `settings.json`, plot profiles.

The client never executes external programs and never opens listening sockets except the optional
built-in mock server on `127.0.0.1`. A report that demonstrates a crash, unbounded allocation, or
out-of-bounds read from any of the inputs above is in scope.
