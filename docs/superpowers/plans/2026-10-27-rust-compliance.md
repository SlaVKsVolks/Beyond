# Rust Compliance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans as applicable. Use the assigned isolated worktree; the coordinator integrates and obtains independent review.

**Goal:** Improve Rust assurance without changing Horizon behavior and report the precise remaining compliance gaps.

**Architecture:** Preserve native unsafe boundaries until a safe equivalent with identical semantics is established. Apply compatible scoped checks, then reconcile their actual coverage against the acceptance ledger.

**Tech Stack:** Existing Rust/Cargo and Python tools; existing project CI configuration.

**Spec:** `docs/superpowers/specs/2026-10-27-rust-compliance-design.md`

## Global Constraints

- No features removed or changed; no checks weakened; no added or relocated first-party unsafe.
- Preserve edition, MSRV, supported targets/features, Cargo layout and Rust dependencies. The only authorized dependency addition is Python development backport `tomli==2.0.1; python_version < "3.11"`; no install is authorized.
- No installs, game runs, deployment, remote writes, branch protection changes or unsafe exceptions.
- Do not create negative unsafe-code fixtures. Independent review is mandatory for safety, public-contract and concurrency changes.

## Review Focus

1. Unlisted Cargo target or separate workspace: inventory and validate actual scope in Task 1.
2. Missing, malformed or unsupported manifest input: guard tests must fail closed in Task 1.
3. Native error and resource lifetime behavior: preserve source and explicitly report unresolved evidence in Task 2.
4. Allocator counters, reallocations and benchmark results: preserve all metrics and stop if exact safe equivalence is absent in Task 3.
5. Missing tools, CI execution or OS runtime evidence: mark NOT RUN or BLOCKED in Task 4.

## Tasks and shared interfaces

| Task | Owned paths | Interface to other tasks | Acceptance |
| --- | --- | --- | --- |
| 1 Guard | manifests, `tools/check_rust_safety.py`, `tests/test_rust_safety.py`, `tools/pr_ready.py`, CI | Document actual exit behavior and target scope for verifier | Focused guard tests, configured checks and no weakened existing check |
| 2 Native | native OS source only | Report exact callsites, semantics, errors and safe-equivalence blockers | Preserve behavior and tests; no new unsafe |
| 3 Benchmark | allocator examples only | Report metric semantics and equivalence evidence | Preserve all measurements or stop without edits |
| 4 Governance and review | assurance, design and plan documents | Reconcile lane reports and evidence at integration | Independent review and truthful gate statuses |

### Task 1: Guard

- [ ] Inventory first-party workspaces and target/lint scope, including separate manifests and non-default targets.
- [ ] For any new behavioral check, first add focused tests for covered, missing, malformed and unsupported inputs; verify fail-before where practical without writing unsafe fixtures.
- [ ] Implement the smallest compatible guard without changing Rust dependencies or weakening existing checks; run its focused tests and record exact commands/results.
- [ ] Verify configured guard integration and actual execution separately. Any uncovered target or failed gate remains open.

### Task 2: Native boundaries

- [ ] Trace each native call through its consumers, including return values, errors, resource lifetimes and supported operating systems.
- [ ] Change code only if an existing safe API demonstrably preserves those contracts; add a behavioral regression test first and obtain independent review.
- [ ] Otherwise retain code and report the exact blocked boundary. Do not delete or weaken tests.

### Task 3: Allocator examples

- [ ] Identify each reported allocation metric, including allocation, zeroed allocation, deallocation and reallocation behavior.
- [ ] Replace only if an existing safe implementation preserves every metric and observable output. Test the behavior before and after and obtain independent review.
- [ ] Otherwise retain the examples and report the unresolved boundary.

### Task 4: Reconcile and verify

- [ ] Reconcile the full integration diff with every ledger row and independent review findings.
- [ ] Run project-prescribed `python3 tools/pr_ready.py --base <verified-base>` and applicable focused checks on the actual integration inputs; retain full output. A missing tool or unsupported environment is NOT RUN/BLOCKED, not PASS.
- [ ] Record source, configured checks, execution, CI, native runtime and user acceptance independently. Routine isolated automated tests are authorized within the contract; installed GUI/game and production runtime acceptance are not.

## Phase 5 preflight and shared interfaces

| Task | Owner and interface | Gate |
| --- | --- | --- |
| 5 Index repair | Metadata only; exact verified manifest, no Rust blob staged | PASS: 1,545 raw hashes and clean OIDs matched, index entry bytes unchanged, clean index/status. `32-reindex-{preflight,command,readback}` receipts. |
| 6 Owned deadline | Native lane owns `crates/horizon-friends/src/deadline.rs`, `lib.rs`, `direct.rs`, `host.rs`, `session.rs`, `crypto.rs`; exports `deadline(TcpStream, Duration) -> io::Result<Deadline>`, `Read`, `Write`, `cancel(self) -> io::Result<TcpStream>` | Explicit user approval overrides the old borrowed-guard API restriction. Preserve contract and no new unsafe; independent safety/public/concurrency review required. |
| 7 Integration and checks | Coordinator integrates Task 6 only after Task 5 clean-index readback and documentation handoff; verifier consumes frozen integrated source | Focused tests first, then both CI Clippy modes, root no-fail-fast tests, docs, launcher tests and unmodified `pr_ready --base 41af517` with normal parallelism. |

Integration and reindex are serialized; documentation and native source work use disjoint checkouts. The D: task target is the sole verification target, subject to the existing 64 GiB cap and 128 GiB free floor; C: builds remain blocked below 32 GiB free. Preserve original C: cache and other active work. No installs, deployment, remote writes, global configuration changes, application stops or cleanup.

### Task 5: Confirm index boundary

- [x] Verify exact manifest, clean OIDs and raw hashes, stage only its NUL-delimited paths with current authorization, and read back unchanged index entries, empty diffs and status. No Rust semantic blob was staged.

### Task 6: Implement owned deadline

- [ ] Before implementation write focused tests retaining the original deadline/EOF/cancellation assertions and covering trickled reads, blocked writes, shorter preexisting timeouts and fallible restoration, expired release, zero and overflow, partial I/O, interrupts and non-deadline errors. Run them first and report the precise observed RED; do not label unrelated historical failures as a new test RED.
- [ ] Implement the owned API and migrate direct/host/session/crypto handoffs under one absolute 10-second inbound handshake deadline. Preserve cipher and counters and forbid raw-stream escape while either half remains active.
- [ ] Run focused tests, including encrypted payload after handoff and authentication/relay cases; capture exact commands, failures and GREEN scope. Stop at any safe-contract gap; do not weaken tests or add unsafe.
- [ ] Obtain independent review of the full diff, ledger, errors, timeout ordering, ownership, worker lifecycle and concurrent release/expiry race.

### Task 7: Final verification

- [ ] Integrate reviewed Task 6 after clean index readback; reconcile the requirement→invariant→observable ledger with final source and review.
- [ ] Run focused tests followed by both CI Clippy modes, root no-fail-fast tests and docs, launcher tests and unmodified `python3 tools/pr_ready.py --base 41af517`; preserve complete results. Guard's five unsafe packages and console probe remain expected FAIL until actually resolved; do not call them compliant.
- [ ] Record Windows automated checks separately from hosted CI, other operating systems, game, production and user acceptance; mark unavailable or unrun evidence NOT RUN/BLOCKED. Prior root/launcher formatting passed with 384/21 stable-option warnings on their unchanged formatting inputs; rerun applicable prescribed gates for the final source.

## Phase 7 preflight: Windows native boundaries

User approval covers the design boundary and execution; no second routine approval is needed. Use separate worktrees with disjoint source ownership. Serialize Tasks 8–10 dependency/lock mutations and all integration; the coordinator reconciles the root and launcher lock closures. Preserve the existing dirty Windows candidate checkout as read-only reference; do not import unrelated changes. Before each lane edits, review published dependency provenance, license, MSRV, internals and behavior, then record its exact contract and tests. The installed pinned toolchain is 1.99.1. Use only the existing D: target, task cap 64 GiB and free floor 128 GiB; C: builds remain blocked below 32 GiB. No installs, deployment, remote writes, global configuration changes, GUI/game runs, focus changes or cleanup.

| Task | Owned files and shared interfaces | Self-consistency and acceptance |
| --- | --- | --- |
| 8 Threads | `crates/horizon-threads/src/lib.rs`, its `Cargo.toml`, root `Cargo.lock`; launcher lock only if verified scoped closure requires it | Published `thread-priority` 3.1.1 candidate; preserve native current-thread AboveNormal/Idle, boolean/error result and >4 CPU gate; unchanged non-Windows source/tests. |
| 9 Server | `crates/horizon-server/src/signals.rs`, `process.rs`, `main.rs`, relevant Windows console tests, `Cargo.toml`, root `Cargo.lock`; existing service test only if still needed | Published `win32console` 0.1.5 / `winconsole` 0.11.1 candidates; preserve all events/counts, ignored registration failure, callback lifetime and native console-presence condition. |
| 10 Launcher | `launcher/Cargo.toml`, `Cargo.lock`, `src/console.rs`, necessary crate-root attributes, `build.rs`, relevant test roots/support and probe, new in-workspace fixture package if needed; guard/driver and tests only if required | `console_probe() -> &'static Path` remains automatic; safe fixture is inside existing launcher workspace and all-target forbid/guard coverage is verified. |
| 11 Prepare/client | Bounded `horizon-prepare` manifest or roots only after native contract; client allocator analysis read-only | Windows enforcement must not break other platforms. Preserve client metrics, explicit System and failed-operation behavior; report unproven allocator candidate as blocker. |
| 12 Integration | Coordinator integrates reviewed Tasks 8–11 sequentially and owns final shared locks/guard reconciliation | Final source plus locks drive prescribed checks and independent review. No Windows-only proof is presented as full-platform compliance. |

### Task 8: Thread priority

- [ ] Review `thread-priority` 3.1.1 provenance/license/MSRV and native implementation; record safe surface, internal unsafe and dependency closure.
- [ ] Exercise compiler forbid against existing Windows unsafe before changing it, then add focused behavior tests for exact calling-thread priority, AboveNormal/Idle, result/error path, >4 gate, real `BackgroundModeEnd` failure on a fresh non-background owned thread and unaffected sibling/child inheritance. An invalid `ThreadId` handle is not safely constructible and is NOT APPLICABLE; the unrepresentable invalid-handle case is not covered. Preserve existing tests; no new unsafe fixture.
- [ ] Implement only an equivalent safe published API, preserve Linux/Android/macOS source; run targeted Windows checks and obtain independent safety/lifecycle review. If exact readback or behavior cannot be established, report the gap instead of weakening assertions.

### Task 9: Server console and controls

- [ ] Review candidate packages and scoped lock changes. Trace event/counter semantics, ignored registration failure and `GetConsoleWindow` condition; `IsTerminal` is not an equivalent substitute.
- [ ] Add focused tests for repeated requests, event handling, callback lifecycle and console-present/absent/redirected cases in owned hidden children. Do not send global parent-console events or open visible windows. Record native delivery as NOT RUN/BLOCKED if it cannot be observed safely.
- [ ] Implement only equivalent safe calls and run focused tests/Clippy; independently review full source, lifecycle and evidence before integration.

### Task 10: Launcher and probe

- [ ] Review dependency implementation and exact attachment, handle snapshot/restore order, invalid/null filtering, borrowed ownership and first-error behavior. Failed attach remains benign; inherited handles are never closed.
- [ ] Preserve actual window visibility, redirected stdin/stdout/stderr, exit 7 and hardware-probe behavior. Keep `console_probe() -> &'static Path` automatic; prefer a fixture package within the existing launcher workspace rather than a production binary that changes install/default-run/shipping behavior; enforce forbid for all targets and test actual guard/compiler coverage. Change guard/driver only narrowly if the compilation flow requires it, without weaker checks.
- [ ] Preserve macOS titlebar source and assertions; run focused Windows tests and independent review. Real visibility/attachment acceptance remains open if unavailable.

### Task 11: Prepare and allocator boundary

- [ ] Determine whether Windows-scoped `horizon-prepare` forbid can cover applicable targets without breaking preserved non-Windows code; establish a native pre-edit contract before any manifest/root edit. Otherwise record the blocker.
- [ ] Audit any independently published allocator candidate read-only against every existing requested-byte, explicit System, failed operation, peak, tracking and output invariant. Current bounded observations: `stats_alloc` counts before success and lacks peak; `tracking-allocator` changes layout/reentrancy; `alloc_tracker` relative/process peak is withheld pending equivalence proof. Do not substitute RSS or implement an unapproved metrics change; report unresolved gaps.

### Task 12: Integrate and verify

- [ ] Integrate each independently reviewed task in sequence; reconcile shared lock closures and any guard fixture interface. Review full diff, provenance, no added first-party unsafe, feature and platform preservation.
- [ ] Run prescribed formatting, both CI Clippy modes, full root tests and docs, launcher tests and unmodified `python3 tools/pr_ready.py --base 41af517` once on final source; retain complete output. Run actual scoped Windows target forbid checks and state unresolved full-platform guard failures, never call them a pass.
- [ ] Separate compiler/test results, actual Windows native event/console behavior, hosted CI, other OS, game, production and user acceptance. Mark missing evidence NOT RUN/BLOCKED. Reuse prior runs 48/50/51/52 only for unchanged inputs and acceptance layer.
