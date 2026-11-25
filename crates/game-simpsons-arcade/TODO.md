# The Simpsons Arcade Game

Preferred source: original Konami World four-player set 1.

See the [canonical title task](
../../docs/todo/open/games/simpsons-arcade.mdc) for acceptance,
reference evidence and unresolved parity.

**Obsolete preview admission:** the owner requires modern free play only.
The coin and continue controls documented below describe the current
prototype, not an approved shipping experience. Replace them with the
visible shared menu and immediate Start/Enter; see canonical acceptance.

## Native movement preview

After locally publishing the four named actor bundles using the MAME
`art-publish` command, launch on GNU/Linux:

```sh
cargo run --locked -p games-simpsons-arcade --features native-preview \
    --bin simpsons-arcade -- PARENT MARGE_SHA HOMER_SHA BART_SHA LISA_SHA
```

Each hash identifies a verified canonical bundle with the corresponding
`actor/marge`, `actor/homer`, `actor/bart` or `actor/lisa` record.
Marge's bundle also provides the stage background. Insert a virtual coin,
press Start, then Primary to enter the prototype. Close the window to quit.

| Station | Coin | Start | Primary / Secondary | Movement |
| --- | --- | --- | --- | --- |
| Marge | 1 | 5 | Z / X | Arrows |
| Homer | 2 | 6 | F / G | WASD |
| Bart | 3 | 7 | O / P | IJKL |
| Lisa | 4 | 8 | Keypad 1 / 2 | Keypad 8/4/5/6 |

Connected SDL controllers are assigned in order to available stations.
Back inserts a coin, Start claims the station, A confirms or attacks,
B jumps, and the D-pad or left stick moves the character. For a finite hidden
native smoke, insert `--smoke` before PARENT. This is a movement
prototype, not a complete arcade recreation.

Insert `--touch` before PARENT to append a touch panel below the scene.
The panel controls Marge: the coin and triangle insert a virtual coin
and start, A confirms or attacks, B jumps, and the four arrows move.
Use two fingers for diagonal movement. Other stations retain keyboard
and controller mappings. Combine the options as `--smoke --touch` to
run the finite synthetic contact and native rendering check. Physical
touch devices and Android/browser presentation remain unverified.

The isolated attack and jump timings are now modeled. Jump visibly
raises the idle actor art using an approximate elevation curve. Attack
retains its opening facing, with approximate slow horizontal drift and
no vertical drift. Jump permits direction changes. The pose catalogue and
combat encounter extend this preview. Held-jump variants are omitted.

For original attack and jump poses, optionally pass
`--poses FILE` after `--touch` (if used) and before PARENT. The local
UTF-8 catalogue has one whitespace-separated record per line:

```text
NAME ACTION TICK SHA256
```

NAME is marge, homer, bart or lisa; ACTION is attack, jump or walk. TICK is
the zero-based fixed-step offset, and SHA256 identifies a canonical
bundle containing that character's actor record. Every station/action
pair requires tick zero. The most recent pose is held until the next
record or maneuver end. The catalogue is limited to 64 poses and
16 KiB. The original four-hash command still works without a catalogue.
Pose crops retain approximate bottom-center pivots.

For the experimental first enemy, insert
`--enemy APPROACH_SHA STRIKE_SHA` after `--poses FILE` (if used) and
before PARENT. These are separately verified local canonical bundles
with `enemy/approach` and `enemy/strike` records respectively. The
first enemy approaches an active station, can be struck with Primary,
and can damage grounded players. Jump evades a ground strike.
Health indicators and enemy actions are independently authored
prototype rules, not an original-game hitbox or health reconstruction.
The optional encounter supports a provisional 1200-tick continue window.
Insert a coin and press Start before expiry to resume; coin alone changes
the prompt and preserves the unused credit after expiry. Stage progression
and the original game-over interlude are not implemented.

Optional `--audio SHA` follows `--enemy` (if used) and precedes PARENT.
The hash identifies the verified canonical PCM sample catalogue. The
preview selects four station-specific attack samples from that bundle,
using fixed-step cues and the shared native mixer. Without this option
it opens no audio device. Nominal pitch, gain, queue timing and voice
admission are provisional; background music and other sound effects
are not reproduced. The hidden smoke uses a paused device to verify
nonzero queued audio and one cue per held attack.

### Local launch catalogue

Create `simpsons-arcade.local` beside the native executable, or pass
`--config FILE` after `--smoke` and `--touch` (if used). The UTF-8
catalogue is limited to 4 KiB and accepts one `key=value` per line:

```text
version=1
parent=../bundles
marge=MARGE_SHA
homer=HOMER_SHA
bart=BART_SHA
lisa=LISA_SHA
```

Replace the placeholders with locally verified manifest digests. Optional
fields are `poses`, `audio`, and the pair `enemy_approach` and
`enemy_strike`. The pose field names a catalogue file; the other optional
fields name manifest digests. Relative paths resolve from the launch
catalogue directory, not the working directory. Without explicit asset
arguments, the executable reads its adjacent catalogue. It does not scan
for original media, and bundle identity and source checks still apply.
The catalogue is trusted local configuration, not authenticated metadata,
and is not included in the executable package.

### Display selection

After the optional `--smoke --touch` flags, pass
`--display MODE` and/or `--window MODE` before the asset selection.
Display choices are `aspect-fit` (default), `integer`, `fill`,
`stretch` and `adaptive`. Window choices are `windowed` (default)
and `fullscreen` (desktop resolution). Alternatively, set the
optional `display` and `window` fields in the local launch catalogue;
explicit command-line choices override those fields.

Integer mode uses aspect-fit below native logical resolution. Adaptive
currently falls back to aspect-fit for this fixed scene; real camera,
scene and HUD extension remains open. Fill may crop the game HUD and
is rejected when the current fixed touch panel is enabled. Responsive
HUD placement and independent touch overlays remain open. Settings
are edited in the catalogue, not saved automatically by an in-game menu.

### Digital control profiles

After the display options, pass `--controls FILE`, or set the local
launch catalogue's optional `controls=FILE` field. The explicit path
overrides the catalogue selection. The bounded UTF-8 profile uses:

```text
version=1
key 1 right=Q
key 1 secondary=Space
key 2 primary=R
button 2 right=x
```

Station numbers 1-4 are Marge, Homer, Bart and Lisa. Actions are
coin, start, primary, secondary, left, right, up and down. Key names
are SDL scancode names; button names follow SDL's game-controller
mapping names. Unspecified controls keep the defaults. Duplicate
keyboard bindings across stations and duplicate buttons within a
station are rejected, as are unknown names and repeated overrides.
Controller replacement inherits the vacant station's profile.
Analog axes remain unchanged; an interactive
remapping menu and automatic settings persistence remain open.

For the optional single touch panel, select a station and swap button
positions in the same profile, using the default action names as slots:

```text
touch-player=3
touch 3 primary=secondary
touch 3 secondary=primary
touch 3 right=left
touch 3 left=right
```

Each station's complete slot mapping must be a permutation. Symbols and
hit regions move together; the other stations retain keyboard/controller
input. The default remains station 1. Simultaneous independent touch
panels, adaptive layout and actual touchscreen testing remain open.
