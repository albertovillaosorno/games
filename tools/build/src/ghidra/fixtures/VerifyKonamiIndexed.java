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
// Synthetic operand coverage for all documented non-PC indexed encodings.
public class VerifyKonamiIndexed extends VerifyKonamiPc {
  void allIndexed() {
    int cases = 0;
    String[] indexes = {"X", "Y", "U", "S"};
    int[] bases = {0x20, 0x30, 0x50, 0x60};
    for (int k = 0; k < 4; k++)
      for (int mode : new int[] {0, 1, 2, 3, 4, 5, 6, 0x80, 0x81, 0x87})
        for (int indirect : new int[] {0, 8}) {
          int post = bases[k] | mode | indirect;
          int index = 0x4000, effective = index, len = 2;
          switch (mode) {
            case 0:
              index++;
              break;
            case 1:
              index += 2;
              break;
            case 2:
              effective = --index;
              break;
            case 3:
              index -= 2;
              effective = index;
              break;
            case 4:
              effective -= 127;
              len = 3;
              break;
            case 5:
              effective -= 4096;
              effective &= 65535;
              len = 4;
              break;
            case 6:
              break;
            case 0x80:
              effective += 0x20;
              break;
            case 0x81:
              effective -= 127;
              break;
            case 0x87:
              effective = (effective + 0x2081) & 65535;
              break;
            default:
              throw new AssertionError();
          }
          if (indirect != 0) {
            mem(effective, 0x55, 0x70);
            effective = 0x5570;
          }
          mem(effective, 0x96, 0x21);
          reg("A", 0x20);
          reg("B", 0x81);
          reg(indexes[k], 0x4000);
          reg("CC", 0xa5);
          start(0x12, post, 0x81,
              0); // negative byte; 0x8100 word overridden below
          if (mode == 5)
            mem((int) instruction + 2, 0xf0, 0);
          step();
          fallthrough(len);
          eq("indexed A " + Integer.toHexString(post), 0x96, reg("A"));
          eq("indexed update " + Integer.toHexString(post), index,
              reg(indexes[k]));
          eq("indexed flags", 0xa9, reg("CC"));
          // Exercise each mode as a word store too (D also supplies A/B
          // offsets).
          reg("D", 0x2081);
          reg(indexes[k], 0x4000);
          if (indirect != 0) {
            int raw = 0x4000;
            switch (mode) {
              case 2:
                raw--;
                break;
              case 3:
                raw -= 2;
                break;
              case 4:
                raw -= 127;
                break;
              case 5:
                raw = (raw - 4096) & 65535;
                break;
              case 0x80:
                raw += 0x20;
                break;
              case 0x81:
                raw -= 127;
                break;
              case 0x87:
                raw = (raw + 0x2081) & 65535;
                break;
            }
            mem(raw, 0x55, 0x70);
          }
          mem(effective, 0x00, 0x00);
          start(0x58, post, 0x81, 0);
          if (mode == 5)
            mem((int) instruction + 2, 0xf0, 0);
          step();
          fallthrough(len);
          eq("indexed store " + Integer.toHexString(post), 0x2081,
              (mem(effective) << 8) | mem((effective + 1) & 65535));
          eq("indexed store update", index, reg(indexes[k]));
          cases++;
        }
    for (int post : new int[] {0x07, 0x0f, 0xc4, 0xcc}) {
      reg("DP", 0x40);
      reg("A", 0x20);
      reg("CC", 0xa5);
      boolean direct = post >= 0xc4, indirect = (post & 8) != 0;
      int raw = 0x4081, effective = raw;
      if (indirect) {
        mem(raw, 0x55, 0x70);
        effective = 0x5570;
      }
      mem(effective, 0x96, 0x21);
      if (direct)
        start(0x12, post, 0x81);
      else
        start(0x12, post, 0x40, 0x81);
      step();
      fallthrough(direct ? 3 : 4);
      eq("absolute/direct LDA", 0x96, reg("A"));
      reg("D", 0x2081);
      if (direct)
        start(0x58, post, 0x81);
      else
        start(0x58, post, 0x40, 0x81);
      step();
      fallthrough(direct ? 3 : 4);
      eq("absolute/direct STD", 0x2081,
          (mem(effective) << 8) | mem(effective + 1));
      cases++;
    }
    reg("X", 0x4000);
    mem(0x4000, 0x12, 0x34);
    start(0x43, 0x21);
    step();
    fallthrough(2);
    eq("LDX aliases indexed base", 0x1234, reg("X"));
    reg("X", 0x4000);
    start(0x59, 0x21);
    step();
    fallthrough(2);
    eq("STX aliases indexed base", 0x4002, (mem(0x4000) << 8) | mem(0x4001));
    println(
        "PASS non-PC indexed reads/stores " + cases + " forms and alias cases");
  }
  @Override
  protected void run() throws Exception {
    super.run();
    allIndexed();
    println("PASS SYNTHETIC INDEXED TOTAL: " + checks + " assertions");
  }
}
