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
// Reference-model arithmetic vectors, independent of SLEIGH macros.
public class VerifyKonamiBinary extends VerifyKonami {
  private static final int[] BYTE = {
      0, 1, 15, 16, 0x55, 0x7f, 0x80, 0xfe, 0xff};
  private static final int[] WORD = {
      0, 1, 15, 0xff, 0x100, 0x7fff, 0x8000, 0xfffe, 0xffff};
  private static final int[] FLAGS = {0, 1, 0xfe, 0xff};

  int flags(int cc, int result, int sign, int clear) {
    return (cc & ~clear) | ((result & sign) != 0 ? 8 : 0)
        | ((result & (sign * 2 - 1)) == 0 ? 4 : 0);
  }

  int arithmeticFlags(
      int cc, int left, int right, int result, int sign, boolean half) {
    int out = flags(cc, result, sign, half ? 0x2f : 0x0f);
    if ((result & (sign * 2)) != 0)
      out |= 1;
    if (((left ^ right ^ result ^ (result >>> 1)) & sign) != 0)
      out |= 2;
    if (half && ((left ^ right ^ result) & 16) != 0)
      out |= 32;
    return out;
  }

  void fresh() {
    emulator = new ghidra.pcode.emu.PcodeEmulator(currentProgram.getLanguage());
    thread = emulator.newThread();
    code = 0x8000;
  }

  void byteCase(int family, int variant, int left, int right, int cc) {
    String dest = (variant & 1) == 0 ? "A" : "B";
    String other = (variant & 1) == 0 ? "B" : "A";
    boolean indexed = (variant & 2) != 0;
    int opcode = 0x10 + 4 * family + variant;
    reg(dest, left);
    reg(other, 0x69);
    reg("CC", cc);
    reg("X", 0x2000);
    mem(0x2000, right, 0xa5);
    int result = switch (family) {
      case 0 -> right;
      case 1 -> left + right;
      case 2 -> left + right + (cc & 1);
      case 3, 9 -> left - right;
      case 4 -> left - right - (cc & 1);
      case 5, 6 -> left & right;
      case 7 -> left ^ right;
      case 8 -> left | right;
      default -> throw new AssertionError();
    };
    int expected = switch (family) {
      case 1, 2 -> arithmeticFlags(cc, left, right, result, 128, true);
      case 3, 4, 9 -> arithmeticFlags(cc, left, right, result, 128, false);
      default -> flags(cc, result, 128, 14);
    };
    start(opcode, indexed ? 0x20 : right);
    step();
    fallthrough(2);
    String label = String.format(
        "op=%02x lhs=%02x rhs=%02x cc=%02x ", opcode, left, right, cc);
    eq(label + "destination", family == 6 || family == 9 ? left : result & 255,
        reg(dest));
    eq(label + "other accumulator", 0x69, reg(other));
    eq(label + "flags", expected, reg("CC"));
    eq(label + "index", indexed ? 0x2001 : 0x2000, reg("X"));
    eq(label + "operand unchanged", right, mem(0x2000));
    eq(label + "guard unchanged", 0xa5, mem(0x2001));
  }

  void wordCase(int family, boolean indexed, int left, int right, int cc) {
    String[] names = {"D", "X", "Y", "U", "S"};
    String dest = names[family < 10 ? family % 5 : 0];
    // Use a different register as the index to avoid aliasing this test.
    String index = dest.equals("X") ? "Y" : "X";
    int post = dest.equals("X") ? 0x31 : 0x21;
    int opcode = 0x40 + 2 * family + (indexed ? 1 : 0);
    reg(dest, left);
    reg(index, 0x2000);
    reg("CC", cc);
    mem(0x2000, right >>> 8, right & 255, 0xa5);
    int result = family < 5 ? right
        : family == 10      ? left + right
                            : left - right;
    int expected = family < 5
        ? flags(cc, result, 0x8000, 14)
        : arithmeticFlags(cc, left, right, result, 0x8000, false);
    if (indexed)
      start(opcode, post);
    else
      start(opcode, right >>> 8, right & 255);
    step();
    fallthrough(indexed ? 2 : 3);
    String label = String.format(
        "op=%02x lhs=%04x rhs=%04x cc=%02x ", opcode, left, right, cc);
    eq(label + "destination",
        family >= 5 && family < 10 ? left : result & 65535, reg(dest));
    eq(label + "flags", expected, reg("CC"));
    eq(label + "index", indexed ? 0x2002 : 0x2000, reg(index));
    eq(label + "operand unchanged", right, (mem(0x2000) << 8) | mem(0x2001));
    eq(label + "guard unchanged", 0xa5, mem(0x2002));
  }

  @Override
  protected void run() throws Exception {
    super.run();
    int before = checks;
    int bytes = 0;
    for (int family = 0; family < 10; family++)
      for (int variant = 0; variant < 4; variant++) {
        fresh();
        for (int left : BYTE)
          for (int right : BYTE)
            for (int cc : FLAGS) {
              byteCase(family, variant, left, right, cc);
              bytes++;
            }
      }
    int words = 0;
    for (int family = 0; family < 12; family++)
      for (boolean indexed : new boolean[] {false, true}) {
        fresh();
        for (int left : WORD)
          for (int right : WORD)
            for (int cc : FLAGS) {
              wordCase(family, indexed, left, right, cc);
              words++;
            }
      }
    println("PASS BINARY: " + bytes + " byte vectors; " + words
        + " word vectors; " + (checks - before) + " assertions");
  }
}
