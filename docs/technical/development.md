# Development

## Native prerequisites and smoke tests

Android ARM64, browser WASM (WebGPU or compatible fallback) and Windows are
planned targets; no blanket compatibility is promised.

Linux development requires the system SDL2 library and its pkg-config
development metadata. The native graphics adapter requests acceleration
without requiring vertical synchronization. A display-enabled smoke test
can be explicitly included with the following command. The gate briefly maps
and resizes a small test window, reports its actual drawable dimensions,
and checks all presentation modes and protected touch projection:

```sh
GAMES_NATIVE_RENDER_SMOKE=1 cargo validate --no-cache
```

On a Wayland display configured above 1x, SDL2 compatibility may report
window dimensions in drawable pixels. To exercise distinct window and
drawable dimensions on a compatible SDL backend, use:

```sh
SDL_VIDEODRIVER=wayland SDL_VIDEO_WAYLAND_SCALE_TO_DISPLAY=0 \
  SDL_VIDEO_HIGHDPI_DISABLED=0 GAMES_NATIVE_RENDER_SMOKE=1 \
  cargo validate --no-cache
```

Check the reported window and drawable dimensions; setting these options
alone does not establish high-DPI support or physical input acceptance.

The native audio adapter uses a separate, bounded signed-PCM playback queue.
Its paused-device smoke test can run without audible output using SDL2's
dummy audio driver:

```sh
SDL_AUDIODRIVER=dummy GAMES_NATIVE_AUDIO_SMOKE=1 cargo validate --no-cache
```

## Validation and builds

See [TODO.md](../../TODO.md) for the delivery order and typed task contracts.
See [third-party notices](../../THIRD_PARTY_NOTICES.md) for licensing
information.

```sh
cargo validate
```

The validator orchestrates formatting, Clippy, MarkdownLint, task contracts,
tests and compilation checks. Validation uses a content-based local cache;
use `cargo validate --no-cache` for a fresh run or after changing local
reference test inputs. Every fresh run checks all workspace libraries on
Linux and Windows ARM64/x86-64, Android ARM64 and WebAssembly. Install
the corresponding Rust targets; missing targets are errors. Runtime
dependency graphs must exclude source decoders. The installed target
inventory and Node.js version contribute to the cache fingerprint. A fresh
run also links and executes the Simpsons Arcade portable simulation smoke
with the official WebAssembly Rust target and the installed Node.js runtime.
This requires Node.js on the contributor host, but does not require a browser
or an original asset bundle and is not a browser packaging check. Logs are
stored under
`.temp/validate/`.

Place lawfully obtained original files in `.reference/games/<id>/` using
the task ID in [TODO.md](../../TODO.md). Run `cargo download` to fetch upstream
reference sources under `.reference/tools/`. These directories are local.
See the [asset pipeline](asset-pipeline.md) for import and publication.

Local reference data survives cleanup: `.reference/games/` contains
original media, `.reference/tools/` contains downloaded tools and sources,
and `.reference/ledger/` contains durable local reference ledgers.
`.cache/` and `.temp/` are regenerable scratch space, never the only
copy of durable records. Do not remove reference storage during cleanup.

Build the available GNU/Linux x86-64 title preview with:

```sh
cargo run --locked -p games-build -- build simpsons-arcade
```

The optional target is `linux-x86-64`; other targets and unknown titles are
rejected. This produces a release executable without original assets.
Create a local prototype package in a new directory with:

```sh
cargo run --locked -p games-build -- package simpsons-arcade .temp/local-game
```

The package includes the executable, project notices, selected Cargo
dependency licence texts and an executable identity receipt. It requires
compatible system libraries, excludes all game assets and is not yet a
portable release. Supply a trusted output parent that already exists.
The versioned receipt records the length and SHA-256 of every included
file. Publication performs an independent bounded readback; repeat it
without building or executing a package using:

```sh
cargo run --locked -p games-build -- package-check simpsons-arcade \
  .temp/local-game
```

The checker requires a pristine package tree and rejects earlier receipt
formats, missing or added files (including private launch catalogues),
links, incorrect owner-only file modes and changed content. It reports
a receipt SHA-256 for independent comparison. This checks consistency
with the receipt, not provenance,
licence sufficiency, concurrent filesystem isolation or runtime ABI.
The title requires explicit local art bundles; see the
[title TODO](../../docs/todo/open/games/simpsons-arcade.mdc).
For persistent native packaging, set the owner-only local `.env` from
`.env.example`. Use absolute asset and output roots and
`GAMES_TARGET=linux-x86-64`. Create the trusted private
`<output-root>/simpsons-arcade/bin/` parent, then omit the explicit
output argument:

```sh
cargo run --locked -p games-build -- package simpsons-arcade
```

The resulting package is at
`<output-root>/simpsons-arcade/bin/linux-x86-64/`. Asset and executable
roots may be equal, but the resolved asset and package locations must
not overlap. Packaging does not import, search, copy or modify assets.
An existing package is rejected rather than overwritten. Supply a local
launch catalogue pointing at independently verified assets; see the
[asset pipeline](asset-pipeline.md) for shared content storage and
migration limitations.

Keep the engine modular, dependency versions centralised and game code
limited to unique behaviour.
Runtime and title crates live under `crates/`; contributor tools live
under `tools/`. Titles use `crates/games/<id>/`. Keep all title acceptance
and evidence in the canonical task under docs/todo; only the root
TODO.md is a roadmap index.
Keep behavioural tests inline in their source modules. Use the canonical task
record for each title; do not create a separate TODO.md.

## Contributions

Informal language and blunt criticism are fine; harassment, malicious changes
and unauthorised proprietary material are not. Additional platforms are
community-driven. Keep documentation lean, with technical reference material
in this directory and title-specific evidence in the existing task records.

## Locked dependency references

Run `cargo download dependencies` to fetch and materialise the exact locked
external dependency sources under `.reference/tools/dependencies/sources/`.
Cargo verifies and extracts its registry packages. The downloader does
not modify Cargo source configuration or add vendored sources to Git.
The default download command also includes these dependency references.
Direct dependency declarations and lint policy remain in the root
workspace manifest; member manifests inherit them.

## Feature isolation

Secure LAN is an explicit platform capability, enabled by the title's
runtime preview but absent from the asset build deliverable. The validation
gate checks the actual build dependency graph, the default workspace
Clippy configuration and all targets of every workspace member with all
its features enabled independently. Aggregate all-feature workspace
compilation and tests remain mandatory. This avoids linking unrelated
runtime TLS dependencies into import tools merely through workspace-wide
Cargo feature unification, without disabling any compiler or Clippy lint.
