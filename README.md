# games

Modular Rust game engine with fixed-step simulation, deterministic replay,
action-mapped input, asset conversion and native/WASM targets.

**Preserve the gameplay, not the technical debt.**

A non-commercial hobby project focused on portable games, faithful behaviour,
modern performance and playing together. It is not affiliated with the original
developers or publishers.

## Goals

- Faithful gameplay with modern performance, presentation and controls.
- One shared engine that improves across all titles, including earlier games.
- Offline play, portable saves and self-hosted multiplayer where applicable.
- No ads, subscriptions or in-app purchases in our implementations.
- Import lawfully supplied original assets locally; distribute no proprietary
  game files or original executables.

## Platforms

Linux x86-64 is the primary target. Android ARM64, WebAssembly and Windows
are planned. Other 64-bit platforms are welcome through community
contributions.

## Getting started

The Simpsons Arcade Game has an experimental native preview. No title has
completed release acceptance. See the
[title instructions](crates/game-simpsons-arcade/TODO.md) to run the preview.

From the repository root:

```sh
cargo validate
```

Place your original media in `.reference/games/<id>/` using its catalogue task
identifier. See the [development guide](docs/technical/development.md) for
prerequisites, builds and local packages.

## Documentation

- [Technical documentation](docs/technical/README.md)
- [Roadmap and task records](TODO.md)
- [Agent and contribution rules](AGENTS.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

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
| 11 | Super Mario Bros. | NES |
| 12 | Super Mario Bros. 2 | NES (Western) |
| 13 | Super Mario Bros. 3 | NES |
| 14 | Angry Birds Space | Android |
| 15 | Angry Birds Star Wars (2012) | Android |
| 16 | Angry Birds Star Wars II | Android |
| 17 | SUSH | Android |
| 18 | Plants vs. Zombies: Replanted | PC Replanted |
| 19 | Bugs Bunny: Lost in Time | Windows PC |
| 20 | Bugs Bunny & Taz: Time Busters | Windows PC |
| 21 | Luigi's Mansion | GameCube |
| 22 | The Simpsons: Road Rage | Xbox |
| 23 | Taz: Wanted | Windows PC |
| 24 | SpongeBob: Revenge of the Flying Dutchman | GameCube |
| 25 | The Simpsons: Hit & Run | Windows PC |
| 26 | Crash Twinsanity | Xbox |
| 27 | Sly Cooper and the Thievius Raccoonus | PS3 HD |
| 28 | Sly 2: Band of Thieves | PS3 HD |
| 29 | Sly 3: Honor Among Thieves | PS3 HD |
| 30 | The Simpsons Game | PS3 |
| 31 | El Chavo Kart | Xbox 360 |
| 32 | Sly Cooper: Thieves in Time | PS3 |
| 33 | Crash Team Racing Nitro-Fueled | PS4 |
| 34 | Plants vs. Zombies 2 | Android |
| 35 | Plants vs. Zombies 3: Evolved | Android Evolved |
| 36 | SimCity BuildIt | Android |
| 37 | Hytale | PC |
| 38 | Minecraft Java Edition | Latest stable PC |
| 39 | LEGO Star Wars: The Skywalker Saga | PC |
| 40 | Grand Theft Auto VI | PS5 |

## Contributions

Pull requests are welcome. Follow the [contribution rules](AGENTS.md),
run the validator, and sign off scoped Conventional Commits with
`git commit -s`. Respect third-party licences and attribution.

## Licence

Apache-2.0 for original project code. Third-party dependencies and original
games remain subject to their respective rights and licences. Supply only
original media you are entitled to use. No proprietary assets or original game
executables are included.
