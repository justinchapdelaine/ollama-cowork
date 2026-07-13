# ollama-cowork

A local-first desktop document workflow using Tauri, opencode, remote Ollama, mandatory sandbox enforcement, and clean-room document tools.

## Workspace

- `crates/cowork-core`: trusted domain policy and application orchestration.
- `crates/cowork-runtime`: replaceable SRT and artifact-publication adapters.
- `crates/broker-transport`: authenticated broker request translation.
- `crates/opencode-client`: pinned process lifecycle, authenticated localhost API, and SSE parsing.
- `tools/docx-tool`: standalone clean-room DOCX executable.
- `tools/broker-host`: narrow one-job localhost process used by proofs and desktop composition.
- `tools/broker-proof`: broker integration proof executable.
- `scripts/proofs`: retained Spike 001 validation harnesses.
- `docs/test-plans`: retained human- and machine-readable evidence.
- `tests/fixtures`: synthetic validation inputs.

The Tauri desktop application lives under `apps/desktop`. Its composition layer now owns opaque leased job workspaces, cryptographic job credentials, replaceable opencode tool bundles, isolated configuration, loopback allocation plus child-PID listener attestation, bounded supervised child I/O, challenge-proven broker readiness, model-session construction, exact-operation broker authorization, validated artifact decoding, and reverse-order rollback. Broker bootstrap/control material is transferred over inherited stdin rather than written beneath the model workspace. These remain behind replaceable ports; the next milestone is exposing the composed workflow through narrow Tauri commands and the minimal DOCX UI.

## Ollama configuration

The application does not require a specific Ollama IP address. It uses `http://127.0.0.1:11434` as the desktop fallback and accepts `OLLAMA_COWORK_OLLAMA_ORIGIN` and `OLLAMA_COWORK_OLLAMA_MODEL` overrides. Remote proof scripts require `OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN` to be set explicitly so a developer's private LAN address is never a committed default. See `.env.example` for non-sensitive examples.

## Workspace checks

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
git diff --check
```

Spike evidence is intentionally retained. See the [Spike 001 plan](docs/spikes/spike-001-plan.md) and [`docs/test-plans/`](docs/test-plans/).
