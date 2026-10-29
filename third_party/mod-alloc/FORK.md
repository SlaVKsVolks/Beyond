# Beyond's scoped mod-alloc fork

This is a **maintained, partly owned fork**, not unmodified vendor code and not
an unsafe-free dependency. Upstream is James Gober's Apache-2.0 `mod-alloc 1.0.1`,
packaged VCS `b9c3012518e85701d35dfdaf5ad6c8161809c241`. Registry archive SHA-256:
`dd6bc58ad0744471a98d16c3192103aa3c27a2f89b44072efa9cdb76f4453d7f` (72,073 bytes).
The upstream LICENSE and historical README, changelog and original manifest are
retained. Actual package version is `1.0.1-beyond.1`, compiler floor `1.99.1`.

## Boundaries

- The complete upstream `unsafe impl GlobalAlloc for ModAlloc`, including all
  four method bodies, is byte-for-byte unchanged. So is the unsafe body of
  `current_snapshot_or_zeros`; the WHOLE legacy function is unavailable in exact
  mode rather than being repurposed. All other original Rust source files are
  hash-bound, including inactive backtrace and DHAT code.
- New `src/exact.rs`, `src/null_tests.rs`, native fixture/example and the existing
  build/test/example roots have unconditional `#![forbid(unsafe_code)]`.
  New exact bookkeeping is first-party owned. `lib.rs`'s safe binding/module/cfg
  envelope and manifest are registered in `FORK.json`, whose checksum is bound
  by `tools/mod_alloc_integrity.py`. The audit covers every Rust file; it does
  not blanket-exclude this directory. Changes need renewed review/registration.
- Default legacy mode retains upstream counters, reset, Profiler, backtraces,
  symbolication and DHAT capabilities, with their original semantics/limits.
  Legacy best-effort snapshots, destructive reset and TLS event omission are
  **not** exact accounting. Its process-global raw handle also retains the
  upstream assumption of a static allocator, not an enforced lifetime contract.
  Do not use nonstatic direct allocators with that legacy reporting surface.

## Exact mode and preserved application diagnostics

Enable `exact-counters`. Exact mode exports `ModAlloc`, `Snapshot` and
`AccountingError`. Use the referenced allocator's fallible `try_snapshot()` and
`finish_window()`. Legacy infallible snapshot/reset/Profiler/AllocStats are not
exposed in this mode; they cannot represent negative deltas or a sticky error.
`exact-counters` plus `backtraces` (also implied by symbolicate/DHAT) is an
explicit compilation error: the upstream first-event aggregation path may
allocate recursively. This does not remove any existing application capability:
the two Beyond resource diagnostics had no call-site tracing before this fork.

All six counters remain available. Successful alloc and zeroed alloc add one
event, requested bytes and one live block. Successful realloc adds one event,
changes current bytes by **new minus old**, adds only positive growth to total
bytes, and preserves live block count. Deallocation subtracts requested bytes
and one block. Null operations do not record; failed realloc retains the old
block. Arithmetic is checked transactionally; the first error latches forever,
without panic, allocation or partial counter commit. Reporting then returns
`Err`, never a plausible stale value. Format that error outside allocator hooks.

An allocation-free atomic serialization lock defines one logical record order
for all events, snapshots and window boundaries. `finish_window()` captures the
completed window and resets both peaks to the captured current values at that
same record point. It never clears outstanding objects or cumulative totals.
Compute signed net deltas with `i128::from(now) - i128::from(before)` outside the
lock. The absolute next-window peak can be lower than the preceding peak, but
never lower than retained requested bytes at the boundary. RSS stays separate.

**Measurement limit:** these are logical requested bytes, not allocator-internal
footprint, committed pages, RSS, or temporary old-plus-new realloc storage. Since
the frozen adapter records dealloc after System frees, an overlapping allocation
may record before that dealloc. That is a valid operation linearization order,
not an instantaneous physical-heap trace. Lock/state work is fixed-size and has
no TLS, formatting, backend call or callback under the lock. Waiting is not
bounded: scheduler starvation/preemption and asynchronous allocator reentry are
explicit unsupported cases; no universal deadlock/latency claim is made.

The model-library example selects actual unwrapped `std::alloc::System` when
`HORIZON_BENCH_COUNT_ALLOCATIONS` is absent. **Any present value**, including
empty, `0` and `1`, selects exact instrumentation through build-script cfg.
Resource-memory remains instrumented. Loader sequence, held objects, shared
model library, timing boundaries, fingerprint and output meanings are preserved;
negative net changes are no longer unsigned underflow. Errors propagate rather
than silently printing zeros.

## Test-only null backend and commands

Only the fork's library unit-test build with exact mode substitutes published
`alloc-chaos 0.1.0` *inside* the unchanged adapter, via a safe import alias. Normal
library, integration fixture and application builds always use actual System.
There is no release feature enabling injection. Chaos archive SHA-256:
`a33ab740fa933bb116dbff7a397795be9f83a1f30937cb7e82976e914a152699` (19,808 bytes).
Its selected operations return actual null before calling System; this is not
capacity overflow or host exhaustion. Injection is checking-thread-only and
does not prove teardown/concurrent null behavior or an OS memory-limit scenario.

Safe callers are fallible Vec reserve and pinned bytemuck 1.25.2's non-ZST
`try_zeroed_box::<[u8;64]>()` (dev-only `extern_crate_alloc`). Reports verify
operation/layout/new-size metadata as well as unchanged counters. Realloc's
selected failure index 1 of 2 is explicitly **truncated**, not exhaustive.

From this directory with the installed pinned toolchain and an admitted target:

```text
cargo +1.99.1 test --offline --locked
cargo +1.99.1 clippy --offline --locked --all-targets -- -D warnings
cargo +1.99.1 test --offline --locked --features exact-counters --lib --test exact_native -- --test-threads=1
cargo +1.99.1 clippy --offline --locked --features exact-counters --lib --tests --example exact_startup -- -D warnings
cargo +1.99.1 test --offline --locked --features exact-counters --doc
cargo +1.99.1 run --offline --locked --features exact-counters --example exact_startup
```

Exact checks deliberately name targets: upstream example programs exercise the
legacy-only API and are crate-cfg disabled in exact mode, so `--all-targets` is
not a supported exact-mode command. Default mode checks all original targets.
Do not use `--all-features`: exact and allocation-reentrant backtrace modes are
incompatible. Default backtrace/symbolicate/DHAT feature checks are separate.

## Evidence limits

The local ledger is `docs/rust-assurance.md` and the existing ignored assurance
evidence directory. Source/model tests, native System tests, true-null injection,
Windows/Linux lifecycle, real resource workloads, CI and final integration are
separate gates. In particular the Windows startup probe observed **zero**
pre-main operations and exited 101 for missing observation; this is not a
counter failure or a successful pre-main coverage claim. No unsafe constructor
or macro-generated unsafe is added to manufacture that evidence. Main remains
held until the applicable required gates pass.
