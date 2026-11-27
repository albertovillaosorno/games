// Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier: Apache-2.0
// Boundary-Contract:
// - Owns: Synthetic Ghidra p-code conformance fixtures.
// - Must-Not: Use original media, runtime code or private analysis.
// - Allows:
//   - Inputs: Installed Konami language and synthetic instructions.
//   - Outputs: Assertions and explicit coverage counts.
//   - Side effects: Temporary emulator state only.
// - Split-When: Independent instruction families need separate fixtures.
// - Merge-When: Shared fixture helpers remove duplicated setup.
// - Summary: Exercise generated processor p-code.
// - Description: Ghidra Java API glue for the Rust-generated extension.
// - Usage: Run using the headless analyzer post-script interface.
// - Defaults: Fail on a mismatched value or language.
// Synthetic Ghidra integration check; no original program bytes.
// Independent branch truth tables from the pinned Konami dispatch.
public class VerifyKonamiBranches extends VerifyKonami {
  boolean taken(int selector, int cc) {
    boolean c = (cc & 1) != 0;
    boolean v = (cc & 2) != 0;
    boolean z = (cc & 4) != 0;
    boolean n = (cc & 8) != 0;
    boolean result = switch (selector & 7) {
      case 0 -> true;
      case 1 -> !c && !z;
      case 2 -> !c;
      case 3 -> !z;
      case 4 -> !v;
      case 5 -> !n;
      case 6 -> n == v;
      case 7 -> !z&& n == v;
      default -> throw new AssertionError();
    };
    return (selector & 16) == 0 ? result : !result;
  }

  void branch(int opcode, int cc, int displacement) {
    boolean wide = (opcode & 8) != 0;
    reg("CC", cc);
    if (wide) {
      start(opcode, (displacement >>> 8) & 255, displacement & 255);
    } else {
      start(opcode, displacement & 255);
    }
    long next = instruction + (wide ? 3 : 2);
    long expected = (next + (taken(opcode, cc) ? displacement : 0)) & 65535;
    step();
    eq("branch target", expected, thread.getCounter().getOffset());
    eq("branch flags unchanged", cc, reg("CC"));
  }

  @Override
  protected void run() throws Exception {
    super.run();
    int before = checks;
    int cases = 0;
    for (int opcode = 0x60; opcode <= 0x7f; opcode++) {
      boolean wide = (opcode & 8) != 0;
      int[] offsets = {0, 1, -1, wide ? -32768 : -128, wide ? 32767 : 127};
      for (int flags = 0; flags < 16; flags++) {
        for (int displacement : offsets) {
          branch(opcode, 0xa0 | flags, displacement);
          cases++;
        }
      }
    }
    code = 0xff00;
    branch(0x68, 0, 32767);
    code = 0x100;
    branch(0x68, 0, -32768);
    cases += 2;
    eq("branch vector count", 2562, cases);
    println("PASS BRANCH TRUTH TABLE: " + cases + " vectors; "
        + (checks - before) + " assertions");
    reportCoverage();
  }
}
