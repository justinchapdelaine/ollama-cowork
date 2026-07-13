# Spike 001 Tauri runtime-composition milestone

Status: **LIVE RUNTIME PROVISIONING PASS; TAURI COMMAND FLOW PENDING**

The desktop crate now has a concrete, modular provisioner for one DOCX job. It composes the already-proven broker host, SRT bridge, clean-room DOCX executable, opencode client, and validated artifact decoder without moving their policy into Tauri.

## Implemented boundaries

- cryptographic, distinct job-scoped opencode, broker-execution, broker-control, and broker-job credentials exposed through role-specific capability views;
- opaque leased per-run workspaces with separate model and private-output directories;
- replaceable loopback port allocator, versioned tool-bundle/runtime-asset materializer, child-process launcher, and readiness probe;
- fixed three-tool opencode surface: fail-closed `bash`, `docx_inspect`, and `docx_rewrite_section`;
- cleared opencode child environment with isolated config/profile roots;
- restrictive inline global policy and selected `spike-docx` primary agent with the same default-deny policy;
- sharing and automatic opencode updates disabled for the pinned spike runtime;
- broker control/job credentials withheld from the opencode environment and model-workspace filesystem; the broker receives its opaque bootstrap over inherited stdin;
- broker host and opencode child trees supervised with deterministic reverse-order cleanup;
- challenge-proven broker readiness, exact child-PID listener attestation, and strict `127.0.0.1` API clients;
- independent model-session and broker-authorization handles;
- rollback on broker-readiness and opencode-launch failures, with workspace rollback owned by the outer factory.

The current CLIs still require selecting a free loopback port before each process binds, so allocation itself is not an atomic socket handoff. Readiness rejects a listener unless Windows reports that the exact launched child PID owns it. Broker discovery additionally uses a random challenge and HMAC proof rather than transmitting a reusable credential. A competing bind therefore fails closed before opencode configuration or broker control traffic proceeds.

## Automated evidence

On 2026-07-13 the following passed:

- `cargo test --workspace --all-features` (85 tests, plus the opt-in live test described below);
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo fmt --all -- --check`;
- `git diff --check`.

The tests include exact-operation approval binding, secret validation/redaction, oversized-success rejection, bounded child logs, restricted tool-bundle materialization, isolated profile roots, distinct port allocation, child/process-tree listener attestation, independent credential exposure, provider/base-URL/model validation, broker-readiness rollback, opencode-launch rollback, reverse cleanup, workspace leasing, descendant process termination, strict loopback clients, and validated DOCX artifact decoding.

The opt-in host integration test also passed against pinned opencode `1.17.18` and Windows SRT `0.0.65`. It started the real supervised one-job broker and opencode process trees, passed challenge-proven and process-tree-attested readiness, read back and validated the effective restrictive configuration and selected `spike-docx` agent, created an opencode session, connected the event stream, stopped the listeners/process trees, and removed the transient job workspace. It intentionally did not submit a model prompt or mutate a document.

```powershell
cargo build -p ollama-cowork-broker-host -p ollama-cowork-docx-tool
$env:OLLAMA_COWORK_LIVE_OPENCODE = (Get-Command opencode.exe).Source
$env:OLLAMA_COWORK_LIVE_NODE = (Get-Command node.exe).Source
$env:OLLAMA_COWORK_LIVE_SRT_WIN = 'C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe'
cargo test -p ollama-cowork-desktop --test live_runtime -- --ignored --nocapture
```

An initial live attempt failed closed because the cleared Windows child environment was too narrow for the pinned executable. The final profile explicitly passes a non-secret OS compatibility allowlist while continuing to isolate `HOME`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, and `XDG_CONFIG_HOME`. Child stdout/stderr are drained through the shared supervisor and capped at 1 MiB per stream; startup errors read at most 4 KiB from each and redact all job credentials.

## Remaining live gate

Before this milestone is considered integrated into the product flow, drive the same provisioner through narrow Tauri commands and verify that:

1. unexpected user/global/managed configuration cannot broaden the already-validated effective profile;
2. the listener and child trees remain localhost-only and console-free from the packaged app;
3. allow-once, reject, cancel, broker/opencode crash, and app shutdown behave correctly; and
4. the live SRT-backed rewrite creates only a revised DOCX copy while preserving the original.
