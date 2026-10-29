# Beyond thread-priority boundary

Published baseline: `thread-priority` **3.1.1**, MIT, upstream VCS
`8a464d44416e0e9bdf6b2f173befc2babb031674`. Original `.crate` SHA-256:
`8d2e834949be5111506bb252643498af1514f600d9e1dceedaa42afae155b67f`.
The published license, attribution, source, tests and upstream manifest are retained.

The owner authorized this narrowly scoped maintained fork on 2026-10-29. It is
not a general unsafe exemption or a claim that dependency internals are safe.
All original unsafe bodies must remain byte-for-byte unchanged, including
inactive platform source and legacy tests. `tools/thread_priority_integrity.py`
checks the whole published envelope and a review-bound `FORK.json` inventory.
The only permitted Unix source delta is one exact `#[forbid(unsafe_code)]` block:
after successful original `pthread_setschedparam` and its error check, Linux and
Android Idle may return without calling `setpriority` on the caller. The entire
original Unix file must reconstruct to its published hash after removing that
one block. No other scheduling policy, syscall, error check or Windows source
may change. The appended standalone `[workspace]` marker is the only manifest
delta; version, MSRV, features, dependencies and legacy targets remain published.

Idle's static scheduling priority is **zero**, distinct from Linux niceness.
The safe first-party consumer validates the full nice range `[-20, 19]`, supplies
`ThreadPriority::Min`, and verifies actual Idle policy and unchanged niceness.
False remains a best-effort result, not a guarantee of atomic rollback.
First-party consumer/test code forbids unsafe mechanically. The fork's original
unsafe is frozen, not rewritten, moved or expanded.

Maintenance: any upstream update or boundary change needs fresh archive/license
review, full byte comparison, mechanical unsafe enforcement on new code,
independent diff review and native regression checks. Do not merely regenerate
the registry to accept drift. Native tests cover actual positive nice 0/10/19,
static priority zero, parent isolation, helper fallback and invalid inputs.
Native negative niceness is not qualified unless legitimately inherited; no
privilege escalation or synthetic native pass is permitted. Existing Windows
checks and mandatory hosted verification remain required. Linux product support
is deferred; this is not permission to disable its existing checks.

Development and passing source checks do not authorize main promotion. Every
required gate, including resource-backed parity and applicable game acceptance,
must pass first; unavailable Minecraft 26.3 inputs remain an explicit blocker.
