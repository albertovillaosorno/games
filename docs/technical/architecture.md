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
