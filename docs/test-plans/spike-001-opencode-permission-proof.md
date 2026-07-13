# Spike 001 opencode permission lifecycle proof

Verified on 2026-07-12 with opencode `1.17.18` and remote Ollama model `gemma4:12b`.

## Result

The custom `approval_probe` tool explicitly calls opencode's `context.ask`. All required lifecycle outcomes passed:

| Decision | Observed result |
| --- | --- |
| Allow once | `permission.asked` -> reply `once` -> `permission.replied` -> tool completed exactly once -> `session.idle` |
| Reject | `permission.asked` -> reply `reject` -> tool did not complete or execute -> `session.idle` |
| Abort | `permission.asked` -> session abort returned `true` -> tool did not complete or execute -> `session.idle` |

The machine-readable evidence is `spike-001-opencode-permission-proof-result.json`.

The `spike-001-opencode-permission-*-diagnostic.json` files are retained evidence from earlier failed or incomplete stimuli during protocol discovery. They are not the final gate result and are superseded by the passing aggregate result above.

## Compatibility findings

- A replacement custom tool is not automatically approval-gated merely because its permission name is configured as `ask`; it must explicitly invoke `context.ask`.
- This opencode version emits `permission.asked` and accepts decisions at `POST /permission/:requestID/reply` with `{ "reply": "once" | "reject" }`.
- Abort is session-scoped and produces `session.error` followed by `session.idle`; it does not execute the waiting tool.
- Model tool selection is nondeterministic. One allow-once attempt returned text without calling the tool. The harness classified that as a failed stimulus, created a fresh session, and did not confuse it with an authorization success.

## Rerun

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\proofs\opencode\run-permission-proof.ps1
```

The shared preflight fails early if the configured LAN Ollama endpoint is unreachable or exact model `gemma4:12b` is absent.
