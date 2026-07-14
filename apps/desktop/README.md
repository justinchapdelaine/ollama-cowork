# Desktop application

Minimal Tauri 2 composition root and framework-free TypeScript frontend.

Current milestone:

- one bundled `main` WebView;
- one read-only health command, one Rust-owned native DOCX selection command,
  plus narrow start, approve-once, reject, cancel, and poll commands;
- normalized `desktop://health` and `workflow://event` events;
- typed configuration loaded once in Rust;
- a transport-neutral, mutex-owned application service between Tauri and the
  workflow controller, with slow operations dispatched off the UI thread;
- a framework-neutral job factory split into replaceable workspace, credential,
  runtime-provisioning, and cleanup boundaries;
- no shell, filesystem, dialog, network, or process permission exposed to the WebView.

The desktop crate composes the real workflow controller and runtime factory but
does not duplicate broker, SRT, opencode, or DOCX policy. Tauri owns only request
deserialization, background dispatch, normalized event delivery, and app
shutdown. Closing the app asks the controller to cancel and exhaustively clean
up every active job before exit.

The WebView never supplies a filesystem path. Rust opens the native file picker,
canonicalizes the chosen DOCX, and returns only an opaque single-use selection
ID and display name. Spike 001 enforces one active workflow. Start returns its
reserved job ID before provisioning finishes, and cancellation is propagated
through runtime readiness so app shutdown does not wait for the full startup
timeout.

The job factory creates filesystem-safe per-job model/private-output directories,
requires distinct redacted job credentials, rolls back failed construction, and
terminates resources in reverse order without skipping later cleanup after an
error. Its runtime provisioner remains an explicit port backed by the concrete
opencode + broker + SRT adapter. Configuration paths can be replaced through
environment variables without changing the command or controller layers.

Each app run uses a cryptographically opaque workspace namespace. Active Windows
runs and job handles retain exclusive leases; startup removes only released stale
run directories beneath a root carrying a validated app-ownership marker. A
nonempty unowned override is rejected rather than cleaned. Managed opencode and
the process-isolated broker host share the
replaceable `process-supervisor` boundary, which uses a kill-on-close Windows Job
Object so descendants cannot outlive job cleanup.

The composition root also provides a `PublishedDocxDecoder` edge adapter. It accepts only the expected job's canonical `.docx` inside its assigned publication directory, revalidates the package, and verifies the broker-reported SHA-256 before producing frontend-safe artifact metadata. Model transport, broker authorization, DOCX validation, and Tauri event delivery remain independently replaceable.

Ollama is runtime-configurable through `OLLAMA_COWORK_OLLAMA_ORIGIN` and `OLLAMA_COWORK_OLLAMA_MODEL`; the fallback is local Ollama at `http://127.0.0.1:11434`. No private-LAN address is required by the application.

Build production assets and the executable through Tauri so `frontendDist` is embedded:

```powershell
npm run tauri -- build --no-bundle
```

The Tauri development/build hooks first build the broker host and DOCX tool in
the matching Cargo profile and copy the versioned SRT bridge beside the desktop
executable. Runtime asset lookup is a replaceable resolver with explicit
per-component environment overrides; no build-machine checkout path is embedded.
The preparation hook derives Cargo's effective target directory, honors Tauri's
target triple, and stages the helpers for both default and explicit-target
layouts. Startup health executes helper identity checks and validates the copied
bridge against the pinned application copy rather than trusting filenames alone.

Do not use raw `cargo build --release` as the production desktop build command; it may retain the development URL configuration.
