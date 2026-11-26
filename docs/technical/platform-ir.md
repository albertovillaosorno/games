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

## Canonical crate layout

The single integration crate is `crates/platform`. Its only public
integration surface is the directory-based IR. Concrete software and
physical device families remain private.

```text
crates/platform/src/
  lib.rs
  ir/
    mod.rs
    security/
    hardware/
    graphics/
    input/
    audio/
    lan/
    measurement/
    process/
    storage/
    importer/
    toolchain/
    ui/
  software/
    linux/
    windows/
    macos/
    android/
    web/
  hardware/
    playstation/
    xbox/
    nintendo/
    keyboard/
    mouse/
    logitech/
```

Use descriptive capability names, never generic host or native directories.
Only actual OS names identify software backends. Audio playback and
input/device admission are separate capabilities beside graphics, not
subdirectories of graphics. Ordered atlas drawing belongs to the scene
renderer. Games own their session and title runtime composition, not
native host or device implementations. Hardware translates
physical device data without OS APIs, SDKs or software module imports.
XInput belongs to the Windows software adapter. The IR owns neutral
commands, events, machine identification and device transport contracts.

Mandatory admission in `ir/security/` validates typed requests, budgets,
permissions and results. Native security and performance invariants remain
in the concrete OS implementation. Never bypass shared admission.
Both ARM64 and x86-64 are required; portable contracts use explicit
numeric widths and checked conversions.

Every new integration capability follows the same order:

1. Search the owning IR for an existing contract.
1. Extend or define the neutral contract and test its semantics.
1. Implement the Linux adapter against that contract.
1. Implement and verify additional backends against the same tests.

Select backends centrally through compile-time configuration and explicit
capability negotiation. Games and tools must not branch on operating
systems, hardware vendors or backend identities. Ordinary control flow
is not prohibited. Optional features and unavailable hardware must be
reported explicitly without corrupting deterministic gameplay.

The validator must enforce the final directory boundaries rather than
exempt whole crates, including IR-to-backend imports, aliases and direct
SDK dependencies. Remove the existing migration inventory without adding
exceptions. Actual cross-platform compatibility still requires each
backend to satisfy the capability contract and pass conformance tests.

## Migration and verification

Linux presentation, input, audio and authenticated LAN expose portable
handles over private backend ports. Shared measurement uses a process-local
monotonic clock contract. Launch arguments and status output share a process
port. Bounded snapshots, directory observations, saves, preferences, private
staging and publication receipts use storage contracts. Contributor validation
uses typed toolchain requests, environment observations and a private lease,
log and cache-state port. Contributor test fixtures use shared storage and
process ports; title host scheduling uses the shared cooperative wait.
Input feedback passes through shared capability and focus admission before
dispatch to the selected device adapter. Do not infer additional native
support from a portable type or successful compilation of the featureless
core.

Exercise typed contracts with deterministic in-memory adapters before adding
new native backends. Verify event ordering, resource lifetime, capability
rejection and equivalent simulation outcomes. Test actual device, rendering,
audio and storage behaviour separately on each supported target.

Linux x86-64 remains the primary implementation target. Android ARM64,
WebAssembly, Windows and macOS are planned and require runtime and packaging
adapters. Every target must use the same portable contracts, not a
separate game engine. No native Windows runtime is verified here.

The browser save-slot backend uses the shared verified envelope with a
bounded, lowercase hexadecimal representation in origin-local storage. It
accepts a dedicated ASCII namespace instead of a native directory. Slots
are published as single values, and the receipt explicitly reports that
crash durability cannot be established. Browser quota, disabled storage and
unavailable window contexts can reject access. This does not implement
browser asset discovery, graphics, input or a title runtime.

The [P1 isolation task](../todo/completed/contracts/platform-ir.mdc) is the
canonical migration and acceptance record. The original validator inventory
recorded 253 occurrences in 27 files. The migration baseline has been removed:
the preflight rejects every detected occurrence outside its authorized
adapter boundaries, including on cached validation runs. Its parsed checks
are not complete semantic Rust name resolution or arbitrary macro expansion.
The six portable core target checks do not establish native runtime
conformance. See the contract gate task linked from the canonical roadmap
for validation evidence and the remaining migration limits.
