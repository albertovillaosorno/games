# Portable platform capability interface

## Target contract

Keep game rules, fixed-step input and canonical media independent of operating
systems, native window handles, device vendors and backend-specific commands.
Expose typed Rust capability interfaces for requests, events, observations and
errors. These are in-process contracts, not a new wire protocol, bytecode or
universal virtual machine.

The composition boundary selects adapters centrally for the target and
available capabilities. Individual titles describe their requirements and
compose shared services; they must not duplicate operating-system backends.
Unsupported capabilities are explicit rather than inferred from an operating
system name or silently replaced with behaviour that changes gameplay.

- Input: logical actions and device attachment, loss, axis and touch events;
  platform adapters translate physical identities and device-specific APIs.
- Graphics: bounded canonical textures, draw commands and presentation geometry;
  adapters own native surfaces, resource lifetimes and hardware acceleration.
- Audio: canonical PCM, mixing and bounded playback; adapters own device access
  and report the formats and output options they support.
- Haptics: negotiate supported effect classes and limits before requesting
  feedback. Unsupported feedback must not change simulation outcomes.
- Storage: portable settings, saves and verified asset bundles; adapters own
  paths, native permissions and atomic publication where supported.
- Networking and services: shared session and message contracts; adapters own
  transport, native services and lifecycle. Offline functionality stays local.

Introduce neutral contracts only where a concrete consumer requires them.
Reuse existing engine types and shared service boundaries rather than adding
parallel interfaces for each platform. Keep target selection and native APIs
inside adapter or composition modules; do not propagate backend-specific
identifiers into portable game rules.

## Migration and verification

Current Linux presentation, input, audio and storage implementations provide
working adapter boundaries, but the complete title host still depends on
Linux-specific types. Existing native paths remain transitional until their
shared equivalents are implemented and exercised. Do not claim that renaming
or wrapping these modules establishes portable runtime support.

Exercise typed contracts with deterministic in-memory adapters before adding
new native backends. Verify event ordering, resource lifetime, capability
rejection and equivalent simulation outcomes. Test actual device, rendering,
audio and storage behaviour separately on each supported target.

Linux x86-64 remains the primary implementation target. Android ARM64,
WebAssembly and Windows are planned and require runtime and packaging
adapters. Every target must use the same portable contracts, not a
separate game engine. No native Windows runtime is verified here.
