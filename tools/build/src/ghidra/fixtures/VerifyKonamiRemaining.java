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
// Unary and stack semantic evidence.
public class VerifyKonamiRemaining extends VerifyKonamiBinary {
  private static final int[] BOUNDARIES = {
      0, 1, 15, 16, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff};

  int unary(int kind, int value, int sign, int cc) {
    return switch (kind) {
      case 0 -> 0;
      case 1 -> ~value;
      case 2 -> -value;
      case 3 -> value + 1;
      case 4 -> value - 1;
      case 5 -> value;
      case 6 -> value >>> 1;
      case 7 -> (value >>> 1) | ((cc & 1) * sign);
      case 8 -> (value >>> 1) | (value & sign);
      case 9 -> value << 1;
      case 10 -> (value << 1) | (cc & 1);
      default -> throw new AssertionError();
    };
  }

  int unaryFlags(int kind, int value, int sign, int cc, int result) {
    return switch (kind) {
      case 0 -> (cc & ~15) | 4;
      case 1 -> flags(cc, result, sign, 15) | 1;
      case 2 -> arithmeticFlags(cc, 0, value, result, sign, false);
      case 3, 4 -> {
        int changed = arithmeticFlags(cc, value, 1, result, sign, false);
        yield sign == 128 ? (changed & ~1) | (cc & 1) : changed;
      }
      case 5 -> flags(cc, result, sign, 14);
      case 6, 7, 8 -> flags(cc, result, sign, 13) | (value & 1);
      case 9 -> arithmeticFlags(cc, value, value, result, sign, false);
      case 10 ->
        flags(cc, result, sign, 15) | ((value & sign) != 0 ? 1 : 0)
            | (sign == 128 && ((value ^ result) & sign) != 0 ? 2 : 0);
      default -> throw new AssertionError();
    };
  }

  void unaryCase(
      int opcode, int kind, String dest, int value, int sign, int cc) {
    boolean memory = dest.equals("memory");
    int width = sign == 128 ? 1 : 2;
    if (memory) {
      reg("X", 0x2000);
      if (width == 1)
        mem(0x2000, value, 0xa5);
      else
        mem(0x2000, value >>> 8, value & 255, 0xa5);
      start(opcode, width == 1 ? 0x20 : 0x21);
    } else {
      reg(dest, value);
      start(opcode);
    }
    reg("CC", cc);
    int result = unary(kind, value, sign, cc);
    int expected = unaryFlags(kind, value, sign, cc, result);
    step();
    fallthrough(memory ? 2 : 1);
    String label =
        String.format("unary %02x value=%04x cc=%02x ", opcode, value, cc);
    long observed = memory
        ? (width == 1 ? mem(0x2000) : (mem(0x2000) << 8) | mem(0x2001))
        : reg(dest);
    eq(label + "result", result & (sign * 2 - 1), observed);
    eq(label + "flags", expected, reg("CC"));
    if (memory) {
      eq(label + "index", 0x2000 + width, reg("X"));
      eq(label + "guard", 0xa5, mem(0x2000 + width));
    }
  }

  void unaryCases() {
    int before = checks;
    int count = 0;
    int[] bases = {
        0x80, 0x83, 0x86, 0x89, 0x8c, 0x90, 0x93, 0x96, 0x99, 0x9c, 0xa0};
    String[] names = {"A", "B", "memory"};
    for (int kind = 0; kind < bases.length; kind++)
      for (int variant = 0; variant < 3; variant++) {
        fresh();
        for (int value = 0; value < 256; value++)
          for (int cc : new int[] {0, 1, 0xfe, 0xff}) {
            unaryCase(
                bases[kind] + variant, kind, names[variant], value, 128, cc);
            count++;
          }
      }
    for (int kind = 6; kind <= 10; kind++) {
      fresh();
      for (int value : BOUNDARIES)
        for (int cc : new int[] {0, 1, 0xfe, 0xff}) {
          unaryCase(0xa3 + kind - 6, kind, "memory", value, 32768, cc);
          count++;
        }
    }
    int[] wideKinds = {0, 2, 3, 4, 5};
    for (int i = 0; i < wideKinds.length; i++)
      for (int variant = 0; variant < 2; variant++) {
        fresh();
        for (int value : BOUNDARIES)
          for (int cc : new int[] {0, 1, 0xfe, 0xff}) {
            unaryCase(0xc2 + 2 * i + variant, wideKinds[i],
                variant == 0 ? "D" : "memory", value, 32768, cc);
            count++;
          }
      }
    println("PASS UNARY: " + count + " vectors; " + (checks - before)
        + " assertions");
  }

  void stackCases() {
    int before = checks;
    for (int variant = 0; variant < 4; variant++) {
      fresh();
      boolean pull = variant >= 2;
      String stack = (variant & 1) == 0 ? "S" : "U";
      String other = stack.equals("S") ? "U" : "S";
      String[] names = {"CC", "A", "B", "DP", "X", "Y", other, "PC"};
      int[] values = {0xa5, 0x12, 0x34, 0x56, 0x789a, 0xbcde, 0x1357, 0x2468};
      for (int mask = 0; mask < 256; mask++) {
        int count = 0;
        for (int i = 0; i < 8; i++)
          if ((mask & (1 << i)) != 0)
            count += i < 4 ? 1 : 2;
        int sp = 0x2000;
        int first = pull ? sp : sp - count;
        reg(stack, sp);
        start(0x0c + variant, mask);
        int next = (int) instruction + 2;
        for (int i = 0; i < 7; i++) reg(names[i], pull ? 0 : values[i]);
        int cursor = first;
        mem(first - 1, 0x5a);
        mem(first + count, 0x5a);
        for (int i = 0; i < 8; i++)
          if ((mask & (1 << i)) != 0) {
            int value = i == 7 && !pull ? next : values[i];
            if (i >= 4)
              mem(cursor++, pull ? value >>> 8 : 0);
            mem(cursor++, pull ? value & 255 : 0);
          }
        step();
        eq("stack pointer", pull ? sp + count : first, reg(stack));
        eq("stack next PC", pull && (mask & 128) != 0 ? values[7] : next,
            thread.getCounter().getOffset());
        eq("lower guard", 0x5a, mem(first - 1));
        eq("upper guard", 0x5a, mem(first + count));
        cursor = first;
        for (int i = 0; i < 8; i++) {
          boolean present = (mask & (1 << i)) != 0;
          int value = i == 7 && !pull ? next : values[i];
          if (i < 7)
            eq("stack register " + names[i], pull && !present ? 0 : value,
                reg(names[i]));
          if (present) {
            if (i >= 4)
              eq("stack high byte", value >>> 8, mem(cursor++));
            eq("stack low byte", value & 255, mem(cursor++));
          }
        }
      }
    }
    println("PASS STACK: 1024 masks; " + (checks - before) + " assertions");
  }

  @Override
  protected void run() throws Exception {
    super.run();
    unaryCases();
    stackCases();
  }
}
