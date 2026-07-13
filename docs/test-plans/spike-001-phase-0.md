# Spike 001 Phase 0 Evidence

Captured: 2026-07-12 (America/Vancouver)

This is the immutable Phase 0 snapshot. Later sections and linked proof documents record how its initial blockers were resolved; current readiness is summarized in `SPIKE_001_PLAN.md`.

## Repository

- Branch: `branch/spike-01`
- Commit before proof work: `9e474d1c4376f782284e4a538ca146b6f577a6e1`
- Existing user change preserved: `.gitignore`

## Host and runtimes

- Windows kernel version: `10.0.26200.8655`
- Architecture: ARM64 (`PROCESSOR_ARCHITECTURE=ARM64`; 64-bit OS)
- Filesystem: NTFS on fixed `C:` volume (confirmed through `.NET DriveInfo`; privileged volume-management APIs returned access denied)
- Node: `24.18.0`
- npm: `12.0.1` via `npm.cmd` (`npm.ps1` is blocked by the current PowerShell execution policy)
- Cargo: `1.96.1`
- Rust: `1.96.1`
- opencode: pinned `1.17.18` installed through the official npm package; native ARM64 executable reports `1.17.18`
- SRT at capture time: not installed yet; subsequently installed and proven as described below
- system Python: not found on PATH
- LibreOffice: not found on PATH

## Registry resolution

- `@anthropic-ai/sandbox-runtime` published `latest`: `0.0.65`
- Spike candidate pin: `0.0.65`, pending enforcement proof
- Installed package manifest: `0.0.65`; its CLI unexpectedly reports `1.0.0` for `--version`, so evidence and runtime validation use the pinned package manifest/lockfile rather than trusting the CLI version string
- `opencode-ai` published version observed: `1.17.18`
- Registry queries were run live with `npm.cmd view`; the initial sandboxed query timed out and the approved network query succeeded.

## Fixture policy

- Fixture is synthetic and contains no user or proprietary document-skill content.
- Required heading: `Executive Summary`
- Adjacent untouched heading: `Operating Constraints`
- Original canary: `SPIKE001-ORIGINAL-CANARY-7F93D1`
- The fixture must be rendered and visually inspected before use.

Fixture generated at `tests/fixtures/spike-001-original.docx`. It reopened successfully with `python-docx` and contains all required headings/canaries. The packaged DOCX renderer could not start because LibreOffice is absent, so visual QA is explicitly **not completed**. This is acceptable for disposable proof setup under the Documents skill fallback, but the limitation remains recorded.

## Subsequent resolution of Phase 0 blockers

1. Windows SRT setup and the full enforcement matrix passed; see `spike-001-srt-proof.md`.
2. The corrected opencode localhost/auth/default-deny proof subsequently passed; see `spike-001-opencode-server-proof.md`.
3. LibreOffice remains unavailable, so the synthetic DOCX fixture has structural but not rendered visual QA.
