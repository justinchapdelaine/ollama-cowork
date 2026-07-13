# Spike 001 opencode Server and Policy Proof

Result: **PASS on the recorded host**
Completed: 2026-07-12 (America/Vancouver)
opencode: `1.17.18`, Windows ARM64

## Proven behavior

- `opencode serve` ran headlessly from an app-created temporary workspace.
- Listener was only `127.0.0.1:40961`; no `0.0.0.0` listener was present.
- HTTP Basic authentication used an ephemeral random password.
- Unauthenticated `/global/health` returned `401`.
- Authenticated `/global/health` returned healthy version `1.17.18`.
- Authenticated `/doc` returned `200`.
- Effective inline configuration reported `permission: { "*": "deny" }`.
- Configured model remained `ollama-lan/gemma4:12b` with the user-specified private endpoint.
- Server ran with `--pure`; external plugins were disabled.
- Server did not use `--auto`; auto-approval remained false.
- Experimental tool schemas loaded successfully after a bounded first-run delay.
- The live `bash` schema contained the unique description from our fail-closed project tool, confirming that it replaced the built-in `bash` implementation.
- The server process was stopped and the app-created temporary workspace was removed after the proof.

Machine-readable evidence is in `spike-001-opencode-server-proof-result.json`.

## Pinned executable

- Version: `1.17.18`
- SHA-256: `D78D0999EADDF4BAE028FFA88106D37F5962931BB9137396D8C5FD77576DD68D`
- Installation: official `opencode-ai` npm package with postinstall scripts allowed only for `opencode-ai`

## Configuration isolation used

- app-created working directory;
- temporary `XDG_CONFIG_HOME` and `APPDATA`;
- restrictive `OPENCODE_CONFIG_CONTENT`;
- `--pure` external-plugin isolation;
- no CORS origins;
- mDNS left disabled;
- loopback-only hostname;
- ephemeral Basic-auth credential;
- default-deny permissions.

## Findings

The first request to `/experimental/tool/ids` exceeded ten seconds while the isolated profile loaded custom-tool dependencies. A bounded 60-second timeout passed consistently on the subsequent instrumented runs. Health, auth, docs, and config endpoints were responsive before tool loading completed.

The tool-ID inventory still lists built-in tool identifiers even under default-deny policy. Security therefore depends on verified effective permissions and the custom override/tool broker, not on built-in identifiers disappearing from this diagnostic endpoint.

## Outside this individual proof; subsequently proven

- Session creation, async prompt events, and permission-response behavior were subsequently proven in `spike-001-opencode-permission-proof.md` and `spike-001-rust-opencode-client-proof.md`.
- Model-triggered custom-tool behavior under default-deny policy was subsequently exercised by the permission and full DOCX proofs; the fail-closed `bash` override remained unexecuted.
- Remote Ollama reachability and repeated function calling through opencode were subsequently proven in `spike-001-opencode-ollama-proof.md`.
- Rust broker, SRT, clean-room DOCX, exclusive publication, rejection, and cancellation were subsequently proven in `spike-001-broker-proof.md` and `spike-001-opencode-docx-proof.md`.

This document remains the evidence for the narrower server/auth/configuration gate; later proof documents provide the end-to-end evidence.
