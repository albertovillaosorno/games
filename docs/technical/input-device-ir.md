# Physical input device integration

## Ownership

One platform crate owns integration. The public interface is
`crates/platform/src/ir/`. Software backends own OS-specific input and
output, permissions and transport. Physical hardware implementations
translate device reports and encode commands without OS access.

```text
crates/platform/src/
  ir/
    hardware/
    security/
  software/
    linux/
    windows/
  hardware/
    playstation/four.rs
    nintendo/pro_one.rs
    xbox/
    keyboard/
    mouse/
    logitech/
```

The IR hardware bridge owns discovery, attachment, neutral events,
capability negotiation, sensors, axes, wheels, feedback and lifetime.
Keep physical families independent of the software implementations.
XInput is a Windows API and belongs to software/windows, not hardware.

## Contract first

Extend typed IR requests, results, errors and tests before implementing
a device or OS backend. Validate device reports, permissions, bounds,
feedback duration and capability availability through shared security
admission. Software enforces additional OS-specific security invariants.
No generic host or native directories. Unavailable device capabilities
remain explicit and never alter deterministic gameplay.

## Verification

Require positive and negative fixtures for directory ownership, IR-only
consumer access, direct OS and SDK access in physical protocols, private
backend modules, and shared security and device bridge module roots.
Verify ARM64 and x86-64 portability, device conformance with fake
transports, and actual physical inputs, remapping, disconnect, focus loss
and feedback cessation. New device families require real protocol
evidence; no unverified support claims.
