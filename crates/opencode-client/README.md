# opencode client

Tauri-free adapter for the validated opencode `1.17.18` boundary.

It owns replaceable executable discovery (explicit override, PATH, then optional fallback), pinned process startup/shutdown, localhost endpoint validation, Basic-authenticated calls to the approved API subset, and SSE frame parsing. It does not own workflow policy, approvals, document operations, SRT, broker execution, or UI events.

Raw opencode events stop at this adapter boundary. The desktop application will translate them into `cowork-core::WorkflowEvent` values before emitting anything to the frontend.
