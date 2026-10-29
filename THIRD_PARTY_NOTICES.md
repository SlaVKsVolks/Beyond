# Third-party notices

## mod-alloc (maintained Beyond fork)

`third_party/mod-alloc` derives from James Gober's `mod-alloc` 1.0.1 under
Apache-2.0 (`third_party/mod-alloc/LICENSE`). It is not untouched vendor code:
Beyond owns the new safe exact bookkeeping and integration. All upstream unsafe
adapter bodies remain frozen; inactive unsafe modules are also integrity-bound.
See `third_party/mod-alloc/FORK.md` and `FORK.json` for archive provenance, changed
surfaces, supported feature modes, test-only failure injection and proof limits.
The existing upstream README and original manifest are historical attribution,
not a claim that the fork still has upstream's compiler floor or exact API.

## wgpu-hal

`third_party/wgpu-hal` is [wgpu-hal](https://github.com/gfx-rs/wgpu) 30.0.1 by the gfx-rs
developers, with one upstream fix applied
([gfx-rs/wgpu#10124](https://github.com/gfx-rs/wgpu/pull/10124)). It is licensed under MIT or
Apache-2.0; both licenses are in that folder (`LICENSE.MIT`, `LICENSE.APACHE`).

## Pixelify Sans

`crates/horizon-theme/fonts/PixelifySans-*.ttf` is [Pixelify Sans](https://github.com/eifetx/Pixelify-Sans)
by The Pixelify Sans Project Authors, licensed under the SIL Open Font License 1.1
(`crates/horizon-theme/fonts/OFL-PixelifySans.txt`).

## Pixelarticons

`crates/horizon-theme/icons/pixel/*.svg` are icons from
[Pixelarticons](https://github.com/halfmage/pixelarticons) by Gerrit Halfmann, under the MIT
license (`crates/horizon-theme/icons/pixel/LICENSE-pixelarticons.txt`).
