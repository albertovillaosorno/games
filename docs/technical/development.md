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
[title TODO](../../crates/game-simpsons-arcade/TODO.md).
Keep the engine modular, dependency versions centralised and game code
limited to unique behaviour.
All Rust code and development tools live under `crates/`.
Keep behavioural tests inline in their source modules. Use one TODO.md per
title once onboarded.

## Contributions

Informal language and blunt criticism are fine; harassment, malicious changes
and unauthorised proprietary material are not. Additional platforms are
community-driven. Keep documentation lean, with technical reference material
in this directory and title-specific evidence in the existing task records.
