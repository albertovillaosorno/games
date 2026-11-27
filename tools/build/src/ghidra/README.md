# Ghidra processor generation

Contributor-only Rust tool. Generate a processor specification, never an
emulator or a dependency of an authored game. Ghidra requires SLEIGH and
XML; Rust emits these components from instruction and addressing tables.

## Generate and compile

From the repository root, with the official system Ghidra installed:

```sh
mkdir -p .temp/konami-extension
for part in ldefs pspec cspec slaspec; do
  cargo run --locked --quiet -p games-build -- ghidra konami "$part" \
    > ".temp/konami-extension/konami.$part"
done
/opt/ghidra/ghidra_12.2_PUBLIC/support/sleigh \
  .temp/konami-extension/konami.slaspec
```

Copy the four generated descriptions and compiled SLA into a dedicated
Ghidra processor module's `data/languages/` directory. Add an empty
`Module.manifest` at the module root. The local administrator installation
is `/opt/ghidra/ghidra_12.2_PUBLIC/Ghidra/Processors/Konami053248/`.
Restart Ghidra after changing its language components. The generator
does not write privileged paths or execute the analyzer.

Select `Konami053248:BE:16:partial` with compiler specification
`default`. Do not select stock 6809 for the Konami opcode map.

## Scope

The current model has constructors for 178 of the 197 distinct opcode
values documented by the pinned MAME Konami processor dispatch. It
includes register and memory arithmetic, flag updates, direct and
indirect indexed addressing for X/Y/U/S, loads/stores, relative branches,
calls, returns, stack masks and bank-selection register observations.
Interrupt return supports both partial and complete stack frames.
Block move handles zero counts, overlap and 16-bit address wrapping.
Register exchange and transfer preserve the distinct read and write
selectors, including the opcode and postbyte aliases.
This count is not exhaustive instruction semantics verification.

The extension does not model all PC-indexed modes, DP/PC register
transfers, decimal adjustments, special multiplication/division,
multi-bit shifts, block fills or all remaining instructions.
Unsupported forms are left undecodable. The calling convention is a
conservative provisional analysis convention. Hardware flag edge cases,
operand aliasing, all addressing variants and instruction effects need
independent differential tests before claiming full processor support.

The 16-bit CPU address space is not the ROM's physical address space.
The bank-selection observation does not map physical banks automatically.
Use separate verified regions or a bank-aware loader and record bank
identity in the private title ledger. Do not import the whole ROM as a
flat 16-bit executable or claim complete original function discovery.

## Evidence

Ghidra 12.2 compiled the generated SLEIGH specification. A private,
authored 39-byte synthetic fixture produced 16 expected instruction
mnemonics and lengths, including PSHS, PULS, LDA, ADDA, ADCA, LDX,
STA, BNE, BSR, JSR and RTS. Opcode 0x00 remained undecodable.
The headless decompiler completed the synthetic stack function.
This is not validation of every opcode or an original ROM campaign.

A second isolated Ghidra 12.2 p-code fixture exercised 288 supported
exchange/transfer encodings, partial and complete interrupt return
frames, and zero-count, ordinary, overlapping and wrapping block moves.
All 2,633 register, memory and control-flow assertions passed. This
tests the generated p-code, not just the existence of a constructor.
DP and PC transfer operands remain deliberately undecodable.

The opcode and addressing assignments were checked against the MAME
processor descriptions (revision
`89185ed286c7830b4e6daa647d843a929077ecd2`, BSD-3-Clause).
The generator implements its own partial p-code descriptions.
Add additional CPU profiles here only when a title needs an architecture
that Ghidra does not already support. Reuse stock 68000 and Z80 for
compatible Neo Geo programs after verifying the selected ROM layout.
