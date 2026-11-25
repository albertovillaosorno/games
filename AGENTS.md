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
- Keep one TODO.md per title after onboarding; link to the canonical record.
  No additional planning bureaucracy.
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
- Before beginning each new title, inspect ALL previously implemented games
  and apps, including unfinished ones, without exception. Review their code,
  tests and shared mechanisms for reuse. Extract genuinely reusable logic
  into its owning shared crate, migrate existing consumers and preserve
  regression coverage; never copy a previous title's implementation.
  Record the reviewed titles and reuse decisions in existing task evidence.
  Recheck reuse before completing each title; avoid speculative abstractions.
- Keep game logic independent of operating systems and original media
  formats. Platform adapters own OS access; importers own source formats.
- Organise importers by source format and hardware family, not by title.
  Arcade ROM sets use a shared MAME adapter under the importer crate.
  Share ROM admission, region assembly, chip layouts and asset conversion.
  Title or board differences belong in small profiles or reusable strategies.
  Select the MAME machine and pinned revision through a generic build/import
  interface; do not create title-specific importer commands or pipelines.
  MAME is a reference format and emulator, never a remake runtime.
  Verify supported profiles individually; do not assume universal compatibility.
- No reverse dependencies from shared crates to games, apps or tools.
  No title-to-title dependencies. Use a single Cargo workspace.
- All authored Rust code lives under `crates/`, including tools and titles.
  Use crate directories named `game-<id>` or `app-<id>` for new titles.
- Keep tests inline in their corresponding Rust source file using
  `#[cfg(test)] mod tests`. Shared test fixtures may live in the owning crate.
  No root tests or tools directory and no mirrored test-path attributes.
- All dependency versions and lints are owned by the root manifest.
  Member manifests inherit metadata, dependencies and lints.
- Every authored source file has a Boundary-Contract: ownership,
  forbidden responsibilities, inputs, outputs, effects, split and merge
  conditions, summary, description, usage and defaults.
- Remove dead or redundant code with evidence that behaviour is preserved.

## Behaviour and performance

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
  assets in this mode. Use aspect-fit fallback when the scene or camera
  cannot safely support expansion, including narrow display ratios.
  Support windowed and fullscreen presentation; adapt HUD and touch layout.
  Keep gameplay coordinates and timing independent of window dimensions.
  Map pointer and touch coordinates through the actual presentation transform.
  Allow keyboard, controller and touch remapping with per-player profiles,
  sensible defaults and a reset option. Do not hardcode title-specific menus.
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
  Other platforms are community-driven. Do not claim untested support.

## Local inputs and tools

- The maintainer places original game files in `.reference/games/<id>/`.
  Use the exact task ID (for example `simpsons-arcade`, `metal-slug`).
  Keep the selected official edition and its files together.
- Before starting a title, check that its required original media exists
  and is usable. If absent, record the task as blocked in its Evidence
  section, specifying the required edition and path. Keep its open record
  and dependencies intact; do not invent assets or claim completion.
- Recheck blocked titles when inputs become available. Verify the revision,
  record its hash and clear the blocker only when the input is suitable.
  Continue with other unblocked work without violating dependency order.
- Run `cargo download` to fetch current upstream source snapshots, or
  `cargo download craft-libs` to fetch just that repository. Tools and
  sources live under `.reference/tools/`. Each snapshot has a revision
  directory and a LATEST pointer. Use ripgrep to inspect them.
- Download only tools needed for the current task. Official release
  archives are not automatically installed or executed. Use no Python
  in project-owned code; third-party reference files are excluded.
- The only designated reusable upstream project is
  `storytold/craft-libs`. Verify its licences and actual suitability.
- Reference inputs, tool downloads, caches and logs are local only.
  Never execute game files during import or distribute proprietary material.
  Use controlled reference emulation for lawful original gameplay tests.
