# Spike 001 trusted broker integration proof

Verified on 2026-07-12.

The composed Rust `ToolBroker`, `SrtRunner`, clean-room DOCX executable, and `ExclusiveDocxPublisher` passed the synthetic workflow:

- an approved-once mutation ran through pinned Windows SRT and produced a validated, exclusively created revised copy;
- rejection produced no artifact;
- cancellation produced no artifact;
- the source SHA-256 was identical before and after all scenarios.

Machine-readable evidence: `spike-001-broker-proof-result.json`.

The proof executable contains setup and assertions only. Policy remains in `crates/cowork-core`; SRT and publication mechanics remain in `crates/cowork-runtime`; DOCX mutation remains in `tools/docx-tool`.

Rerun:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\proofs\broker\run-broker-proof.ps1
```

This individual gate does not cover authenticated loopback transport or opencode custom-tool translation. Those layers were subsequently proven in `spike-001-opencode-docx-proof.md`.
