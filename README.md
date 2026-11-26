# games

A shared Rust engine for independent, faithful game reconstructions with
modern controls, presentation, performance and portability.

**Preserve the gameplay, not the technical debt.**

A non-commercial hobby project focused on portable games, faithful behaviour,
modern performance and playing together. It is not affiliated with the original
developers or publishers.

## Goals

- Faithful gameplay with modern performance, presentation and controls.
- One shared engine: improvements to common systems benefit earlier and
  future titles without duplicating implementations.
- Portable capability contracts: add platforms and devices through shared
  adapters, not individual game ports.
- Offline play, portable saves and self-hosted multiplayer where applicable.
- No ads, subscriptions or in-app purchases in our implementations.
- Import lawfully supplied original assets locally; distribute no proprietary
  game files or original executables.

## Platforms

Linux x86-64 is the primary development target. Windows, Android ARM64
and WebAssembly are planned through shared adapters. No title has
completed release acceptance.

## Getting started

From the repository root:

```sh
cargo validate
```

Place your original media in `.reference/games/<id>/` using its catalogue task
identifier. See the [development guide](docs/technical/development.md) for
prerequisites, builds and local packages.

## Documentation

- [Technical documentation](docs/technical/README.md)
- [Platform portability](docs/technical/platform-ir.md)
- [Roadmap and task records](TODO.md)
- [Agent and contribution rules](AGENTS.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## Catalogue

Preferred source editions are planning choices, not guarantees of support. Exact
revisions will be selected when onboarding each title. Status describes the
independent reconstruction, not the original game's release status. Local media
is not proof of provenance, successful conversion or playable parity.

| # | Title | Original edition | Status |
| --- | --- | --- | --- |
| 01 | The Simpsons Arcade Game | Arcade (4P) | Playable preview |
| 02 | Metal Slug | Neo Geo | Graphics imported |
| 03 | Metal Slug 2 | Neo Geo | Media present |
| 04 | Metal Slug X | Neo Geo | Media present |
| 05 | Metal Slug 3 | Neo Geo | Media present |
| 06 | Metal Slug 4 | Neo Geo | Media present |
| 07 | Metal Slug 5 | Neo Geo | Media present |
| 08 | Metal Slug 6 | Atomiswave | Media present |
| 09 | Metal Slug 7 | Nintendo DS | Media present |
| 10 | Metal Slug XX | Steam PC | Media present |
| 11 | Super Mario Bros. | NES | Awaiting media |
| 12 | Super Mario Bros. 2 | NES (Western) | Awaiting media |
| 13 | Super Mario Bros. 3 | NES | Awaiting media |
| 14 | Angry Birds Space | Android | Awaiting media |
| 15 | Angry Birds Star Wars (2012) | Android | Awaiting media |
| 16 | Angry Birds Star Wars II | Android | Awaiting media |
| 17 | SUSH | Android | Awaiting media |
| 18 | Plants vs. Zombies: Replanted | PC Replanted | Awaiting media |
| 19 | Bugs Bunny: Lost in Time | Windows PC | Awaiting media |
| 20 | Bugs Bunny & Taz: Time Busters | Windows PC | Awaiting media |
| 21 | Luigi's Mansion | GameCube | Awaiting media |
| 22 | The Simpsons: Road Rage | Xbox | Awaiting media |
| 23 | Taz: Wanted | Windows PC | Awaiting media |
| 24 | SpongeBob: Revenge of the Flying Dutchman | GameCube | Awaiting media |
| 25 | The Simpsons: Hit & Run | Windows PC | Reference present |
| 26 | Crash Twinsanity | Xbox | Awaiting media |
| 27 | Sly Cooper and the Thievius Raccoonus | PS3 HD | Awaiting media |
| 28 | Sly 2: Band of Thieves | PS3 HD | Awaiting media |
| 29 | Sly 3: Honor Among Thieves | PS3 HD | Awaiting media |
| 30 | The Simpsons Game | PS3 | Awaiting media |
| 31 | El Chavo Kart | Xbox 360 | Awaiting media |
| 32 | Sly Cooper: Thieves in Time | PS3 | Awaiting media |
| 33 | Crash Team Racing Nitro-Fueled | PS4 | Awaiting media |
| 34 | Plants vs. Zombies 2 | Android | Awaiting media |
| 35 | Plants vs. Zombies 3: Evolved | Android Evolved | Awaiting media |
| 36 | SimCity BuildIt | Android | Awaiting media |
| 37 | CamScanner | Android | Awaiting media |
| 38 | Hytale | PC | Conditional |
| 39 | Minecraft | Latest stable PC | Conditional |
| 40 | LEGO Star Wars: The Skywalker Saga | PC | Conditional |
| 41 | Grand Theft Auto VI | PS5 | Conditional |

Status definitions: playable preview is incomplete gameplay; graphics
imported is verified conversion, not gameplay; media/reference present
means local inputs await further admission; awaiting media means no
suitable input is recorded; conditional means future P5 activation.
No title is marked complete without its canonical task acceptance.

## Contributions

Pull requests are welcome. Follow the [contribution rules](AGENTS.md),
run the validator, and sign off scoped Conventional Commits with
`git commit -s`. Respect third-party licences and attribution.

## Licence

Apache-2.0 for original project code. Third-party dependencies and original
games remain subject to their respective rights and licences. Supply only
original media you are entitled to use. No proprietary assets or original game
executables are included.
