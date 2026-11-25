# Development

## Native prerequisites and smoke tests

Android ARM64, browser WASM (WebGPU or compatible fallback) and Windows are
planned targets; no blanket compatibility is promised.

Linux development requires the system SDL2 library and its pkg-config
development metadata. The native graphics adapter requests acceleration
without requiring vertical synchronization. A display-enabled smoke test
can be explicitly included with:

```sh
GAMES_NATIVE_RENDER_SMOKE=1 cargo validate --no-cache
```

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
reference test inputs. Logs are stored under
`.temp/validate/`.

Place lawfully obtained original files in `.reference/games/<id>/` using
the task ID in [TODO.md](../../TODO.md). Run `cargo download` to fetch upstream
reference sources under `.reference/tools/`. These directories are local.
See the [asset pipeline](asset-pipeline.md) for import and publication.

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
The title requires explicit local art bundles; see the
[title TODO](../../docs/todo/open/games/simpsons-arcade.mdc).
Keep the engine modular, dependency versions centralised and game code
limited to unique behaviour.
Runtime and title crates live under `crates/`; contributor tools live
under `tools/`. Titles use `crates/games/<id>/`. Keep all title acceptance
and evidence in the canonical task under docs/todo; only the root
TODO.md is a roadmap index.
Keep behavioural tests inline in their source modules. Use one TODO.md per
title once onboarded.

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
