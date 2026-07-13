# Desktop application

Minimal Tauri 2 composition root and framework-free TypeScript frontend.

Current milestone:

- one bundled `main` WebView;
- one read-only `get_desktop_health` command;
- one normalized `desktop://health` event;
- typed configuration loaded once in Rust;
- no shell, filesystem, dialog, network, or process permission exposed to the WebView.

The desktop crate composes workspace adapters but does not duplicate broker, SRT, opencode, or DOCX policy. The next milestone will add the narrow Spike 001 workflow commands and normalized `WorkflowEvent` bridge.

Ollama is runtime-configurable through `OLLAMA_COWORK_OLLAMA_ORIGIN` and `OLLAMA_COWORK_OLLAMA_MODEL`; the fallback is local Ollama at `http://127.0.0.1:11434`. No private-LAN address is required by the application.

Build production assets and the executable through Tauri so `frontendDist` is embedded:

```powershell
npm run build
npm run tauri -- build --no-bundle
```

Do not use raw `cargo build --release` as the production desktop build command; it may retain the development URL configuration.
