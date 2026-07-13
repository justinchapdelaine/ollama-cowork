# Spike 001 Proof-Harness Modularity

The proof harness is intentionally separated from the future product implementation, but it now follows the same replaceable boundaries.

## Shared configuration

`scripts/proofs/common/SpikeProofRuntime.psm1` is the single source of truth for:

- repository and evidence paths;
- pinned opencode and SRT versions;
- pinned Windows SRT helper location;
- Ollama origin, OpenAI-compatible base URL, provider ID, and model ID;
- opencode executable discovery.
- reusable remote Ollama health and exact-model preflight.

Proof entrypoints may override explicit parameters for targeted tests, but they do not duplicate defaults.

## Shared opencode runtime

The module owns:

- app-created workspace/config/profile directories;
- ephemeral server password and Basic-auth header;
- restrictive inline provider/permission configuration;
- `--pure`, localhost-only opencode startup;
- health readiness;
- authenticated JSON and raw HTTP requests;
- process shutdown, bounded waiting, failure-log preservation, and verified temp cleanup.

Individual proofs own only their scenario and assertions:

- `run-server-proof.ps1`: listener, authentication, effective permissions, and fail-closed bash override.
- `run-ollama-tool-loop-proof.ps1`: session creation, allowed echo tool, repeated model/tool/result/final loop.
- `run-permission-proof.ps1`: explicit custom-tool `context.ask` plus allow-once, reject, and abort lifecycle assertions. Its standalone SSE driver keeps opencode event/API compatibility logic out of the shared process runtime.

Remote-model proofs run the shared Ollama preflight before starting opencode. An unreachable LAN host or missing exact model therefore fails with a direct configuration diagnosis instead of being misclassified as a model, tool, or permission failure.

## SRT separation

- `run-proof.ps1`: Windows setup inputs, policy/canary scenario, and evidence presentation.
- `coordinator.mjs`: one SRT session, wrapped process execution, timeout handling, reset, and result classification.
- `probe.mjs`: deliberately harmless allowed/denied operations.

The SRT entrypoint consumes the same central version, helper path, and Ollama origin as the opencode proofs.

## Clean-room fixture separation

`create_synthetic_docx_fixture.py` creates only synthetic test input. It is not the production DOCX tool and shares no implementation with it.

## Production mapping

The proof boundaries map to the implemented Rust interfaces and retained integration seams:

```text
SpikeProofRuntime        -> app configuration + OpencodeRuntime adapter
opencode proof scenario  -> integration tests
SRT coordinator          -> SandboxRunner adapter
probe operations         -> security regression fixtures
broker proof             -> ToolBroker interface + adapter composition
DOCX CLI                 -> standalone DocumentTool process boundary
```

The DOCX proof now implements that final boundary as `tools/docx-tool`. Its contract, filesystem policy, package copying, section semantics, and CLI transport are separate modules. `scripts/proofs/docx/srt-docx-coordinator.mjs` owns only SRT orchestration and assertions; it does not contain document mutation logic.

The trusted application-policy layer now lives in `crates/cowork-core`. It defines the transport-independent `ToolBroker`, job and approval domain types, plus `SandboxRunner` and `ArtifactPublisher` ports. It imports no opencode, SRT, DOCX, HTTP, or Tauri types. Registered paths are canonicalized, broker tokens are compared through SHA-256 digests in constant time, and a one-time mutation approval is consumed before the runner is invoked.

Concrete edge implementations live in `crates/cowork-runtime`: `SrtRunner` invokes a fixed versioned Node/SRT bridge, while `ExclusiveDocxPublisher` validates required DOCX ZIP parts and uses exclusive-create publication with numbered collision handling. `tools/broker-proof` composes these modules for integration testing only; it contains no reusable policy or document logic. Reusable authentication and request translation live in `crates/broker-transport` rather than the proof/development broker executable.

`crates/broker-transport` owns the bounded authenticated localhost server lifecycle and request translation. `tools/broker-host` is only the proof/development composition root. It binds explicitly to `127.0.0.1` and uses distinct ephemeral execution and control credentials. Thin opencode tools receive only the execution URL/token through the child-process environment and expose structured DOCX arguments. Approval, rejection, and cancellation use the separate control credential and a job/action-correlated decision route; `/execute` cannot grant approval. The host constructs broker requests from server-owned job identity, job token, source hash, and canonical paths, keeping transport, policy, sandboxing, publication, and document mutation distinct.

`crates/opencode-client` owns pinned opencode process lifecycle, strict loopback endpoint validation, Basic-authenticated access to the approved API subset, and raw SSE parsing including the live `payload` wrapper. `crates/cowork-core` owns normalized `WorkflowCommand`, `WorkflowEvent`, `ModelSession`, and related transport-neutral types. Raw opencode events must be translated before the Tauri frontend receives them.

No production module should import proof-harness types. The proofs validate behavior and contracts; production adapters will implement those contracts independently.
