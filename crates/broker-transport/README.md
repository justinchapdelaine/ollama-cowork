# Broker transport

Transport-level request and authentication helpers for the localhost broker boundary.

This crate knows the versioned broker request contract and owns the bounded localhost HTTP server lifecycle. It does not own jobs, approvals, SRT, DOCX operations, or Tauri integration. A development/proof host and the future desktop composition root can reuse it without importing one another.
