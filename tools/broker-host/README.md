# Proof/development loopback broker host

This proof/development composition root owns authenticated localhost transport and one document job. It constructs broker requests from server-owned job identity, token, source hash, and paths. Model-visible tools submit only fixed structured operations. The future Tauri host will reuse `broker-transport` rather than depending on this executable.

The host binds explicitly to `127.0.0.1`, caps request bodies, and delegates policy, execution, and publication to the core and adapter crates. `GET /health` and `POST /execute` require an ephemeral execution credential available to the thin model-visible tools. `POST /decision` requires a different control credential that is never placed in the model/tool environment. Execution cannot grant approval; an explicit job/action-correlated control decision must occur first.
