# Windows sandbox spike

Date: 2026-07-04

Note: this checked-in spike intentionally uses placeholders for hostnames, user profile paths, and LAN endpoints. Raw machine-specific captures belong in ignored `docs/local/` notes.

## Purpose

Validate the Windows MVP direction for Ollama Cowork: a local-first desktop agent that uses Ollama for reasoning, will run side-effecting work inside a Windows OS-level sandbox, reviews a diff, and applies approved changes back to the real project.

The current target is a Codex-inspired elevated Windows OS sandbox, not Windows Sandbox, containers, or a VM-first runtime.

## Tentative product decisions

1. The MVP threat model is protection against accidental or over-broad agent actions during normal local coding tasks. It is not malware-grade containment.
2. Elevated Windows OS sandboxing should be the default runtime. If first-run admin setup is required to create the low-privilege users, ACL policy, and network controls, the MVP should require that setup rather than silently falling back to a weaker default.
3. Network should be off by default. Networked commands, package installs, downloads, and remote Git operations should require approval.
4. The MVP workspace model should be copy-then-patch. Direct edits to the source folder can be a future runtime mode.
5. The model should only request structured tools. It should not emit free-form shell text that bypasses validation.
6. The first smoke-test model was `lfm2.5-thinking:latest` only because it is small and fast enough for plumbing checks. The intended serious baseline is Gemma 4, and the remote `gemma4:12b` probe now makes it the current MVP baseline candidate.

These decisions can change if the dedicated sandbox-user prototype exposes unacceptable friction or missing controls.

## Settled MVP direction

The following decisions are accepted for the first implementation pass:

1. Ollama backend configuration must support both local and remote base URLs, such as `http://127.0.0.1:11434` and a user-supplied LAN Ollama URL. Model calls are host-side app behavior, not sandbox command network access.
2. Thinking is enabled by default. The UI should render `message.thinking` as a separate collapsible block while keeping `message.content`, `message.tool_calls`, and tool results as distinct message types.
3. Manual approvals are the MVP default. Auto-review can be added later through a separate `ApprovalReviewer`; the acting agent must not approve its own risky actions.
4. Whole-patch approval is the first diff/apply target. Per-file and per-hunk approval should be possible later without redesigning the runtime.

Modularity is a core requirement. Each major subsystem should be replaceable behind an interface so the MVP can start narrow without blocking later runtime, model, approval, or diff/apply improvements.

## Auto-approval direction

Codex has an auto-review concept where eligible approvals can be reviewed by another agent, but auto-review does not change the sandbox boundary. Ollama Cowork can use the same pattern, but the acting agent should not approve its own risky action.

Recommended shape:

- `ToolPolicy` deterministically classifies each tool call first.
- Clearly safe, in-sandbox read/edit/test commands can run without approval.
- Risky commands produce an `ApprovalRequest`.
- Manual user approval is the MVP default for network, install, destructive, or host-affecting actions.
- A future `ApprovalReviewer` can use a separate model or rule engine to approve low-risk requests, with strict deny rules for categories that should never be auto-approved.
- Every approval decision should be logged with command, cwd, requested capability, reviewer, reason, and resulting sandbox permissions.

This keeps the design modular without treating model judgment as a security boundary.

## Modular architecture targets

Keep each replaceable part behind a narrow interface:

- `ModelBackend`: Ollama first, possible future providers later.
- `AgentOrchestrator`: message loop, tool-call parsing, tool-result feedback.
- `ToolRegistry`: structured tool schemas and handlers.
- `ToolPolicy`: deterministic allow/prompt/deny decisions.
- `ApprovalReviewer`: user approval first, optional auto-review later.
- `WorkspaceRuntime`: Windows elevated OS sandbox first, unelevated and VM runtimes later.
- `CommandRunner`: timeout, stdout/stderr capture, exit code, process-tree cleanup.
- `DiffEngine`: changed-file inventory and patch generation.
- `PatchApplier`: apply approved changes to the source folder.
- `SessionStore`: SQLite task logs, tool calls, approvals, command outputs, and applied patches.

The agent should not know which `WorkspaceRuntime` is active.

## Environment survey

Host OS, from elevated read-only query:

```text
OsName:         Microsoft Windows 11 Pro
OsVersion:      10.0.26200
OsBuildNumber:  26200
OsArchitecture: ARM 64-bit Processor
```

Optional Windows features, from elevated read-only query:

```text
VirtualMachinePlatform                  Disabled
Microsoft-Hyper-V-All                   Disabled
Containers                              Disabled
Containers-DisposableClientVM           Disabled
Containers-HNS                          Disabled
Containers-SDN                          Disabled
Containers-Server-For-Application-Guard Disabled
```

This is useful evidence for the MVP direction: Codex elevated OS sandboxing works here even though VMP, Hyper-V, Windows Sandbox, and Containers are disabled.

## Codex sandbox observations

The local Codex config contains:

```toml
[windows]
sandbox = "elevated"
```

Commands launched by Codex run as:

```text
<HOSTNAME>\CodexSandboxOffline
```

The sandbox command identity is not an administrator:

```text
IsAdmin: False
```

Local sandbox principals exist:

```text
CodexSandboxOffline  Enabled  Local user
CodexSandboxOnline   Enabled  Local user
CodexSandboxUsers    Local group
```

Codex sandbox files observed:

```text
<USER_PROFILE>\.codex\.sandbox
<USER_PROFILE>\.codex\.sandbox-bin
```

Codex app resources observed:

```text
codex.exe
codex-command-runner.exe
codex-windows-sandbox-setup.exe
```

Recent sandbox logs show:

```text
setup refresh: spawning codex-windows-sandbox-setup.exe
setup refresh: processed 2 write roots (read roots delegated); errors=[]
helper launch resolution: using copied command-runner path ...\codex-command-runner-<version>.exe
read-acl-only mode: applying read ACLs
junction: created <SANDBOX_PROFILE>\.codex\.sandbox\cwd\... -> <WORKSPACE_ROOT>
```

The repo ACL includes `CodexSandboxUsers` and a per-workspace SID with modify permissions. This suggests Codex grants controlled filesystem access to selected roots, then runs commands through a sandbox command runner as the low-privilege sandbox identity.

## Safe tests run

### Filesystem boundary

Inside-workspace write:

```text
Path:   <WORKSPACE_ROOT>\.sandbox-spike-inside-write.tmp
Result: succeeded
Cleanup: succeeded
```

Outside-workspace write:

```text
Path:   <WORKSPACE_SIBLING>\ollama-cowork-outside-write.tmp
Result: Access to the path is denied.
```

Conclusion: Codex's current elevated sandbox grants write access inside the workspace and blocks a sibling-path write outside the workspace.

### Network boundary

Inside the sandbox:

```text
curl.exe --head --max-time 5 https://<external-doc-host>
curl: (7) Failed to connect to <external-doc-host> port 443
```

After an explicit escalated command approval, fetching an external documentation page succeeded.

Conclusion: the sandboxed command path is offline by default, and approval can cross that boundary.

### Ollama reachability

Initial result before Ollama was installed:

```text
Invoke-RestMethod http://127.0.0.1:11434/api/version
Unable to connect to the remote server
```

Outside the sandbox with host-level check:

```text
Get-Command ollama
Found: False
```

After installing Ollama and pulling `lfm2.5-thinking:latest`, the local API became reachable from inside the Codex sandbox:

```text
GET http://127.0.0.1:11434/api/version
{"version":"0.31.1"}
```

The `ollama` CLI was still not visible on the sandbox command `PATH`, but the API was usable. This is enough for the intended product architecture because the host app should call Ollama directly; sandbox commands should not need the Ollama CLI.

The installed model inventory included:

```text
name:               lfm2.5-thinking:latest
size:               731163903
family:             lfm2
parameter_size:     1.2B
quantization_level: Q4_K_M
context_length:     128000
capabilities:       completion, tools, thinking
```

Conclusion: local Ollama API access works, including from this sandboxed Codex command context. External network was still blocked, so localhost access and internet access should be treated as distinct policy categories in Ollama Cowork.

Remote Ollama check:

```text
Host:    <LAN_OLLAMA_HOST>:11434
Version: 0.31.1
```

The remote Ollama server was not reachable from inside the Codex sandbox, but it was reachable after explicit host-level network approval. Its model inventory includes:

```text
gemma4:12b
  family:             gemma4
  parameter_size:     11.9B
  quantization_level: Q4_K_M
  context_length:     262144
  capabilities:       completion, tools, thinking, vision

gemma4:latest
  family:             gemma4
  parameter_size:     8.0B
  quantization_level: Q4_K_M
  capabilities:       completion, tools, thinking
```

Conclusion: Gemma 4 testing can use the remote Ollama host even though this development machine cannot load `gemma4:12b` locally. This reinforces the need for the app to support a configurable Ollama base URL, not only `localhost`.

### Ollama tool-call smoke test

Model tested for a cheap plumbing smoke test:

```text
lfm2.5-thinking:latest
```

Simple chat worked, but the model included `<think>...</think>` content even when the request set `think=false`.

A first tool-call probe with a `list_files(path)` tool did produce a valid `tool_calls` array, but only after a long thinking preamble. The model chose a poor inferred path (`/repository`) when the prompt said "repository root" without explicitly defining the root path.

A constrained prompt with a low `num_predict` cap did not reach the tool call before hitting:

```text
done_reason: length
```

With a higher token cap and explicit instruction to call `list_files` with path `.`, the model produced the desired tool call:

```json
{
  "function": {
    "name": "list_files",
    "arguments": {
      "path": "."
    }
  }
}
```

Implications:

- `lfm2.5-thinking:latest` is good enough for an initial local API and tool-call smoke test.
- Its behavior should not be treated as authoritative for the final product model.
- Gemma 4 has now had a basic remote tool-call evaluation and looks like the current MVP baseline candidate.
- The orchestrator should key off structured `tool_calls`, not natural-language content.
- Thinking text should be displayed separately from tool execution plumbing.
- Tool prompts need precise workspace conventions, such as "`.` is the workspace root."
- Token budgets must account for thinking tokens, or tool calls may be truncated before they appear.
- Additional model candidates can still be tested later, but they are not required before starting the MVP architecture.

### Remote Gemma 4 tool-call probe

Remote model:

```text
Host:  <LAN_OLLAMA_HOST>:11434
Model: gemma4:12b
```

Most initial Gemma 4 probes in this section set `think=false` to isolate tool-call behavior. A follow-up probe with `think=true` showed that Gemma 4 returns thinking in a separate `message.thinking` field while keeping `message.content` empty and `message.tool_calls` clean.

UX direction: thinking should be enabled by default and rendered as a separate collapsible message section, similar to Codex-style thinking/reasoning disclosure. Tool execution should still use only structured `tool_calls`.

Simple chat probe:

```text
Prompt: Reply with exactly: ready
Result: ready
Total duration: about 5.1s including model load
```

Single tool-call probe:

```json
{
  "message": {
    "role": "assistant",
    "content": "",
    "tool_calls": [
      {
        "function": {
          "name": "list_files",
          "arguments": {
            "path": "."
          }
        }
      }
    ]
  },
  "done_reason": "stop"
}
```

This returned in under one second after the model was loaded.

Full tool loop probe:

1. User asked Gemma 4 to list the repository root and summarize it.
2. Gemma 4 returned a clean `list_files(path=".")` tool call with empty natural-language content.
3. The local tool result supplied these root entries:

```text
.agents, .git, docs, .gitignore, LICENSE, README.md
```

4. Gemma 4 returned a concise summary and did not request another tool call.

Conclusion: `gemma4:12b` looks much more promising than `lfm2.5-thinking:latest` for the MVP agent baseline. It produced clean structured tool calls, kept thinking separate when enabled, and handled the second tool-result turn correctly.

Thinking-enabled tool-call probe:

```json
{
  "message": {
    "role": "assistant",
    "content": "",
    "thinking": "The user wants to list the files in the repository root. I should use the `list_files` tool and provide \".\" as the path.",
    "tool_calls": [
      {
        "function": {
          "name": "list_files",
          "arguments": {
            "path": "."
          }
        }
      }
    ]
  }
}
```

Conclusion: Gemma 4 thinking output is compatible with the desired UI model because it arrives separately from natural-language content and structured tool calls.

### Copy and diff

A temp source folder was copied, then the copy was modified with:

- one changed file,
- one new file,
- one deleted file.

`git diff --no-index` detected all three change categories:

```text
GitDiffExitCode: 1
DiffLineCount:   21
HasChangedLine:  True
HasNewFile:      True
HasDeletedFile:  True
```

Conclusion: Git plumbing can support the first reviewable diff prototype. A Rust-native diff library can be evaluated later if Git availability becomes a product concern.

## Tests not run yet

These require explicit approval because they would make system-level changes:

1. Create Ollama Cowork's own `OllamaCoworkSandboxOffline` and `OllamaCoworkSandboxOnline` users.
2. Create an `OllamaCoworkSandboxUsers` group.
3. Apply and later clean up ACL grants for a copied workspace.
4. Add firewall rules for offline-by-default sandbox users.
5. Prove an online sandbox user or per-command network exception can reach approved endpoints.
6. Validate profile/temp cleanup for the sandbox users.

These still need implementation work:

1. Launch a command as the dedicated sandbox user from our own runner.
2. Capture stdout, stderr, exit code, and timing.
3. Enforce timeout and kill a process tree.
4. Generate a structured change list, not only a raw patch.
5. Apply selected changes back to the source folder.
6. Later, optionally test additional Ollama models for structured tool-call reliability, latency, and thinking behavior.

## Current recommendation

Continue with the elevated Windows OS sandbox MVP.

Next build step:

1. Add a small Rust or PowerShell-assisted spike runner that operates on a copied sample workspace.
2. First run it without creating system users, using the current Codex sandbox only for development safety.
3. Then, with explicit admin approval, create the Ollama Cowork sandbox users/group and validate our own command-launch, ACL, network, timeout, diff, apply, and cleanup behavior.

Do not invest in VMP/HCS, Windows Sandbox, containers, or VM image lifecycle until the OS-level sandbox spike fails a concrete requirement.
