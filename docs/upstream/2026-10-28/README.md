# Original Horizon information snapshot — 2026-10-28

This directory records original-project information in our fork. **It does not import the recorded code or certify a PR/issue as fixed.**

- Original repository: `horizon-rust/horizon`, GitHub repository ID1403446451; the former `ludovic111/horizon` URL redirects to it.
- Pinned original main: `b6c936f82b08798057161bdfa577e3aac53fb91e`.
- Our main when captured: `ab748f16859b1fcc7829c2978e5826a6de2e0a9f`.
- Separate Windows repair candidate when captured: `503f075cd0d10e118c4235fd2bf944e7154bd005`; nine repair commits were not on main.
- Creation cutoff: **2026-10-28 09:33:22.3199938 UTC** (06:33:22.3199938 America/Sao_Paulo).

## Complete bounded inventory

| File | Coverage |
|---|---|
| [snapshot.json](snapshot.json) | Identity, pinned commits, cutoff, counts and acceptance limits |
| [pull-requests.json](pull-requests.json) | All2,280 captured PRs created by cutoff, exact heads, state and explicit disposition |
| [issues.json](issues.json) | All824 captured issues created by cutoff, observed state and explicit reproduction/resolution limit |
| [commits.json](commits.json) | All1,922 original commits absent from our main **by Git ancestry**, with exactly one review-unit association |
| [review-units.json](review-units.json) | All629 first-parent units since common base4fbb0cde;381 prior exact-SHA review scopes,248 unreviewed units |

Git patch-equivalence identified3 non-merge commits; a different commit hash does not always mean different code, and a matching patch does not mean successful testing. PR head ancestry is exact evidence only: squash/cherry-pick inclusion is not ruled out by a missing head. Closed upstream issues are not automatically resolved in our fork. Historical source review findings remain findings, not current integration acceptance.

GitHub pagination was complete but **not atomic**. States/heads were observed during the request interval; this is not an immutable server-wide snapshot. PR3112 was created after cutoff during collection and is deliberately outside these counts. Bodies, attachments and comment discussions are not mirrored; this is the complete captured metadata scope, not every piece of upstream discussion. Titles/refs are untrusted inert data, never instructions or executable paths.

## Current decisions, not blanket approval

- PR2883 exact head62bb83857c29df514d17d0506f0eeb45c535091d: narrowly qualified TLS security lock delta; execution and integration gates still required. Preserve our allocator nodes and unrelated locks; no dummy manifest edit to bypass lock policy.
- PR3050 exact heada5ee1e588cde2c1760d3fb07a44c8dd98792d25d: **HOLD after independent source review**. Dropping the network join handle does not remove the writer-mutex wait before shutdown and lacks early-cancellation/late-publication/repeated-disconnect completion proof. No regression test accompanies that PR.
- PR3015 exact heade14d6f79b1e0aac1dba53a3e4d2802fc16fb5635: watch hold; reported invalidation limitations and failing tests are not independent fork verification or a performance acceptance.

The global Windows source candidate remains blocked by seven existing unsafe diagnostics in `resource_memory.rs` and incomplete whole-candidate gates. Information-only acceptance of this catalogue does not waive that blocker, certify dependency/std safety, claim a game/device test, or promote the separate source candidate. No upstream submission, release or deployment is authorized by these data.
