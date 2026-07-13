# Desktop application

Minimal Tauri 2 composition root and framework-free TypeScript frontend.

Current milestone:

- one bundled `main` WebView;
- one read-only `get_desktop_health` command;
- one normalized `desktop://health` event;
- typed configuration loaded once in Rust;
- a framework-neutral job factory split into replaceable workspace, credential,
  runtime-provisioning, and cleanup boundaries;
- no shell, filesystem, dialog, network, or process permission exposed to the WebView.

The desktop crate composes workspace adapters but does not duplicate broker, SRT, opencode, or DOCX policy. The next milestone will add the narrow Spike 001 workflow commands and normalized `WorkflowEvent` bridge.

The job factory creates filesystem-safe per-job model/private-output directories,
requires distinct redacted job credentials, rolls back failed construction, and
terminates resources in reverse order without skipping later cleanup after an
error. Its runtime provisioner is still an explicit port: the next implementation
step is the concrete opencode + broker + SRT provisioner, followed by the narrow
Tauri command/event bridge.

Each app run uses a cryptographically opaque workspace namespace. Active Windows
runs and job handles retain exclusive leases; startup removes only released stale
run directories. Managed opencode and the process-isolated broker host share the
replaceable `process-supervisor` boundary, which uses a kill-on-close Windows Job
Object so descendants cannot outlive job cleanup.

The composition root also provides a `PublishedDocxDecoder` edge adapter. It accepts only the expected job's canonical `.docx` inside its assigned publication directory, revalidates the package, and verifies the broker-reported SHA-256 before producing frontend-safe artifact metadata. Model transport, broker authorization, DOCX validation, and Tauri event delivery remain independently replaceable.

Ollama is runtime-configurable through `OLLAMA_COWORK_OLLAMA_ORIGIN` and `OLLAMA_COWORK_OLLAMA_MODEL`; the fallback is local Ollama at `http://127.0.0.1:11434`. No private-LAN address is required by the application.

Build production assets and the executable through Tauri so `frontendDist` is embedded:

```powershell
npm run build
npm run tauri -- build --no-bundle
```

Do not use raw `cargo build --release` as the production desktop build command; it may retain the development URL configuration.
