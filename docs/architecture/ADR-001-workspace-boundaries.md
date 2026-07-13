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
  -> opencode-client -> cowork-core workflow ports/types

proof/development hosts
  -> the same reusable crates

docx-tool
  -> no application crates
```

`cowork-core` must not import Tauri, opencode, SRT, HTTP-server, or DOCX-library types. `cowork-runtime` implements core ports and owns SRT/publication mechanics. `broker-transport` owns authentication, bounded localhost server lifecycle, and versioned request translation. `opencode-client` owns pinned process lifecycle, the approved authenticated API subset, and raw SSE parsing. The standalone DOCX executable remains a process boundary.

Each active workflow is composed from three independent job-scoped handles: a model session, a trusted mutation-authorization boundary, and deterministic cleanup. Broker approval is established before opencode receives `once`; if that reply fails, the unconsumed broker approval is revoked and the isolated job runtime is terminated. The opencode event stream is async and explicitly cancellable, while buffering/backpressure belongs to the session adapter rather than core policy.

The opencode model adapter further separates session provisioning, synchronous command calls, async event streaming, raw-event translation, and bounded delivery. The adapter waits for stream readiness before prompt submission and owns no broker authorization. The desktop composition root will combine this model handle with independent broker authorization and cleanup handles through `WorkflowJobFactory`.

Proof scripts, evidence, and synthetic fixtures retain their Spike 001 names and locations because they are historical verification artifacts, not production modules.

## Consequences

- The desktop crate is a composition root instead of the home of business policy.
- Runtime, transport, model backend, and document-tool implementations can be replaced independently.
- All Rust packages share dependency resolution and `Cargo.lock`.
- Proof executables may compose production crates but production crates must never import proof code.
- Product expansion beyond the one validated DOCX workflow requires new adapters and acceptance evidence, not speculative generic abstractions in the core.
