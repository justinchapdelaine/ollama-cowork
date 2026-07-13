# Spike 001 Tauri shell milestone

Verified on 2026-07-12 on Windows ARM64.

Versions:

- Tauri Rust: `2.11.5`
- Tauri CLI: `2.11.4`
- Tauri JavaScript API: `2.11.1`
- Vite: `8.1.4`
- TypeScript: `7.0.2`

Passed gates:

- framework-free TypeScript frontend production build;
- full locked Rust workspace test suite;
- desktop crate compile check;
- optimized Tauri build without bundling;
- real executable startup and six-second process-liveness check;
- responsive main window with title `Ollama Cowork`;
- release binary uses the Windows GUI subsystem and opens no companion console window;
- opencode version probes and the future managed server use `CREATE_NO_WINDOW`, preventing console-mode child processes from flashing during startup;
- screenshot-based visual inspection of the production asset build;
- screenshot inspection confirming the heading, local prerequisite readiness, all three component cards, pinned versions, configured-but-unverified endpoint/model, and milestone footer;
- no Tauri shell, filesystem, dialog, or network plugin exposed to the WebView;
- one bundled main-window capability using only `core:default`;
- one read-only `get_desktop_health` command and normalized `desktop://health` event;
- typed configuration loaded once in the Rust composition root.

The initial post-rename executable was mistakenly rebuilt with raw `cargo build --release`, which selected the development URL and rendered a localhost connection error. Rebuilding through `npm run tauri -- build --no-bundle` embedded the production assets. The corrected executable was relaunched and visually inspected with the Computer Use plugin; it rendered the intended dark health dashboard. Production desktop artifacts must therefore be built through the Tauri CLI rather than raw Cargo.

A subsequent early/late window audit after adding hidden opencode child-process flags found only the `Ollama Cowork` application window; no console window was exposed by the release app or its startup version probe.

Code inspection confirms that the SRT runner applies the same Windows `CREATE_NO_WINDOW` policy to its Node bridge process. A visible console-window audit during a Tauri-owned document workflow remains part of the next milestone's acceptance test.

The Ollama endpoint is displayed as `CONFIGURED`, not `READY`, until a live connectivity probe verifies it. Consequently, overall readiness remains pending even when opencode and SRT are locally available; the configured private-LAN host is not treated as reachable merely because it is present in configuration.

The next milestone is the narrow DOCX workflow composition and UI controls, reusing the validated workspace crates.
