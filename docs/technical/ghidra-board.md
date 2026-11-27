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

## Bounded bank-context discovery

The same Rust command emits the Ghidra bank-context traversal script:

```sh
cargo run --locked --quiet -p games-build -- ghidra-board simpsons flow \
  > "$scratch/DiscoverBankFlow.java"
```

Add `-postScript DiscoverBankFlow.java 200000` after the mapper in the
headless command. The last argument is an explicit maximum number of
address/context states (1 to 1,000,000). A budget failure rolls back the
instruction admission transaction. The script verifies the processor,
fixed block and all overlays before traversal; it seeds only hardware
interrupt and reset vectors and does not scan arbitrary data. It admits
one bounded nonoverlapping instruction at a time and reports candidate
entry identities, direct edges, selector observations and unknown
contexts in deterministic order.

The pinned MAME board masks the selector to six bits. An immediate SETLN
sets a known bank; an indexed selector is unknown. A call invalidates
the return-side bank context without an established callee summary.
A contiguous decoded immediate-B/store-direct/SETLN-direct/pull chain
with the same direct-page byte produces a separate conditional edge,
assuming DP is in RAM and the complete chain executes. The unknown edge
remains. No unresolved path is assigned an arbitrary physical bank.
Logical targets in the banked window become physical overlay addresses
only with a known or explicitly conditional context.

A fully synthetic image covering every selector and bank is generated
as hex, with no original media:

```sh
scratch=$(mktemp -d)
mkdir "$scratch/project"
cargo run --locked --quiet -p games-build -- ghidra-board simpsons \
  > "$scratch/MapKonamiBoard.java"
cargo run --locked --quiet -p games-build -- ghidra-board simpsons flow \
  > "$scratch/DiscoverBankFlow.java"
cargo run --locked --quiet -p games-build -- ghidra-board simpsons fixture \
  > "$scratch/physical.hex"
xxd -r -p "$scratch/physical.hex" "$scratch/physical.bin"
dd if="$scratch/physical.bin" of="$scratch/fixed.bin" bs=8192 skip=60 \
  count=4 status=none
for name in first second; do
  analyzeHeadless "$scratch/project" "$name" \
    -import "$scratch/fixed.bin" -loader BinaryLoader \
    -loader-baseAddr 0x8000 -processor Konami053248:BE:16:partial \
    -cspec default -noanalysis -scriptPath "$scratch" \
    -postScript MapKonamiBoard.java "$scratch/physical.bin" \
    -postScript DiscoverBankFlow.java 5000 > "$scratch/$name.log" 2>&1
  ! grep -q 'ERROR' "$scratch/$name.log" || exit 1
  grep -q 'MAPPED_OK=67 UNIQUE_PHYSICAL_BYTES=524288' \
    "$scratch/$name.log" || exit 1
  grep -q 'FLOW_OK states=595 roots=67 edges=791' \
    "$scratch/$name.log" || exit 1
  sed -nE 's/.*> (FLOW_[A-Z]+ .*) +\(GhidraScript\).*/\1/p' \
    "$scratch/$name.log" > "$scratch/$name.flow"
done
cmp "$scratch/first.flow" "$scratch/second.flow" || exit 1
awk '/^FLOW_SELECTOR/ {split($3,a,"=");split($4,b,"=");print a[2],b[2]}' \
  "$scratch/first.flow" | LC_ALL=C sort -u > "$scratch/selectors"
awk 'BEGIN {for(i=0;i<256;i++)printf "%02x %02d\n",i,i%64}' \
  > "$scratch/expected"
cmp "$scratch/selectors" "$scratch/expected" || exit 1
test "$(grep -c 'call-if-dp-ram-chain' "$scratch/first.flow")" = 1
```

Two fresh Ghidra 12.2 projects produced identical traversal reports.
All 256 selector values map to their exact six-bit physical bank;
all 64 distinct banks have a candidate entry at logical 0x6000.
Two different bank-end entries remain distinct: the first bank's
one-byte final instruction falls through to fixed ROM, and the final
physical bank's vector-alias byte remains an explicit decode gap.
A matching direct-page chain produces exactly one conditional edge;
a differing offset does not. Dynamic selectors retain unknown edges.

The local World four-player program was also checked by MAME 0.289
ROM verification. The physical region assembled in pinned board order
matched the previously admitted fixed image. A fresh headless traversal
produced 2,170 address/context states, 62 candidate roots, 2,430 edges,
13 bank-overlay candidate roots and 47 unresolved observations. Six
conditional direct-page call edges were recovered. The report is
discovery evidence, not an authoritative function inventory. Computed
dispatch tables, caller summaries, function-body ownership, exact
edition admission and canonical ledger synchronization remain open.
