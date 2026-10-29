# mod-alloc

Allocation profiling for Rust. Tracks allocation counts, total
bytes, peak resident memory, and current resident memory by
wrapping the system allocator via [`GlobalAlloc`].

Originally designed upstream as a lean replacement for `dhat` with MSRV 1.75.
This maintained Beyond fork requires Rust 1.99.1; the historical upstream
compiler-floor claim does not apply to this fork. There are zero external
dependencies on the production counter hot path.

## Installing as the global allocator

```no_run
use mod_alloc::{ModAlloc, Profiler};

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

fn main() {
    let p = Profiler::start();

    let v: Vec<u64> = (0..1000).collect();
    drop(v);

    let stats = p.stop();
    println!("Allocations: {}", stats.alloc_count);
    println!("Total bytes: {}", stats.total_bytes);
    println!("Peak bytes (absolute): {}", stats.peak_bytes);
}
```

## Counter semantics

The four Tier 1 counters track allocator activity since the
installed [`ModAlloc`] began counting (or since the last
[`ModAlloc::reset`] call):

| Counter         | Updated on `alloc`            | Updated on `dealloc` |
|-----------------|-------------------------------|----------------------|
| `alloc_count`   | `+= 1`                        | (unchanged)          |
| `total_bytes`   | `+= size`                     | (unchanged)          |
| `current_bytes` | `+= size`                     | `-= size`            |
| `peak_bytes`    | high-water mark of `current`  | (unchanged)          |

`realloc` is counted as one allocation event. `total_bytes`
increases by the growth delta on a growing realloc and is
unchanged on a shrinking realloc.

## Status

**v1.0.0 — Stable API.** The public surface is frozen.
Breaking changes after `1.0.0` require a major version bump
per Semantic Versioning.

Path to `1.0.0`:

- `v0.9.0`: Real `GlobalAlloc` impl + Tier 1 counters.
- `v0.9.1`: Tier 2 inline backtrace capture (`backtraces`).
- `v0.9.2`: Symbolication for reports (`symbolicate`).
- `v0.9.3`: DHAT-compatible JSON output (`dhat-compat`).
- `v0.9.4`: `dhat_compat` drop-in surface for `dhat-rs`.
- `v0.9.5`: Tier 2 per-allocation overhead cut ~32x by
  removing the per-thread arena layer.
- `v1.0.0`: `#[non_exhaustive]` on the data structs likely
  to gain fields; public surface frozen.

Default builds ship Tier 1 counters only.

## Backtraces (`backtraces` feature)

With `mod-alloc = { version = "0.9", features = ["backtraces"] }`
and `RUSTFLAGS="-C force-frame-pointers=yes"`, each tracked
allocation captures up to 8 frames of its call site via inline
frame-pointer walking on `x86_64` and `aarch64`. Per-call-site
aggregation is exposed via `ModAlloc::call_sites` (available
only with the `backtraces` feature); the result is raw return
addresses. Symbolication ships in v0.9.2.

Aggregation-table size is controlled by the `MOD_ALLOC_BUCKETS`
environment variable at process start (default 4,096 buckets,
~384 KB).
