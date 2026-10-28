# Beyond fork identity and status

Decision date: 2026-10-28.

The owner named this fork **Beyond** and explicitly stated that its code does not need to remain identical to upstream Horizon. The owner's requirements and custom Rust rules govern future implementation choices. This is not permission to bypass checks, grant unsafe exceptions, remove features, or claim unverified compliance.

## Completed identity changes

- GitHub repository renamed from `SlaVKsVolks/horizon` to `SlaVKsVolks/Beyond`.
- The repository retains stable ID `1434304408`, public visibility, `main` as default branch, history and its upstream fork relationship.
- Local `origin` now uses `https://github.com/SlaVKsVolks/Beyond.git`. Existing linked worktrees share this repository configuration.
- Maintained fork documentation identifies the project as Beyond and distinguishes inherited Horizon reference material from verified fork behavior.
- No release, deployment, GitHub Pages change or upstream submission was made. The repository had no active GitHub Pages site at the rename preflight.

## Deliberately not changed in this identity pass

- Rust source, Cargo package/crate names, executable names and lockfiles.
- Existing saved worlds, configuration, credentials, environment-variable compatibility and wire/persisted identifiers.
- Local checkout/worktree directory paths: moving the shared Git directory would risk breaking other active worktrees and chats.
- Upstream attribution, license, historical records and upstream references.
- Runtime updater, signing, telemetry and service endpoints: renaming a repository does not establish new Beyond services or trust keys.

The tracked rename documents and this chat's existing reviewed work are prepared for synchronization to the dedicated `repair/rust-integration-2026-10-27` branch, not for a merge into `main`; this avoids replacing newer independent work on `main` or triggering its release path. Actual push/readback must be confirmed separately. The GitHub repository identity rename itself applies to the whole repository. Check the actual branch before assuming these documents are on the default branch.

## Meaning of the pending redesign question

Some inherited **developer tools**, not ordinary gameplay, make detailed measurements with a custom allocator. No safe replacement preserving their exact counters has been established. Redesigning a tool could change what it measures; removing it would lose that diagnostic, so the tradeoff must be explicit. A different measurement must never be presented as the same one.

Some native source exists only for deferred Linux/macOS behavior, such as file-copy optimizations, thread-priority hints and macOS window styling. Windows does not execute those branches. Replacing or removing them could make it easier to enforce the unsafe ban across every first-party target, but could change non-Windows behavior.

The owner asked for this explanation. **No permission to remove those tools or platform paths has been inferred.** Windows gameplay and existing user data remain protected. Full compliance is still blocked as recorded in [Rust assurance](rust-assurance.md); the isolated console experiment is not a production fix.

## Next implementation boundary

A full internal Beyond rename needs a scoped migration plan and tests for Cargo/build-tool references, binaries/UI, compatibility identifiers, update/service identity and data lookup. A Rust-compliance redesign needs an approved contract stating which upstream diagnostics or deferred-platform behavior may change. Neither is accomplished by merely changing the repository name or by editing this document.

## Later explicit owner decisions: capabilities and platforms

These 2026-10-28 decisions supersede the earlier pending-permission statements above:

- macOS-specific first-party code may be removed. Deleting obsolete code is preferred to leaving commented-out unsafe implementations; upstream history and attribution remain preserved. This does not authorize modifying vendored/dependency source, changing remote CI, or deleting history.
- Linux support is deferred. Do not claim Linux support/validation, and do not weaken checks or hide Linux unsafe just to obtain a passing result. Retained unresolved Linux boundaries stay explicit until a safe implementation and appropriate evidence exist.
- Beyond must retain the original project's other capabilities, including memory-measuring tools. Their implementation may be redesigned or improved; deleting the tools, silently substituting RSS for requested bytes, or discarding peak/failure/reallocation accounting is not an acceptable compliance fix.
- Windows gameplay, existing data and compatibility remain protected. Exact upstream source identity is not required; observable capabilities and the owner's Rust rules govern the redesign.

The next bounded plan is [macOS-native removal and capability preservation](superpowers/plans/2026-10-28-beyond-platform-capabilities.md). Recording these decisions does not itself remove source, integrate the console prototype, complete the internal rename or establish compliance.
