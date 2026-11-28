# Development

## Native prerequisites and smoke tests

Linux x86-64 and a WebGL 2 browser preview are available. Android ARM64
has local debug packaging; Windows remains planned. No blanket
compatibility is promised.

Linux development requires the system SDL2 library and its pkg-config
development metadata. The native graphics adapter requests acceleration
without requiring vertical synchronization. A display-enabled smoke test
can be explicitly included with the following command. The gate briefly maps
and resizes a small test window, reports its actual drawable dimensions,
and checks all presentation modes, protected touch projection and the
Simpsons Arcade language selector using procedural assets:

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
Its smoke checks paused admission and resumed silent queue consumption.
It can run without audible output using SDL2's
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
It also links the separate browser storage smoke with its opt-in capability
feature. The official CLI reference is available via
`cargo download wasm-bindgen-release`; its version is pinned to the locked
binding ABI. Verify the upstream checksum before extracting the CLI. The
storage smoke requires generated browser bindings and is not executed by
the Node.js core gate.
This requires Node.js on the contributor host, but does not require a browser
or an original asset bundle and is not a browser packaging check. Logs are
stored under
`.temp/validate/`.

Place lawfully obtained original files in `.reference/games/<id>/` using
the task ID in [TODO.md](../../TODO.md). Run `cargo download` to fetch upstream
reference sources under `.reference/tools/`. These directories are local.
See the [asset pipeline](asset-pipeline.md) for import and publication.

Local reference data survives cleanup: `.reference/games/` contains
original media and the single per-title `ledger.jsonl` function inventory.
`.reference/tools/` contains downloaded reference sources; older durable
reference records also remain protected.
`.cache/` and `.temp/` are regenerable scratch space, never the only
copy of durable records. Do not remove reference storage during cleanup.

The mandatory per-game analysis and acceptance lifecycle is defined in
[AGENTS.md](../../AGENTS.md). The shared tooling acceptance record is
[reference-ledger](../todo/completed/build/reference-ledger.mdc).
Keep game-specific semantic, implementation, manual regression and
final owner acceptance evidence in the existing title task.

## Reference dependency work queue

The ledger checker accepts an optional positive group-output limit:

```sh
cargo run --locked -p games-build -- ledger check PATH BYTES ROWS 10
```

The checker reports exactly three dispositions: pending, implemented and
discarded. Implementation score is 1 only for implemented functions;
pending and discarded score 0. Pending or discarded rows cannot contain
a final Rust mapping. Retain intermediate evidence in research notes.
A listing review does not certify semantics; implemented and discarded
rows require explicit semantic completion, reviewed dependencies,
evidence and zero unresolved flow observations.

For final ledger acceptance use:

```sh
cargo run --locked -p games-build -- ledger check PATH BYTES ROWS --complete
```

The command fails if any function is pending, semantics or dependencies
are incomplete, or any control flow remains unresolved. Discards require
evidence and remain zero in the implementation score.

After reciprocal graph validation, the optional group limit reports
strongly connected pending components, external dependencies, callers
and unresolved flow counts. It prioritises fewer pending dependencies
and unknown flows, then larger groups and higher fan-in, with stable
identity tie-breaking. Close related behaviour as a batch through
existing shared Rust mechanisms and regression tests. Graph-ready means
only that no known pending external callee remains; semantic gaps may
still block implementation. The report is read-only.
Pending dependencies and callers are conservatively projected through
discarded functions, so discarding a transport stub does not hide its
pending destination. Implemented functions terminate that projection;
discarded functions never become pending group members.

## Bounded private research retention

Research retention never traverses original media, tool installations,
analysis projects, canonical assets or historical unclassified captures.
Only direct run directories under
`.reference/investigations/archived-disposable/` are eligible. A completed
run must contain exactly two regular, single-link files: an opaque
`captures.tar.zst` package and `.games-disposable-v1`. Do not package
original media or unique evidence. The owner must review and classify the
run, record retained evidence and reproduction inputs in the canonical
private ledger, and stop all writers before sealing it. Unsealed runs are
protected; unexpected entries in a sealed run reject the preview.

Before sealing, append a record to the title's private ledger. Store
the following JSON object on a single line, replacing the example values
with the exact run directory name, retained evidence and reproduction
inputs. The read-only ledger checker validates this record and rejects
duplicate classified run names.

```json
{
  "kind": "archived-investigation",
  "run": "reviewed-run",
  "status": "completed-archived-regenerable",
  "evidence": "retained findings and supporting references",
  "reproduce": "REPRODUCTION_INPUTS_AND_COMMAND"
}
```

The UTF-8 seal contains these five lines (replace the placeholders):

```text
games-disposable-v1
status=completed-archived-regenerable
evidence=title/ledger.jsonl
sha256=RETAINED_RECORD_SHA256
reproduce=REPRODUCTION_INPUTS_AND_COMMAND
```

The digest identifies one exact line in
`.reference/games/<title>/ledger.jsonl`, excluding its final LF. The guard
requires a classified archived-investigation record, not an arbitrary
line with a matching hash. Its run name and reproduction note must
match the directory and seal, and the retained evidence must be nonempty.
Later ledger appends do not change an unchanged retained record.
Conflicting records naming the same run, malformed JSON rows and ambiguous
classification fields are rejected at preview and apply, including records
with distinct hashes. The seal and record are owner
attestations: the guard does not inspect the opaque archive or certify
reproducibility. No historical capture is automatically moved or classified.

Preview using minimum age in seconds, logical byte quota, observed entry
bound and aggregate logical scan-byte bound, respectively:

```sh
cargo run --locked -p games-build -- retention plan \
  604800 10737418240 10000 107374182400
```

The preview reports selected names, logical bytes, retained record hashes,
protected unsealed runs and a plan hash. Age includes the newest metadata
change in the sealed run. The oldest eligible runs are selected only
while above quota; fresh archives can leave the quota unmet. No physical
disk-block savings are promised. Apply only after reviewing the preview:

```sh
cargo run --locked -p games-build -- retention apply \
  604800 10737418240 10000 107374182400 REVIEWED_PLAN_HASH
```

The Linux adapter pins owner-controlled directory descriptors, rejects
links, validates retained record hashes, serializes cooperating invocations
and re-observes the plan before removing only its named files and empty run
directories. Do not manually mutate sealed runs while applying. An
interruption may leave a partially removed run; inspect and reclassify it
before retrying. Unsupported hosts return an explicit failure. An absent
archive root yields an empty read-only preview.

## Native title preview

Build the available GNU/Linux x86-64 title preview with:

```sh
cargo run --locked -p games-build -- build simpsons-arcade
```

The optional native target is `linux-x86-64`. The browser target is
`webassembly`; Android uses the package command instead. The build
produces an authored release executable or an unbound Wasm module.
Create a local prototype package in a new directory with:

```sh
cargo run --locked -p games-build -- package simpsons-arcade .temp/local-game
```

The package includes the executable, project notices, selected Cargo
dependency licence texts and an executable identity receipt. It requires
compatible system libraries, excludes all game assets and is not yet a
portable release. Supply a trusted output parent that already exists.
When local `.env` configuration exists, explicit package output must
not overlap the resolved asset directory. Invalid or untrusted local
configuration prevents publication. Explicit scratch packaging without
local configuration remains available. The path check is a preflight,
not isolation from concurrent filesystem changes.
The versioned receipt records the length and SHA-256 of every included
file. Publication performs an independent bounded readback; repeat it
without building or executing a package using:

```sh
cargo run --locked -p games-build -- package-check simpsons-arcade \
  .temp/local-game
```

The checker requires a pristine package tree and rejects earlier receipt
formats, missing or added files (including private launch catalogues),
links, directories not controlled by the current owner, incorrect
owner-only file modes and changed content. It reports a receipt SHA-256
for independent comparison. This checks consistency
with the receipt, not provenance,
licence sufficiency, concurrent filesystem isolation or runtime ABI.
The title requires explicit local art bundles; see the
[title TODO](../../docs/todo/open/games/simpsons-arcade.mdc).
On a GNU/Linux x86-64 contributor host, download the pinned binding CLI
with `cargo download wasm-bindgen-release`, verify the archive checksum,
and extract the official release into its downloaded version directory
under `.reference/tools/wasm-bindgen-release/0.2.127/`. Packaging checks
the executable identity before staging and invoking it. Build and check
an asset-free browser package with:

```sh
cargo run --locked -p games-build -- package simpsons-arcade \
  .temp/local-web webassembly
cargo run --locked -p games-build -- package-check simpsons-arcade \
  .temp/local-web webassembly
```

Serve the package directory through a trusted local HTTP server with
JavaScript and Wasm MIME types; open `index.html` in a compatible browser.
Select the local canonical assets directory containing `launch.local`.
Firefox on GNU/Linux has passed this workflow. On the tested Epiphany
51.2/WebKitGTK 2.54.2 host, the GTK portal offered only file selection
and supplied no canonical relative paths; use the verified browser
workflow instead. Directory selection on that host remains unsupported.
After normal exit or asynchronous launch failure, the packaged status
region reports the terminal message; reload the page to launch again.
The catalogue must use `parent=.`, windowed presentation, no native
settings/control/device paths, and `launch-poses.txt` if poses are used.
No original assets are included in the package or automatically uploaded.
Display mode, selected language, master volume, admitted effects and music
levels, and control edits use a fixed origin-local preference namespace.
Control edits take
effect on the next launch; fullscreen remains unavailable. If storage is
unavailable at startup, display, volume and language remain session-only
and control editing is not offered. Incompatible stored values are rejected
rather than replaced; an invalid locale preference is left untouched, with
a session-only language choice. A well-formed but unsupported locale
falls back to the title default without changing the saved tag until
the player explicitly selects a language.
Browser control profiles reject keyboard names the adapter cannot translate.
They also reject controller buttons outside the standard Gamepad mapping;
extended native paddles, miscellaneous and touchpad buttons remain native-only.
International backslash and keypad comma have portable profile identities
(`IntlBackslash` and `Keypad ,`).
Native SDL may report keypad comma as keypad decimal; the observed X11
and Wayland backends cannot distinguish them. Preserve the distinct
identities and verify the actual device before relying on that binding.
Successful browser writes do not guarantee crash durability. Preferences
are isolated by browser origin and profile, and contain no game assets.
Physical device acceptance and broader browser compatibility are not
established.

For persistent packaging, set the owner-only local `.env` from
`.env.example`. Use absolute asset and output roots and
`GAMES_TARGET=linux-x86-64` or `GAMES_TARGET=webassembly`. Create the trusted
private
`<output-root>/simpsons-arcade/bin/` parent, then omit the explicit
output argument:

```sh
cargo run --locked -p games-build -- package simpsons-arcade
```

The resulting package is at
`<output-root>/simpsons-arcade/bin/<target>/`. Asset and executable
roots may be equal, but the resolved asset and package locations must
not overlap. Packaging does not import, search, copy or modify assets.
An existing package is rejected rather than overwritten. Supply a local
launch catalogue pointing at independently verified assets; see the
[asset pipeline](asset-pipeline.md) for shared content storage and
migration limitations.

For persistent display, language, volume and control profiles, create an
existing
owner-only settings directory outside the package and asset store. Set
`settings=DIR` in the private local launch catalogue, or launch with
`--settings DIR`. Relative catalogue paths are anchored to that file.
Native gameplay rejects settings directories that overlap the selected
asset directory. All native preference entry points, including LAN
controller launch and direct control editing, reject settings directories
that overlap the running executable's directory. Both checks resolve paths
before comparison, including links. These are preflights, not isolation
guarantees against concurrent filesystem changes.
Without this selection, display, language and volume changes are
session-only and control editing/reset is unavailable. Simpsons Arcade
currently offers English (default) and Mexican Spanish in Settings.
The standalone LAN controller restores the locale of its own admitted
settings directory; it does not own the host's settings menu.
The Audio submenu of Settings cycles
master and admitted source levels through 100, 75, 50, 25 and 0 percent.
Saved effects and music levels override their launch defaults independently
when the matching source is admitted; unavailable sources have no row and
retain any saved setting. Audio startup reports the requested output and
resolved master and admitted source percentages; it does not identify the
physical output selected by a system default. A normal launch with the directory
selected enables the next-launch control editor; smoke never writes it.
With the optional Simpsons Arcade bonus art and existing audio/music
selections, set `bonus_music=PUMP_SHA,RESULT_SHA` in the private launch
catalogue to admit the two independently verified local stereo cues.
Explicit native launch accepts `--bonus-music PUMP_SHA RESULT_SHA` after
`--music SHA`. Both cues use the existing music volume; neither is
distributed with the executable.
With the bonus music pair admitted, `handoff_music` additionally accepts
four comma-separated SHA values in defeat, victory, transition and
instructions order. Native launch accepts `--handoff-music` followed by
the same four values after `--bonus-music`. These finite cues also use
the existing music volume.
With the bonus music pair and second-stage art admitted, set
`stage_two_music=SHA` for the finite second-stage stereo excerpt.
Native explicit launch accepts `--stage-two-music SHA` after the
optional handoff selection. The excerpt plays once upon stage entry;
no exact repeating loop is claimed.
Native launch options follow the order shown in usage. For a path value
starting with `--`, use an explicit relative path such as `./--controls`
to distinguish the filename from a misplaced option.

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

## Android local debug packaging

On a Linux x86-64 contributor host, install the Android ARM64 Rust target,
Android SDK platform 35, Build Tools 35.0.0, NDK 28.2.13676358, CMake
and a JDK with `javac`. Set `ANDROID_HOME` to the installed SDK root.
Use a trusted existing output parent and a new package directory:

```sh
cargo run --locked -p games-build -- package simpsons-arcade \
  .temp/android-game android-arm64
cargo run --locked -p games-build -- package-check simpsons-arcade \
  .temp/android-game android-arm64
```

The package contains an unsigned debug APK, dependency notices and a
receipt. It targets API 35 with minimum API 24 and ARM64 native code.
Sign a separate copy with an appropriate local test key before installing.
Original game assets and signing credentials are never bundled. This is
not a production release or a claim of physical-device compatibility.
