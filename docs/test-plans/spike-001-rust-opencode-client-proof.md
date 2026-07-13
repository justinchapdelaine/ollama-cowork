# Spike 001 Rust opencode client proof

Verified on 2026-07-12 against an isolated authenticated opencode `1.17.18` server bound to `127.0.0.1`.

The Tauri-free Rust client successfully called health, created a session, and read its empty message list without exposing credentials on the command line. Unit tests additionally cover strict loopback endpoint validation and both direct and live `payload`-wrapped SSE frames.

Machine-readable evidence: `spike-001-rust-opencode-client-proof-result.json`.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\proofs\opencode\run-rust-client-proof.ps1
```
