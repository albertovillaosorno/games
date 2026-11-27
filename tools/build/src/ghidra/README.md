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

The current model has constructors for all 197 distinct opcode
values documented by the pinned MAME Konami processor dispatch. It
includes register and memory arithmetic, flag updates, direct and
indirect indexed addressing for X/Y/U/S/PC, loads/stores, relative branches,
calls, returns, stack masks and bank-selection register observations.
Interrupt return supports both partial and complete stack frames.
Block move handles zero counts, overlap and 16-bit address wrapping.
Register exchange and transfer preserve the distinct read and write
selectors, including the opcode and postbyte aliases.
This admits each documented opcode value and all 104 documented indexed
postbyte values, not exhaustive instruction semantics verification.

The extension leaves DP exchange/transfer operands undecodable because
the hidden low byte of the register is not characterized. It does not
model hardware cycle timing. PC predecrement forms can overlap the
current instruction; Ghidra function discovery and return-site analysis
remain provisional for such unusual control flows. Division by
zero follows the pinned emulator model; the physical result is unknown.
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
DP transfer operands remain deliberately undecodable.

A third isolated fixture exercised all accumulator and half-carry/carry
combinations for decimal adjustment, 217 multiplication and division
vectors, 521 signed absolute-value cases, 1,440 immediate and indexed
repeated shifts and rotations, two single rotates, and byte/word fills.
The additional 13,570 assertions passed, for 16,203 total when the
earlier transfer, interrupt and block-move cases were rerun. Word
rotate-left follows the pinned emulator model that clears overflow;
byte rotate-left retains its distinct overflow calculation.

An indexed-operand fixture tested 104 additional PC transfer/exchange
encodings, 20 direct and indirect PC-indexed byte reads, PC word access
and control flow, and all 84 non-PC indexed forms with byte loads and
word stores. All 4,226 assertions passed, including the prior transfer,
interrupt and block-move checks. The fixture also exposed an incorrect
nested indirect operand export: pointers are now explicitly loaded before
export. The 16,203-assertion arithmetic suite passed again.

The opcode and addressing assignments were checked against the MAME
processor descriptions (revision
`89185ed286c7830b4e6daa647d843a929077ecd2`, BSD-3-Clause).
The generator implements its own partial p-code descriptions.
Add additional CPU profiles here only when a title needs an architecture
that Ghidra does not already support. Reuse stock 68000 and Z80 for
compatible Neo Geo programs after verifying the selected ROM layout.

## Reproducible semantic fixtures

The Java files in `fixtures/` are synthetic tests of the Ghidra p-code
API, not a second processor implementation. The expectations are authored
from the pinned reference model; they do not execute MAME. Generate the
four descriptions and compile the SLA as above, then verify all five
files against the installed module before running the tests:

```sh
installed=/opt/ghidra/ghidra_12.2_PUBLIC/Ghidra/Processors/Konami053248
for part in ldefs pspec cspec slaspec sla; do
  cmp ".temp/konami-extension/konami.$part" \
    "$installed/data/languages/konami.$part" || exit 1
done
scratch=$(mktemp -d)
mkdir "$scratch/project"
printf '\\000' > "$scratch/synthetic.bin"
analyzeHeadless "$scratch/project" konami-check \
  -import "$scratch/synthetic.bin" \
  -processor Konami053248:BE:16:partial -cspec default -noanalysis \
  -scriptPath "$PWD/tools/build/src/ghidra/fixtures" \
  -postScript VerifyKonamiIndexed.java \
  -postScript VerifyKonamiArithmetic.java \
  -postScript VerifyKonamiBranches.java > "$scratch/check.log" 2>&1
! grep -q 'ERROR' "$scratch/check.log" || exit 1
grep -q 'PASS SYNTHETIC INDEXED TOTAL: 4226 assertions' \
  "$scratch/check.log" || exit 1
grep -q 'PASS TOTAL SYNTHETIC P-CODE: 16203 assertions' \
  "$scratch/check.log" || exit 1
grep -q 'PASS BRANCH TRUTH TABLE: 2562 vectors; 5125 assertions' \
  "$scratch/check.log" || exit 1
```

Require the explicit markers as well as headless success: a failed script
can otherwise be reported only in the log. No original media is needed.
The three runs share 2,633 base assertions; do not sum those duplicates.

| Category | Admitted semantic evidence |
| --- | --- |
| EXG/TFR | 392 of 512 encodings (all non-DP selector pairs and aliases) |
| Indexed addressing | 104 of 104 postbytes for byte loads |
| Indexed word stores | 84 non-PC forms and selected PC forms |
| Conditional branches | 32 of 32 opcodes; all 16 NZVC combinations |
| Branch displacements | Five signed boundary cases per flag combination |
| Branch target wrapping | Two explicit 16-bit overflow/underflow cases |
| DAA | All 1,024 A/H/C combinations |
| MUL/LMUL/DIVX | 217 deterministic product and quotient vectors |
| ABS | 521 byte and word signed vectors |
| Repeated shifts | 1,440 immediate and indexed vectors |
| RTI/BMOVE/BSET | Partial and full frames; zero, overlap, wrap as applicable |

The installed module passed these fixtures on 2026-11-27. The reference
model explicitly leaves DP transfer selector 4 unimplemented: its read
returns zero and its write has no effect. This is not a characterization
of the physical register or its hidden byte. All 120 DP-dependent
encodings remain undecodable; no hardware equivalence is asserted.
The full opcode-by-addressing-mode semantic matrix, exact boundary
instruction fetch and live differential execution against MAME remain
open. The branch wrap cases keep instruction bytes inside the address
space; a Ghidra p-code decode at 0xfffc failed in this environment.
