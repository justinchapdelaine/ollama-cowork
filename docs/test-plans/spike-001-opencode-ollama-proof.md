# Spike 001 Remote Ollama and opencode Tool-Loop Proof

Result: **PASS on the recorded host**
Completed: 2026-07-12 (America/Vancouver)

## Remote provider

- Ollama host: configured private-LAN test endpoint (address redacted)
- Ollama version: `0.31.2`
- OpenAI-compatible base URL: configured test origin plus `/v1`
- Exact model tag: `gemma4:12b`
- Exact tag appeared in both `/api/tags` and `/v1/models`.
- No model was pulled, renamed, or substituted.

The host was unreachable from the restricted Codex command environment but reachable from the approved normal-user host context. This confirms the plan's separation between sandbox networking and trusted host/opencode networking.

## Direct OpenAI-compatible API proof

The first request returned:

- finish reason `tool_calls`;
- function `echo_value`;
- schema-valid arguments `{"value":"ready"}`.

An explicit standards-conformant assistant tool-call plus tool-result second turn returned exact final text `DONE` with finish reason `stop`.

One earlier dynamically reconstructed PowerShell second-turn payload produced an extra tool call and malformed-looking content. Replacing it with an explicit OpenAI-compatible assistant/tool message structure resolved the issue. The passing opencode runs below confirm opencode handles this message construction correctly.

## Repeated opencode proof

Pinned opencode `1.17.18` ran localhost-only with:

- ephemeral Basic authentication;
- `--pure`;
- no `--auto`;
- isolated app-created configuration/workspace;
- `permission: { "*": "deny", "echo_value": "allow" }`;
- fail-closed custom `bash` override loaded but not executed;
- provider `ollama-lan/gemma4:12b`.

Three independent sessions passed:

| Run | `echo_value` calls | Input | Output | Final text | Bash executed |
|---|---:|---|---|---|---|
| 1 | 1 | `ready` | `ready` | `DONE` | no |
| 2 | 1 | `ready` | `ready` | `DONE` | no |
| 3 | 1 | `ready` | `ready` | `DONE` | no |

Compact machine-readable evidence is in `spike-001-opencode-ollama-proof-result.json`.

Reruns require the intended endpoint to be provided explicitly through `OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN`; the repository contains no private-LAN default.

## What this proves

- The configured private Ollama endpoint is live from the intended host context.
- The user-provided model tag exists.
- The OpenAI-compatible endpoint supports function calling.
- opencode can create sessions, call the model, execute one allowed custom tool, feed the result back, and receive a clean final response.
- Tool calls, tool results, reasoning parts, and final text are distinguishable in opencode's message representation.
- The chosen model/tool loop was repeatable across three independent sessions.

## What remains

- Approval-event and permission-response behavior for `docx_rewrite_section: ask`.
- Proving a model-triggered bash call is denied before execution, not merely unused.
- Rust tool-broker mediation.
- Clean-room DOCX inspect/rewrite/validate execution through the already proven SRT boundary.
