# Architecture

## Principles

- Reuse first, extend second, implement once. One canonical implementation per
  shared capability; compose configuration and specialised strategies rather
  than duplicate engines.
- Preserve gameplay while improving frame pacing, resolution, controls,
  portability and maintainability. Separate simulation from rendering.
- GPU-accelerated rendering where appropriate, efficient CPU computation and
  adaptive RAM/VRAM budgets. Support NVIDIA, AMD and Intel through portable
  graphics APIs. Optional advanced graphics features only where beneficial; no
  mandatory vendor-specific hardware.
- Offline single-player, local saves, native controller support, configurable
  touch controls and self-hosted multiplayer where applicable. No
  advertisements, subscriptions or in-app purchases in our implementations.
- Behavioural tests protect observable gameplay, not incidental implementation
  details. Architecture and security contracts protect shared boundaries. Treat
  imported files, mods and network data as untrusted.
- User-supplied original media, one preferred source edition per title,
  automatic asset import. No original game binaries, extracted assets or leaked
  source are distributed. Original game code is not a runtime dependency.

## Extraction before reuse

When a later game needs behaviour implemented in an earlier title, search
the engine, IR and all previous titles first. Move genuinely reusable
mechanisms to their owning shared module, migrate the existing consumer,
and use that same implementation in the new game. Verify both consumers.
For example, portable sprite placement and touch geometry belong in the
engine; scene submission and physical device access belong to platform IR;
character-specific animation and reference assets belong to the title.
Request windows, audio and rendering through neutral capability contracts.
GPU selection is a platform preference, not a game-specific driver path.

## Unified integration architecture

The final integration owner is the single `crates/platform` crate. Its
canonical structure is `src/ir/`, `src/software/` and `src/hardware/`.
Only the IR is a public integration interface. Software and hardware
implementation modules are private; target selection and composition are
centralised. The former separate hardware crate design is superseded.

- IR owns portable commands, events, inputs, outputs, capability discovery,
  errors and common behaviour for OS and physical-device interaction.
  Extend and test the IR before implementing any concrete backend.
- Software owns OS-specific services and the physical transport to devices.
  Select an OS backend centrally, for example software/linux,
  software/windows or software/macos. Place host SDKs, native handles,
  driver access, target conditions and native performance work there.
- Hardware owns pure device-family and model-specific protocol translation.
  Examples include hardware/playstation/four.rs,
  hardware/nintendo/pro_one.rs, hardware/xbox, hardware/keyboard,
  hardware/mouse and hardware/logitech. It never accesses the operating
  system, native SDKs, host services or software implementation modules.
  XInput is a software/windows API, not a physical controller family.
- The IR hardware bridge owns neutral device discovery, transport requests,
  capability negotiation, controller/keyboard/pointer actions, sensors,
  wheels, feedback, connection lifetime and error semantics. Software
  performs host I/O; hardware interprets and encodes bounded device data.
- Identification is a shared service: software observes the machine and
  attached devices, the IR admits typed OS, architecture and device
  descriptors and selects compatible capabilities. Consumers use
  capabilities, not OS, vendor or model branches. Keep one registry;
  split identification into its own crate only if genuinely necessary.
- Centralise mandatory admission in ir/security: validate request and
  response shapes, device reports, resource budgets, permissions and
  lifetime constraints before and after dispatch. Preserve native-only
  invariants in software; never duplicate or bypass shared checks.
- Implement missing device transport or drivers in the appropriate
  software adapter where lawful, safe and technically feasible. Do not
  bypass platform permissions, invent device support or silently report
  success. Unsupported capabilities remain explicit and testable.
- AArch64 and x86-64 are mandatory for all portable code. Use explicit
  numeric widths, checked conversions, explicit wire endianness and
  architecture-independent serialization. No native ABI, pointer-size,
  alignment, instruction-set or OS assumptions outside software.
  Verify compilation and deterministic behaviour on both architectures;
  other targets are optional and must not weaken these requirements.
- Mandatory validator tests cover directory ownership, private backends,
  IR-only public imports, host API isolation, OS/vendor references,
  dependency direction and architecture portability. Reject new split
  integration crates and reduce legacy allowances to zero. Cover negative
  fixtures as well as accepted neutral code. Verify actual target and
  device conformance before claiming platform support.

## Capability ownership and naming

Title runtime composition coordinates gameplay, presentation and IR requests.
It does not implement native device, operating-system or graphics APIs.
Software backends own separate audio, input and graphics capabilities.
Rendering a scene is the responsibility of graphics/scene_renderer.rs;
audio playback and input acquisition are not graphics responsibilities.
Use the actual capability name, not an ambiguous native or host directory.
Shared window and device lifetime may be coordinated through explicit
contracts rather than hidden coupling between implementations.

Only the root TODO.md indexes the roadmap. Every title records its
requirements, progress, verification and blockers in its canonical
docs/todo task record. Do not add another TODO.md inside a crate, tool
or documentation directory.

The canonical migration remains the
[P1 platform task](../todo/completed/contracts/platform-ir.mdc).
No new title work may bypass that prerequisite.

## Shared engine and platform contracts

The Rust library now has fixed-step scheduling and scripted-input replay,
logical keyboard, mouse, controller and touch routing, viewport and
localisation policies, and display frame-pacing configuration. These modules
have synthetic behaviour tests and do not require an installed original game.

The library also provides local-first multiplayer session rules, byte-bounded
message/schema validation, archive-entry checks, integrity-checked save
envelopes and constant-memory performance accounting. The native platform
adapter provides bounded mutual TLS 1.3 and explicit player enrolment.
On Linux, descriptor-relative archive staging enforces private files,
no-overwrite creation, exact payload lengths and collision rejection. Physical
LAN verification and credential provisioning remain pending. Mod sandboxing and
complete title acceptance remain open.

See the [platform capability interface](platform-ir.md) for the intended
portable adapter boundary. The existing title host is Linux-specific.

The Linux graphics adapter presents bounded RGBA frames and ordered sprite
scenes using persistent accelerated textures,
nearest-neighbor scaling and aspect-preserving letterboxing.
The shared asset crate defines deterministic versioned manifests, SHA-256
content identities and checked RGBA8, PCM16 and indexed-tile descriptors.
The importer admits an immutable, size-bounded ZIP source and checks each
flat archive member before conversion without an original-media digest.
The conversion interface assembles a canonical manifest with deduplicated,
validated payload staging requests.
The Linux platform stages content by SHA-256 and completes a private bundle
only when its exact payload set matches the manifest. GNU/Linux supports
atomic publication without replacement and native verified read-back.
