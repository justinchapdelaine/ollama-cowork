# Spike 001 clean-room DOCX through SRT

Verified on 2026-07-12 using the standalone Rust DOCX tool and pinned Windows SRT `0.0.65`.

The synthetic `Executive Summary` section was replaced inside SRT. The tool created a new output in the sole write-allowed directory, reopened and validated the package, preserved the adjacent `Operating Constraints` section and its canary, refused to overwrite the output, and left the source SHA-256 unchanged. SRT reset completed without error.

Machine-readable evidence: `spike-001-srt-docx-proof-result.json`.

Rerun:

```powershell
cargo build --manifest-path tools\docx-tool\Cargo.toml
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\proofs\docx\run-srt-docx-proof.ps1
```

This proves the narrow synthetic workflow, not general Word fidelity.
