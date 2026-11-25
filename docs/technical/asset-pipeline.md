# Asset pipeline

The host build tool selects arcade originals by MAME machine and revision.
The importer owns supported media layouts, word-lane assembly, reusable
chip layouts and canonical graphics conversion. Unknown profiles fail closed.

```sh
cargo run --locked -p games-build -- mame import simpsons world-4p-set-1 \
  .reference/games/simpsons-arcade/simpsons.zip 3000000 1048576 7209088 14
```

Use `mame verify` for read-only archive and required media-lane admission.
Source snapshots require the Linux or Android regular-file pinning adapter
and trusted procfs. Other hosts reject source requests before path access
until an equivalent adapter is implemented. Android device operation is
not yet verified.
On GNU/Linux, use
`mame publish MACHINE REV PARENT PATH SOURCE ENTRY TOTAL COUNT` or
`mame check MACHINE REV PARENT MANIFEST_SHA256`. The four numeric limits
bound source bytes, individual decoded members, total bytes and member count.
Create a trusted, private output parent inside the ignored reference directory
before publication. `zip verify PATH TITLE REV SOURCE ENTRY TOTAL COUNT`
remains available for explicit generic flat ZIP admission. Neither command
establishes authenticity of original media. New version-two manifests record
only the selected title and layout; canonical payload and manifest hashes
remain required. Existing canonical version-one bundles remain readable at
their original addresses without propagating source fingerprints.

For the admitted unencrypted Neo Geo cartridge layouts, use the same
conversion and publication transaction with the `neo` source adapter:

```sh
cargo run --locked -p games-build -- neo import metal-slug mvs-aes-neo-v1 \
  .reference/games/metal-slug/metal-slug.neo 27529216 16777216 27525120 5
```

The admitted graphics profiles are `metal-slug`, `metal-slug-2` and
`metal-slug-x`, each with revision `mvs-aes-neo-v1`.

`neo verify PATH TITLE REV SOURCE ENTRY TOTAL COUNT` checks structural
container bounds only. The selected profile additionally validates the
exact source region sizes and converts fixed-layer and sprite indices.
GNU/Linux supports `neo publish TITLE REV PARENT PATH SOURCE ENTRY TOTAL
COUNT` and `neo check TITLE REV PARENT MANIFEST_SHA256`. Original program
regions are not executed or retained in canonical bundles. Audio, palettes,
scene reconstruction and gameplay are not provided by these profiles.
On GNU/Linux, republish legacy canonical bundles with

```text
bundle upgrade PARENT MANIFEST_SHA256 MANIFEST_BYTES PAYLOAD_BYTES \
  TOTAL_BYTES COUNT
```

The four numeric limits bound metadata, per-payload and total logical bytes,
and record count. This command verifies the pinned canonical bundle and writes
a new version-two sibling; it never replaces or removes the old bundle.
Keep the original bundle until its consumers use the new canonical address.

Author one passive-media replacement from a pinned base descriptor:

```text
bundle replace-media PARENT BASE_SHA256 KEY RAW MANIFEST_BYTES \
  PAYLOAD_BYTES TOTAL_BYTES COUNT
```

Supply exact canonical RGBA8 pixels or signed 16-bit little-endian PCM,
not a compressed media container. The command admits the base manifest
within the explicit limits, preserves the slot format and length, and
publishes a separate one-record bundle without changing the base. Titles
still decide which media slots may be replaced. For the Simpsons preview,
select `audio_mod=MOD_SHA` alongside `audio=BASE_SHA` in the local catalogue,
or use `--audio BASE_SHA --audio-mod MOD_SHA`. Only the four attack clips
`audio/sample-0004` through `audio/sample-0007` are accepted. For example,
slot `audio/sample-0004` needs 8,335 mono frames (16,670 bytes) at the
canonical 10 kHz rate. No executable or game-data replacement is permitted.

For an exact uncompressed PCM16 WAV export, use `bundle replace-wave`
with the same arguments, replacing RAW with the WAV path. The supported
RIFF layout has the standard 44-byte header without additional chunks.
The base descriptor bounds the input size, and its sample rate, channels
and frame count must all match; no resampling or conversion is performed.

The selected Simpsons bundle is verified locally. Its observed palette RAM
and mixer assignments match independent reference values; bounded tilemap
admission and exact PCM reference comparison also pass. The optional GNU/Linux
`native-preview` feature provides `mame render-check` for selected imported
tiles and `mame render-bench` for measured host submission intervals.
These diagnostics establish asset access, not gameplay or screen parity.
See the [title TODO](../../crates/game-simpsons-arcade/TODO.md) and
the performance contract in [TODO.md](../../TODO.md) for reproducible
commands, observations and limitations. Other MAME profiles and platform
publication remain open.
