# games

Modular Rust game engine with fixed-step simulation, deterministic replay,
action-mapped input, asset conversion and native/WASM targets.

**Preserve the gameplay, not the technical debt.**

A non-commercial hobby project focused on portable games, faithful behaviour,
modern performance and playing together. It is not affiliated with the original
developers or publishers.

## Principles

- Reuse first, extend second, implement once. One canonical implementation per
  shared capability; compose configuration and specialised strategies rather
  than duplicate engines.
- Preserve gameplay while improving frame pacing, resolution, controls,
  portability and maintainability. Separate simulation from rendering.
- GPU-accelerated rendering where appropriate, efficient CPU computation and
  adaptive RAM/VRAM budgets. Support NVIDIA, AMD and Intel through portable
  graphics APIs. Optional advanced graphics features only where beneficial; no
  mandatory vendor-specific hardware.
- Offline single-player, local saves, native controller support, configurable
  touch controls and self-hosted multiplayer where applicable. No
  advertisements, subscriptions or in-app purchases in our implementations.
- Behavioural tests protect observable gameplay, not incidental implementation
  details. Architecture and security contracts protect shared boundaries. Treat
  imported files, mods and network data as untrusted.
- User-supplied original media, one preferred source edition per title,
  automatic asset import. No original game binaries, extracted assets or leaked
  source are distributed. Original game code is not a runtime dependency.

## Platforms

Linux x86-64 is the primary development and testing target. Android ARM64 and
browser (WASM, WebGPU or compatible fallback) are intended targets. Other 64-bit
operating systems and architectures are welcome through community contributions.
No blanket compatibility promise.
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

## Development

The project is in bootstrap. No game is currently playable. The catalogue is a
planned scope, not a release announcement.

See [TODO.md](TODO.md) for the delivery order and typed task contracts.
See [third-party notices](THIRD_PARTY_NOTICES.md) for licensing information.

```sh
cargo validate
```

The validator orchestrates formatting, Clippy, MarkdownLint, task contracts,
tests and compilation checks. Validation uses a content-based local cache;
use `cargo validate --no-cache` for a fresh run or after changing local
reference test inputs. Logs are stored under
`.temp/validate/`.

Place lawfully obtained original files in `.reference/games/<id>/` using
the task ID in TODO.md. Run `cargo download` to fetch upstream
reference sources under `.reference/tools/`. These directories are local.
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
The title requires explicit local art bundles (see its TODO). Keep the
engine modular, dependency
versions centralised and game code limited to unique behaviour.
All Rust code and development tools live under `crates/`.
Keep behavioural tests inline in their source modules. Use one TODO.md per
title once onboarded.

## Shared bootstrap contracts

The Rust library now has fixed-step scheduling and scripted-input replay,
logical keyboard/controller/touch routing, viewport and localisation policies,
and display frame-pacing configuration. These modules have synthetic behaviour
tests and do not require an installed original game.

The library also provides local-first multiplayer session rules, byte-bounded
message/schema validation, archive-entry checks, integrity-checked save
envelopes and constant-memory performance accounting. The native platform
adapter provides bounded mutual TLS 1.3 and explicit player enrolment.
On Linux, descriptor-relative archive staging enforces private files,
no-overwrite creation, exact payload lengths and collision rejection. Physical
LAN verification and credential provisioning remain pending. Mod sandboxing and
playable titles remain open. The Linux graphics adapter presents bounded
RGBA frames and ordered sprite scenes using persistent accelerated textures,
nearest-neighbor scaling and aspect-preserving letterboxing.
The shared asset crate defines deterministic versioned manifests, SHA-256
content identities and checked RGBA8, PCM16 and indexed-tile descriptors.
The importer validates a pinned immutable ZIP source and checks each flat
archive member before conversion. The conversion interface assembles a
canonical manifest with deduplicated, validated payload staging requests.
The Linux platform stages content by SHA-256 and completes a private bundle
only when its exact payload set matches the manifest. GNU/Linux supports
atomic publication without replacement and native verified read-back.

The host build tool selects arcade originals by MAME machine and revision.
The importer owns trusted source identities, word-lane assembly, reusable
chip layouts and canonical graphics conversion. Unknown profiles fail closed.

```sh
cargo run --locked -p games-build -- mame import simpsons world-4p-set-1 \
  .reference/games/simpsons-arcade/simpsons.zip 3000000 1048576 7209088 14
```

Use `mame verify` for read-only archive admission. On GNU/Linux, use
`mame publish MACHINE REV PARENT PATH SOURCE ENTRY TOTAL COUNT` or
`mame check MACHINE REV PARENT MANIFEST_SHA256`. The four numeric limits
bound source bytes, individual decoded members, total bytes and member count.
Create a trusted, private output parent inside the ignored reference directory
before publication. `zip verify PATH TITLE REV SHA256 SOURCE ENTRY TOTAL COUNT`
remains available for explicit generic flat ZIP admission. The selected
Simpsons bundle is verified locally. Its observed palette RAM and mixer
assignments match independent reference values; bounded tilemap admission
and exact PCM reference comparison also pass. The optional GNU/Linux
`native-preview` feature provides `mame render-check` for selected imported
tiles and `mame render-bench` for measured host submission intervals.
These diagnostics establish asset access, not gameplay or screen parity.
See the title and performance TODO records for reproducible commands,
observations and limitations. Other MAME profiles and other platform
publication remain open.

## Catalogue

Preferred source editions are planning choices, not guarantees of support. Exact
revisions will be selected when onboarding each title.

| # | Title | Original edition |
| --- | --- | --- |
| 01 | The Simpsons Arcade Game | Arcade (4P) |
| 02 | Metal Slug | Neo Geo |
| 03 | Metal Slug 2 | Neo Geo |
| 04 | Metal Slug X | Neo Geo |
| 05 | Metal Slug 3 | Neo Geo |
| 06 | Metal Slug 4 | Neo Geo |
| 07 | Metal Slug 5 | Neo Geo |
| 08 | Metal Slug 6 | Atomiswave |
| 09 | Metal Slug 7 | Nintendo DS |
| 10 | Metal Slug XX | Steam PC |
| 11 | Angry Birds Space | Android |
| 12 | Angry Birds Star Wars (2012) | Android |
| 13 | Angry Birds Star Wars II | Android |
| 14 | SUSH | Android |
| 15 | Plants vs. Zombies: Replanted | PC Replanted |
| 16 | Bugs Bunny: Lost in Time | Windows PC |
| 17 | Bugs Bunny & Taz: Time Busters | Windows PC |
| 18 | The Simpsons: Road Rage | Xbox |
| 19 | Taz: Wanted | Windows PC |
| 20 | SpongeBob: Revenge of the Flying Dutchman | GameCube |
| 21 | The Simpsons: Hit & Run | Windows PC |
| 22 | Crash Twinsanity | Xbox |
| 23 | Sly Cooper and the Thievius Raccoonus | PS3 HD |
| 24 | Sly 2: Band of Thieves | PS3 HD |
| 25 | Sly 3: Honor Among Thieves | PS3 HD |
| 26 | The Simpsons Game | PS3 |
| 27 | El Chavo Kart | Xbox 360 |
| 28 | Sly Cooper: Thieves in Time | PS3 |
| 29 | Crash Team Racing Nitro-Fueled | PS4 |
| 30 | Plants vs. Zombies 2 | Android |
| 31 | Plants vs. Zombies 3: Evolved | Android Evolved |
| 32 | SimCity BuildIt | Android |
| 33 | Hytale | PC |
| 34 | Grand Theft Auto VI | PS5 |

## Contributions

Pull requests are welcome. Use Conventional Commits and sign off each commit
under the Developer Certificate of Origin (DCO) with `git commit -s`. Run the
validator. Informal language and blunt criticism are fine; harassment, malicious
changes and unauthorised proprietary material are not. Additional platforms are
community-driven.

Keep documentation lean: README, title TODOs and code. Respect third-party
licence and attribution requirements.

## Licence

Apache-2.0 for original project code. Third-party dependencies and original
games remain subject to their respective rights and licences. Supply only
original media you are entitled to use. No proprietary assets or original game
executables are included.
