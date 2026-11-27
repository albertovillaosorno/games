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
// Authored synthetic instructions; no reference ROM data.
public class VerifyKonamiPc extends VerifyKonami {
  void pcTransfers() {
    int[] s = {0, 1, 2, 3, 5, 6, 7};
    int cases = 0;
    for (int op : new int[] {0x3e, 0x3f})
      for (int mode : new int[] {0, 0x80})
        for (int alias : new int[] {0, 8})
          for (int i = 0; i < 7; i++)
            for (int j = 0; j < 7; j++) {
              if (i != 6 && j != 6)
                continue;
              for (int k = 0; k < 6; k++) reg(regs[k], initial[k]);
              reg("CC", 0xa5);
              reg("DP", 0x55);
              start(op, mode | (s[j] << 4) | s[i] | alias);
              int next = (int) instruction + 2;
              int[] out = initial.clone();
              int source = i == 6 ? next : read(i),
                  target = j == 6 ? next : read(j);
              int expectedPc = next;
              if (mode == 0) {
                if (i == 6)
                  expectedPc = target;
                else
                  write(out, i, target);
              }
              if (j == 6)
                expectedPc = source;
              else
                write(out, j, source);
              step();
              eq("PC exchange/transfer control flow", expectedPc,
                  thread.getCounter().getOffset());
              for (int k = 0; k < 6; k++)
                eq("PC exchange/transfer " + regs[k], out[k], reg(regs[k]));
              eq("PC transfer flags", 0xa5, reg("CC"));
              eq("PC transfer DP", 0x55, reg("DP"));
              cases++;
            }
    println("PASS PC transfers " + cases + " encodings");
  }
  void pcAddressing() {
    int cases = 0;
    for (int mode : new int[] {0, 1, 2, 3, 4, 5, 6, 0x80, 0x81, 0x87})
      for (int indirect : new int[] {0, 8}) {
        int post = 0x70 | mode | indirect;
        reg("A", 0x20);
        reg("B", 0x81);
        reg("CC", 0xa5);
        start(0x12, post, 0x43, 0x21, 0x55);
        int p = (int) instruction, next = p + 2, effective = next;
        switch (mode) {
          case 0:
            next++;
            break;
          case 1:
            next += 2;
            break;
          case 2:
            effective--;
            next--;
            break;
          case 3:
            effective -= 2;
            next -= 2;
            break;
          case 4:
            next++;
            effective += 0x43;
            break;
          case 5:
            next += 2;
            effective = (effective + 0x4321) & 65535;
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
        boolean inline = effective >= p && effective <= p + 3;
        if (indirect != 0) {
          if (!inline)
            mem(effective, 0x55, 0x70);
          effective = (mem(effective) << 8) | mem((effective + 1) & 65535);
        }
        if (effective < p || effective > p + 4)
          mem(effective, 0x96);
        int expected = mem(effective);
        step();
        eq("PC indexed read mode " + Integer.toHexString(post), expected,
            reg("A"));
        eq("PC indexed next mode " + Integer.toHexString(post), next,
            thread.getCounter().getOffset());
        eq("PC indexed B", 0x81, reg("B"));
        cases++;
      }
    println("PASS PC indexed byte reads " + cases + " forms");
  }
  void pcWordAndFlow() {
    reg("D", 0x1234);
    start(0x58, 0x71, 0, 0);
    int p = (int) instruction;
    step();
    fallthrough(4);
    eq("PC postincrement store high", 0x12, mem(p + 2));
    eq("PC postincrement store low", 0x34, mem(p + 3));
    start(0x40 + 1, 0x75, 0xff, 0xf0);
    p = (int) instruction;
    mem(p + 2 - 16, 0x99, 0x45);
    step();
    fallthrough(4);
    eq("PC negative word offset", 0x9945, reg("D"));
    start(0x08, 0x74, 0x81);
    p = (int) instruction;
    step();
    fallthrough(3);
    eq("LEA PC negative byte displacement", p + 2 - 127, reg("X"));
    start(0xa8, 0x74, 0x10);
    p = (int) instruction;
    step();
    eq("PC relative JMP", p + 2 + 0x10, thread.getCounter().getOffset());
    reg("S", 0x2000);
    start(0xa9, 0x75, 0x00, 0x20);
    p = (int) instruction;
    step();
    eq("PC relative JSR", p + 2 + 0x20, thread.getCounter().getOffset());
    eq("PC relative JSR stack", 0x1ffe, reg("S"));
    eq("PC relative JSR return", p + 4, (mem(0x1ffe) << 8) | mem(0x1fff));
    reg("S", 0x2000);
    start(0xa9, 0x7a, 0x55);
    p = (int) instruction;
    step();
    eq("PC decrement indirect JSR target", 0x7a55,
        thread.getCounter().getOffset());
    eq("PC decrement JSR return", p + 1, (mem(0x1ffe) << 8) | mem(0x1fff));
    start(0x58, 0x72, 0xff);
    p = (int) instruction;
    reg("D", 0xabcd);
    step();
    eq("PC decrement store next", p + 1, thread.getCounter().getOffset());
    eq("PC decrement store word", 0xabcd, (mem(p + 1) << 8) | mem(p + 2));
    start(0x39, 0x73);
    p = (int) instruction;
    step();
    eq("PC decrement bank selection", 0x39, reg("BANK"));
    eq("PC decrement bank next", p, thread.getCounter().getOffset());
    println("PASS PC word access and flow");
  }
  @Override
  protected void run() throws Exception {
    super.run();
    pcTransfers();
    pcAddressing();
    pcWordAndFlow();
    println("PASS SYNTHETIC PC TOTAL: " + checks + " assertions");
  }
}
