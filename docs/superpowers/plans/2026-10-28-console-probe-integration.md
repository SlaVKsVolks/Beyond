# Reviewed Windows console fixture integration

## Pre-edit contract

Goal: remove the standalone first-party unsafe console probe from this Rust-assurance branch by integrating the already reviewed safe Windows fixture and its complete bounded Cargo-build owner from the Windows candidate. This is integration of an approved design, not a new allocator/console redesign. Main and upstream remain untouched.

Base: `822cc79daae7e086ff5f31100249da3015196211`. Reuse source from committed Windows candidate `7ecda218e58320fcb40ab4b8abc516e1c44855b4`, never its dirty TLS locks. Preserve the unconditional launcher forbid and prior macOS removal. No first-party unsafe, installs, dependency upgrades, lost diagnostic behavior, relaxed assertions, new ignores, app stops or visible processes.

Owned paths: launcher `tests/support/{mod.rs,console_probe.rs,console-probe/Cargo.toml,console-probe/Cargo.lock}`, `Cargo.toml`, `Cargo.lock`, `build.rs`; `tools/{rust_direct_inputs.py,check_rust_safety.py}`; `tests/{test_rust_direct_inputs.py,test_rust_safety.py}`; this contract and `docs/rust-assurance.md`. No sibling runtime dependency: copied files and locks must be self-contained.

| Requirement | Invariant / observable evidence | Initial status |
|---|---|---|
| Native protocol | Real native GetConsoleWindow/IsWindowVisible queries through admitted published safe APIs; stdin/stdout/stderr, exit 7 and hardware-query protocol unchanged. Full hidden/startup tests. | NOT RUN on composed branch |
| Owned compiler lifecycle | Preserve exact reviewed suspended spawn, job ownership, finite operation/log/teardown bounds, authoritative empty-tree confirmation, failure retention and checked Cargo artifact identity. Existing helper regression suite. | Reviewed source available; integration NOT RUN |
| Safe scope | Both fixture root and its package forbid unsafe. Register its actual separate workspace and audit actual Cargo metadata; direct-rustc reintroduction must fail. Real-repository and controlled input regressions. | FAIL: old unsafe probe and direct compiler |
| Compatibility / supply chain | Exact admitted dev dependency versions/closure; no existing package upgrade. Separate fixture lock; preserve launcher macOS removals and every existing lint. Reuse provenance only after hash equality. | NOT RUN |
| Resource isolation | Fresh short owned test homes, no self-update/audio, existing serialized cache; never user data. Preserve useful diagnostics. | Required on every runtime test |
| Acceptance | Scoped fmt/check/guarded all-target Clippy, fixture fmt/guarded checks, full launcher and Python suites, unmodified readiness, independent full-diff review. Record every failure and original ignore/warning. | NOT RUN |

Sequence: (1) reproduce the real audit gap and add failing audit regression; (2) integrate exact reviewed fixture/support and minimum manifest/build metadata closure, then enforce actual standalone workspace coverage; (3) run scoped and prescribed gates, review and record exact remaining blockers. A successful console slice does not clear allocator or four package-wide enforcement gaps. No main promotion while required gates fail.

Limits: native attached-console/ConPTY/restoration, other Windows releases, actual Rust 1.85, game/resources and hosted CI remain separate acceptance. No new positive-console manufacturing or game run is authorized by this integration. Retain existing bounded source-informed legacy-layout tests; do not claim them as an actual old-toolchain execution.

Ruling: reuse the already approved fixture design and previous dependency review after exact binding rather than request another questionnaire/design approval. Cost if wrong: a composition defect; fresh composed checks and independent review are mandatory.

## Execution / acceptance ledger

Executed inline after the explicit repair request, reusing the approved design. Actual audit RED was five failures/13 tests after correcting sandbox-only access failures; GREEN was40/40. Exact imports/provenance bind four source/lock files, the fixture manifest's sole new forbid, five admitted archives and16used-path files. No existing package upgrade/removal.

Scoped format/check/guarded all-target Clippy and native launcher tests pass:197passed/4originalignores. Python384run/11skipped, no failures; launcher/fixture doc/unit zero counts are not invented native coverage. Completed unmodified readiness still exits1 only on four retained package-wide enforcement gaps. Its other selected gates pass. All five locks unchanged during the completed verification lifecycle. Full results and limits are in [Rust assurance](../../rust-assurance.md) and ignored `.superpowers/sdd/2026-10-28-console-probe-integration/` receipts/logs.

Independent fresh-context full-diff review: no change-specific Critical/Important issue; scoped repair-branch preservation approved, main promotion not approved. Ruling: accept the scoped review while retaining every declined runtime/MSRV/game/CI/full-profile boundary and the global gate failure. Cost if wrong: unverified composition/runtime behavior outside the executed hidden-host and source scope; no main/upstream promotion follows.

Cleanup: four exact empty test-home directories removed after all producers completed. New native failure/output evidence retained (~243MB apparent); exact negative lock/reference qualification is incomplete. Existing shared D cache grew from59,455,118,055to63,014,823,118apparent bytes, below64GiB. The1GiB initial growth estimate was low; the enforced space reserve was not crossed. This cache is mixed-owner acceptance/runtime evidence and not deletion-qualified by this slice. No file bytes reclaimed or broad cleanup claimed; ownership, purpose and cleanup conditions are in `retention.json`.
