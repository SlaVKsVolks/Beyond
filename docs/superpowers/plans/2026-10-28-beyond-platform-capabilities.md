# Beyond Platform and Capability Preservation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Use separate editing worktrees and a serialized build writer.

**Goal:** Remove the known first-party macOS-native unsafe paths without changing Windows behavior, and establish a capability-preserving allocator redesign boundary.

**Architecture:** Remove unsupported native branches instead of commenting out unsafe or masking checks. Preserve the shared APIs, existing Windows implementation and deferred Linux source. Review diagnostic redesign against observable capability semantics rather than upstream code identity.

**Tech Stack:** Existing Rust/Cargo and Python; installed pinned Rust 1.99.1; existing D verification cache.

**Spec:** `../specs/2026-10-27-rust-compliance-design.md`, especially the later Beyond capability-preservation decision; `../../beyond-fork.md`.

## Global Constraints

- No new or relocated first-party unsafe, custom dependency evasion, weakened checks or unsafe exceptions.
- macOS-specific source may be removed; preserve Windows capabilities, public handoffs and user data. Linux support is deferred, not declared verified or made compliant through exclusion tricks.
- All other original capabilities, including diagnostics, must remain; exact upstream implementation identity is not required. No RSS substitution or silent loss of allocation measurement semantics.
- Preserve edition/MSRV, protocol and persisted identifiers, original attribution and history. No bulk internal rename in this change.
- No installs, GUI/game/focus changes, global console events, remote CI changes, release, deployment, main-branch merge or force push. Later sync is only to the reviewed Beyond repair branch.
- D is the sole serialized build target: 64 GiB task cap and 128 GiB free floor. No C builds below 32 GiB free. Retain useful evidence; cleanup needs exact-file/lock/live-reference qualification.

## Review Focus

1. No macOS-only removal may delete a Windows implementation, test assertion or Linux fallback.
2. Shared API calls to `apply_appearance`, thread hints and `clone_or_copy` must remain valid on Windows.
3. Manifest/lock cleanup must remove only obsolete direct edges; no unrelated upgrades or third-party patching.
4. Linux unsafe remains explicitly visible as a residual gap; no full-package/OS acceptance claim from Windows tests.
5. Diagnostic redesign must retain measurement capabilities and a real uninstrumented latency path, not just familiar output labels.

---

### Task 1: Remove known macOS-native unsafe boundaries

**Files:** `crates/horizon-threads/src/lib.rs`, its `Cargo.toml`, `crates/horizon-prepare/src/files.rs`, `launcher/src/titlebar.rs`, `launcher/Cargo.toml`, scoped root/launcher lock closure only if required; assurance ledger. No vendor/CI/history edits.

**Interfaces:** Preserve `favour_current() -> bool`, `idle_current() -> bool`, `behind_server() -> bool`, `apply_appearance(&Window, Appearance)`, `clone_or_copy(&Path, &Path) -> io::Result<()>` and the complete Windows titlebar render path. Apple-specific hints/appearance/clone optimization are intentionally removed; other-platform fallbacks are explicit.

- [ ] Freeze base/hash identities and record the per-file Rust pre-edit contract. Inventory the Apple QoS functions/tests, macOS clone branch and AppKit appearance path; distinguish these three boundaries from any remaining safe/historical macOS references. No claim of a repository-wide macOS purge.
- [ ] Capture the existing Windows characterization: threads package tests, prepare package tests, full launcher `--lib --bins --tests`, and current lint/guard status. Reuse unchanged successful receipts only after exact relevant source/lock/toolchain equivalence is established. Do not label inactive macOS code a Windows behavioral RED.
- [ ] Remove Apple QoS implementations and Apple-only readback tests; broaden unsupported fallbacks and update only Apple-support expectations. Preserve Linux background/idle bodies and every Windows priority assertion unchanged.
- [ ] Remove macOS clonefile implementation; keep Linux clone-first and the shared copy/hash/publication lifecycle unchanged. The unsupported-platform fallback continues to allow normal copy.
- [ ] Remove AppKit `apply_appearance` implementation and keep its existing safe no-op handoff on supported paths. Preserve the Windows renderer body byte-for-byte. Remove only direct `objc2-app-kit`/`raw-window-handle` edges proven unused elsewhere in launcher source; preserve transitive packages still needed by third parties. Remove Apple's direct libc selector only if no remaining thread source requires it there.
- [ ] Assess whether unconditional launcher package forbid now covers every actual Cargo target; add it only if source/target inventory establishes no remaining package unsafe. The standalone direct-rustc probe remains separately blocked until truly integrated.
- [ ] Run pinned meaningful checks on changed scope: package tests, launcher full tests and all-target Clippy with `-D warnings -F unsafe_code`; use edition 2024 for threads and actual manifest editions for prepare/launcher. Reconcile exact diff, target scope, locks, warnings/ignored tests and remaining Linux/probe/client gaps. Obtain independent spec and quality review before integrating.

### Task 2: Preserve allocator diagnostic capabilities by design

**Files:** `crates/horizon-client/examples/model_library_bench.rs` and `resource_memory.rs` are read-only in this task; record findings in the existing assurance ledger/plan workspace. No allocator implementation, client build script or manifest change in this task.

**Interfaces:** Preserve the allocation tracking switch, requested live bytes, per-step requested-byte peak, successful realloc/failure accounting, original workload outputs/fingerprints, and the genuinely uninstrumented latency mode. Requested payload bytes and allocator/process overhead must be distinguished.

- [ ] Reconcile both examples' actual metrics and error behavior with the new owner decision. Treat System/layout delegation as an implementation constraint to analyze, not permission to change or mislabel the measurement.
- [ ] Audit a bounded published safe allocator/tracker candidate against success/failure, realloc, peak reset, reentrancy, tracking overhead and disabled-mode behavior. Reuse previous rejected-candidate evidence where its rejection still applies; do not repeat broad library research or write a custom unsafe wrapper.
- [ ] Produce a requirement-to-observable-test matrix and exact proposed safe API/error/measurement contract. Propose safe bounded tests for live/peak/reset/realloc/disabled mode and failure paths; do not deliberately exhaust host memory or author unsafe fixtures.
- [ ] Obtain independent review of equivalence and exposed differences. If any required capability is unestablished, leave examples intact and report the exact gap. A redesigned product implementation needs its own concrete approved contract before Rust edits; this task does not authorize deleting diagnostics or claiming the blocker solved.

### Task 3: Final acceptance and synchronization

- [ ] Reconcile the integrated Task 1 diff and Task 2 capability matrix with the updated spec; independent review and pre-edit contracts must be complete.
- [ ] On final source run prescribed root/launcher formatting, both existing CI Clippy modes, full root tests/docs, launcher tests and unmodified `python tools/pr_ready.py --base 41af517`. Retain failures and full relevant logs. Do not waive the Linux/client/probe coverage failures or substitute a partial gate for full compliance.
- [ ] Report source/compiler proof separately from actual OS/MSRV/CI/game/user acceptance. Record useful evidence and retained-output cleanup conditions. Commit reviewed work and, within the existing user authorization, sync only the dedicated Beyond repair branch; verify the remote SHA and unchanged main. No PR/release is authorized.

## Handoff

The user supplied the platform/capability decisions and retained Terra/high subagent execution preference. This bounded plan requires review before implementation. MacOS removal is authorized; allocator capability reduction, Linux support work, diagnostic product redesign without an exact contract, and a full internal branding migration are not hidden inside this plan.
## Execution readback, 2026-10-28

Owner approved this plan. Task 1's bounded removal is integrated at `dc810e287` after independent specification/quality review; Windows and retained Linux source remain unchanged in the reviewed diff. Launcher Cargo-target unconditional forbid and its real-repository regression are in place. Task 2's read-only assessment and corrected hook/failure analysis passed independent review; neither candidate preserves the complete contract, so both diagnostic examples remain intact and their product redesign remains BLOCKED.

Task 3 ran every prescribed final command, with exact results and the scratch-wrapper Python-path failure/recovery in [Rust assurance](../../rust-assurance.md). Formatting, both CI Clippy modes, full root tests/docs, launcher tests/forbid Clippy and Python tests passed. Unmodified readiness still exits 1 on four package-forbid gaps and the direct console probe; full-profile acceptance is not complete. No checks were weakened, no original diagnostic capability was removed, and no game/other-OS/MSRV/hosted-CI acceptance is claimed. Only reviewed repair-branch synchronization is authorized; main, release and deployment remain out of scope.
