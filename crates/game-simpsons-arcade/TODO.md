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
Back inserts a coin, Start claims the station, A confirms, B is reserved,
and the D-pad or left stick moves the character. For a finite hidden
native smoke, insert `--smoke` before PARENT. This is a movement
prototype, not a complete arcade recreation.
