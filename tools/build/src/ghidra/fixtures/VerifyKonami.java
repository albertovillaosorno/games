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
import ghidra.app.script.GhidraScript;
import ghidra.pcode.emu.PcodeEmulator;
import ghidra.pcode.emu.PcodeThread;
import ghidra.program.model.address.Address;

public class VerifyKonami extends GhidraScript {
  PcodeEmulator emulator;
  PcodeThread<byte[]> thread;
  long code = 0x8000;
  long instruction;
  int checks;
  int opcode;
  final java.util.SortedSet<Integer> covered = new java.util.TreeSet<>();
  String[] regs = {"A", "B", "X", "Y", "U", "S"};
  int[] selectors = {0, 1, 2, 3, 5, 6};
  int[] initial = {0x12, 0x34, 0x4567, 0x89ab, 0xcdef, 0x2468};

  Address at(long x) {
    return currentProgram.getAddressFactory()
        .getDefaultAddressSpace()
        .getAddress(x);
  }
  void eq(String name, long want, long got) {
    checks++;
    if (want != got)
      throw new AssertionError(name + " expected 0x" + Long.toHexString(want)
          + " got 0x" + Long.toHexString(got));
  }
  void reg(String name, int v) {
    var r = currentProgram.getRegister(name);
    thread.getState().setVar(
        r, thread.getArithmetic().fromConst(v, r.getMinimumByteSize()));
  }
  long reg(String name) {
    return thread.getState()
        .inspectRegisterValue(currentProgram.getRegister(name))
        .getUnsignedValue()
        .longValue();
  }
  void mem(int addr, int... values) {
    byte[] bytes = new byte[values.length];
    for (int i = 0; i < bytes.length; i++) bytes[i] = (byte) values[i];
    thread.getState().setConcrete(at(addr), bytes);
  }
  int mem(int addr) {
    return thread.getState().inspectConcrete(at(addr), 1)[0] & 0xff;
  }
  void start(int... bytes) {
    opcode = bytes[0];
    instruction = code;
    code += 4;
    mem((int) instruction, bytes);
    thread.overrideCounter(at(instruction));
  }
  void step() {
    thread.stepInstruction();
    covered.add(opcode);
  }
  void reportCoverage() {
    var codes = new java.util.StringJoiner(" ");
    for (int op : covered) codes.add(String.format("%02x", op));
    println("COVERED OPCODES: " + codes);
  }
  void fallthrough(int n) {
    eq("fallthrough", instruction + n, thread.getCounter().getOffset());
  }
  int read(int i) {
    if (i == 0)
      return initial[0] | 0x1000;
    if (i == 1)
      return (initial[0] << 8) | initial[1];
    return initial[i];
  }
  void write(int[] out, int i, int v) {
    out[i] = i < 2 ? v & 0xff : v;
  }
  void transfer() {
    int cases = 0;
    for (int op : new int[] {0x3e, 0x3f})
      for (int mode : new int[] {0, 0x80})
        for (int alias : new int[] {0, 8})
          for (int i = 0; i < regs.length; i++)
            for (int j = 0; j < regs.length; j++) {
              for (int k = 0; k < regs.length; k++) reg(regs[k], initial[k]);
              reg("CC", 0xa5);
              reg("DP", 0x55);
              int[] expected = initial.clone();
              if (mode == 0)
                write(expected, i, read(j));
              write(expected, j, read(i));
              start(op, mode | (selectors[j] << 4) | selectors[i] | alias);
              step();
              fallthrough(2);
              for (int k = 0; k < regs.length; k++)
                eq(String.format("%02x %02x %s", op,
                       mode | (selectors[j] << 4) | selectors[i] | alias,
                       regs[k]),
                    expected[k], reg(regs[k]));
              eq("unchanged flags", 0xa5, reg("CC"));
              eq("unchanged DP", 0x55, reg("DP"));
              cases++;
            }
    println("PASS transfer/exchange " + cases + " encodings");
  }
  void rti() {
    for (int k = 0; k < regs.length; k++) reg(regs[k], initial[k]);
    reg("DP", 0x55);
    reg("S", 0x1000);
    mem(0x1000, 0x01, 0xab, 0xcd);
    start(0x9f);
    step();
    eq("partial return PC", 0xabcd, thread.getCounter().getOffset());
    eq("partial return S", 0x1003, reg("S"));
    eq("partial return CC", 1, reg("CC"));
    for (int k = 0; k < 5; k++)
      eq("partial unchanged " + regs[k], initial[k], reg(regs[k]));
    eq("partial unchanged DP", 0x55, reg("DP"));
    reg("S", 0x1000);
    mem(0x1000, 0x81, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0x24,
        0x68, 0xac);
    start(0x9f);
    step();
    eq("full return PC", 0x68ac, thread.getCounter().getOffset());
    eq("full return S", 0x100c, reg("S"));
    eq("full return CC", 0x81, reg("CC"));
    eq("full return A", 0xfe, reg("A"));
    eq("full return B", 0xdc, reg("B"));
    eq("full return DP", 0xba, reg("DP"));
    eq("full return X", 0x9876, reg("X"));
    eq("full return Y", 0x5432, reg("Y"));
    eq("full return U", 0x1024, reg("U"));
    println("PASS RTI partial and complete frames");
  }
  void block() {
    reg("CC", 0xa5);
    reg("X", 0x2000);
    reg("Y", 0x3000);
    reg("U", 3);
    mem(0x3000, 0x12, 0x34, 0x56);
    mem(0x2000, 0, 0, 0, 0x77);
    start(0xb6);
    step();
    fallthrough(1);
    eq("BMOVE X", 0x2003, reg("X"));
    eq("BMOVE Y", 0x3003, reg("Y"));
    eq("BMOVE U", 0, reg("U"));
    eq("BMOVE CC", 0xa5, reg("CC"));
    eq("BMOVE byte 0", 0x12, mem(0x2000));
    eq("BMOVE byte 1", 0x34, mem(0x2001));
    eq("BMOVE byte 2", 0x56, mem(0x2002));
    eq("BMOVE bound", 0x77, mem(0x2003));
    start(0xb6);
    step();
    fallthrough(1);
    eq("BMOVE empty X", 0x2003, reg("X"));
    eq("BMOVE empty Y", 0x3003, reg("Y"));
    eq("BMOVE empty U", 0, reg("U"));
    eq("BMOVE empty guard", 0x77, mem(0x2003));
    reg("X", 0x3001);
    reg("Y", 0x3000);
    reg("U", 3);
    mem(0x3000, 0x12, 0x34, 0x56, 0x78);
    start(0xb6);
    step();
    fallthrough(1);
    for (int i = 0; i < 4; i++) eq("BMOVE overlap " + i, 0x12, mem(0x3000 + i));
    reg("X", 0xffff);
    reg("Y", 0x3000);
    reg("U", 2);
    mem(0x3000, 0x5a, 0xa5);
    start(0xb6);
    step();
    fallthrough(1);
    eq("BMOVE wrap X", 1, reg("X"));
    eq("BMOVE wrap last", 0x5a, mem(0xffff));
    eq("BMOVE wrap zero", 0xa5, mem(0));
    println("PASS BMOVE zero, normal, overlapping and wrapping");
  }
  @Override
  protected void run() throws Exception {
    if (!currentProgram.getLanguageID().toString().equals(
            "Konami053248:BE:16:partial"))
      throw new AssertionError(
          "wrong processor " + currentProgram.getLanguageID());
    emulator = new PcodeEmulator(currentProgram.getLanguage());
    thread = emulator.newThread();
    transfer();
    rti();
    block();
    println("PASS SYNTHETIC P-CODE: " + checks + " assertions");
  }
}
