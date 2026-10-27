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
