# ADR-001: Production workspace boundaries

Status: accepted
Date: 2026-07-12

## Decision

Use a root Cargo workspace with one lockfile and keep production policy, runtime adapters, transport, clean-room tools, desktop composition, and proof harnesses separate.

Dependency direction:

```text
apps/desktop/src-tauri
  -> cowork-core
  -> cowork-runtime -> cowork-core
  -> broker-transport -> cowork-core
  -> process-supervisor -> cowork-core cleanup port
  -> opencode-client -> cowork-core workflow ports/types + process-supervisor

proof/development hosts
  -> the same reusable crates

docx-tool
  -> no application crates
```

`cowork-core` must not import Tauri, opencode, SRT, HTTP-server, or DOCX-library types. `cowork-runtime` implements core ports and owns SRT/publication mechanics. `broker-transport` owns authentication, challenge-response endpoint proof, the blocking IPv4-loopback server, and versioned request translation. `process-supervisor` owns job-scoped child-process trees, bounded log capture, and child-PID listener attestation. `opencode-client` owns pinned opencode lifecycle, the approved authenticated API subset, and raw SSE parsing while delegating process-tree enforcement to the supervisor. The standalone DOCX executable and narrow one-job broker host remain process boundaries.

The broker host is process-isolated because Rust cannot safely force-cancel an
arbitrary in-process request or SRT operation. Desktop composition supervises
that host with the same Windows Job Object boundary as opencode; teardown can
therefore terminate a stalled broker and all descendants without blocking the
Tauri process indefinitely. The blocking server is additionally gated behind
`broker-transport`'s opt-in `server` feature, enabled by `broker-host` but not by
the desktop dependency.

Each active workflow is composed from three independent job-scoped handles: a model session, a trusted mutation-authorization boundary, and deterministic cleanup. Broker approval is bound to the exact structured rewrite operation before opencode receives `once`; changed arguments are rejected without consuming approval. If that reply fails, the unconsumed broker approval is revoked and the isolated job runtime is terminated. The opencode event stream is async and explicitly cancellable, while buffering/backpressure belongs to the session adapter rather than core policy.

The opencode model adapter further separates session provisioning, synchronous command calls, async event streaming, raw-event translation, and bounded delivery. The adapter waits for stream readiness before prompt submission and owns no broker authorization. The desktop composition root will combine this model handle with independent broker authorization and cleanup handles through `WorkflowJobFactory`.

The desktop factory separates per-job workspace creation, cryptographic credential
generation, runtime provisioning, and cleanup behind narrow ports. Aggregate credentials are private and exposed only through model-process, broker-bootstrap, and trusted-control capability views. Construction
is atomic at each boundary: failures remove transient workspace state, while a
completed job shuts down runtime resources in reverse construction order before
removing that workspace. Published revised documents are outside the transient
workspace and are never removed by job cleanup.

The concrete desktop provisioner is assembled from replaceable port-allocation,
versioned tool-bundle/asset-materialization, process-launch, and readiness adapters. It creates four
distinct 256-bit credentials, gives opencode only its Basic-auth password and
broker execution token, and transfers broker control/job credentials directly to the broker over inherited stdin, outside the model process and workspace filesystem. The opencode child starts with a cleared environment, isolated
profile roots, duplicated global/dedicated-agent default-deny policy, sharing
and autoupdate disabled, and only the three Spike 001 custom tool files. Broker
and opencode children are supervised independently and rolled back in reverse
order if any subsequent startup step fails.

Workspace paths are nested beneath a cryptographically opaque per-run namespace.
An exclusive Windows lease prevents another app instance from deleting an active
run, and active job handles retain cloned leases even if the factory is dropped.
Startup removes only syntactically valid stale run directories whose lease is no
longer held, so restarted `job-1` counters cannot collide with crash leftovers.

Proof scripts, evidence, and synthetic fixtures retain their Spike 001 names and locations because they are historical verification artifacts, not production modules.

## Consequences

- The desktop crate is a composition root instead of the home of business policy.
- Runtime, process supervision, transport, model backend, and document-tool implementations can be replaced independently.
- All Rust packages share dependency resolution and `Cargo.lock`.
- Proof executables may compose production crates but production crates must never import proof code.
- Product expansion beyond the one validated DOCX workflow requires new adapters and acceptance evidence, not speculative generic abstractions in the core.
