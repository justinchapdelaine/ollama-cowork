# Spike 001 Feasibility Assessment and Implementation Plan

Status: Tauri shell and modular workflow-composition foundation complete; concrete job runtime provisioning and controls are next
Scope: one DOCX workflow using Tauri, opencode, remote Ollama, SRT, and clean-room DOCX tooling
Verification date: 2026-07-11 (America/Vancouver)

## Executive recommendation

Spike 001's headless architecture is **feasible on the recorded host**, and the first minimal Tauri shell now builds and starts successfully. The required risk-reduction sequence was completed in this order:

1. SRT on Windows, including its one-time elevated installation, positive filesystem cases, negative filesystem cases, and network denial.
2. opencode headless server, authentication, configuration isolation, permission events, and a same-name `bash` override.
3. remote Ollama discovery and a real opencode tool-call loop using `gemma4:12b`.
4. a clean-room DOCX round trip through SRT that creates a revised copy and leaves the source byte-for-byte unchanged.

Those gates, the trusted broker integration, allow/reject/abort behavior, reusable broker server, and Rust opencode client now pass with recorded evidence. Tauri implementation must reuse these boundaries rather than recreating them inside the desktop crate.

The architecture is viable in principle because current primary documentation confirms that:

- Tauri 2 can expose narrow Rust commands to a WebView and constrain frontend capabilities.
- `opencode serve` is a localhost-default headless HTTP server with OpenAPI, sessions, messages, SSE events, and permission-response APIs.
- opencode supports OpenAI-compatible Ollama endpoints, granular `allow`/`ask`/`deny` permissions, and same-name custom tools that take precedence over built-ins.
- Ollama exposes model discovery and OpenAI-compatible chat completions with tool calling.
- SRT now has **alpha Windows support**, using a dedicated local account, Windows Filtering Platform rules, NTFS ACLs, a restricted token, and a Job Object.
- A standalone clean-room Rust tool can copy the DOCX ZIP package, replace the validated narrow `word/document.xml` section, reopen the result, and preserve the source hash without requiring Python.

The main feasibility risk is not Tauri or DOCX manipulation. It is proving that **every model-reachable command path is either denied or mediated by the trusted Rust tool broker and SRT-wrapped**, especially on alpha Windows SRT, while opencode remains useful as a hidden backend.

## Feasibility assessment

| Area | Assessment | Evidence and condition |
|---|---|---|
| Tauri desktop shell | Feasible | Tauri 2 supports narrow Rust commands, scoped capabilities, native dialogs, and managed child processes/sidecars. Do not grant the WebView shell/process permissions. |
| Hidden opencode backend | Verified headlessly | opencode `1.17.18` ran on authenticated localhost with the approved API subset, restrictive configuration, permission lifecycle, and thin structured tools. Tauri lifecycle integration remains to be tested. |
| Pre-installed opencode | Verified spike prerequisite | opencode `1.17.18` is installed and version-gated by the runtime/client boundary. Packaging or installing it remains out of scope. |
| Remote Ollama | Verified on the recorded LAN host | Ollama `0.31.2` exposed the exact `gemma4:12b` tag through discovery and OpenAI-compatible endpoints. The proof harness now fails early when the private host is unavailable. |
| `gemma4:12b` tool calling | Verified headlessly | Repeated opencode sessions completed structured tool loops, and the final DOCX proof passed allow, reject, and abort scenarios. Model latency remains operationally significant. |
| SRT command enforcement | Verified for the prototype host; still alpha | Pinned SRT `0.0.65` passed filesystem, network, descendant, timeout, reset, broker, and DOCX gates on Windows ARM64/NTFS. Productized setup and upgrades remain later work. |
| Generic Office edit blocking | Verified headlessly with layered controls | Default-deny opencode policy, fail-closed `bash`, structured custom tools, trusted broker validation, exact private artifact checking, and revised-copy-only DOCX execution all passed. Tauri must not expose raw backend routes. |
| Clean-room DOCX rewrite | Verified for the narrow synthetic workflow | The standalone Rust tool creates a new output, validates required package parts, reopens it, preserves the adjacent canary, rejects overwrite, and leaves the original hash unchanged. General Word fidelity is not claimed. |
| Preview | Feasible as a staged enhancement | First show a structured before/after text preview. Add LibreOffice headless PDF rendering only if installed and its fidelity/process behavior passes a separate proof. |
| Productized sidecars | Plausible later, out of scope | Tauri supports external binaries, but opencode/SRT versioning, Windows installation, signing, and updates should not block this spike. |

The completed proofs provide no reason to replace opencode for Spike 001. A custom backend would reintroduce sessions, events, tool routing, approvals, and provider plumbing. Reconsider only if Tauri integration reveals an uncontrolled authority path or unacceptable lifecycle/reliability behavior.

## Security invariants

These are acceptance conditions, not aspirations:

1. The original DOCX is never opened for write and never replaced.
2. Every revised document is created at a trusted, collision-safe path inside the selected output directory.
3. The model cannot select an arbitrary executable, arbitrary output path, or raw command string for DOCX operations. opencode custom tools request operations from the trusted Rust tool broker; they do not spawn SRT, Python, or the DOCX executable directly.
4. opencode built-in `edit`, `write`, and `apply_patch` are denied for this spike. Prefer denying `edit` globally because it controls all three.
5. Built-in `bash` is denied and also replaced by a fail-closed same-name custom tool during the enforcement proof. If a shell capability is later needed, the replacement accepts structured operations, not free-form shell text, and launches only through SRT.
6. The Rust host never exposes opencode's shell endpoint to the WebView.
7. The WebView has no Tauri shell or unrestricted filesystem capability.
8. opencode binds to `127.0.0.1`, mDNS remains disabled, no CORS origin is added unless strictly required, and HTTP Basic authentication uses an ephemeral password supplied by the Rust host.
9. There is no global permission `allow`, no approval bypass, and no model-controlled “always allow” persistence.
10. SRT denies network access for the DOCX process and permits only the minimum read/write paths needed for the selected input, tool runtime, temporary work, and revised output.
11. Host/opencode network access to Ollama is separate from the sandboxed DOCX process. The model endpoint is never added to the DOCX sandbox allowlist.
12. A timeout terminates the full DOCX process tree, and cleanup is verified after success, failure, cancellation, and host crash recovery.

## Repository and planned app structure

The validated Rust workspace now uses the following production, desktop, tool, and retained-proof boundaries.

```text
ollama-cowork/
  README.md
  Cargo.toml                     # unified Rust workspace
  Cargo.lock                     # one locked dependency graph
  crates/
    cowork-core/                 # domain policy, broker, normalized workflow contracts
    cowork-runtime/              # SRT runner + exclusive artifact publisher
    broker-transport/            # authenticated localhost protocol/server
    opencode-client/             # pinned process, HTTP API subset, SSE parsing
    process-supervisor/          # kill-on-close job-scoped process trees
  docs/
    spikes/
      spike-001-plan.md           # verified feasibility and implementation plan
    architecture/
      ADR-001-workspace-boundaries.md
      spike-001-proof-modularity.md
    test-plans/
      spike-001-*.md/json         # retained proof narratives and evidence
  apps/
    desktop/                      # Tauri composition root + TypeScript WebView
      src/                         # WebView UI; no shell/process authority
      src-tauri/
        src/
          commands/               # narrow invoke handlers
          composition/            # wires reusable crates; no duplicate policy
          config/                  # validated centralized app config
        capabilities/              # least-privilege Tauri capabilities
  tools/
    docx-tool/                     # standalone clean-room DOCX executable
    broker-host/                   # narrow process-isolated one-job host
    broker-proof/                  # integration proof executable
  scripts/
    runtime/                       # fixed SRT bridge
    proofs/
      common/                      # shared pinned config + runtime lifecycle
      opencode/                    # isolated server and tool-loop scenarios
      srt-windows/                 # policy coordinator + hostile probes
      broker/                      # broker and full headless scenarios
      docx/                        # clean-room DOCX/SRT scenario
  tests/
    fixtures/                      # synthetic DOCX inputs
```

Keep dependencies pointing inward through small interfaces:

```text
WebView -> narrow Tauri commands -> cowork-core workflow contracts/services
desktop composition -> opencode-client / broker-transport / cowork-runtime
opencode-client + broker host lifecycle -> process-supervisor
opencode custom tools -> authenticated broker-transport -> cowork-core ToolBroker
cowork-runtime implements SandboxRunner / ArtifactPublisher ports
SRT -> standalone clean-room docx-tool
```

`cowork-core` must not import Tauri, opencode, Ollama, SRT, HTTP-server, LibreOffice, or DOCX-library types. The desktop crate composes adapters but does not duplicate their policy or transport logic.

## Tauri architecture proposal

### Trust boundary

The Rust process is the trusted policy enforcement point. The WebView is an untrusted presentation client even though its assets are bundled.

Expose only task-shaped commands, for example:

- select one DOCX
- create/cancel a document job
- submit an instruction
- approve/reject one normalized action
- subscribe to normalized job events
- reveal a completed artifact
- test the configured model connection

Do not expose “run command,” arbitrary URL fetch, arbitrary path read/write, raw opencode proxying, or arbitrary sidecar invocation.

### Runtime ownership

Define an `OpencodeRuntime` abstraction with two adapters:

- `ManagedOpencode`: intended path. Rust selects a free loopback port, generates a random password, starts `opencode serve --hostname 127.0.0.1 --port <port>`, captures logs, polls `/global/health`, monitors exit, and kills the child tree on shutdown.
- `ExternalOpencode`: development-only adapter. It connects to an explicitly configured loopback URL and requires credentials. It must reject non-loopback URLs.

Use the external adapter for the first API proof, then the managed adapter for the actual vertical slice. Do not use the JS SDK inside the WebView. A small Rust HTTP/SSE client based on the live OpenAPI schema keeps credentials and unrestricted endpoints behind the trusted host. Generate or hand-model only the API subset Spike 001 needs.

### Tauri permissions

- Grant native dialog access only as needed for the picker.
- Avoid the shell and process plugins in the frontend entirely.
- Prefer Rust-owned child-process APIs; if a Tauri sidecar plugin is later used, scope it to exact binary names and arguments.
- Keep remote content disabled. Do not grant Tauri command access to remote origins.
- Validate every incoming path after canonicalization and reject UNC/device paths, alternate data streams, symlink/reparse-point escapes, wrong extensions, and paths outside the selected root.

### Event and approval normalization

Translate opencode events into product events such as `job_started`, `assistant_text`, `action_requested`, `action_started`, `artifact_ready`, and `job_failed`. The UI should never need to understand opencode part names or raw commands.

The Rust host owns approval state. An opencode permission request is necessary but not sufficient: the host maps it to a known action, verifies its arguments and paths, displays a friendly summary, records a one-time decision, and only then responds. Do not expose opencode's session-level “always” choice in Spike 001.

### Trusted tool broker

The opencode custom tools must call a small authenticated local `ToolBroker` owned by the Rust host. For Spike 001, an ephemeral-token loopback endpoint is the simplest candidate; a named pipe can replace it later without changing the tool contract. The broker binds only to loopback, accepts only versioned structured operations, and rejects missing/invalid tokens, unknown jobs, stale document hashes, unapproved mutations, arbitrary paths, arbitrary executables, and arbitrary command strings.

The broker, not the TypeScript adapter, canonicalizes trusted paths, generates SRT policy, launches the fixed clean-room DOCX executable, enforces timeout/output limits, validates the result, and publishes the final artifact. The endpoint must not be a generic command runner.

## opencode integration proposal

### Controlled configuration

Start opencode in a controlled, app-created job workspace that contains no user project configuration. Isolate its user configuration directories where the installed version permits, and provide the final restrictive runtime configuration through `OPENCODE_CONFIG_CONTENT`, which current documentation places after project configuration in precedence. Also define and select a dedicated Spike 001 agent with the same restrictive permission posture. The proof must enumerate loaded tools/agents/plugins and fail closed if user-global, project, managed, or `.opencode` content can add capabilities or broaden policy. Never start opencode with `--auto`.

Initial permission posture:

```jsonc
{
  "permission": {
    "*": "deny",
    "docx_inspect": "allow",
    "docx_rewrite_section": "ask"
  }
}
```

This is illustrative and must be validated against the installed opencode schema/version. The prompt request should also use an explicit tool allowlist if the tested API version supports it. Never use `"permission": "allow"`, `"*": "allow"`, or `--auto`.

### API subset

Use only:

- `GET /global/health`
- `GET /doc` during development/version validation
- session create/get/abort
- async prompt submission
- session messages
- `GET /event` or `/global/event` SSE
- the permission response endpoint
- provider/config reads needed for diagnostics

Explicitly do not proxy `/session/:id/shell`, config mutation, auth mutation, TUI control, sharing, MCP mutation, or arbitrary file APIs to the WebView.

### Custom tool boundary

Use TypeScript custom tools only as thin, reviewed adapters. Their arguments are schema-validated and sent to the authenticated Rust `ToolBroker` as a fixed operation. They must not concatenate a shell command or spawn SRT, Python, or the DOCX executable. The broker and trusted executable independently validate the relevant inputs.

Prove the documented same-name precedence with `.opencode/tools/bash.ts`, but keep `bash: deny` as the primary rule. The override is defense in depth and a regression sentinel, not permission to enable generic shell execution.

The DOCX tools should return compact structured JSON. Large extracted document text should be bounded and section-oriented to avoid wasting model context.

### Version policy

Record the exact opencode version in every proof result. Pin the exact version used by the spike after validation. At startup, reject unsupported versions rather than assuming API/config compatibility.

Use the same deliberate policy for fast-moving dependencies such as SRT:

1. At the beginning of the proof, query the registry for the newest published non-prerelease version.
2. Inspect that published package/release and run the proof suite against it.
3. Pin the exact version that passed in the manifest/lockfile and evidence report.
4. Do not auto-upgrade a working spike. When a newer release appears, test it in a compatibility/security update branch and move the pin only after all positive and negative gates pass.

This gives the spike current behavior without making future builds non-reproducible. A longer-term update cadence and user-facing upgrade mechanism are post-spike product decisions.

## SRT enforcement proposal

### Current upstream state

SRT is an Apache-2.0 beta research preview. Windows support is documented as **alpha**, with bundled x64/arm64 `srt-win.exe` helpers and a one-time elevated installation. At review time, the npm registry page reported published version `0.0.64` while the repository `main` branch declared `0.0.65`; therefore, do not infer the installable version from `main`. Resolve and record the newest published version immediately before the proof:

```powershell
$srtVersion = npm view @anthropic-ai/sandbox-runtime version
npm view "@anthropic-ai/sandbox-runtime@$srtVersion" dist.integrity dist.tarball
npx "@anthropic-ai/sandbox-runtime@$srtVersion" windows-install
```

The installer provisions a dedicated `srt-sandbox` local account, group membership/state, and WFP filters. Commands run as that account under a restricted token and Job Object. Filesystem access is granted with session-specific NTFS ACEs; outbound network is fenced by SID and allowed only through SRT's controlled loopback proxy path.

This setup is a meaningful machine mutation and must be a documented, explicit developer/admin step. Do not attempt silent fallback if installation, initialization, or policy application fails.

### Enforcement design

Define a `SandboxRunner` interface whose production Spike 001 adapter is `SrtRunner`. It accepts a structured launch request:

- fixed executable identity
- fixed operation enum
- canonical working directory
- canonical input/output/temp paths
- timeout and output limits
- network policy fixed to deny

It returns structured stdout/stderr, exit status, timeout/cancellation state, canary-verification results, and optional violation records when the tested platform/version exposes them. Windows acceptance must depend on enforced denials and unchanged canaries, not on violation telemetry being available.

For each document job:

1. Rust creates a private job directory and output directory under the selected workspace and issues an ephemeral broker token scoped to that job.
2. Rust generates SRT settings from canonical trusted paths, never from model text or repository files.
3. SRT is initialized for that job/workspace.
4. The authenticated Rust tool broker launches the clean-room DOCX executable only through SRT; opencode's TypeScript tool never launches it directly.
5. The sandboxed tool writes only to a private temporary artifact. Rust verifies its location, extension, ZIP/DOCX validity, size limits, and original hash, then publishes it to an exclusively reserved no-overwrite destination.
6. SRT cleanup/reset runs in a finally/RAII path; the next startup also checks crash recovery.

Because current Windows SRT does not support per-execution allow-read/allow-write overrides, avoid sharing one broad initialized sandbox across unrelated workspaces. Serialize Spike 001 jobs or use a separately owned sandbox session per selected workspace until concurrency behavior is proven.

### Required SRT proof matrix

The SRT gate passes only if all rows behave as expected under the `srt-sandbox` identity:

| Probe | Expected result |
|---|---|
| Read selected synthetic DOCX | allowed |
| Write new file in controlled output | allowed |
| Modify/delete original DOCX with a deliberately hostile helper | denied by SRT independently of tool validation; source hash remains unchanged |
| Read a canary in the caller profile outside allowed paths | denied |
| Write outside workspace/output/temp | denied |
| HTTP request to LAN Ollama | denied |
| Direct TCP connection with proxy variables removed | denied |
| Spawn a child/grandchild and attempt the same violations | denied |
| Launch an unrelated app/surrogate process | remains contained or test fails |
| Timeout/cancel a descendant process tree | all descendants terminate |
| Crash before reset, then initialize again | stale ACEs recovered/removed as documented |

Record effective Windows edition/build, filesystem type, SRT/npm/Node versions, install state, generated policy, command identity, and results. Alpha support means a green test on one machine is evidence for that tested configuration, not a cross-platform security claim.

## Remote Ollama configuration and testing plan

### Centralized configuration

Use one app-owned configuration object and derive both opencode provider config and diagnostics from it:

```text
ModelEndpointConfig
  provider_id: "ollama-lan"
  kind: OllamaOpenAICompatible
  scheme: "http"
  host: configured at runtime; localhost is the desktop fallback
  port: 11434
  api_base_path: "/v1"
  model_id: "gemma4:12b"
  connect_timeout_ms: 3000
  request_timeout_ms: 120000
```

Keep secrets/credentials in runtime state or the OS credential store later, not committed config. The configured host is a trusted host-process destination, never a sandbox exception.

Derive the opencode provider entry rather than maintaining a second copy:

```jsonc
{
  "model": "ollama-lan/gemma4:12b",
  "provider": {
    "ollama-lan": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "Ollama (LAN)",
      "options": { "baseURL": "${configuredOllamaOrigin}/v1" },
      "models": {
        "gemma4:12b": { "name": "gemma4:12b" }
      }
    }
  }
}
```

Validate the installed opencode schema before using this exact shape.

### Exact same-LAN validation commands

Run these from the intended Windows development host:

```powershell
$base = $env:OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN
if ([string]::IsNullOrWhiteSpace($base)) {
  throw 'Set OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN to the intended test host.'
}

Invoke-RestMethod "$base/api/version"
$tags = Invoke-RestMethod "$base/api/tags"
$tags.models | Select-Object name, model, modified_at, size

Invoke-RestMethod "$base/v1/models"

$body = @{
  model = 'gemma4:12b'
  messages = @(@{ role = 'user'; content = 'Reply with exactly: ready' })
  stream = $false
} | ConvertTo-Json -Depth 8

Invoke-RestMethod `
  -Method Post `
  -Uri "$base/v1/chat/completions" `
  -ContentType 'application/json' `
  -Body $body
```

Then run a function-calling request with one harmless tool such as `echo_value(value: string)`, assert one valid tool call, provide the tool result in a second turn, and assert a concise final response. Finally run the same two-turn test through opencode using the exact centralized provider configuration.

### Pass criteria

- `/api/tags` contains an exact `gemma4:12b` name/model entry.
- `/v1/models` and `/v1/chat/completions` work with the configured base URL.
- Direct Ollama and opencode-mediated tool calls both produce schema-valid arguments.
- The second tool-result turn completes without repeated calls or malformed state.
- Three to five repeated runs succeed; one lucky response is insufficient evidence of reliability.
- Errors identify DNS/routing/refusal, missing model, timeout, unsupported tool calls, and malformed responses separately.

Do not pull, rename, or substitute a model as part of this validation.

## Minimal provider/model config design

Keep the persisted user-editable shape small and vendor-neutral:

```text
ProviderProfile
  id
  kind
  display_name
  endpoint { scheme, host, port, base_path }
  model_id
  timeouts
```

Keep derived and runtime-only state separate:

```text
ProviderRuntimeState
  normalized_base_url
  last_checked_at
  server_version
  discovered_models[]
  selected_model_present
  capabilities_observed { chat, tools, thinking }
  health { unknown, checking, ready, degraded, unreachable }
  last_error
```

Use an adapter interface such as `ModelProviderProbe` so a future local Ollama or different provider does not alter UI/application logic. Do not infer tool capability solely from a model registry flag; record it only after a live tool-call proof.

## Clean-room DOCX tool plan

### Clean-room rules

- Do not clone, vendor, open, copy, paraphrase, or derive code, prompts, scripts, fixtures, directory structure, or detailed implementation text from Anthropic's proprietary document skills.
- Use only public capability categories already stated in the project brief and independently designed contracts.
- Use open-source library documentation, ECMA Office Open XML specifications as needed, and synthetic fixtures authored for this project.
- Record dependency licenses and versions.

### Tool contract

Use a versioned, language-neutral JSON-over-stdio CLI contract so the implementation can be replaced without changing opencode or Tauri:

```text
docx-tool inspect --request <json-file>
docx-tool rewrite-section --request <json-file>
docx-tool validate --request <json-file>
```

Inputs contain only canonical paths supplied by Rust, a section selector, bounded replacement text, and an output name token. Outputs contain a schema version, document summary, candidate section identifiers, before/after text, warnings, output metadata, and hashes.

The model should select a stable section identifier returned by `inspect`, not invent an XML path. The trusted host resolves the identifier to the original inspected document revision and rejects stale requests.

### Narrow behavior

`inspect`:

- verify `.docx`, size limit, ZIP integrity, and required package parts;
- enumerate body paragraphs and heading-style sections;
- return bounded text and stable section IDs;
- report unsupported constructs relevant to the target section (tables, text boxes, tracked changes, fields, content controls, headers/footers).

`rewrite-section`:

- open the source read-only;
- require one unambiguous section ID and expected source hash;
- write only to a newly created temporary output;
- replace only the section body, preserving unrelated package parts as far as the chosen library allows;
- return only a private temporary output inside the broker-created job directory;
- never choose or write the user-visible final destination;
- reopen and validate the output before reporting success.

`validate`:

- reopen the revised DOCX;
- confirm expected text is present in the selected section;
- confirm unrelated sampled sections still match;
- confirm the original hash and timestamp are unchanged;
- report package/relationship errors and fidelity warnings.

After tool validation, the Rust host independently reopens and validates the temporary artifact, reserves a collision-safe `.revised.docx` destination using a no-overwrite operation, and publishes the artifact. If reservation or publication fails, it leaves the original untouched and reports no completed artifact.

### Implementation choice

The original first-proof preference was `python-docx` behind the CLI contract because it is mature for opening and saving existing DOCX files. Live preflight confirmed that this machine has no `python`, `python3`, or `py` command, while Cargo/Rust are available. The implemented standalone Rust DOCX executable now passes package preservation, revised-copy, validation, and SRT proofs behind the replaceable `DocumentTool` process boundary. It must remain independent of Tauri application code; no Python runtime is required for Spike 001.

Do not assign `paragraph.text` blindly and claim formatting preservation: replacing paragraph content can collapse run-level formatting. The first accepted fixture should intentionally use a simple heading plus plain body paragraphs. Complex inline formatting, tables, tracked changes, headers, text boxes, and content controls are out of scope and should produce a clear unsupported/fidelity warning.

The CLI boundary makes it possible to replace Python with a Rust or other implementation later without changing the app or agent contract.

### Preview strategy

The required first preview is a safe structured before/after text view generated from the tool result. An optional document-rendering gate may invoke LibreOffice headlessly through a separate, fixed `Previewer` adapter and SRT policy to export PDF/PNG. LibreOffice is not on PATH in the current environment and conversion fidelity must be visually checked, so rendered preview is not a prerequisite for proving the security architecture.

## Step-by-step spike plan

### Phase 0 — freeze evidence and versions

1. Query the registry for the newest published SRT version, inspect that published package/release, and record Windows build/edition, NTFS usage, Node/npm, opencode, SRT, Python, `python-docx`, Tauri CLI, Rust, and optional LibreOffice versions.
2. Create only synthetic DOCX fixtures with a heading named “Executive Summary,” plain paragraphs, adjacent untouched sections, and a unique canary.
3. Define the original-hash and no-overwrite assertions used in every later phase.

Exit gate: version manifest and fixtures exist; no proprietary skill material has been used.

### Phase 1 — SRT Windows proof

1. Review the exact pinned SRT release and its Windows install/uninstall behavior.
2. With explicit administrator approval, run the one-time `windows-install` step.
3. Execute the full positive/negative proof matrix above with a harmless fixed test program.
4. Test descendant processes, timeout, cancellation, reset, and crash recovery.
5. Document any residual ACEs, WFP rules, local accounts/groups, logs, and clean uninstall/reinstall behavior.

Exit gate: all denials are enforced by the OS boundary, not merely by app checks. Otherwise Spike 001 is blocked and unrestricted shell execution is not used as a fallback.

### Phase 2 — opencode server and policy proof

1. Confirm pre-installed `opencode` on PATH and record its version.
2. Start it explicitly on `127.0.0.1` with mDNS off/default and a generated Basic-auth password.
3. Verify `/global/health`, `/doc`, authentication failure without credentials, and no listener on non-loopback interfaces.
4. Prove configuration precedence using an app-created working directory, isolated config locations, restrictive `OPENCODE_CONFIG_CONTENT`, and a dedicated agent; ensure global/project/user/managed settings and `.opencode` content cannot widen app policy.
5. Verify session creation, async prompting, SSE events, cancellation, and permission responses.
6. Use default-deny permissions with only `docx_inspect: allow` and `docx_rewrite_section: ask`; prove `edit`, `write`, `apply_patch`, bash, task, skill, web, MCP, formatter/LSP, and unexpected custom-tool paths are unavailable and cannot change a `.docx` canary.
7. Add a fail-closed same-name `bash` tool and prove precedence. Also test that denied bash remains denied.

Exit gate: the API subset works and every generic mutation/command attempt fails closed.

### Phase 3 — remote Ollama and tool-loop proof

1. Run the exact same-LAN discovery and chat commands.
2. Confirm `gemma4:12b` exactly; do not substitute or pull.
3. Test direct two-turn function calling repeatedly.
4. Generate the opencode provider config from the centralized profile.
5. Repeat the tool loop through opencode and capture normalized events/errors.

Exit gate: repeatable schema-valid tool calls and a successful second turn through opencode.

### Phase 4 — clean-room DOCX proof through SRT

1. Implement only the versioned inspect/rewrite/validate CLI contract.
2. Run `inspect` through SRT with network denied.
3. Rewrite the synthetic Executive Summary through SRT into a collision-safe revised copy.
4. Validate the revised package and semantic change.
5. Assert the original hash, bytes, and timestamp are unchanged.
6. Run hostile path, symlink/reparse, oversized input/output, malformed ZIP, ambiguous heading, unsupported content, and destination-collision tests.

Exit gate: one valid rewrite succeeds and all safety-negative tests fail closed.

### Phase 5 — opencode custom DOCX tools

1. Add thin `docx_inspect` and `docx_rewrite_section` custom tool adapters plus the authenticated Rust `ToolBroker` endpoint.
2. Ensure adapters accept structured arguments only and request fixed broker operations; only the Rust broker may invoke the SRT runner.
3. Have the model inspect, propose replacement text, and stop at a permission request before mutation.
4. Approve once through the API, have Rust validate the approval and broker request, create and validate a private temporary output through SRT, publish a no-overwrite revised copy, and return structured artifact metadata.
5. Prove rejection/cancellation creates no artifact and does not mutate the original.

Exit gate: the headless end-to-end workflow passes without Tauri.

### Phase 6 — minimal Tauri vertical slice

1. Create the Tauri 2 shell with least-privilege capabilities.
2. Add the centralized config and runtime interfaces.
3. Implement external-opencode development mode, then managed child-process mode.
4. Add one DOCX picker, one instruction input, normalized assistant/event output, one friendly approval card, a text diff preview, and an artifact link.
5. Run the same acceptance workflow from the UI.

Exit gate: UI behavior does not broaden any backend permission and never displays raw shell/opencode terminology in the normal flow.

### Phase 7 — evidence and decision

1. Repeat all security regressions against the integrated app.
2. Record limitations, exact versions, setup requirements, and manual QA results.
3. Decide: proceed with opencode/SRT, pin and harden, or stop/revisit the backend/sandbox.

## Smallest vertical-slice demo

The demo is complete when a user:

1. opens a minimal Tauri app;
2. selects one synthetic or disposable DOCX;
3. asks to make its Executive Summary more concise;
4. sees a friendly proposed-action card and before/after text;
5. approves once;
6. receives `name.revised.docx` in the controlled output directory;
7. can open the revised artifact; and
8. sees evidence that the original hash is unchanged and the document tool ran with network denied through SRT.

No rendered document preview is required if LibreOffice is unavailable; a trustworthy before/after text preview plus a valid revised DOCX is sufficient for this spike.

## Verification matrix: assumptions

### Verified from current primary documentation

- opencode server defaults to `127.0.0.1:4096`, supports Basic auth through `OPENCODE_SERVER_PASSWORD`, publishes OpenAPI at `/doc`, health at `/global/health`, and SSE events.
- opencode exposes sessions, messages, async prompts, abort, and permission-response APIs.
- opencode custom tools live in `.opencode/tools`, can invoke other-language scripts, and same-name custom tools take precedence over built-ins.
- opencode permissions support `allow`, `ask`, and `deny`; `edit` covers `edit`, `write`, and `apply_patch`.
- opencode documents Ollama through `@ai-sdk/openai-compatible` with a `/v1` base URL.
- Ollama documents `/api/tags`, OpenAI-compatible `/v1/chat/completions`, and tool support.
- Tauri 2 capabilities constrain what WebViews can access; bundled frontend code is the default API origin boundary.
- The upstream SRT README documents Windows alpha support, its elevated installation, dedicated account, WFP/ACL design, and limitations. The published npm version and repository `main` version can differ, so the proof must resolve and pin the newest published package rather than treating `main` as a release.
- `python-docx` 1.2.0 documentation describes opening and saving existing documents and manipulating paragraphs/runs.
- LibreOffice documents headless/invisible conversion parameters, but rendering fidelity remains an empirical question.

### Verified in the current workspace/environment

- Repository branch is `branch/spike-01` at initial commit `9e474d1` when inspected.
- The three source-of-truth files are under `docs/local/`; that directory is intentionally ignored by the user's existing `.gitignore` modification.
- Node 24.18.0, Cargo 1.96.1, and Rust 1.96.1 are available.
- opencode `1.17.18`, Node `24.18.0`, Cargo `1.96.1`, and Rust `1.96.1` are available and were used by the passing proofs. A system Python and `soffice` are not available on PATH; the implemented DOCX tool is standalone Rust and does not require Python.
- SRT `0.0.65` is pinned locally and its one-time Windows setup is installed. The Phase 1 matrix passed on this Windows ARM64/NTFS host using the versioned ARM64 helper at `C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe`; detailed evidence is in `docs/test-plans/spike-001-srt-proof.md` and `spike-001-srt-proof-result.json`.
- The proven Windows read policy relies on the dedicated `srt-sandbox` account's default lack of access to caller files plus precise `allowRead` grants. A broad `denyRead` on `C:\Users\<user>` blocked traversal to the nested allowed workspace and is not part of the passing policy.
- Requests to the configured private-LAN endpoint's `/api/version`, `/api/tags`, and `/v1/models` routes initially failed from the restricted execution environment. This proved only that the sandbox/session could not reach the private endpoint; it did not prove the Ollama host was down.
- Normal-user host validation subsequently reached Ollama `0.31.2`; `/api/tags` and `/v1/models` both contained exact tag `gemma4:12b`. Three independent opencode sessions each called `echo_value` exactly once with `ready`, received `ready`, and completed with exact final text `DONE`; the default-denied bash path did not execute.

### Historical evidence superseded by current proof

- Prior project testing reported remote `gemma4:12b` tool calls and separated thinking/tool-call content on this host. Spike 001 subsequently reran and recorded current discovery, repeated tool-loop, permission, broker, SRT, and DOCX evidence under `docs/test-plans/`.

### Completed headless validation

- The SRT Windows policy and helper placement remained effective behind the Rust broker and clean-room DOCX runtime.
- The narrow clean-room DOCX executable passes through SRT `0.0.65` on the synthetic fixture: it publishes a new copy, reopens and validates the package, preserves the adjacent section canary, rejects overwrite, leaves the source hash unchanged, and resets SRT cleanly.
- The transport-independent Rust broker core is implemented in `crates/cowork-core`. Tests verify invalid token, pending approval, rejection, cancellation, stale hash, and approval reuse all fail before runner execution; a valid approval is consumed before execution.
- The real broker/SRT/publication composition passes. `crates/cowork-runtime` provides the fixed SRT runner and exclusive DOCX publisher; `crates/broker-transport` owns reusable authenticated request translation and the blocking localhost server used inside the process-isolated broker host. An approved-once job produced a validated revised copy, while rejected and cancelled jobs produced no artifacts and the source hash remained unchanged.
- Authenticated loopback transport and thin opencode DOCX-tool translation now pass end to end. Allow-once created exactly one artifact through opencode, the Rust broker, SRT, and the clean-room tool; reject and abort created none. The broker bound to `127.0.0.1` with distinct ephemeral execution and control credentials. Model-visible tools received only the execution credential and structured DOCX arguments; approval, rejection, and cancellation used a separate job/action-correlated control route, and execution could not approve itself. Evidence is in `docs/test-plans/spike-001-opencode-docx-proof.md` and its JSON result.
- Final pre-Tauri modularization is complete: `cowork-core` exposes transport-neutral workflow commands/events and model-session ports; `broker-transport` owns authenticated localhost contracts/server behavior; `process-supervisor` owns kill-on-close child trees; and `opencode-client` owns pinned opencode management, the approved authenticated API subset, and SSE parsing. The Rust client passed live localhost health/session/message calls, and the complete headless allow/reject/abort proof remained green after extraction.
- The approval-gated custom-tool lifecycle is verified on opencode `1.17.18`. The tool must explicitly call `context.ask`; configuration `ask` alone does not gate replacement custom tools. Allow-once emitted `permission.asked`, accepted `once` through `/permission/:requestID/reply`, executed exactly once, emitted `permission.replied`, and became idle. Reject emitted the request/reply events and did not execute. Session abort returned `true`, emitted `session.error`, became idle, and did not execute. Evidence is in `docs/test-plans/spike-001-opencode-permission-proof.md` and its JSON result.
- The configured LAN Ollama host was temporarily unreachable during one rerun on 2026-07-12 and later returned. The proof harness now performs a shared endpoint and exact-model preflight so availability failures are not misclassified as permission or model failures.

### Remaining app and product validation

- Whether the proposed app-created workspace, isolated config locations, restrictive `OPENCODE_CONFIG_CONTENT`, dedicated agent, and live tool inventory fully prevent opencode user/global/project/managed configuration from widening capabilities.
- Whether the Tauri composition exposes any model-reachable or WebView-reachable path outside the validated narrow API and tool set, including direct opencode shell/config endpoints.
- Tauri child-process startup, readiness, cancellation, crash recovery, and shutdown behavior for the pinned opencode version on Windows.
- DOCX fidelity on a disposable, non-sensitive representative real-world fixture; the current acceptance proof intentionally covers only the synthetic narrow format.
- LibreOffice availability, packaging, conversion fidelity, and containment.
- Productized SRT installation/uninstallation, signing, upgrades, and helper placement; these remain outside Spike 001.

## Open questions and blockers

No remaining question blocks starting the minimal Tauri vertical slice. The following items must be resolved or validated before calling the desktop prototype complete:

1. **Tauri configuration isolation:** Confirm that the desktop-created workspace, isolated profile/config locations, restrictive inline config, dedicated agent, and tool inventory preserve the proven default-deny posture when composed inside Tauri.
2. **Tauri authority surface:** Confirm that neither the WebView nor model can reach raw opencode shell, config mutation, filesystem, sharing, MCP mutation, or arbitrary broker routes.
3. **Process lifecycle:** Prove startup readiness, cancellation, timeout, crash recovery, and shutdown for opencode, the broker server, and SRT-owned descendants from the packaged desktop process.
4. **DOCX semantics and fidelity:** Keep the implemented exact single-`Heading1` behavior fail-closed for ambiguity and unsupported structures. Add one disposable, non-sensitive representative document for manual fidelity QA.
5. **LAN transport:** Plaintext HTTP to Ollama is accepted only as an explicit private-LAN prototype assumption; it provides no confidentiality. Revisit TLS/auth if the network threat model changes.
6. **Optional preview:** LibreOffice availability, packaging, conversion fidelity, and containment remain unverified and non-blocking while structured preview is sufficient.
7. **Productized SRT setup:** Installation/uninstallation, signing, upgrades, and helper placement are post-spike concerns; runtime startup must still fail closed on an unsupported or missing helper.

Regression blockers are failed SRT enforcement, any generic/shell Office mutation path, broadened opencode permissions/configuration, unreliable required model tool calling, or rejection/cancellation creating an artifact. Missing rendered preview remains non-blocking if structured preview and the revised DOCX validate.

## Source documents inspected

### Project source of truth

- `docs/local/PROJECT_BRIEF.md`
- `docs/local/SPIKE_001_DOCX_OPENCODE_SRT.md`
- `docs/local/REFERENCES.md`

### Current primary sources

- [Tauri 2 documentation](https://v2.tauri.app/)
- [Tauri: Calling Rust from the frontend](https://v2.tauri.app/develop/calling-rust/)
- [Tauri: Capabilities](https://v2.tauri.app/security/capabilities/)
- [Tauri: Embedding external binaries](https://v2.tauri.app/develop/sidecar/)
- [Tauri dialog plugin](https://v2.tauri.app/plugin/dialog/)
- [opencode server and OpenAPI](https://opencode.ai/docs/server/)
- [opencode SDK](https://opencode.ai/docs/sdk/)
- [opencode providers](https://opencode.ai/docs/providers/)
- [opencode permissions](https://opencode.ai/docs/permissions/)
- [opencode tools](https://opencode.ai/docs/tools/)
- [opencode custom tools](https://opencode.ai/docs/custom-tools/)
- [Ollama model listing API](https://docs.ollama.com/api/tags)
- [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility)
- [Anthropic Sandbox Runtime repository and README](https://github.com/anthropic-experimental/sandbox-runtime)
- [Published Anthropic Sandbox Runtime npm package](https://www.npmjs.com/package/@anthropic-ai/sandbox-runtime)
- [SRT package metadata](https://raw.githubusercontent.com/anthropic-experimental/sandbox-runtime/main/package.json)
- [python-docx 1.2.0: working with documents](https://python-docx.readthedocs.io/en/latest/user/documents.html)
- [python-docx 1.2.0: working with text](https://python-docx.readthedocs.io/en/latest/user/text.html)
- [LibreOffice command-line parameters](https://help.libreoffice.org/latest/en-US/text/shared/guide/start_parameters.html)
- [OpenAI Codex sandboxing concepts](https://developers.openai.com/codex/concepts/sandboxing)
- [Anthropic skills repository top-level licensing notice](https://github.com/anthropics/skills)

The proprietary document-skill contents were not cloned, vendored, or used as implementation source.

## Final go/no-go rule

**Go to Tauri implementation:** the required headless phases, modular boundaries, exact versions, and results are recorded and passing.
**Pause and redesign** if SRT does not fail closed, if opencode retains an uncontrolled command/edit path, or if the Ollama model cannot complete repeated structured tool loops.
**Do not downgrade** to unrestricted shell execution to preserve demo progress.
