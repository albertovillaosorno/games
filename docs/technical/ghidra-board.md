# Ghidra board memory mapping

The contributor-only Rust command generates a Ghidra script for a
selected board's memory map. It does not emulate bank switching, run
original software or become a dependency of the authored game.

## Simpsons Arcade

Use the installed Konami processor language and a private, already
assembled 512 KiB physical main-program image for the chosen edition.
The mapper deliberately accepts raw program bytes, not a ROM archive;
archive handling and original selection stay outside the Ghidra script.
The fixed CPU window must exactly match the last 32 KiB of that image.

From the repository root, use a new private scratch directory that
does not start with a dot, and choose a fresh Ghidra project:

```sh
scratch=$(mktemp -d)
mkdir "$scratch/project"
cargo run --locked --quiet -p games-build -- ghidra-board simpsons \
  > "$scratch/MapKonamiBoard.java"
dd if="$PHYSICAL_ROM" of="$scratch/fixed.bin" bs=8192 skip=60 count=4
analyzeHeadless "$scratch/project" simpsons-main \
  -import "$scratch/fixed.bin" -loader BinaryLoader \
  -loader-baseAddr 0x8000 \
  -processor Konami053248:BE:16:partial -cspec default \
  -noanalysis -scriptPath "$scratch" \
  -postScript MapKonamiBoard.java "$PHYSICAL_ROM"
```

Keep authoritative analysis projects in private per-title storage, not
disposable scratch. Record the exact ROM edition and printed SHA-256
alongside analysis results. The successful mapper emits 64 distinct
8 KiB overlays at CPU 0x6000..0x7fff, a fixed 32 KiB block at
0x8000..0xffff, RAM and volatile device views. Banks 60 through 63
alias physical bytes in the fixed region. The default address space
does not contain an arbitrary selected bank. Hardware bank context,
interrupt state and indirect calls still need static analysis.

The mapper validates the complete image length and existing fixed
bytes before mutation, checks every overlay's readback, and rolls back
its Ghidra transaction on a mapping failure. It rejects already mapped
programs. It does not certify the edition's authenticity or resolve
bank-dependent calls automatically.

## Verification

Ghidra 12.2 headless loaded a synthetic 524,288-byte image; all 64
overlays, their address ranges, sizes and permissions, the fixed
region, RAM and volatile device view were independently checked.
Short, oversized and fixed-mismatch images were refused before mapping;
the initial one-block program remained unchanged. No original game
content is included in the generator or its fixtures.
