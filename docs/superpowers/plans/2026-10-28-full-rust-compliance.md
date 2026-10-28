# Full Rust-rule compliance completion implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Preserve isolated editing checkouts and one serialized compiler writer.

**Goal:** Close the remaining first-party unsafe and enforcement boundaries and pass all applicable Rust-profile assurance gates without losing original capabilities.
**Architecture:** Preserve existing Windows behavior and diagnostic workloads; use only established safe interfaces, exact observable contracts and unconditional package/target forbids. No compliance by deleting capabilities or suppressing failures.
**Tech Stack:** Existing Rust/Cargo and Python, unchanged editions/MSRVs, pinned installed Rust1.99.1 for current verification.
**Spec:** Existing `../specs/2026-10-27-rust-compliance-design.md`, later owner capability/platform decisions, `../../rust-assurance.md`, and current owner direction to finish without partial handoff. Design/questions are delegated within these existing requirements, not permission to silently change them.

## Global Constraints
- No new/expanded first-party unsafe, relocation into generated/C/macros/custom dependencies, weakened tests/lints or feature exclusion tricks.
- Retain original diagnostic capabilities, requested live/step peak/realloc/failure accounting, workload/fingerprint and genuinely uninstrumented latency baseline. A concrete preserving allocator implementation must be established before edits.
- Windows behavior and player data remain protected. Linux support deferred, not an exemption for retained unsafe source; macOS native removal already done.
- Preserve editions, target/workspace layout, existing provider/model/recovery policy, apps/games, attribution/history. The owner's later explicit approval permits correcting only demonstrated inconsistent MSRV declarations to existing dependency requirements; do not upgrade dependencies or raise the workspace baseline. No deploy/release/main promotion/force push. Only reviewed repair-branch sync remains authorized.
- Every meaningful change has pre-edit contract, real fail-before/pass-after evidence where applicable, independent review and target/lint proof. External/platform/CI/toolchain/fixture acceptance is not inferred from source.
- Existing task cache cap64GiB, D free floor128GiB, no builds before admission and no new cache merely to evade limits. Installs/unavailable tools need explicit approval.

## Review Focus
1. Retained allocations across peak resets and a lower second-window peak; correct negative live deltas.
2. Failed realloc retains original data/accounting; no fake allocator-null test through capacity overflow or panic.
3. Scheduling and signal replacement preserves OS semantics, not just names/success values.
4. Copy fallback and errors preserve independent copies, cleanup and atomic checked publication.
5. Every discovered standalone, build, example and platform target is actually covered; checks cannot pass by omitting the hard targets.

### Task 1: Close native package boundaries
**Files:** prepare files/manifest, threads source/manifest, server signals/manifest, narrowly scoped root/launcher locks, real audit regressions in tests/test_rust_safety.py.
**Interfaces:** Preserve clone_or_copy, priority hints and stop-request APIs; produce unconditional package forbids and observed regression results. Assigned exact pre-edit contract is plan workspace native-brief.md.
- [ ] Establish exact safe published API/source/semantic compatibility before native edits; stop an unsupported boundary rather than weaken it.
- [ ] Observe actual repository-audit failure before manifest/source changes; then safe replacements and package enforcement.
- [ ] Run scoped package tests/all-target forbid Clippy/format and lock/target/source comparisons under serialized admission; independent spec/quality review.

### Task 2: Complete capability-preserving allocator redesign
**Files:** two client allocation examples and minimal shared safe support/manifest edges only after a preserving contract is established. Current examples remain read-only until then.
**Interfaces:** requested live, exact step peak/reset preserving outstanding blocks, success/failure/realloc, output/fingerprint and disabled baseline as specified in existing assessment.
- [ ] Reuse rejected-candidate evidence; audit bounded genuinely new published observation APIs, including safe observer hooks rather than only prebuilt counters.
- [ ] Establish concrete safe implementation and safe deterministic tests; independent design review before edits. No custom unsafe wrapper or reduced metric.
- [ ] Real failing bounded tests, safe implementation, all-target client forbid, full resource/fingerprint and matched baseline proof when authorized fixtures exist.

### Task 3: Enforcement, CI, MSRV and runtime acceptance
**Files:** minimal necessary assurance tooling/configuration and existing ledger, only after exact contracts and current configuration inspection.
**Interfaces:** unmodified safety/readiness passes plus every applicable profile gate PASS or justified N/A; no unresolved required FAIL/BLOCKED accepted as complete.
- [ ] Verify actual target coverage and installed declared compiler availability; request required install authority rather than silently upgrading floors.
- [ ] Verify configured CI and actual required-check execution/protection without main/release deployment; do not replace a real failing job with an unconditional pass.
- [ ] Run remaining applicable normal/failure/resource/native checks with isolated owned resources, never user data or parent-console events; missing fixtures/authority remain exact blockers.
- [ ] Full prescribed final format/CI Clippy/tests/docs/launcher/Python/readiness on final source, independent final review, reviewed repair-branch synchronization/readback and qualified cleanup.
