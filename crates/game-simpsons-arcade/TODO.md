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
retains its opening facing. Horizontal drift starts two ticks after
admission, at one quarter-pixel per held direction tick through offset
18. Its fractional remainder persists between actions; vertical drift
is absent. Jump retains the last horizontal stride until landing
or another horizontal direction is pressed. Depth follows held input.
The pose catalogue and combat encounter extend this preview. Hold
Jump for a longer, higher
flight; a short tap lasts 31 ticks and a full hold 58 ticks. The existing
short-jump art is reused unless optional middle poses are supplied.
Press Primary during Jump for a forward aerial attack. The initial height
selects its speed; the attack lands automatically and can hit the
experimental enemy once. Press both buttons together with a neutral stick
for a stationary 30-tick special attack; a directional chord instead
starts the same variable-height jump.

For original attack and jump poses, optionally pass
`--poses FILE` after `--touch` (if used) and before PARENT. The local
UTF-8 catalogue has one whitespace-separated record per line:

```text
NAME ACTION TICK SHA256
```

For stations, NAME is marge, homer, bart or lisa; ACTION is attack,
attack-moving,
air-attack, hurt, jump, jump-lift, special, walk or walk-up. TICK is
the zero-based fixed-step offset, and SHA256 identifies a canonical
bundle containing that character's actor record. Every station/action
pair requires tick zero. The most recent pose is held until the next
record or maneuver end. The catalogue is limited to 180 poses and
16 KiB. The original four-hash command still works without a catalogue.
Pose crops retain approximate bottom-center pivots.
An optional jump-lift sequence consists of exactly one tick-zero middle
pose for each of the four characters. When present, the preview retains
the opening pose during sustained lift, selects the middle pose near
the apex, and falls back to the original jump sequence for descent.
The older catalogue format still works without these four records.
An optional air-attack sequence has a tick-zero pose for every character
and permits offsets through tick 59. Without it the ground-attack pose
sequence is used as a provisional fallback. An optional special sequence
requires all four tick-zero records and permits offsets through tick 29.
Without it the aerial or ground-attack sequence is reused.
An optional walk-up sequence for all four characters uses the same
cycle phase as walk. Upward input selects it, including diagonals;
otherwise the forward walking sequence is used. Missing walk-up art
falls back to walk. Held walking input advances the cycle even at a
blocked boundary, and neutral input resets it.
An optional attack-moving sequence for all four characters has the
same duration as attack. Horizontal input on the tick after attack
admission selects it for the rest of that attack. Its first preview
tick and missing moving art use the neutral sequence. Hit timing and
recovery are unchanged.
An optional hurt sequence requires tick-zero art for each station.
Marge, Homer and Lisa permit offsets through 13; Bart through 16.
Without hurt art, the idle sprite is used during the reaction.
An optional `enemy attack TICK SHA256` sequence starts at zero, permits
offsets through 33 and uses the bundle record `enemy/strike`. The enemy
uses the single strike bundle when that sequence is absent.
An optional `enemy walk TICK SHA256` sequence similarly starts at zero,
permits offsets through 47 and uses the bundle record `enemy/approach`.
The walking phase advances only on approach displacement. It wraps
every 48 ticks and resets when an attack ends.
An optional `enemy defeat TICK SHA256` sequence begins at zero, permits
offsets through 65 and uses the bundle record `enemy/defeat`.
Without it, the approach sprite is retained during the defeat timeline.

Optional `--shadow SHA` follows the audio selection (if used) and
precedes `--world` or PARENT. It selects an independently isolated
`actor/shadow` mask from a verified local canonical bundle. Marge,
Homer and Lisa display it during jumps, aerial attacks and specials;
Bart uses it only during the observed early special ticks. The mask
tracks the ground point, not actor lift. Without this option, no extra
atlas is allocated. Native alpha blending and overlap ordering remain
approximate.

For the experimental first enemy, insert
`--enemy APPROACH_SHA STRIKE_SHA` after `--poses FILE` (if used) and
before PARENT. These are separately verified local canonical bundles
with `enemy/approach` and `enemy/strike` records respectively. The
first enemy approaches an active station, can be struck with Primary,
and can damage grounded players. Jump evades a ground strike.
The punch lasts 34 simulation ticks, dealing damage once at tick 14.
A grounded player attack resolves its single hit at offset ten;
this timing is observed for all four stations. A single hit defeats the
first enemy, which stops dealing and receiving damage immediately.
The defeated enemy remains visible for 52 ticks, blinks on odd offsets
53 through 65, and disappears at offset 66. Its independently authored
motion travels up to 66 pixels away from the attacker, clipped to the
current movement bounds. Its defeat art retains approximate pivots.
The next approach tick re-evaluates whether a new punch can begin.
A nonlethal light hit interrupts the current maneuver and prevents
movement and action admission for 14 ticks (17 for Bart). Held
directional input resumes on recovery; early button presses are lost.
The optional encounter models two health stocks and a 32-column meter
that drains and refills one column every two ticks. The first depleted
stock automatically restores play after a recovery interval; the final
stock opens a provisional 1200-tick continue window. Press Start before
expiry to rejoin for free. If all stations expire, the main menu returns
and the encounter resets. Pausing freezes movement, combat, maneuver
timing, meter animation and recovery. Enemy actions, damage, respawn
positions, recovery delays and stock-marker artwork remain provisional.
With the verified extended street, reinforcements arrive from the right,
left and right at camera travel 50, 82 and 154 respectively. They reuse
the first enemy's provisional combat and pose rules. Each is admitted
only once per run. Full stage progression and the original game-over
interlude are not implemented.

Optional `--audio SHA` follows `--enemy` (if used) and precedes PARENT.
The hash identifies the verified canonical PCM sample catalogue. The
preview selects four station-specific attack samples from that bundle,
using fixed-step cues and the shared native mixer. Ground attacks keep
the station-specific four- or five-tick onset. Aerial attacks and
neutral simultaneous-button attacks emit the same clip at tick four.
Ordinary jumps, including directional button chords, emit no attack
cue. Without this option it opens no audio device. Nominal pitch, gain,
queue timing and voice admission are provisional; background music and
other sound effects remain unimplemented.

Optionally insert `--audio-mod MOD_SHA` immediately after `--audio SHA`.
The explicitly selected passive replacement bundle may replace only the
four attack slots, `audio/sample-0004` through `audio/sample-0007`.
The base and replacement must have matching PCM layouts. A damaged or
unapproved replacement is rejected, not silently ignored. See the root
README for the bounded raw PCM and WAV authoring commands.
The hidden smoke uses a paused device to verify
nonzero queued audio and one cue per held ground, aerial or special
attack. Pause suspends playback and Settings keeps it suspended;
returning to the main menu clears audio.

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
asset fields are `poses`, `audio`, `audio_mod`, `shadow`, `world`,
and the pair
`enemy_approach` and `enemy_strike`. The pose field names a catalogue
file; the other asset fields name manifest digests. The `audio_mod`
field requires `audio`. Relative paths resolve from the launch catalogue
directory, not the working directory. Without explicit asset
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
or `--world SHA` after the audio and shadow options (if used) and
before PARENT. Generate
the local strip with the original observations and the build command:

```sh
cargo run --locked -p games-build -- mame strip-publish simpsons \
  world-4p-set-1 OUTPUT PARENT SHA256 28 16 \
  RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS \
  RAM REGISTERS SCROLL PALETTE RGB32 OBJECTS CONTROLS
```

The command verifies two to eight complete scenes before stitching exactly
overlapping world interiors. The title verifies the strip against the
selected base art. The latest verified strip supports up to 474
logical columns (320 plus 154 additional pixels); wider surfaces use
aspect-fit without stretching the world. Earlier strips still work. The four
station indicators
spread across the width, menus and touch panel center, and hit regions
follow the panel. With a verified strip, the same fixed-step camera and
movement bounds
apply in every display mode. The provisional opening street advances
rightward while retaining the active party and living initial enemy
within the original-width play area. Reinforcements enter beyond its
edges and approach independently in world coordinates. Full stage
progression remains open.
Fill may crop the game HUD and is rejected with touch enabled.
Settings
can also be changed through the main or pause menu: Up/Down selects,
Primary cycles display or toggles desktop fullscreen, and Back returns.
Menus temporarily use aspect-fit when Fill is selected for gameplay.
Settings describes whether the selected world strip permits adaptive width.
Changes apply immediately. To retain them across launches, create a
dedicated private directory and pass `--settings DIR` after the display
options and before the control options. Alternatively set `settings=DIR`
in the launch catalogue; relative paths resolve from that file. An explicit
settings path overrides the catalogue. Saved display values override
catalogue defaults, and explicit display or window flags override saved
values. Only in-game display changes are stored; smoke runs never write
settings.
The directory must exist and be trusted, and must not contain game-progress
slots. The versioned settings payload is stored atomically in slot-0.gms.
Malformed data or failed persistence is reported as an error. A saved
Fill preference is safely opened as aspect-fit when touch is enabled.

### Digital control profiles

After the display options, pass `--controls FILE`, or set the local
launch catalogue's optional `controls=FILE` field. The explicit path
overrides the catalogue selection. With a settings directory, an explicit
profile is saved after successful native startup and takes precedence over
the catalogue on future launches. Use `--reset-controls` instead of
`--controls FILE` to persist the default bindings; reset requires a settings
directory and is unavailable in smoke mode. The settings directory stores
the profile separately from display settings in slot-1.gms. Smoke can
select a profile without replacing saved controls. The bounded UTF-8
profile uses:

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
Digital controller axes can be assigned per station in the same file:

```text
axis 2 leftx=none/none
axis 2 rightx=left/right
axis-threshold 2 rightx=20000/9000
axis 2 righttrigger=none/primary
```

Axis names follow SDL: leftx, lefty, rightx, righty, lefttrigger and
righttrigger. The two actions correspond to negative and positive
motion; none disables a direction. Triggers accept only positive
bindings. Press magnitudes must be 1-32767 and release magnitudes
must be smaller (including zero). Defaults remain 16000 to press
and 10000 to release. Both the host and remote controller support
these digital profiles; the remote controller uses station 1's
profile. Proportional movement and an interactive remapping menu
remain open.

For optional touch panels, select stations and swap button
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
input. The default remains station 1. To enable several local stations,
use `touch-players=1,2,3,4` instead of `touch-player=3`. Select any nonempty
subset; duplicate seats or selection records are rejected. Panels stack
in ascending seat order below the game, with visible seat numbers when
more than one is enabled. All selected seats can hold controls at once;
keyboard and controllers remain usable. A contact stays assigned to the
initial panel until release; dragging elsewhere releases held actions
without activating another panel. A LAN host rejects any selection
that includes a reserved remote seat. Physical touchscreen testing and
interactive panel layout editing remain open.

### Explicit LAN hosting

Pass `--lan-host FILE` before the asset selection (and after the other
optional controls and display flags). For example, prepare this private
host catalogue with one remote seat:

```text
version=1
address=192.168.1.20:45000
name=games.test
root=host-root.der
leaf=host-leaf.der
key=host-key.der
peer2_root=guest-root.der
peer2_leaf=guest-leaf.der
```

Replace the address with the host's actual private interface and the
certificate paths with separately approved DER credentials. Relative paths
are resolved from the catalogue directory. Use a valid server certificate
for the selected name and a valid client-auth certificate for each remote
seat. Keep each private key on its owning machine in an owner-only regular
file; exchange only the public root and leaf certificates over a trusted
channel. Credential issuance and a trust-confirmation UI are not provided.

Seat reservations use the numbered `peer1` through `peer4` field pairs.
Reserve one to three seats; local keyboards and controllers skip them.
Touch is rejected when its configured seat is remote. The host starts
the world and controls menus and Pause. A verified client can join an
assigned seat once the world is playing. The main menu distinguishes local
Start from remote joining. The host screen is authoritative;
the client protocol carries control transitions, not a replicated world.
No LAN listener opens without the explicit flag, and hidden smoke cannot
open one.

On the controller machine, create a separate private catalogue:

```text
version=1
address=192.168.1.20:45000
name=games.test
root=guest-root.der
leaf=guest-leaf.der
key=guest-key.der
host_root=host-root.der
host_leaf=host-leaf.der
```

Use the host's address, its certificate name, and the corresponding
approved certificate pair. Launch without game assets:

```sh
simpsons-arcade --lan-controller controller.local
```

Watch the host screen. Enter or gamepad Start joins when the host has
started the world. Arrow keys or the D-pad/left stick move; Z/A attacks
and X/B jumps by default. The controller panel shows its effective digital
bindings. Optionally add `controls=FILE` to its catalogue, resolving the
existing version-one control profile relative to the catalogue directory.
Use `settings=DIR` in the controller catalogue or append `--settings DIR`
to select a dedicated controller settings directory. Append
`--controls FILE` to override and persist the profile after successful
connection and native startup. Append `--reset-controls` instead to
persist default bindings. Saved profiles override catalogue defaults;
use a different settings directory for each controller and the host.
Only station 1 bindings are used on a standalone controller, regardless of
the host-assigned seat. Optionally add `touch=on` to show the seven shared
touch controls below the hints; only station 1 may be touch-enabled in
the selected profile. Simultaneous contacts and drag-off releases use
the same remappable regions as the cabinet. The host-only Pause button
is neither drawn nor mapped. Touch is disabled by default (`touch=off`).
The host handles Pause and menus. Close the controller window to
disconnect. Re-launch with the same approved identity to rejoin; no
automatic discovery or reconnection is attempted.

Use matching protocol-version-two host and controller builds; the local
credential catalogue format remains version one. An idle controller sends
a heartbeat every 500 ms. The host acknowledges valid heartbeats and
controls at a bounded rate. It disconnects a peer after three seconds
without a valid decoded message and releases its held controls on the
next eligible simulation tick. The controller also closes after three
seconds without a valid host acknowledgement. Re-launch to rejoin if
either lease expires. Runtime transport errors explain why the controller
closed and how to rejoin; invalid frames also advise matching builds.
A loopback test stops host polling while the socket
remains open to verify controller-side expiry.

#### Manual credential preparation

On each participating machine, use a new empty private directory with
`umask 077`. The following OpenSSL example issues a separate root and
leaf for one endpoint. On the host set `role=serverAuth`; on each
controller set `role=clientAuth`. Use the exact name configured in the
catalogues for the host; `games.test` is an example, not a service.

```sh
umask 077
role=serverAuth
name=games.test
openssl req -new -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
  -nodes -subj "/CN=Local game root" -days 30 \
  -addext 'basicConstraints=critical,CA:TRUE,pathlen:0' \
  -addext 'keyUsage=critical,keyCertSign,cRLSign' \
  -keyout ca-key.pem -out ca.pem
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
  -nodes -subj "/CN=$name" -keyout leaf-key.pem -out leaf.csr
printf 'basicConstraints=critical,CA:FALSE\n' > leaf.ext
printf 'keyUsage=critical,digitalSignature\n' >> leaf.ext
printf 'extendedKeyUsage=%s\n' "$role" >> leaf.ext
if [ "$role" = serverAuth ]; then
  printf 'subjectAltName=DNS:%s\n' "$name" >> leaf.ext
fi
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca-key.pem \
  -CAcreateserial -days 30 -extfile leaf.ext -out leaf.pem
openssl x509 -in ca.pem -outform DER -out root.der
openssl x509 -in leaf.pem -outform DER -out leaf.der
openssl pkcs8 -topk8 -nocrypt -in leaf-key.pem -outform DER \
  -out key.der
chmod 600 key.der
```

Keep the CA signing key and leaf private keys local and protected.
Use an IP subject alternative name instead of a DNS entry if the
catalogue name is an IP address. Exchange only `root.der` and
`leaf.der` with approved participants over a trusted channel. Compare
the SHA-256 fingerprints of received public DER files independently
before configuring them; do not trust a certificate merely because it
was received over the network. For example:

```sh
sha256sum root.der leaf.der
```

Name the received public files as shown in the catalogues above. Point
each catalogue's own `root`, `leaf` and `key` fields at its own local
DER files. Do not reuse a participant's root or private key for another
seat. Replace expired credentials and explicitly update the corresponding
approved public files. This is manual local administration, not an
in-game certificate issuance or discovery service.

The standalone controller profile was exercised with an optimized native
build using two processes on loopback under Xvfb and software OpenGL.
The old Right binding changed no host pixels, remapped H changed 19,926
pixels, and subsequent captures after release were identical. The
automated native gate also covers button remapping, focus loss and
controller detach. Physical controllers and multi-device play remain
unverified.

Two independent native loopback diagnostics also suspended the host
process without closing its socket. The controller timed out after
approximately three seconds and the same identity rejoined after the
host resumed. In the second diagnostic, the host's offline label
changed 1,008 pixels confined to the reserved seat's HUD; rejoin
restored the earlier frame exactly. Remote movement then changed
19,539 world pixels. This is process suspension on one machine, not
physical network-partition or multi-device acceptance.

For an opt-in visible native performance report, use `--measure` before
`--touch`, `--display` or the asset selection instead of `--smoke`.
Startup reports the renderer, load interval, RSS and executable size.
Gameplay reports are emitted when leaving play, changing logical or
drawable extent, viewport or display settings, and closing the host.
They include host work time through successful presentation, intervals
between consecutive successful submissions, separate 60 Hz budgets,
sampled RSS and separate oldest applied local-input and remote-decode
ages for frames with observed controls. Remote timing starts at successful
authenticated host decoding and includes delayed admission and execution.
The first submission of each gameplay segment starts the interval clock;
it contributes no interval or FPS sample.
They exclude menu-only and failed submissions, client-side input and
network transit, physical scanout and GPU-completion timing.
