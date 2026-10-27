# Rust assurance contract and acceptance ledger

This record captures the pre-edit contract for the 2026-10-27 bounded Rust-assurance work against base `41af517023e96905a03e1e8999178dbce6a82a92`. It is not evidence that a gate has passed. The coordinator must reconcile it with the final integrated diff and actual check output.

## Contract

**Goal:** Make safe, reviewable progress toward the user's Rust profile while retaining Horizon's behavior and honestly reporting blockers. **Non-goals:** feature changes, dependency or layout changes, tool installation, game execution, deployment, remote writes, branch protection changes, unsafe exceptions and universal safety claims.

**Assumptions:** Existing behavior and test assertions are contracts unless the user says otherwise. Root workspace edition 2024/MSRV 1.85, launcher's edition 2021, and telemetry-server/mod-registry MSRV 1.90 remain unchanged. Existing unsafe is not authorization to expand or relocate it. Source inspection cannot establish unsafe-free dependencies. Platform-specific acceptance needs its own environment and permission.

**Ownership:** Separate worktrees and disjoint paths: guard owns manifests, guard and tests, pr_ready/CI; native owns native OS files; benchmark owns allocation-counting examples; governance owns this record, design and plan; verifier is read-only. No lane may silently expand ownership. Shared interface is the integrated diff and evidence, not imports from sibling worktrees.

**Errors and edge cases:** Missing or malformed manifests, unsupported targets/features, missing tools, empty scopes and guard failure must be surfaced, never silently passed. Preserve platform-specific return and error behavior, unavailable or denied OS operations, allocator accounting including realloc/zeroed/deallocation, and existing test observables. Do not author negative unsafe fixtures.

**Resource, concurrency and lifecycle:** Preserve allocation counters and benchmark semantics, native scheduling, signal registration and repeated shutdown requests, thread behavior, handles, process lifetimes, reflink cleanup and publication. Do not add shared state, leaks, lock cycles or unbounded growth. Independent review is required for changes touching safety, concurrency, resource lifecycle, FFI, security or public contracts. No external effects, game process or OS-setting mutations are authorized.

## Requirement → invariant → observable acceptance

Statuses here are initial pre-edit statuses and must be updated with integrated evidence. NOT RUN and BLOCKED are never PASS.

| Requirement | Invariant/property | Observable check and initial status | Residual gap |
| --- | --- | --- | --- |
| Preserve features, APIs and errors | No behavior, target, dependencies, version floors or existing checks lost | Diff and manifest review; focused normal/failure tests and prescribed checks: NOT RUN | Platform behavior and full supported matrix not established |
| No added or relocated first-party unsafe | No new unsafe or evasion through generated code, macros, C, build scripts or dependencies | Full diff and target/source inspection: NOT RUN | Existing unsafe must be assessed separately; dependency internals are outside proof |
| Forbid unsafe across all first-party packages and targets | Actual compiler lint applies to every target, including separate workspaces | Manifest/target scope plus guarded safe builds: BLOCKED | Existing native and allocator unsafe targets cannot be blanket-forbidden while preserving features; no negative unsafe fixture allowed |
| Fail-closed guard and CI | Missing, malformed and uncovered inputs fail visibly; required CI blocks integration | Focused guard tests, configuration and execution evidence: NOT RUN | CI execution and remote branch protection are distinct and not authorized here |
| Preserve resource, concurrency and lifecycle behavior | Allocation metrics, OS scheduling, signal/handle lifetimes and cleanup remain intact | Focused tests and independent diff review: NOT RUN | Native scheduling, shutdown, console, reflink and appearance runtime not tested |
| Preserve failure paths | Errors, timeouts, cancellation, I/O and resource exhaustion retain relevant semantics | Focused tests where applicable and review: NOT RUN | No claim for untested paths; property testing, fuzzing and fault injection only where justified |
| Prescribed project verification | All applicable configured format, lint, build and test checks pass for actual inputs | `python3 tools/pr_ready.py --base <verified-base>` and relevant focused checks: NOT RUN | Tool availability, supported targets and required resources must be recorded |
| Independent review | Reviewer did not implement safety/public/concurrency changes and checks ledger, full diff and evidence | Review report: NOT RUN | Required prior to acceptance |
| Runtime and user acceptance | Separate source, configured, execution, CI, OS runtime and user layers | Runtime and user acceptance: NOT RUN | No game or native runtime actions authorized; no production or deployment acceptance |

## Known native boundaries

The native lane's source assessment found no established safe existing equivalent preserving all semantics for thread priority (`crates/horizon-threads/src/lib.rs`), supervisor signal handlers (`crates/horizon-server/src/signals.rs`), console-aware process creation (`crates/horizon-server/src/process.rs`), clone-before-copy (`crates/horizon-prepare/src/files.rs`), launcher console attachment and macOS titlebar appearance (`launcher/src/console.rs`, `launcher/src/titlebar.rs`), and Windows console-visibility test probe (`launcher/tests/support/console_probe.rs`). Its report is a bounded source assessment, not an exhaustive compiler inventory or proof that no external library exists. The counting allocators in `crates/horizon-client/examples/model_library_bench.rs` and `resource_memory.rs` also use unsafe. Do not claim full profile compliance until every applicable target and required gate passes without violating the contract.
