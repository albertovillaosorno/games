# Agent instructions

## Workflow

- Use one agent initially. Follow the catalogue order and root README.
- Run `cargo validate` before completing work. Do not weaken checks,
  suppress diagnostics, or remove tests simply to obtain a passing result.
- Stay on main. Use Git and sign Conventional Commits with DCO.
  Run `cargo validate` before committing; push validated commits only.
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
- Keep game logic independent of operating systems and original media
  formats. Platform adapters own OS access; importers own source formats.
- No reverse dependencies from shared crates to games, apps or tools.
  No title-to-title dependencies. Use a single Cargo workspace.
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
