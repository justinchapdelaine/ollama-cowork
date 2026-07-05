# Ollama Cowork

A local-first desktop coworking agent that uses Ollama models and is being built toward sandboxed, reviewable workspace changes.

## Direction

- Desktop shell: Tauri v2.
- Core backend: Rust.
- Model backend: Ollama API, with local and remote base URL support.
- First model baseline: remote `gemma4:12b` via Ollama.
- Planned runtime model: copy-then-patch workspace.
- Planned first sandbox target: elevated Windows OS-level sandbox.
- Planned approval model: manual approvals first, auto-review later as a separate reviewer layer.

## Current status

This repo currently contains the first app scaffold and architecture spine:

- Tauri project structure.
- Minimal frontend probe screen.
- Rust core module boundaries for model, runtime, tools, policy, approvals, diff/apply, and sessions.
- Initial `OllamaBackend` capable of probing `/api/version` and `/api/tags`.
- Explicit workspace root selection for tool execution, including a native folder picker.
- Read-only `list_files` tool-call probe through the Rust tool registry.
- Planning direction for modular user-manageable skills and extensions.
- Planning direction for modular MCP integration.
- Planning docs under `docs/`.

## Prerequisites

The scaffold expects a standard Tauri development setup:

- Rust and Cargo.
- Node.js and npm.
- Windows WebView2 runtime.
- Ollama running locally or on a reachable LAN host.

## Development

After installing dependencies:

```powershell
npm install
npm run tauri:dev
```

For a remote Ollama server, enter its base URL in the app, for example:

```text
http://<lan-ollama-host>:11434
```

## Docs

- [Windows sandbox spike](docs/windows-sandbox-spike.md)
- [Engineering guidelines](docs/engineering-guidelines.md)
- [Skills and extensions](docs/skills-and-extensions.md)
- [MCP integration](docs/mcp-integration.md)
