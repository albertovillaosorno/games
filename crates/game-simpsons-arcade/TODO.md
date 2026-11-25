# The Simpsons Arcade Game

Preferred source: original Konami World four-player set 1.

See the [canonical title task](
../../docs/todo/open/games/simpsons-arcade.mdc) for acceptance,
reference evidence and unresolved parity.

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
currently locks ground displacement; distinct pose art and actual
enemy combat are not yet implemented. Held-jump variants are omitted.

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
