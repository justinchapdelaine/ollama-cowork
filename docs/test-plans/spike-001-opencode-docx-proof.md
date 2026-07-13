# Spike 001 headless opencode-to-DOCX proof

Verified on 2026-07-12 with opencode `1.17.18`, remote Ollama `gemma4:12b`, Windows SRT `0.0.65`, the authenticated Rust loopback broker host, and the clean-room Rust DOCX tool.

## Result

| Decision | Permission/tool result | Published artifacts |
| --- | --- | ---: |
| Allow once | permission requested; separate Rust control decision accepted; replied `once`; tool completed through broker/SRT | 1 |
| Reject | permission requested; separate Rust rejection accepted; replied `reject`; tool did not complete | 0 |
| Abort | permission requested; separate Rust cancellation accepted; session abort returned true; tool did not complete | 0 |

The broker bound only to `127.0.0.1` and required distinct ephemeral execution and control Bearer tokens. The TypeScript tools received only the execution URL/token through process environment and accepted only structured DOCX arguments; they contained no control credential, SRT, executable, source, output, or job-token parameters. The permission driver submitted the job/action-correlated control decision before replying to opencode, and the broker execution route could not approve itself.

Machine-readable evidence: `spike-001-opencode-docx-proof-result.json`.

Rerun:

```powershell
$env:OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN = 'http://<private-lan-host>:11434'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\proofs\broker\run-opencode-docx-proof.ps1
```

Model latency was substantial across three fresh scenarios, but every required authorization and artifact assertion passed.
