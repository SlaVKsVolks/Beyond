# Rust assurance design

Base: `41af517023e96905a03e1e8999178dbce6a82a92`. This is a bounded improvement to Horizon's Rust assurance, not a claim of complete compliance.

## Intent and constraints

Improve mechanical enforcement and evidence without removing or changing features, weakening existing checks, adding first-party unsafe or moving it into generated code, macros, C, build scripts or dependencies. Preserve the edition, MSRV, supported targets and features, Cargo layout and Rust dependencies. The only authorized dependency addition is the Python development TOML backport `tomli==2.0.1; python_version < "3.11"`; no install is authorized. Routine isolated automated tests may use their own process priority, temporary probes, ephemeral loopbacks and non-serving availability binds. Do not install tools, run the game or installed GUI, deploy, change remote state or grant an unsafe exception. Existing first-party unsafe is a boundary to investigate, not permission to expand it. Safe-source checks do not prove dependency internals safe.

The root workspace declares edition 2024 and Rust 1.85; launcher uses edition 2021, while telemetry-server and mod-registry declare Rust 1.90. Each workspace and each applicable target must be assessed independently. Vendored `third_party/wgpu-hal` is not first-party source. Treat claimed target and lint coverage as provisional until mechanically verified.

## Approaches

1. Blanket forbid: would make existing targets containing unsafe fail, so it does not preserve the requested features.
2. Move calls behind new wrappers or dependencies: would evade the explicit constraint and is rejected.
3. Add safe, scoped mechanical enforcement where compatible; leave unresolved unsafe boundaries visible and fail closed for full compliance. This is the selected approach. A partial guard or an inventory is not the profile's required complete `forbid(unsafe_code)` gate.

## Boundaries and acceptance

Operating-system priority, signal handling, console, reflink and macOS appearance code has native unsafe operations. Allocator-counting examples also contain unsafe. A safe equivalent must preserve actual output, errors, resource ownership, lifetime and performance behavior before replacement. If none is established under the existing dependencies, preserve the code and mark the relevant gate BLOCKED. Do not delete tests or substitute a weaker observable.

The assurance ledger in [rust-assurance.md](../../rust-assurance.md) governs pre-edit invariants and acceptance. Each lane owns disjoint files in a separate worktree. The coordinator reconciles changes and evidence. An independent reviewer checks the ledger, full diff and evidence for safety, concurrency, lifecycle and public-contract changes. Source configuration, executed build and test results, CI execution, native runtime and user acceptance are separate evidence layers. A required gate that is NOT RUN, FAIL or BLOCKED means full compliance remains incomplete.

## Phase 5: approved deadline API

The user explicitly approved replacing the borrowed deadline guard with `pub fn deadline(stream: TcpStream, limit: Duration) -> io::Result<Deadline>`, where `Deadline: Read + Write` and `cancel(self) -> io::Result<TcpStream>`. This narrowly supersedes the prior no-public-API-change boundary; all unsafe, dependency, feature, MSRV, lint and behavior safeguards remain. The inbound hello, relay/code answer, exchange and encrypted confirmation must share one absolute deadline; the current `HANDSHAKE` is 10 seconds (`crates/horizon-friends/src/host.rs`).

The wrapper captures and restores original socket timeouts fallibly, preserves shorter existing timeouts and recomputes monotonic remaining time for every operation, including interrupts and partial I/O. A timeout maps to EOF on read only after actual deadline expiry; ordinary peer EOF remains EOF. Expired writes return `TimedOut`; zero expires immediately and `Instant::checked_add` overflow returns `InvalidInput`. Cancellation serializes Active→Released versus Expired, cancels and joins its worker once and never releases an expired stream. Split/release cannot expose the raw stream until both wrappers are consumed. No mutex is held across I/O and resources remain bounded. Existing non-deadline errors remain errors. Crypto cipher/counters move across the wrapper boundary without repeating confirmation or resetting a nonce after partial write.

Keep existing 100 ms, less-than-two-second, 300 ms, EOF and cancellation assertions, changing only the owned API usage. The acceptance ledger and Task 6/7 in the plan define regression and independent review requirements. Prior failed runs 24, 27 and 31 are real baseline evidence, not proof of fail-before for new tests. Windows automated proof does not establish other OS, CI, game or production acceptance.

### Timeout safety ruling — Task 6 review round 1

An actual `TimedOut` or `WouldBlock` from an owned blocking socket makes the wrapper Failed and terminal; do not retry or wait to conceal an early OS timeout. Before the absolute deadline preserve the original error, and require `cancel`/reunite to fail closed. At or after expiry only a deadline-limited read timeout may map to EOF; elapsed writes return `TimedOut`. This supersedes any assumption that a shorter OS socket timeout can leave a reusable stream. The [Windows socket-options documentation](https://learn.microsoft.com/en-us/windows/win32/winsock/sol-socket-socket-options) describes the connection as indeterminate after blocking send/receive timeout; a Rust write error reporting zero bytes is not proof of reusable socket state.

Test successful cancellation and timeout restoration separately from shorter-timeout failures, whose original error is preserved and whose release must fail. Keep the original 100 ms/<2 s/300 ms, EOF and cancel assertions unchanged. If an early read timeout prevents the original EOF assertion, replan an owned nonblocking controlled logical deadline; never weaken, ignore or retry the assertion. Recheck the clock after joining/restoring before release; split halves cannot cancel or re-split and must both be consumed before any raw escape. The API requires exclusive blocking socket ownership: callers retain no clones and do not mutate shared socket options. Remove the unused `Duration` warning. These are review requirements, not verified behavior.
