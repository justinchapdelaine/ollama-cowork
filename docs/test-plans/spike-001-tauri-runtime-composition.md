# Spike 001 Tauri runtime-composition milestone

Status: **ALLOW-ONCE UI PUBLICATION VERIFIED; REMAINING FAILURE LIFECYCLES PENDING**

The desktop crate now has a concrete, modular provisioner for one DOCX job. It composes the already-proven broker host, SRT bridge, clean-room DOCX executable, opencode client, and validated artifact decoder without moving their policy into Tauri.

The provisioner is now wired through a transport-neutral application service to
a Rust-owned native selection command plus five workflow commands: start,
approve once, reject, cancel, and poll. `workflow://event` carries only the normalized core event contract to
the main WebView. Internal integration diagnostics are mapped to stable,
frontend-safe error codes and messages. The WebView receives no generic shell,
filesystem, patch, network, or process capability.

The WebView receives an opaque, single-use selection ID instead of a local path.
The replaceable host path policy rejects UNC/device paths and Windows reparse
points before a selection can become workflow authority.
The application reserves and returns a job ID before background provisioning,
enforces one active Spike 001 workflow, and propagates cooperative cancellation
through broker/opencode readiness. Runtime helpers use a relocatable sibling
layout prepared by target-aware Tauri build hooks. Transient cleanup requires a validated
app-owned root marker before inspecting or removing stale runs.

The frontend is split into an injectable typed Tauri bridge, a pure workflow
state machine, an injected scheduler/polling coordinator, and a DOM renderer.
The renderer builds elements with `textContent`; assistant output, approval
summaries, errors, artifact metadata, and health details never enter an
`innerHTML` path. The coordinator subscribes before enabling the form, binds
early events to the reserved job, serializes polling, and stops polling for
every terminal state.

## Implemented boundaries

- cryptographic, distinct job-scoped opencode, broker-execution, broker-control, and broker-job credentials exposed through role-specific capability views;
- opaque leased per-run workspaces with separate model and private-output directories;
- replaceable loopback port allocator, versioned tool-bundle/runtime-asset materializer, child-process launcher, and readiness probe;
- fixed runtime tool bundle containing fail-closed `bash`, `docx_inspect`, and `docx_rewrite_section`, with `bash` disabled again by a deny-only per-request prompt profile;
- cleared opencode child environment with isolated config/profile roots;
- restrictive inline global policy plus an explicitly selected `spike-docx` primary agent with the same default-deny policy and attachment-aware workflow context;
- remote Models.dev fetching disabled so the explicitly configured Ollama provider does not depend on public-internet metadata during startup;
- sharing and automatic opencode updates disabled for the pinned spike runtime;
- broker control/job credentials withheld from the opencode environment and model-workspace filesystem; the broker receives its opaque bootstrap over inherited stdin;
- broker host and opencode child trees supervised with deterministic reverse-order cleanup;
- challenge-proven broker readiness, exact child-PID listener attestation, and strict `127.0.0.1` API clients;
- independent model-session and broker-authorization handles;
- rollback on broker-readiness and opencode-launch failures, with workspace rollback owned by the outer factory.

The current CLIs still require selecting a free loopback port before each process binds, so allocation itself is not an atomic socket handoff. Readiness rejects a listener unless Windows reports that the exact launched child PID owns it. Broker discovery additionally uses a random challenge and HMAC proof rather than transmitting a reusable credential. A competing bind therefore fails closed before opencode configuration or broker control traffic proceeds.

## Automated evidence

On 2026-07-15 the following passed:

- `cargo test --workspace --all-features` (121 tests, plus the two opt-in live tests described below);
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo fmt --all -- --check`;
- `git diff --check`.
- `npm test` from `apps/desktop` (8 state/coordinator tests);
- `npm run build` from `apps/desktop`.

The prerequisite-health adapter executes identity commands for Node, the broker
host, the DOCX tool, and the SRT helper, and verifies the copied SRT bridge
against the version compiled into the desktop. The runtime-preparation hook
derives Cargo's effective target directory from `cargo metadata`, honors
Tauri's `TAURI_ENV_TARGET_TRIPLE`, and stages the same assets for default and
explicit-target layouts. A custom-target proof passed using
`aarch64-pc-windows-msvc` and an isolated Cargo target directory.
SRT update discovery and runtime trust are intentionally separate. The app pins
the proof-tested npm package `0.0.65`, exact helper response `srt-win 0.0.1`,
helper length, and SHA-256, and revalidates the helper at workflow provisioning.
Maintainers may query the official npm registry for a newer candidate, but must
run the complete Windows enforcement matrix and deliberately advance every
recorded identity before that candidate becomes trusted.
The native Tauri hook additionally proves that a host-target dev run reuses the
normal Cargo output instead of compiling a duplicate dependency tree; only the
two helper executables and bridge are staged into the target-specific sibling
layout.

The minimal UI subsequently launched successfully through `tauri dev` with the
terminal hidden. The desktop process remained responsive, its WebView loaded
from Vite over IPv6 loopback, startup logs contained no application error, and
the configured private-LAN Ollama endpoint exposed exact model `gemma4:12b`.
After the Windows automation runtime was restarted, Computer Use connected to
the rebuilt app, selected only `tests/fixtures/spike-001-original.docx`, entered
the narrow rewrite instruction, and submitted the workflow through the real
Tauri UI. The app reached the friendly `Create a revised DOCX copy?` approval
card with `Allow once`, `Reject`, and `Cancel workflow`. The user selected
`Allow once`; the app reported `job-1.revised.docx` with SHA-256
`03d7a21a583a05a29c6e494189fa790a32d2768186add562007c18d17a019cf7`.
Independent clean-room validation reopened the published package, confirmed
the concise replacement under `Executive Summary`, preserved the adjacent
`Operating Constraints` canary, and confirmed the source remained at SHA-256
`6c692abedd83e1f74fb9313a41790a977483bb4202691d87a5618cf3727b7a29`.
This verifies the interactive allow-once and SRT-backed publication path but
does not claim the remaining reject/cancel/crash scenarios.

The tests include exact-operation approval binding, trusted-inspection-backed current/proposed approval data, secret validation/redaction, oversized-success rejection, bounded child logs, restricted tool-bundle materialization, deny-only prompt-profile validation, exact prompt request serialization without an attachment path, isolated profile roots, distinct port allocation, child/process-tree listener attestation, independent credential exposure, provider/base-URL/model validation, broker-readiness rollback, opencode-launch rollback, reverse cleanup, workspace leasing, descendant process termination, strict loopback clients, role-filtered and bounded assistant-part assembly, exact SRT helper identity rejection, validated DOCX artifact decoding, both valid opencode permission/tool event orders, and fail-closed rejection of any execution signal before host approval.

The opt-in host integration tests pass against the proof-recorded opencode `1.17.18` executable and Windows SRT `0.0.65`. The provisioning gate starts the real supervised one-job broker and opencode process trees, passes challenge-proven and process-tree-attested readiness, validates the restrictive configuration and selected `spike-docx` agent, creates a session, connects the event stream, and removes the transient workspace. The model gate additionally initializes and verifies the exact fail-closed `bash`, read-only `docx_inspect`, and approval-gated `docx_rewrite_section` schemas; explicitly selects `spike-docx`; disables `bash` at request scope; tells the model only that one DOCX attachment is already job-bound; and repeats the natural-language request in two fresh private-LAN `gemma4:12b` sessions. Both sessions permit read-only inspection and reach an exact `RewriteSection` approval request without authorizing mutation.

Live diagnosis found two Windows opencode executables that both reported `1.17.18` but had different hashes and behavior. The passing proof/npm build is pinned to length `179946888` and SHA-256 `D78D0999EADDF4BAE028FFA88106D37F5962931BB9137396D8C5FD77576DD68D`; the PATH-first desktop-installed build was rejected because its hash differed and its managed SSE/tool-loading path reset connections. Desktop resolution now honors an explicit override, otherwise selects a matching candidate from PATH/fallback locations, and health fails closed if neither version nor executable identity matches. The provisioner streams the executable through the same exact length/digest check immediately before each launch attempt. The version probe and child readiness each have bounded retries, while configuration, identity, policy, broker, and model failures remain non-retriable.

```powershell
cargo build -p ollama-cowork-broker-host -p ollama-cowork-docx-tool
$env:OLLAMA_COWORK_LIVE_OPENCODE = "$env:APPDATA\npm\node_modules\opencode-ai\bin\opencode.exe"
$env:OLLAMA_COWORK_LIVE_NODE = (Get-Command node.exe).Source
$env:OLLAMA_COWORK_LIVE_SRT_WIN = 'C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe'
cargo test -p ollama-cowork-desktop --test live_runtime -- --ignored --nocapture
```

Initial live attempts exposed a too-narrow cleared Windows environment, inconsistent Windows home variables, missing first-run tool initialization, a same-version executable mismatch, a translator that rejected the intentionally allowed inspection tool, a public Models.dev lookup, an implicit-agent prompt request, a model-visible fail-closed `bash` override, a model that sometimes asked for conversational confirmation instead of invoking the approval-gated rewrite proposal, and nondeterministic ordering between opencode's tool `running` and `permission.asked` events. The final profile passes a non-secret OS compatibility allowlist, derives `HOMEDRIVE`/`HOMEPATH` from the isolated per-job home, continues to isolate `HOME`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, and `XDG_CONFIG_HOME`, disables remote model-metadata fetching, explicitly selects the agent, disables `bash` without enabling any additional tool, explains the host approval contract without exposing a document path, preloads and validates each required custom tool by its exact ID, description, and parameters field, and separates read-only inspection from mutation events. Inspection envelopes must match the active job and selected source digest before their section text can populate an approval proposal. Model-controlled identifiers and retained tool states are bounded. OpenCode preapproval lifecycle states remain transport details; trusted execution starts only after the Rust host has accepted the exact one-time broker authorization and successfully replied to the matching permission request. Child stdout/stderr are drained through the shared supervisor and capped at 1 MiB per stream; internal diagnostics are bounded and secret-redacted.

## Command and lifecycle evidence

Unit tests use fake workflow and frontend-event adapters to prove that the
application service preserves the narrow command contract, sanitizes internal
errors, and keeps Tauri out of domain policy. Core controller tests prove that
shutdown cancels and cleans every active job and remains idempotent. The actual
Tauri exit callback invokes that shutdown boundary; supervised cleanup remains
responsible for terminating broker, opencode, and SRT process trees.

## Remaining live gate

The runtime/model path and implemented minimal UI are now live-verified through allow-once publication. Before this milestone is considered fully integrated into the product flow, verify that:

1. unexpected user/global/managed configuration cannot broaden the already-validated effective profile;
2. the listener and child trees remain localhost-only and console-free from the packaged app;
3. reject, cancel, broker/opencode crash, and app shutdown behave correctly; and
4. the verified SRT-backed rewrite remains no-overwrite and original-preserving under repeated runs.
