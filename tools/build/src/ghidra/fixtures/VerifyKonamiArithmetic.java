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
// Ghidra integration test against independent synthetic arithmetic vectors.
public class VerifyKonamiArithmetic extends VerifyKonami {
  private static final int[] BYTE = {0, 1, 2, 0x55, 0x7f, 0x80, 0xaa, 0xff};
  private static final int[] WORD = {
      0, 1, 2, 0x1234, 0x7fff, 0x8000, 0x8001, 0xabcd, 0xffff};
  int nz(int cc, int value, int sign) {
    return (cc & ~0xc) | (value == 0 ? 4 : 0) | ((value & sign) != 0 ? 8 : 0);
  }
  int zc(int cc, long value, int carryBit) {
    return (cc & ~5) | (value == 0 ? 4 : 0) | ((value & carryBit) != 0 ? 1 : 0);
  }
  void decimalAdjust() {
    int cases = 0;
    for (int a = 0; a < 256; a++)
      for (int h : new int[] {0, 0x20})
        for (int c : new int[] {0, 1}) {
          int flags = 0xde | h | c;
          reg("A", a);
          reg("B", 0x77);
          reg("CC", flags);
          int sum = a + (((a & 15) > 9 || h != 0) ? 6 : 0)
              + ((a > 0x99 || c != 0) ? 0x60 : 0);
          int expected = sum & 255;
          int expectedCC =
              nz((flags & ~2) | (sum > 255 ? 1 : 0), expected, 0x80);
          start(0xb1);
          step();
          fallthrough(1);
          eq("DAA A", expected, reg("A"));
          eq("DAA B", 0x77, reg("B"));
          eq("DAA flags", expectedCC, reg("CC"));
          cases++;
        }
    println("PASS DAA " + cases + " exhaustive accumulator/carry combinations");
  }
  void multiplyDivide() {
    int flags = 0xff, cases = 0;
    for (int a : BYTE)
      for (int b : BYTE) {
        reg("A", a);
        reg("B", b);
        reg("CC", flags);
        int product = a * b;
        start(0xb3);
        step();
        fallthrough(1);
        eq("MUL D", product, reg("D"));
        eq("MUL flags", zc(flags, product, 0x80), reg("CC"));
        cases++;
      }
    for (int x : WORD)
      for (int y : WORD) {
        reg("X", x);
        reg("Y", y);
        reg("CC", flags);
        long product = (long) x * y;
        start(0xb4);
        step();
        fallthrough(1);
        eq("LMUL X", product >>> 16, reg("X"));
        eq("LMUL Y", product & 65535, reg("Y"));
        eq("LMUL flags", zc(flags, product, 0x8000), reg("CC"));
        cases++;
      }
    for (int x : WORD)
      for (int b : BYTE) {
        reg("X", x);
        reg("A", 0x69);
        reg("B", b);
        reg("CC", flags);
        int quotient = b == 0 ? 0 : x / b, rem = b == 0 ? 0 : x % b;
        start(0xb5);
        step();
        fallthrough(1);
        eq("DIVX X", quotient, reg("X"));
        eq("DIVX B", rem, reg("B"));
        eq("DIVX A", 0x69, reg("A"));
        eq("DIVX flags", zc(flags, quotient, 0x80), reg("CC"));
        cases++;
      }
    println("PASS MUL/LMUL/DIVX " + cases + " product/quotient vectors");
  }
  void absolute() {
    int flags = 0xff, cases = 0;
    for (int op : new int[] {0xcc, 0xcd})
      for (int old = 0; old < 256; old++) {
        String target = op == 0xcc ? "A" : "B";
        String other = op == 0xcc ? "B" : "A";
        reg(target, old);
        reg(other, 0x69);
        reg("CC", flags);
        int expected = Math.abs((byte) old) & 255;
        int cc = nz((flags & ~3) | (old >= 128 ? 1 : 0) | (old == 128 ? 2 : 0),
            expected, 0x80);
        start(op);
        step();
        fallthrough(1);
        eq("ABS8 value", expected, reg(target));
        eq("ABS8 other", 0x69, reg(other));
        eq("ABS8 flags", cc, reg("CC"));
        cases++;
      }
    for (int old : WORD) {
      reg("D", old);
      reg("CC", flags);
      int expected = Math.abs((short) old) & 65535;
      int cc =
          nz((flags & ~3) | (old >= 32768 ? 1 : 0) | (old == 32768 ? 2 : 0),
              expected, 0x8000);
      start(0xce);
      step();
      fallthrough(1);
      eq("ABS16 D", expected, reg("D"));
      eq("ABS16 flags", cc, reg("CC"));
      cases++;
    }
    println("PASS ABS " + cases + " signed boundary vectors");
  }
  int[] shifted(int family, int value, int cc, int count) {
    for (int i = 0; i < (count & 15); i++) {
      int old = value, in = cc & 1, out;
      switch (family) {
        case 0:
          out = old & 1;
          value = old >>> 1;
          break;
        case 1:
          out = old & 1;
          value = (old >>> 1) | (in << 15);
          break;
        case 2:
          out = old & 1;
          value = ((short) old >> 1) & 65535;
          break;
        case 3:
          out = old >>> 15;
          value = (old << 1) & 65535;
          cc = (cc & ~2) | (((old ^ value) & 0x8000) != 0 ? 2 : 0);
          break;
        case 4:
          out = old >>> 15;
          value = ((old << 1) | in) & 65535;
          cc &= ~2;
          break;
        default:
          throw new AssertionError();
      }
      cc = nz((cc & ~1) | out, value, 0x8000);
    }
    return new int[] {value, cc};
  }
  void shifts() {
    int cases = 0;
    int[] counts = {0, 1, 2, 7, 15, 16, 17, 0xff};
    for (int family = 0; family < 5; family++)
      for (int old : WORD)
        for (int count : counts)
          for (int carry : new int[] {0, 1}) {
            int flags = 0xfe | carry;
            int[] expected = shifted(family, old, flags, count);
            reg("D", old);
            reg("CC", flags);
            start(0xb8 + family * 2, count);
            step();
            fallthrough(2);
            eq("shift immediate D", expected[0], reg("D"));
            eq("shift immediate flags", expected[1], reg("CC"));
            cases++;
            reg("A", count);
            reg("B", 0x69);
            reg("X", 0x2200);
            reg("CC", flags);
            mem(0x2200, old >>> 8, old & 255, 0x77);
            start(0xb9 + family * 2, 0x21);
            step();
            fallthrough(2);
            eq("shift memory", expected[0], (mem(0x2200) << 8) | mem(0x2201));
            eq("shift memory guard", 0x77, mem(0x2202));
            eq("shift indexed X", 0x2202, reg("X"));
            eq("shift count A", count, reg("A"));
            eq("shift indexed flags", expected[1], reg("CC"));
            cases++;
          }
    reg("X", 0x2200);
    reg("CC", 2);
    mem(0x2200, 0x80, 0);
    start(0xa7, 0x26);
    step();
    fallthrough(2);
    eq("ROLW value", 0, (mem(0x2200) << 8) | mem(0x2201));
    eq("ROLW flags", 5, reg("CC"));
    reg("A", 0x80);
    reg("CC", 0);
    start(0xa0);
    step();
    fallthrough(1);
    eq("ROLA value", 0, reg("A"));
    eq("ROLA flags", 7, reg("CC"));
    println("PASS repeated shifts " + cases
        + " immediate/indexed vectors and two single rotates");
  }
  void fills() {
    for (int opcode : new int[] {0xcf, 0xd0}) {
      int width = opcode == 0xcf ? 1 : 2;
      reg("A", 0x12);
      reg("B", 0x34);
      reg("CC", 0xa5);
      reg("X", 0x2300);
      reg("U", 3);
      mem(0x2300, 0, 0, 0, 0, 0, 0, 0x77);
      start(opcode);
      step();
      fallthrough(1);
      eq("BSET X", 0x2300 + 3 * width, reg("X"));
      eq("BSET U", 0, reg("U"));
      for (int i = 0; i < 3 * width; i++)
        eq("BSET byte", width == 1 || i % 2 == 0 ? 0x12 : 0x34,
            mem(0x2300 + i));
      eq("BSET bound", width == 2 ? 0x77 : 0, mem(0x2300 + 3 * width));
      eq("BSET A", 0x12, reg("A"));
      eq("BSET B", 0x34, reg("B"));
      eq("BSET CC", 0xa5, reg("CC"));
      start(opcode);
      step();
      fallthrough(1);
      eq("BSET empty X", 0x2300 + 3 * width, reg("X"));
      reg("X", 0xffff);
      reg("U", 2);
      start(opcode);
      step();
      fallthrough(1);
      eq("BSET wrap X", (0xffff + 2 * width) & 65535, reg("X"));
      for (int i = 0; i < 2 * width; i++)
        eq("BSET wrap byte", width == 1 || i % 2 == 0 ? 0x12 : 0x34,
            mem((0xffff + i) & 65535));
    }
    println("PASS BSET/BSETW ordinary, empty and wrapping");
  }
  @Override
  protected void run() throws Exception {
    super.run();
    int before = checks;
    decimalAdjust();
    multiplyDivide();
    absolute();
    shifts();
    fills();
    println("PASS ADDITIONAL SYNTHETIC P-CODE: " + (checks - before)
        + " assertions");
    println("PASS TOTAL SYNTHETIC P-CODE: " + checks + " assertions");
    reportCoverage();
  }
}
