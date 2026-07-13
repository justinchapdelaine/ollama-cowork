# Spike 001 Windows SRT Proof Result

Result: **PASS on the recorded host**
Completed: 2026-07-12 (America/Vancouver)
Machine scope: Windows ARM64, NTFS, SRT `0.0.65`

## What this proves

The pinned Windows SRT runtime can enforce the Spike 001 document-tool boundary on this machine when initialized as one session with precise read/write grants and a network-deny policy.

The final machine-readable report is `docs/test-plans/spike-001-srt-proof-result.json`.

## Pinned components

- npm package: `@anthropic-ai/sandbox-runtime@0.0.65`
- Node: `24.18.0`, machine-wide under `C:\Program Files\nodejs`
- ARM64 helper: `C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe`
- Helper SHA-256: `17A63AA8C010662B3E723F75D13D8672C69BEECA8D072F4B2DCE7484E850023A`
- Synthetic source SHA-256 before/after: `6C692ABEDD83E1F74FB9313A41790A977483BB4202691D87A5618CF3727B7A29`

The helper in `Program Files` is byte-identical to the ARM64 helper in the pinned npm package.

## Installed Windows state

The approved one-time `windows-install` action created:

- local account `srt-sandbox` with SID `S-1-5-21-690048078-3894724011-2590843110-1007`;
- local group `sandbox-runtime-users`;
- DPAPI-protected setup/credential state under `%LOCALAPPDATA%\sandbox-runtime\state.db`;
- four WFP filters keyed to the sandbox account SID;
- loopback proxy permit range `60080–60089`.

The final WFP status was `installed`, with four filters and the expected SID/port range.

## Passing matrix

| Probe | Expected | Observed |
|---|---|---|
| Read selected synthetic DOCX | allow | exit 0 |
| Write controlled output | allow | exit 0 |
| Read outside granted roots | deny | `EPERM` |
| Write outside output | deny | `EPERM` |
| Modify original DOCX | deny | `EPERM`; source hash unchanged |
| HTTP to private Ollama | deny | `EACCES` |
| Direct TCP after removing proxy variables | deny | `EACCES` |
| Descendant process writes outside output | deny | child received `EPERM` |
| Timed-out sandbox process writes a delayed survivor canary | terminate | timeout observed; no delayed canary appeared after reset |

Every ordinary allow/deny probe reached the sandboxed helper. A non-zero SRT initialization error is not counted as a security denial by the harness.

## Cleanup and crash recovery

- The successful matrix reported no initialization or reset error.
- Post-run ACL inspection found no `srt-sandbox` ACE on the repository or original fixture.
- Post-run process inspection found no remaining proof coordinator, probe, or `srt-win` process.
- A post-run `acl recover` reported zero dead brokers and zero orphan ACEs.
- During harness development, a deliberately terminated/hung proof left one dead broker and four orphan ACEs; the documented `acl recover` command pruned that broker and revoked all four ACEs. This provides observed crash-recovery evidence, but the final harness avoids repeated CLI initialization and uses one library session.

## Windows-specific findings

### Helper placement is required

SRT performs its WFP behavioral verification before applying session filesystem grants. When `srt-win.exe` lived only inside the user-owned repository, `srt-sandbox` could not execute it during that pre-grant verification. The pinned ARM64 helper was copied to a versioned `Program Files` path and provided through `windows.srtWin.path`.

This should become an explicit installer/tool-broker prerequisite. Do not fall back to an unpinned helper or add broad read access to the user's profile.

### Use SID-default read isolation on Windows

A broad `denyRead` on `C:\Users\<user>` prevented Node from traversing `C:\Users\<user>\Documents` to reach the explicitly allowed repository. The passing policy instead relies on the dedicated sandbox account having no inherent access to the caller's files, then grants read access only to:

- the controlled repository/tool root;
- the controlled output directory;
- the machine-wide Node runtime.

The outside-read canary confirms the sandbox account could not read another path in the caller's temp tree.

### Prefer one initialized session

Running the CLI separately for every probe repeatedly applied and restored the same ACL policy; one restore hung during harness development. The passing harness imports `SandboxManager`, initializes once, wraps all commands, and resets once. This matches the proposed long-lived `SandboxRunner`/tool-broker architecture more closely.

### Version reporting inconsistency

The installed package manifest and lockfile report `0.0.65`, while `srt --version` reports `1.0.0`. The spike validates the pinned package manifest, lockfile, helper hash, and behavior rather than trusting the CLI version string alone.

## What this does not prove

- SRT behavior on other Windows builds, architectures, filesystems, or enterprise security policies.
- Rust tool-broker integration.
- Execution of the future clean-room DOCX runtime under this policy.
- opencode command-path closure or configuration isolation.
- current remote Ollama reachability/tool calling.
- production installer, upgrade, signing, or uninstall UX.

## Decision

Phase 1 is green. Proceed to the opencode server/policy proof, while retaining these SRT startup gates:

1. exact pinned package version and helper hash;
2. installed sandbox account and WFP verification;
3. explicit versioned helper path readable by `srt-sandbox`;
4. one initialized session per controlled workspace/job scope;
5. precise Windows read/write grants and network deny;
6. fail-closed cleanup/recovery checks.
