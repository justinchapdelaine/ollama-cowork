# Clean-room DOCX tool

This executable is an independent Spike 001 adapter. It reads one versioned JSON request from stdin or `--request <path>` and writes one JSON response to stdout.

Module boundaries:

- `contract`: transport-neutral request/response types and validation;
- `filesystem`: canonical paths, revised-copy/no-overwrite rules, and hashing;
- `package`: OPC/ZIP package reading and copy-with-one-part-replaced;
- `document`: deliberately narrow Heading1 section semantics;
- `main`: CLI transport only.

The tool does not know about Tauri, opencode, Ollama, approvals, or SRT. The future `DocumentTool` adapter invokes it, while `ToolBroker` and `SandboxRunner` independently enforce authorization and sandbox policy.

Supported Spike 001 behavior is intentionally narrow: exactly one matching `Heading1`, plain replacement paragraphs, a new `.docx` output, package reopening, and source hash preservation. Tables, nested sections, tracked changes, headers, content controls, text boxes, and fidelity claims beyond the synthetic fixture are unsupported.
