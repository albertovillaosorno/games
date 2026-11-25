# Agent instructions

## Workflow

- Use one agent initially. Follow the catalogue order and root README.
- Run `cargo validate` before completing work. Do not weaken checks,
  suppress diagnostics, or remove tests simply to obtain a passing result.
- Stay on main. Use Git and sign scoped Conventional Commits with DCO.
  Run `cargo validate` before committing; push validated commits only.
  Use `type(scope): description`; scope is mandatory, lowercase kebab-case.
  Subjects must not exceed 72 characters. Use a meaningful body.
  Never create branches or worktrees. Preserve concurrent work.
- Coordinate with other agents: claim distinct tasks and avoid overlapping
  edits. Never overwrite changes from another agent.
- Follow the root [TODO.md](TODO.md) and its typed task records.
- Run `cargo validate` after changing any code, roadmap or task.
  Use `cargo validate --no-cache` to force a fresh validation, including
  after changes to local reference media or external test prerequisites.
  Cache is under `.cache/validate/`; run logs are under `.temp/validate/`.
- Keep task headings, identifiers, priorities and dependency order aligned.
  Move records to the completed directory only with actual evidence.
- Keep the root README concise: purpose, goals, platform status, entry
  points and catalogue. Put implementation details, commands, limits and
  technical design under docs/technical; keep title evidence in tasks.
- Keep the canonical title task under docs/todo; do not duplicate it
  with a second title roadmap. No additional planning bureaucracy.
- Keep every authored text line within 80 characters. Direct task links
  may use the narrow MarkdownLint exception used by Malbolge.
- Use Rust for project code, tools and validation. Python is forbidden.
  Other languages are allowed only for necessary platform glue or shaders;
  justify the requirement and keep the interface narrow.

## Architecture

- Search existing code and dependencies before writing any capability.
  Reuse first, extend second, implement once only if absent.
- Extract shared mechanisms; express title-specific behaviour through
  parameters, composition, strategies and narrow adapters.
- The engine adapts to the title, never the reverse. When an existing
  shared mechanism cannot express a legitimate requirement, extend that
  mechanism and migrate its consumers rather than bypassing or copying it.
  Remove arbitrary capacities, fixed tick widths, player counts, geometry,
  durations and other title-derived restrictions from shared mechanisms.
  Use caller-selected types, parameters, policies and explicit budgets.
  Never truncate, silently clamp or narrow valid title requirements merely
  to satisfy an engine default. Preserve checked arithmetic, allocation
  admission, security invariants and real platform limits; make resource
  budgets explicit and adjustable within validated physical constraints.
  Test both existing behaviour and the newly supported requirements.
- Before beginning each new title, inspect ALL previously implemented games
  and apps, including unfinished ones, without exception. Review their code,
  tests and shared mechanisms for reuse. Extract genuinely reusable logic
  into its owning shared crate, migrate existing consumers and preserve
  regression coverage; never copy a previous title's implementation.
  Record the reviewed titles and reuse decisions in existing task evidence.
  Recheck reuse before completing each title; avoid speculative abstractions.
- Before implementing any new capability in any title, search the engine,
  IR and every existing game/app for equivalent mechanisms. If the only
  implementation is inside an older title, move it into the correct
  shared owner, migrate the older title and use that same implementation
  in the new title. Do not copy and later promise to deduplicate.
  Run the existing consumers' regression tests and the new consumer's
  tests; retain title-specific rules, art and tuning in the title.
  This applies to sprites, animation, menus, camera, audio, input,
  physics, networking, rendering, storage and all other shared behaviour.
- Request capabilities through the IR, never a concrete OS or SDK. A game
  asks for audio, a window, a renderer or a GPU preference. Device
  discovery, driver selection, admission and explicit fallback belong to
  the platform; no vendor-specific rendering logic in game code.
- Keep game logic independent of operating systems and original media
  formats. Platform adapters own OS access; importers own source formats.
- Enforce portable typed capability IR across all titles and tools. Only
  explicit platform adapter/composition directories may select targets,
  use native SDKs, access host services or translate device/vendor codes.
  Shared contracts own commands, events, results, errors and capability
  negotiation. Reuse existing types; do not create one universal VM.
  Cover input, haptics, graphics, presentation, audio, storage, network,
  timing and lifecycle. No native handles or OS branches leak into titles.
  Resolve the P1 platform-ir task before new title work. The portability
  gate rejects every detected occurrence; never use migration allowances.
  A new OS or backend must extend shared support, not fork title code.
  Test portable contracts with fake adapters and real supported targets.
- Contributor tools, including validate, build and download, obey the
  same IR-first boundary as titles. Move host process, storage, clock,
  environment and toolchain access to the platform capability owner.
  A development host may lack a capability; return a typed unsupported
  error rather than disabling the architecture check or hiding a target
  predicate. Tool-specific validation and asset logic stay in the tools.
- Use exactly one integration crate, `crates/platform`. Its only public
  integration API is `src/ir/`. All capabilities have directories with
  mod.rs roots, including security, hardware and identification. Software
  implementations belong under `src/software/<actual-os>/`, physical
  protocols under `src/hardware/<device-family>/`. No generic host or
  native directories. Names describe real responsibilities, e.g. toolchain.
  Title composition belongs in descriptive runtime modules, not generic
  native or host directories. Keep audio, input and graphics as distinct
  software capabilities; graphics owns scene rendering, not audio/input
  implementations. Physical protocols never access OS APIs or software
  modules. XInput is
  a Windows software API, not a device family.
- IR first: define neutral typed requests, results, errors, capabilities
  and conformance tests before implementing a backend. Share security
  admission before and after dispatch; keep native-only invariants in
  the OS implementation. Identify the system, architecture and devices
  through neutral descriptors. Consumers never branch by OS or vendor.
  Missing lawful device transport belongs in the OS adapter; unsupported
  capabilities remain explicit. ARM64 and x86-64 portability is mandatory.
  Reject native handles, pointer-width, alignment, endianness and ABI
  assumptions in portable code. Verify both architectures.
- Select backends centrally and keep implementation modules private.
  Enforce these rules with positive and negative validator tests.
  Reject every detected violation; no per-file migration allowances.
- Organise importers by source format and hardware family, not by title.
  Arcade ROM sets use a shared MAME adapter under the importer crate.
  Share ROM admission, region assembly, chip layouts and asset conversion.
  Title or board differences belong in small profiles or reusable strategies.
  Select the MAME machine and supported layout through a generic build/import
  interface; do not create title-specific importer commands or pipelines.
  MAME is a reference format and emulator, never a remake runtime.
  Verify supported profiles individually; do not assume universal compatibility.
- Import through the optional public `platform::ir::importer` capability:
  one import entry point, typed format/family/profile requests, shared
  canonical asset contracts and bounded transactions. The importer crate
  owns pure format decoders (MAME, NEO and subsequent formats), never
  host access or title runtimes. The IR delegates; do not duplicate
  format conversion in tools, engine or titles. Keep import dependencies
  out of runtime-only builds. Research diagnostics may inspect format
  adapters directly; production conversion must use the import IR.
- No reverse dependencies from shared crates to games, apps or tools.
  No title-to-title dependencies. Use a single Cargo workspace.
- Runtime Rust lives under `crates/`, titles under `crates/games/<id>/`
  or `crates/apps/<id>/`, and contributor tools under `tools/<id>/`.
- Keep tests inline in their corresponding Rust source file using
  `#[cfg(test)] mod tests`. Shared test fixtures may live in the owning crate.
  No separate integration test trees or mirrored test-path attributes.
- Only the root TODO.md is allowed. Keep title tasks and acceptance in
  the canonical docs/todo records, never a second roadmap in a crate.
- Keep optional runtime capabilities out of unrelated tool dependency
  graphs. Validate strict Clippy for the default workspace and each
  workspace member's independent all-feature graph; also run aggregate
  workspace tests and compilation with all features. Do not disable a
  lint or omit any member. Asset build must never activate secure LAN.
- All dependency versions and lints are owned by the root manifest.
  Member manifests inherit metadata, dependencies and lints. Each direct
  dependency has one canonical workspace declaration; reject aliases that
  duplicate package identities and member-local versions or sources.
  Only platform may declare target-specific dependency tables. All other
  manifests inherit neutral dependencies, never select a system or ABI.
  Use `cargo download dependencies` to retrieve locked source references.
- Every authored source file has a Boundary-Contract: ownership,
  forbidden responsibilities, inputs, outputs, effects, split and merge
  conditions, summary, description, usage and defaults.
- Remove dead or redundant code with evidence that behaviour is preserved.

## Behaviour and performance

- Ship modern free-play experiences only; do not require virtual coins,
  payment or a classic arcade mode. Use shared visible menus, immediate
  Start/Enter, character selection where appropriate, local multiplayer
  joining, pause, settings and exit. Keep historical payment mechanics
  as reference evidence, not shipped interaction. Verify real native
  input and visible gameplay in addition to finite synthetic smoke.
- Preserve gameplay, not technical debt. Test public behaviour, properties
  and invariants rather than internal calls or incidental constants.
- Use deterministic simulation independent of rendering FPS. Provide
  responsive rendering, uncapped FPS where supported and native controls.
- Share touch input and configure layouts per title.
- Share persistent display and input settings across titles. Use a responsive
  resizable viewport with aspect-preserving letterbox by default. Offer integer
  pixel scaling, fill/crop and explicit stretch as selectable display modes.
  Offer adaptive height-fit rendering: retain the original vertical field
  and uniform scale while expanding the camera horizontally to render real
  additional world content. Never stretch, crop, duplicate pixels or invent
  assets in this mode. Aspect-fit fallback is temporary for unsupported
  scenes. Manually adapt each title's camera, scene data and HUD to enable
  real extension where needed, preserving the original presentation mode.
  Keep unsupported scenes open until verified; never claim fallback is full
  adaptive support. Use aspect-fit for inherently narrow display ratios.
  Support windowed and fullscreen presentation; adapt HUD and touch layout.
  Keep gameplay coordinates and timing independent of window dimensions.
  Map pointer and touch coordinates through the actual presentation transform.
  Allow keyboard, mouse, controller and touch remapping with persistent
  per-player profiles, sensible defaults and a reset option. Default to one
  local player; for action games use WASD, Space to jump, and left mouse
  button for primary attack where applicable. Additional players join
  explicitly with independent, nonconflicting controls. Escape toggles
  the single local pause menu; do not require separate pause keys per
  keyboard seat. Keep control instructions and join bindings in menus,
  not over active gameplay. Do not hardcode title-specific menus.
- GPU acceleration by default where useful, including 2D. Use efficient
  CPU computation, parallelism and adaptive bounded memory caches.
- Use the best resource for each workload; never maximise utilisation
  for its own sake. Preserve quality and measure actual performance.
- Optional advanced graphics must not burden games that do not need them.

## Safety and distribution

- Use one preferred original edition per title; import assets locally.
  Never ship original executables, proprietary assets or leaked source.
- Keep non-distributable materials in ignored local reference storage.
- Treat all external media, saves, mods and network inputs as untrusted.
  Bound parsing and allocation; reject traversal and unsafe extraction.
- Offline single-player, portable saves, user-hosted multiplayer,
  no required commercial backend or monetisation. Use secure defaults.
- Primary target Linux x86-64; Android ARM64 and Web are intended.
  Windows and macOS are planned through shared adapters. Do not claim
  untested support.

## Local inputs and tools

- The maintainer places original game files in `.reference/games/<id>/`.
  Use the exact task ID (for example `simpsons-arcade`, `metal-slug`).
  Keep the selected official edition and its files together.
- Before starting a title, check that its required original media exists
  and is usable. If absent, record the task as blocked in its Evidence
  section, specifying the required edition and path. Keep its open record
  and dependencies intact; do not invent assets or claim completion.
- Recheck blocked titles when inputs become available. Inspect media format,
  edition and layout; do not require or store original-media SHA or checksum
  inventories. Attempt bounded extraction without a pre-known source digest.
  Preserve structural, path, size and resource safety checks. Keep generated
  asset integrity and build/cache identities separate from original admission.
  Record actual extraction results and any unsupported layout.
  Continue with other unblocked work without violating dependency order.
- Run `cargo download` to fetch current upstream source snapshots, or
  `cargo download craft-libs` to fetch just that repository. Tools and
  sources live under `.reference/tools/`. Each snapshot has a revision
  directory and a LATEST pointer. Use ripgrep to inspect them.
- Use lawful local reference emulators (MAME, and other appropriate
  systems) to investigate original behaviour and asset layouts. Fetch
  upstream source snapshots under `.reference/tools/` and use ripgrep
  (`rg`) to locate device definitions, memory maps, rendering, audio and
  timing implementations. Record the exact revision and independent
  observations in the canonical task. Observe and compare in controlled
  runs; do not execute originals during import. Do not copy incompatible
  emulator code or turn the remake into an emulator. Implement reusable
  mechanisms once in their correct shared owner.
- Download only tools needed for the current task. Official release
  archives are not automatically installed or executed. Use no Python
  in project-owned code; third-party reference files are excluded.
- The only designated reusable upstream project is
  `storytold/craft-libs`. Verify its licences and actual suitability.
- Reference inputs, tool downloads, caches and logs are local only.
  Never execute game files during import or distribute proprietary material.
  Use controlled reference emulation for lawful original gameplay tests.
