// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns: Read-only Ghidra dispatch fact extraction glue.
// - Must-Not: Infer selector values or modify the analysis project.
// - Allows:
//   - Inputs: Mapped program and explicit fact output path.
//   - Outputs: Bank-aware instruction ownership and predecessor chains.
//   - Side effects: Explicit private fact export.
// - Split-When: Another processor needs a different API adapter.
// - Merge-When: Common Ghidra fact export supersedes this adapter.
// - Summary: Export verified decoded instruction facts.
// - Description: The Rust resolver owns selector bounds.
// - Usage: Headless post-script ExportDispatchFacts.
// - Defaults: Stop a chain at a join, call, root or unknown predecessor.

//! Narrow Ghidra integration for dispatch facts.

pub(super) const SCRIPT: &str = r#"// Contributor-only read-only fact export.
// @category Games
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Instruction;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.*;
import com.google.gson.Gson;

public class ExportDispatchFacts extends GhidraScript {
  private record Site(String bank, String site, List<String> chain,
      List<String> addresses, String owner) {}
  private record Facts(String sha256, Map<String,String> owners,
      Set<String> code, List<Site> sites) {}
  private String bank(Address a) {
    if (!a.getAddressSpace().isOverlaySpace()) return "fixed";
    String name = a.getAddressSpace().getName();
    if (!name.matches("rom_bank_[0-9]{2}"))
      throw new IllegalStateException("unexpected overlay");
    return "bank-" + name.substring(9);
  }
  private String hex(Address a) {
    return String.format(Locale.ROOT, "%04x", a.getOffset());
  }
  private String key(Address a) { return bank(a) + ":" + hex(a); }
  private Address normalize(Address from, Address to) {
    long n = to.getOffset();
    if (n >= 0x8000 && n < 0xfff2) return toAddr(n);
    if (n >= 0x6000 && n < 0x8000 &&
        from.getAddressSpace().isOverlaySpace())
      return from.getAddressSpace().getAddress(n);
    return null;
  }
  private String digest() throws Exception {
    if (!currentProgram.getLanguageID().getIdAsString().equals(
        "Konami053248:BE:16:partial"))
      throw new IllegalStateException("processor");
    var memory = currentProgram.getMemory();
    var hash = MessageDigest.getInstance("SHA-256");
    byte[] page = new byte[8192];
    for (int i=0; i<64; i++) {
      var block = memory.getBlock(String.format(
          Locale.ROOT, "rom_bank_%02d", i));
      if (block == null || !block.isOverlay() || block.isWrite() ||
          !block.isExecute() || block.getSize() != page.length ||
          block.getStart().getOffset() != 0x6000 ||
          memory.getBytes(block.getStart(),page) != page.length)
        throw new IllegalStateException("bank map");
      hash.update(page);
      if (i>=60) {
        byte[] fixed = new byte[8192];
        if (memory.getBytes(toAddr(0x8000+(i-60)*8192),fixed)
              != fixed.length || !Arrays.equals(page,fixed))
          throw new IllegalStateException("fixed alias");
      }
    }
    return HexFormat.of().formatHex(hash.digest());
  }
  private String instructionText(Instruction ins) throws Exception {
    int op=ins.getBytes()[0] & 255;
    String text=ins.toString();
    if (ins.getFlowType().isConditional() &&
        Arrays.asList(ins.getFlows()).contains(ins.getFallThrough()))
      return "JOIN " + text;
    if (op==0x10 || op==0x11 || op==0x14 || op==0x15 ||
        op==0x1c || op==0x1d || op==0x24 || op==0x25 ||
        op==0x2c || op==0x2d || op==0x30 || op==0x31 ||
        op==0x34 || op==0x35 || op==0x42 || op==0x44 || op==0x46 ||
        op==0xb8) return "IMM " + text;
    return text;
  }
  @Override protected void run() throws Exception {
    if (getScriptArgs().length != 1)
      throw new IllegalArgumentException("expected private fact path");
    var digest = digest();
    var listing = currentProgram.getListing();
    var fm = currentProgram.getFunctionManager();
    var predecessors = new TreeMap<Address,Set<Address>>();
    var owners = new TreeMap<String,String>();
    var code = new TreeSet<String>();
    var computed = new ArrayList<Instruction>();
    var it = listing.getInstructions(true);
    int count = 0;
    while(it.hasNext()) {
      monitor.checkCancelled();
      if (++count > 1000000) throw new IllegalStateException("budget");
      var ins = it.next();
      var a = ins.getAddress();
      var owner = fm.getFunctionContaining(a);
      if (owner != null) owners.put(key(a),key(owner.getEntryPoint()));
      for (int i=0;i<ins.getLength();i++) code.add(key(a.add(i)));
      var targets = new ArrayList<>(List.of(ins.getFlows()));
      if (ins.getFallThrough()!=null) targets.add(ins.getFallThrough());
      for(var t:targets) {
        var target=normalize(a,t);
        if(target!=null) predecessors.computeIfAbsent(
            target,k->new TreeSet<>()).add(a);
      }
      if (owner!=null && ins.getFlowType().isComputed() &&
          (ins.getFlowType().isCall() || ins.getFlowType().isJump()))
        computed.add(ins);
    }
    var sites=new ArrayList<Site>();
    for(var ins:computed) {
      var a=ins.getAddress();
      var owner=fm.getFunctionContaining(a);
      var chain=new ArrayList<String>();
      var addresses=new ArrayList<String>();
      addresses.add(key(a));
      chain.add(instructionText(ins));
      for(int i=0;i<64 && !a.equals(owner.getEntryPoint());i++) {
        var pred=predecessors.getOrDefault(a,Set.of());
        if(pred.size()!=1) break;
        var prev=listing.getInstructionAt(pred.iterator().next());
        if(prev==null || prev.getFlowType().isCall() ||
            !owner.equals(fm.getFunctionContaining(prev.getAddress())) ||
            !a.equals(prev.getFallThrough())) break;
        chain.add(instructionText(prev));
        addresses.add(key(prev.getAddress()));
        a=prev.getAddress();
      }
      Collections.reverse(chain);
      Collections.reverse(addresses);
      sites.add(new Site(bank(ins.getAddress()),hex(ins.getAddress()),chain,
          addresses,key(owner.getEntryPoint())));
    }
    try(var writer=Files.newBufferedWriter(Path.of(getScriptArgs()[0]))) {
      new Gson().toJson(new Facts(digest,owners,code,sites),writer);
    }
    println("DISPATCH_FACTS instructions="+count+" sites="+sites.size());
  }
}"#;
