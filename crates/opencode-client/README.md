# opencode client

Tauri-free adapter for the validated opencode `1.17.18` boundary.

It owns replaceable executable discovery (explicit override, PATH, then optional fallback), pinned process startup/shutdown, localhost endpoint validation, Basic-authenticated calls to the approved API subset, and SSE frame parsing. It does not own workflow policy, approvals, document operations, SRT, broker execution, or UI events.

Raw opencode events stop at this adapter boundary. The adapter translates them into transport-neutral `cowork-core::ModelEvent` values; the core controller then produces `WorkflowEvent` values before the desktop emits anything to the frontend.

`OpencodeEventTranslator` is stateful and session-scoped. It converts only the expected DOCX permission, tool, text snapshot/delta, idle, and failure events into transport-neutral `cowork-core::ModelEvent` values. The permission ID is retained only for the approval reply; unrelated tool-call IDs remain adapter details and are not incorrectly equated with it. Unexpected permissions or tools fail closed instead of crossing into the workflow controller. Tool output interpretation is injected through `ValidatedArtifactDecoder`, whose contract requires the expected broker schema, job, published DOCX, media type, and digest to be verified without coupling this crate to the broker implementation.

Ordinary API calls remain bounded blocking requests. Both HTTP clients reject redirects, bypass system proxies, reject URL user information, and accept only an explicit IPv4-loopback root origin. SSE uses a separate async client, incremental UTF-8-safe decoder, and state-carrying cancellation signal so concurrent and late waiters cannot miss shutdown. Both SSE readers enforce the same per-frame limit, and diagnostic error bodies are bounded; stalled async error bodies remain cancellable. `bounded_model_event_channel` provides nonblocking, fixed-capacity delivery for the model-session pump and reports backpressure instead of growing memory without limit.

`OpencodeModelSession` implements the core `ModelSession` port without importing broker, SRT, DOCX, Tauri, or frontend types. Replaceable `OpencodeSessionCommands`, `OpencodeSessionProvisioner`, and `OpencodeEventStream` ports isolate API compatibility and make the lifecycle independently testable. Construction waits for the authenticated SSE connection before a prompt can be submitted; the background pump translates only the configured session, stops on terminal events, reports premature disconnects and backpressure, and is cancelled and joined during shutdown.
