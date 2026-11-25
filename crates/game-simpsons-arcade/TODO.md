# The Simpsons Arcade Game

Preferred source: original Konami World four-player set 1.

See the [canonical title task](
../../docs/todo/open/games/simpsons-arcade.mdc) for acceptance,
reference evidence and unresolved parity.

The prototype uses modern free play only. Original paid admission is
reference evidence, not a supported interaction.

## Native movement preview

After locally publishing the four named actor bundles using the MAME
`art-publish` command, launch on GNU/Linux:

```sh
cargo run --locked -p games-simpsons-arcade --features native-preview \
    --bin simpsons-arcade -- PARENT MARGE_SHA HOMER_SHA BART_SHA LISA_SHA
```

Each hash identifies a verified canonical bundle with the corresponding
`actor/marge`, `actor/homer`, `actor/bart` or `actor/lisa` record.
Marge's bundle also provides the stage background. The visible main menu
shows each seat's selected character and Start key. Left/Right selects
a vacant character before joining; Enter starts seat 1 immediately.
Other seats can choose and join during play. Up/Down navigates the menu
and Primary confirms. Pause opens Resume, Settings, Main Menu and Exit;
Pause or Start resumes.
Close the window or select Exit to quit.

| Station | Pause | Start | Primary / Secondary | Movement |
| --- | --- | --- | --- | --- |
| Marge | Escape | Enter | Z / X | Arrows |
| Homer | F2 | 6 | F / G | WASD |
| Bart | F3 | 7 | O / P | IJKL |
| Lisa | F4 | 8 | Keypad 1 / 2 | Keypad 8/4/5/6 |

Connected SDL controllers are assigned in order to available stations.
Back pauses, Start joins immediately, A confirms or attacks,
B jumps, and the D-pad or left stick moves the character. For a finite hidden
native smoke, insert `--smoke` before PARENT. This is a movement
prototype, not a complete arcade recreation.

Insert `--touch` before PARENT to append a touch panel below the scene.
The panel controls seat 1 by default: the pause symbol suspends the game
and the triangle joins or resumes, A confirms or attacks, B jumps, and
the four arrows move.
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
Press Start before expiry to rejoin for free. If all stations expire,
the main menu returns and the encounter resets. Pausing freezes movement,
combat, maneuver timing and the countdown. The countdown, health and
respawn rules remain provisional. Stage progression and the original
game-over interlude are not implemented.

Optional `--audio SHA` follows `--enemy` (if used) and precedes PARENT.
The hash identifies the verified canonical PCM sample catalogue. The
preview selects four station-specific attack samples from that bundle,
using fixed-step cues and the shared native mixer. Without this option
it opens no audio device. Nominal pitch, gain, queue timing and voice
admission are provisional; background music and other sound effects
The hidden smoke uses a paused device to verify
nonzero queued audio and one cue per held attack. Pause suspends playback
and Settings keeps it suspended; returning to the main menu clears audio.

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
fields are `poses`, `audio`, `world`, and the pair `enemy_approach` and
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
widens the logical frame only when a matching world strip is selected;
otherwise it uses aspect-fit. Use the optional `world` catalogue digest
or `--world SHA` after `--audio` (if used) and before PARENT. Generate
the local strip with the original observations and the build command:

```sh
cargo run --locked -p games-build -- mame strip-publish simpsons \
  world-4p-set-1 OUTPUT PARENT SHA256 28 16 \
  RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS \
  RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS
```

The command verifies both complete scenes before stitching exactly
overlapping world interiors. The title verifies the strip against the
selected base art. The currently verified strip supports up to 362
logical columns (320 plus 42 additional pixels); wider surfaces use
aspect-fit without stretching the world. The four station indicators
spread across the width, menus and touch panel center, and hit regions
follow the panel. Movement bounds and stage progression remain unchanged.
Fill may crop the game HUD and is rejected with touch enabled.
Independent simultaneous touch overlays remain open. Settings
can also be changed through the main or pause menu: Up/Down selects,
Primary cycles display or toggles desktop fullscreen, and Back returns.
Menus temporarily use aspect-fit when Fill is selected for gameplay.
Changes apply immediately. To retain them across launches, create a
dedicated private directory and pass `--settings DIR` after the display
options and before `--controls`. Alternatively set `settings=DIR` in the
launch catalogue; relative paths resolve from that file. An explicit
settings path overrides the catalogue. Saved display values override
catalogue defaults, and explicit display or window flags override saved
values. Only in-game changes are stored; smoke runs never write settings.
The directory must exist and be trusted, and must not contain game-progress
slots. The versioned settings payload is stored atomically in slot-0.gms.
Malformed data or failed persistence is reported as an error. A saved
Fill preference is safely opened as aspect-fit when touch is enabled.

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

Seat numbers 1-4 default to Marge, Homer, Bart and Lisa. Actions are
pause, start, primary, secondary, left, right, up and down. Key names
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
panels and actual touchscreen testing remain open.
