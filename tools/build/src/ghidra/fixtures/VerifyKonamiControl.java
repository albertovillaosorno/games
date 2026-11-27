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
// Remaining data, control-flow and bank-observation instructions.
public class VerifyKonamiControl extends VerifyKonamiRemaining {
  void storesAndLea() {
    int before = checks;
    String[] regs = {"A", "B", "D", "X", "Y", "U", "S"};
    int[] ops = {0x3a, 0x3b, 0x58, 0x59, 0x5a, 0x5b, 0x5c};
    fresh();
    for (int i = 0; i < regs.length; i++) {
      int width = i < 2 ? 1 : 2;
      int sign = width == 1 ? 128 : 32768;
      for (int value : new int[] {0, 1, sign - 1, sign, sign * 2 - 1})
        for (int cc : new int[] {0, 0xff}) {
          String index = regs[i].equals("X") ? "Y" : "X";
          int post = (regs[i].equals("X") ? 0x30 : 0x20) + width - 1;
          reg(regs[i], value);
          reg(index, 0x2000);
          reg("CC", cc);
          mem(0x1fff, 0xa5, 0, 0, 0xa5);
          start(ops[i], post);
          step();
          fallthrough(2);
          eq("store flags", flags(cc, value, sign, 14), reg("CC"));
          eq("store source", value, reg(regs[i]));
          eq("store index", 0x2000 + width, reg(index));
          eq("store value", value,
              width == 1 ? mem(0x2000) : (mem(0x2000) << 8) | mem(0x2001));
          eq("store low guard", 0xa5, mem(0x1fff));
          eq("store high guard", width == 1 ? 0 : 0xa5, mem(0x2000 + width));
        }
    }
    String[] wide = {"X", "Y", "U", "S"};
    for (int i = 0; i < wide.length; i++)
      for (int value : new int[] {0, 1, 0xffff})
        for (int cc : new int[] {0, 0xff}) {
          reg("CC", cc);
          start(8 + i, 7, value >>> 8, value & 255);
          step();
          fallthrough(4);
          eq("LEA address", value, reg(wide[i]));
          eq("LEA flags", i >= 2 ? cc : (cc & ~4) | (value == 0 ? 4 : 0),
              reg("CC"));
        }
    println("PASS STORE/LEA: 94 vectors; " + (checks - before) + " assertions");
  }

  void controls() {
    fresh();
    int before = checks;
    for (int cc : new int[] {0, 1, 0x55, 0xaa, 0xfe, 0xff})
      for (int value = 0; value < 256; value++) {
        if (code > 0xe000)
          fresh();
        reg("CC", cc);
        start(0x3c, value);
        step();
        fallthrough(2);
        eq("ANDCC", cc & value, reg("CC"));
        reg("CC", cc);
        start(0x3d, value);
        step();
        fallthrough(2);
        eq("ORCC", cc | value, reg("CC"));
      }
    fresh();
    for (int value = 0; value < 256; value++) {
      reg("CC", 0xa5);
      start(0x38, value);
      step();
      fallthrough(2);
      eq("SETLN immediate", value, reg("BANK"));
      eq("SETLN immediate flags", 0xa5, reg("CC"));
      reg("X", 0x2000);
      mem(0x2000, value);
      start(0x39, 0x20);
      step();
      fallthrough(2);
      eq("SETLN indexed", value, reg("BANK"));
      eq("SETLN indexed flags", 0xa5, reg("CC"));
      eq("SETLN indexed X", 0x2001, reg("X"));
      reg("B", value);
      reg("X", 0xff80);
      reg("CC", 0xa5);
      start(0xb0);
      step();
      fallthrough(1);
      eq("ABX X", (0xff80 + value) & 65535, reg("X"));
      eq("ABX flags", 0xa5, reg("CC"));
      reg("B", value);
      reg("CC", 0xf3);
      start(0xb2);
      step();
      fallthrough(1);
      int signed = (byte) value;
      eq("SEX D", signed& 65535, reg("D"));
      eq("SEX flags", flags(0xf3, signed, 32768, 12), reg("CC"));
    }
    println("PASS CONTROL: 4096 vectors; " + (checks - before) + " assertions");
  }

  void flow() {
    fresh();
    int before = checks;
    for (int op : new int[] {0xa8, 0xa9})
      for (int mode : new int[] {7, 0x0f, 0x26, 0x2e}) {
        reg("S", 0x2000);
        reg("X", 0x3000);
        reg("CC", 0xa5);
        mem(0x3000, 0x40, 0x00);
        int target = mode == 0x26 ? 0x3000 : 0x4000;
        if (mode == 7)
          start(op, mode, 0x40, 0);
        else if (mode == 0x0f)
          start(op, mode, 0x30, 0);
        else
          start(op, mode);
        int next = (int) instruction + (mode < 16 ? 4 : 2);
        step();
        eq("JMP/JSR target", target, thread.getCounter().getOffset());
        eq("JMP/JSR flags", 0xa5, reg("CC"));
        eq("JMP/JSR S", op == 0xa9 ? 0x1ffe : 0x2000, reg("S"));
        if (op == 0xa9) {
          eq("JSR return word", next, (mem(0x1ffe) << 8) | mem(0x1fff));
          start(0x8f);
          step();
          eq("RTS target", next, thread.getCounter().getOffset());
          eq("RTS S", 0x2000, reg("S"));
        }
      }
    for (int op : new int[] {0xaa, 0xab})
      for (int offset : new int[] {-128, -1, 0, 1, 127}) {
        reg("S", 0x2000);
        reg("CC", 0xa5);
        if (op == 0xaa)
          start(op, offset & 255);
        else
          start(op, (offset >>> 8) & 255, offset & 255);
        int next = (int) instruction + (op == 0xaa ? 2 : 3);
        step();
        eq("BSR target", next + offset, thread.getCounter().getOffset());
        eq("BSR S", 0x1ffe, reg("S"));
        eq("BSR return word", next, (mem(0x1ffe) << 8) | mem(0x1fff));
        eq("BSR flags", 0xa5, reg("CC"));
      }
    for (int op : new int[] {0xac, 0xad}) {
      int sign = op == 0xac ? 128 : 32768;
      String name = op == 0xac ? "B" : "X";
      for (int value : new int[] {0, 1, 2, sign, sign * 2 - 1})
        for (int cc : new int[] {0, 0xff}) {
          reg(name, value);
          reg("CC", cc);
          start(op, 0xf0);
          int result = (value - 1) & (sign * 2 - 1);
          int next = (int) instruction + 2;
          step();
          eq("decrement branch counter", result, reg(name));
          eq("decrement branch flags",
              flags(cc, result, sign, 14) | (value == sign ? 2 : 0), reg("CC"));
          eq("decrement branch target", next + (result != 0 ? -16 : 0),
              thread.getCounter().getOffset());
        }
    }
    reg("CC", 0xa5);
    reg("X", 0xffff);
    reg("Y", 0x3000);
    reg("U", 0);
    mem(0x3000, 0x12);
    start(0xb7);
    step();
    fallthrough(1);
    eq("MOVE source advance", 0x3001, reg("Y"));
    eq("MOVE destination wrap", 0, reg("X"));
    eq("MOVE count wrap", 0xffff, reg("U"));
    eq("MOVE byte", 0x12, mem(0xffff));
    eq("MOVE flags", 0xa5, reg("CC"));
    start(0xae);
    step();
    fallthrough(1);
    eq("NOP flags", 0xa5, reg("CC"));
    eq("NOP X", 0, reg("X"));
    eq("NOP U", 0xffff, reg("U"));
    println("PASS FLOW: " + (checks - before) + " assertions");
  }

  @Override
  protected void run() throws Exception {
    super.run();
    storesAndLea();
    controls();
    flow();
    reportCoverage();
  }
}
