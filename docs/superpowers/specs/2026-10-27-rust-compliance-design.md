# Rust assurance design

Base: `41af517023e96905a03e1e8999178dbce6a82a92`. This is a bounded improvement to Horizon's Rust assurance, not a claim of complete compliance.

## Intent and constraints

Improve mechanical enforcement and evidence without removing or changing features, weakening existing checks, adding first-party unsafe or moving it into generated code, macros, C, build scripts or dependencies. Preserve the edition, MSRV, supported targets and features, Cargo layout and dependencies. Do not install tools, run the game, deploy, change remote state or grant an unsafe exception. Existing first-party unsafe is a boundary to investigate, not permission to expand it. Safe-source checks do not prove dependency internals safe.

The root workspace declares edition 2024 and Rust 1.85; launcher uses edition 2021, while telemetry-server and mod-registry declare Rust 1.90. Each workspace and each applicable target must be assessed independently. Vendored `third_party/wgpu-hal` is not first-party source. Treat claimed target and lint coverage as provisional until mechanically verified.

## Approaches

1. Blanket forbid: would make existing targets containing unsafe fail, so it does not preserve the requested features.
2. Move calls behind new wrappers or dependencies: would evade the explicit constraint and is rejected.
3. Add safe, scoped mechanical enforcement where compatible; leave unresolved unsafe boundaries visible and fail closed for full compliance. This is the selected approach. A partial guard or an inventory is not the profile's required complete `forbid(unsafe_code)` gate.

## Boundaries and acceptance

Operating-system priority, signal handling, console, reflink and macOS appearance code has native unsafe operations. Allocator-counting examples also contain unsafe. A safe equivalent must preserve actual output, errors, resource ownership, lifetime and performance behavior before replacement. If none is established under the existing dependencies, preserve the code and mark the relevant gate BLOCKED. Do not delete tests or substitute a weaker observable.

The assurance ledger in [rust-assurance.md](../../rust-assurance.md) governs pre-edit invariants and acceptance. Each lane owns disjoint files in a separate worktree. The coordinator reconciles changes and evidence. An independent reviewer checks the ledger, full diff and evidence for safety, concurrency, lifecycle and public-contract changes. Source configuration, executed build and test results, CI execution, native runtime and user acceptance are separate evidence layers. A required gate that is NOT RUN, FAIL or BLOCKED means full compliance remains incomplete.
