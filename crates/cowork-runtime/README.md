# Spike adapters

Replaceable edge implementations for `cowork-core` ports:

- `SrtRunner`: invokes a fixed Node/SRT bridge with a versioned structured request; it accepts no arbitrary executable or command from broker callers.
- `ExclusiveDocxPublisher`: validates required DOCX ZIP parts and publishes with exclusive create semantics, choosing a numbered filename rather than overwriting.

The bridge contains SRT-specific mechanics only. It cannot authorize operations; approval and job policy remain in `cowork-core`.
