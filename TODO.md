# Games TODO

This is the canonical public roadmap. Work in phase order.

Open records live in `docs/todo/open/`; completed records move to
`docs/todo/completed/` only after their acceptance criteria pass. Keep
evidence of validation and do not mark planned capabilities as implemented.

Use `cargo validate` before committing or pushing changes.

## P0 - Bootstrap

Completed. See `docs/todo/completed/foundation/`.

## P1 - Contracts

### TODO - Offline and multiplayer contracts

Verify local and self-hosted play without commercial services.

[TODO - Offline and multiplayer contracts](docs/todo/open/contracts/network.mdc)

### TODO - Host and asset safety

Protect the host from malformed or malicious user-supplied data.

[TODO - Host and asset safety](docs/todo/open/contracts/security.mdc)

### TODO - Performance regression contracts

Measure useful throughput without degrading fidelity.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Performance regression contracts](docs/todo/open/contracts/performance.mdc)

### TODO - Integrate architecture checks

Make the P1 contracts a required part of the existing validator.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Integrate architecture checks](docs/todo/open/contracts/contract-gate.mdc)

## P2 - Asset importer

### TODO - Shared dependency selection

Evaluate reusable Rust libraries before creating new mechanisms.

[TODO - Shared dependency selection](docs/todo/open/assets/dependency-audit.mdc)

### TODO - Canonical asset contract

Define the shared asset format and manifest consumed by runtimes.

[TODO - Canonical asset contract](docs/todo/open/assets/asset-contract.mdc)

### TODO - Universal asset importer

Provide one shared import and conversion interface.

[TODO - Universal asset importer](docs/todo/open/assets/importer.mdc)

### TODO - Reference behaviour fixtures

Establish reproducible behaviour comparisons from lawful local references.

[TODO - Reference behaviour fixtures](docs/todo/open/assets/reference-tests.mdc)

### TODO - Title onboarding

Create title packages and their TODO.md files when implementation begins.

[TODO - Title onboarding](docs/todo/open/assets/onboarding.mdc)

## P3 - Build system

### TODO - Universal build command

Expose one build command accepting a title and optional target.

[TODO - Universal build command](docs/todo/open/build/build-command.mdc)

### TODO - Native Linux packaging

Produce portable Linux x86-64 executables with local asset discovery.

[TODO - Native Linux packaging](docs/todo/open/build/native.mdc)

### TODO - Android and browser packaging

Support Android ARM64 and browser targets where compatible.

[TODO - Android and browser packaging](docs/todo/open/build/mobile-web.mdc)

### TODO - Build and distribution checks

Verify build outputs, portability and distribution boundaries.

[TODO - Build and distribution checks](docs/todo/open/build/release-checks.mdc)

## P4 - Title implementation

Implement the following titles sequentially after the foundation gates.
Each record owns title-specific acceptance. Shared rules remain in
AGENTS.md and the foundation contracts.

### TODO - The Simpsons Arcade Game

Original Konami arcade four-player World set 1;
preserve original four-player cooperative gameplay and pixel art.

[TODO - The Simpsons Arcade Game](docs/todo/open/games/simpsons-arcade.mdc)

### TODO - Metal Slug

original Neo Geo arcade assets.

[TODO - Metal Slug](docs/todo/open/games/metal-slug.mdc)

### TODO - Metal Slug 2

original Neo Geo arcade assets.

[TODO - Metal Slug 2](docs/todo/open/games/metal-slug-2.mdc)

### TODO - Metal Slug X

original Neo Geo arcade assets.

[TODO - Metal Slug X](docs/todo/open/games/metal-slug-x.mdc)

### TODO - Metal Slug 3

original Neo Geo arcade assets.

[TODO - Metal Slug 3](docs/todo/open/games/metal-slug-3.mdc)

### TODO - Metal Slug 4

original Neo Geo arcade assets.

[TODO - Metal Slug 4](docs/todo/open/games/metal-slug-4.mdc)

### TODO - Metal Slug 5

original Neo Geo arcade assets.

[TODO - Metal Slug 5](docs/todo/open/games/metal-slug-5.mdc)

### TODO - Metal Slug 6

original Atomiswave arcade release; compare official console compilation
resources.

[TODO - Metal Slug 6](docs/todo/open/games/metal-slug-6.mdc)

### TODO - Metal Slug 7

original Nintendo DS release, preserving its distinctive content.

[TODO - Metal Slug 7](docs/todo/open/games/metal-slug-7.mdc)

### TODO - Metal Slug XX

Steam PC edition preferred for complete official content; verify original sprite
fidelity against PSP and PS4. Higher display resolution alone does not imply
superior source sprites.

[TODO - Metal Slug XX](docs/todo/open/games/metal-slug-xx.mdc)

### TODO - Angry Birds Space

latest complete accessible official Android release, subject to checking content
completeness; preserve gravity-based physics and levels.

[TODO - Angry Birds Space](docs/todo/open/games/angry-birds-space.mdc)

### TODO - Angry Birds Star Wars (2012)

latest complete accessible official Android release, preferred source; compare
official console-exclusive levels and asset quality before pinning. Preserve
original trilogy-themed levels, character abilities and gameplay-based unlocks.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Angry Birds Star Wars (2012)](docs/todo/open/games/angry-birds-star-wars.mdc)

### TODO - Angry Birds Star Wars II

latest complete accessible official Android release; preserve character
abilities, both sides and levels, with gameplay-based unlocks.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Angry Birds Star Wars II](docs/todo/open/games/angry-birds-star-wars-2.mdc)

### TODO - SUSH

Android reference; independently implement a shared virtual pet with care,
evolution, cosmetics and authenticated two-user synchronisation. Support a user-
controlled PWA and local caching where practical.

[TODO - SUSH](docs/todo/open/apps/sush.mdc)

### TODO - Plants vs. Zombies: Replanted

PC; latest suitable official build. Use Replanted as the sole PvZ 1 base rather
than building the 2009 original separately. Preserve its content and modern
features.

[TODO - Plants vs. Zombies: Replanted](docs/todo/open/games/pvz-replanted.mdc)

### TODO - Bugs Bunny: Lost in Time

Windows PC provisional base; compare PlayStation if necessary.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Bugs Bunny: Lost in Time](docs/todo/open/games/bugs-bunny-lost-in-time.mdc)

### TODO - Bugs Bunny & Taz: Time Busters

Windows PC provisional base; compare PlayStation and select the best release.
Preserve cooperative gameplay.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Bugs Bunny & Taz: Time Busters](docs/todo/open/games/bugs-bunny-taz-time-busters.mdc)

### TODO - The Simpsons: Road Rage

original Xbox, provisional base; preserve arcade driving, passenger missions and
original content.

[TODO - The Simpsons: Road Rage](docs/todo/open/games/simpsons-road-rage.mdc)

### TODO - Taz: Wanted

Windows PC provisional asset source; compare Xbox, GameCube and PS2 assets
before pinning; choose the most complete and highest-quality official resources.

[TODO - Taz: Wanted](docs/todo/open/games/taz-wanted.mdc)

### TODO - SpongeBob: Revenge of the Flying Dutchman

GameCube, selected base.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - SpongeBob: Revenge of the Flying Dutchman](docs/todo/open/games/spongebob-revenge-of-the-flying-dutchman.mdc)

### TODO - The Simpsons: Hit & Run

PC, selected base preference. Reconstruct the driving, missions and open world
independently.

[TODO - The Simpsons: Hit & Run](docs/todo/open/games/simpsons-hit-and-run.mdc)

### TODO - Crash Twinsanity

original Xbox provisional source; compare actual PS2 assets, lighting and
content before pinning because visual differences are not solely resolution or
performance. Preserve platforming, partner mechanics and the original campaign.

[TODO - Crash Twinsanity](docs/todo/open/games/crash-twinsanity.mdc)

### TODO - Sly Cooper and the Thievius Raccoonus

PlayStation 3 HD release in The Sly Collection.

[TODO - Sly Cooper and the Thievius Raccoonus](docs/todo/open/games/sly-1.mdc)

### TODO - Sly 2: Band of Thieves

PlayStation 3 HD release in The Sly Collection.

[TODO - Sly 2: Band of Thieves](docs/todo/open/games/sly-2.mdc)

### TODO - Sly 3: Honor Among Thieves

PlayStation 3 HD release in The Sly Collection.

[TODO - Sly 3: Honor Among Thieves](docs/todo/open/games/sly-3.mdc)

### TODO - The Simpsons Game

PlayStation 3, selected HD base; preserve levels, characters, abilities and
cooperative gameplay where applicable.

[TODO - The Simpsons Game](docs/todo/open/games/simpsons-the-game.mdc)

### TODO - El Chavo Kart

Xbox 360, selected base. Modern racing controls, local multiplayer, responsive
rendering and shared racing infrastructure.

[TODO - El Chavo Kart](docs/todo/open/games/el-chavo-kart.mdc)

### TODO - Sly Cooper: Thieves in Time

PlayStation 3.

[TODO - Sly Cooper: Thieves in Time](docs/todo/open/games/sly-4.mdc)

### TODO - Crash Team Racing Nitro-Fueled (2019)

PS4, latest complete suitable official revision with available official content,
preferred source; compare Xbox One X resources before pinning if they offer
higher asset quality. This remake of the 1999 original is the sole CTR target,
not a separate original PS1 port. Preserve adventure mode, racing physics,
tracks, characters, original remake content and local multiplayer; remove
monetisation from the independent implementation and target native PC and
uncapped rendering.

<!-- MarkdownLint-disable-next-line MD013 MD044 -->
[TODO - Crash Team Racing Nitro-Fueled (2019)](docs/todo/open/games/crash-team-racing.mdc)

### TODO - Plants vs. Zombies 2

latest complete accessible Android release; offline-first, gameplay-based
progression and no monetisation in the independent implementation.

[TODO - Plants vs. Zombies 2](docs/todo/open/games/pvz-2.mdc)

### TODO - Plants vs. Zombies 3: Evolved

preferred official Android edition, latest complete lawfully accessible release;
verify exact package and availability. Reimplement local gameplay and required
independent services.

[TODO - Plants vs. Zombies 3: Evolved](docs/todo/open/games/pvz-3.mdc)

### TODO - SimCity BuildIt

latest complete lawfully accessible official Android release; city-building
game. Preserve construction, zoning/building progression, traffic, utilities,
economy, production chains, city expansion, disasters and relevant events.
Implement offline-first city simulation and independently authored replacements
for necessary services; no ads, purchases, subscriptions or monetisation-
oriented delays. Optional independent multiplayer/trading only where feasible;
missing server-side rules or resources must be documented rather than presumed
recoverable from the APK.

[TODO - SimCity BuildIt](docs/todo/open/games/simcity-buildit.mdc)

## P5 - Future experiments

These conditional tasks stay last. Do not activate them before the
preceding catalogue is complete and the selected official media exists.

### TODO - Hytale

Wait for the official stable 1.0 PC release. Evaluate an independent Rust client
and server.

[TODO - Hytale](docs/todo/open/games/hytale.mdc)

### TODO - Grand Theft Auto VI

Evaluate the best official PlayStation edition after Hytale. Use PS5 with PS5
Pro enhancements or PS6 if a suitable edition exists.

[TODO - Grand Theft Auto VI](docs/todo/open/games/gta-vi.mdc)
