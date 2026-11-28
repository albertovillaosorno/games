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
// - Owns:
//   - Contributor-only board memory layout and Ghidra script generation.
// - Must-Not:
//   - Execute original programs, access host files or modify runtime code.
// - Allows:
//   - Inputs: Admitted static board profile.
//   - Outputs: Deterministic Ghidra Java integration source.
//   - Side effects: Explicit contributor output through the toolchain IR.
// - Split-When:
//   - Additional boards require different mapping mechanisms.
// - Merge-When:
//   - Shared board generation machinery owns equivalent logic.
// - Summary:
//   - Emit an explicit bank-aware memory mapper for Ghidra.
// - Description:
//   - Preserve physical bank identity without inventing bank transitions.
// - Usage:
//   - games-build ghidra-board simpsons
// - Defaults:
//   - Reject unknown boards and extra arguments.

//! Board-specific Ghidra integration script generation.

mod bounds;
mod domain;
mod facts;
mod reconcile;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::num::NonZeroU64;

use games_platform::ir::toolchain::write_line;

const FLOW: &str = r#"// Copyright (c) 2026 Alberto Villa Osorno.
// SPDX-License-Identifier: Apache-2.0
// Boundary-Contract:
// - Owns: Bounded static bank-context traversal in a mapped Ghidra program.
// - Must-Not: Execute ROMs, guess bank contexts or overwrite defined data.
// - Allows:
//   - Inputs: Mapped bank layout, vector roots and explicit visit budget.
//   - Outputs: Deterministic candidate roots, edges and unresolved contexts.
//   - Side effects: Admits only nonoverlapping decoded instructions.
// - Split-When: Another board requires a different bank-selection policy.
// - Merge-When: Shared mapping and flow policy removes duplicate logic.
// - Summary: Recover direct static bank transitions conservatively.
// - Description: Ghidra Java integration, not a title runtime.
// - Usage: Headless post-script after MapKonamiBoard.
// - Defaults: Unknown bank stays unknown; calls invalidate return context.
// @category Games
import ghidra.app.script.GhidraScript;
import ghidra.app.util.PseudoDisassembler;
import ghidra.program.disassemble.Disassembler;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;
import java.util.ArrayDeque;
import java.util.HashSet;
import java.util.Locale;
import java.util.TreeSet;

public class DiscoverBankFlow extends GhidraScript {
  private static final int UNKNOWN = -1;
  private static final int PAGE = 0x2000;
  private static final int BANKS = 64;
  private static final int WINDOW = 0x6000;
  private static final int FIXED = 0x8000;
  private static final int VECTORS = 0xfff2;
  private final MemoryBlock[] banks = new MemoryBlock[BANKS];
  private final ArrayDeque<State> pending = new ArrayDeque<>();
  private final HashSet<State> seen = new HashSet<>();
  private final TreeSet<String> edges = new TreeSet<>();
  private final TreeSet<String> roots = new TreeSet<>();
  private final TreeSet<String> unknown = new TreeSet<>();
  private final TreeSet<String> selectors = new TreeSet<>();
  private record State(Address address, int bank) {}
  private PseudoDisassembler pseudo;
  private Disassembler disassembler;

  private int byteAt(Address address) throws Exception {
    return currentProgram.getMemory().getByte(address) & 255;
  }

  private void verifyLayout() {
    if (!currentProgram.getLanguageID().getIdAsString().equals(
            "Konami053248:BE:16:partial"))
      throw new IllegalArgumentException("incorrect processor");
    var memory = currentProgram.getMemory();
    var fixed = memory.getBlock("fixed_rom");
    if (fixed == null || !fixed.isExecute() || fixed.isOverlay()
        || fixed.getStart().getOffset() != FIXED || fixed.getSize() != 0x8000
        || memory.getBlock(toAddr(WINDOW)) != null)
      throw new IllegalStateException("invalid fixed bank map");
    for (int bank = 0; bank < BANKS; bank++) {
      var block =
          memory.getBlock(String.format(Locale.ROOT, "rom_bank_%02d", bank));
      if (block == null || !block.isOverlay() || !block.isExecute()
          || block.getStart().getOffset() != WINDOW || block.getSize() != PAGE)
        throw new IllegalStateException("invalid physical bank " + bank);
      banks[bank] = block;
    }
  }

  private String context(int bank) {
    return bank == UNKNOWN ? "?" : String.format(Locale.ROOT, "%02d", bank);
  }

  private String identity(Address address) {
    if (!address.getAddressSpace().isOverlaySpace())
      return String.format(Locale.ROOT, "fixed:%04x", address.getOffset());
    for (int bank = 0; bank < BANKS; bank++)
      if (banks[bank].contains(address))
        return String.format(
            Locale.ROOT, "bank%02d:%04x", bank, address.getOffset());
    throw new IllegalStateException("unrecognized overlay");
  }

  private void enqueue(
      String from, long logical, int bank, String kind, boolean root) {
    Address address;
    if (logical >= FIXED && logical < VECTORS) {
      address = toAddr(logical);
    } else if (logical >= WINDOW && logical < FIXED && bank != UNKNOWN) {
      address = banks[bank].getStart().add(logical - WINDOW);
    } else {
      unknown.add(String.format(Locale.ROOT, "%s %s logical=%04x context=%s",
          from, kind, logical, context(bank)));
      return;
    }
    String target = identity(address);
    edges.add(from + " " + kind + " " + target + " context=" + context(bank));
    if (root)
      roots.add(target);
    pending.add(new State(address, bank));
  }

  private Instruction decode(State state) throws Exception {
    Address address = state.address();
    var listing = currentProgram.getListing();
    Instruction existing = listing.getInstructionAt(address);
    if (existing != null) {
      var block = currentProgram.getMemory().getBlock(address);
      long bound = block.isOverlay() ? FIXED : VECTORS;
      return address.getOffset() + existing.getLength() <= bound ? existing
                                                                 : null;
    }
    if (listing.getDefinedDataContaining(address) != null
        || listing.getInstructionContaining(address) != null)
      return null;
    var block = currentProgram.getMemory().getBlock(address);
    int length;
    try {
      length = pseudo.disassemble(address).getLength();
    } catch (Exception error) {
      return null;
    }
    if (length < 1
        || address.getOffset() + length > (block.isOverlay() ? FIXED : VECTORS))
      return null;
    Address end = address.addNoWrap(length - 1);
    if (!block.contains(end))
      return null;
    for (int offset = 1; offset < length; offset++) {
      Address part = address.add(offset);
      if (listing.getDefinedDataContaining(part) != null
          || listing.getInstructionContaining(part) != null)
        return null;
    }
    disassembler.disassemble(address, new AddressSet(address, end), false);
    return listing.getInstructionAt(address);
  }

  // A prior reviewed board pattern stores a constant in direct-page RAM,
  // selects it, and restores B/CC. The chain is conditional on DP in RAM
  // and execution through every instruction; never replace unknown context.
  private int conditionalBank(Address call) throws Exception {
    if (call.getAddressSpace().isOverlaySpace()
        || call.getOffset() < FIXED + 10)
      return UNKNOWN;
    var listing = currentProgram.getListing();
    Address load = call.subtract(10);
    Address store = call.subtract(8);
    Address setter = call.subtract(5);
    Address pull = call.subtract(2);
    if (byteAt(load) != 0x11 || byteAt(store) != 0x3b
        || byteAt(store.add(1)) != 0xc4 || byteAt(setter) != 0x39
        || byteAt(setter.add(1)) != 0xc4
        || byteAt(store.add(2)) != byteAt(setter.add(2)) || byteAt(pull) != 0x0e
        || byteAt(pull.add(1)) != 5)
      return UNKNOWN;
    Address[] sequence = {load, store, setter, pull, call};
    for (int i = 0; i < sequence.length - 1; i++) {
      var instruction = listing.getInstructionAt(sequence[i]);
      if (instruction == null
          || !sequence[i + 1].equals(instruction.getFallThrough()))
        return UNKNOWN;
    }
    return byteAt(load.add(1)) & 63;
  }
  private void visit(State state) throws Exception {
    String source = identity(state.address()) + "@" + context(state.bank());
    Instruction instruction = decode(state);
    if (instruction == null) {
      unknown.add(source + " decode");
      return;
    }
    int bank = state.bank();
    int opcode = byteAt(state.address());
    if (opcode == 0x38) {
      if (instruction.getLength() != 2)
        throw new IllegalStateException("invalid immediate bank selector");
      int selector = byteAt(state.address().add(1));
      bank = selector & 63;
      selectors.add(source
          + String.format(
              Locale.ROOT, " selector=%02x bank=%02d", selector, bank));
    } else if (opcode == 0x39) {
      bank = UNKNOWN;
      unknown.add(source + " indexed-selector");
    }
    var flow = instruction.getFlowType();
    for (Address target : instruction.getFlows()) {
      enqueue(source, target.getOffset(), bank,
          flow.isCall() ? "call" : "branch", flow.isCall());
      if (flow.isCall() && bank == UNKNOWN && target.getOffset() >= WINDOW
          && target.getOffset() < FIXED) {
        int candidate = conditionalBank(state.address());
        if (candidate != UNKNOWN)
          enqueue(source, target.getOffset(), candidate, "call-if-dp-ram-chain",
              true);
      }
    }
    if (flow.isComputed() && !flow.isTerminal())
      unknown.add(source + " computed-transfer");
    if (instruction.getFallThrough() != null) {
      // Return-side bank changes cannot be ruled out without a summary.
      int successor = flow.isCall() ? UNKNOWN : bank;
      enqueue(source, instruction.getFallThrough().getOffset(), successor,
          flow.isCall() ? "return-site" : "fallthrough", false);
    }
  }

  @Override
  protected void run() throws Exception {
    if (getScriptArgs().length != 1)
      throw new IllegalArgumentException("expected maximum state visits");
    int limit = Integer.parseInt(getScriptArgs()[0]);
    if (limit < 1 || limit > 1000000)
      throw new IllegalArgumentException("invalid state budget");
    verifyLayout();
    pseudo = new PseudoDisassembler(currentProgram);
    disassembler =
        Disassembler.getDisassembler(currentProgram, monitor, message -> {});
    int tx = currentProgram.startTransaction("Discover direct bank flow");
    boolean commit = false;
    try {
      // Use only hardware interrupt/reset vectors, never scan raw data.
      for (int vector : new int[] {0xfff6, 0xfff8, 0xfffc, 0xfffe}) {
        int target = (byteAt(toAddr(vector)) << 8) | byteAt(toAddr(vector + 1));
        enqueue(String.format(Locale.ROOT, "vector:%04x", vector), target,
            UNKNOWN, "entry", true);
      }
      while (!pending.isEmpty()) {
        monitor.checkCancelled();
        State state = pending.remove();
        if (!seen.add(state))
          continue;
        if (seen.size() > limit)
          throw new IllegalStateException("state budget exceeded");
        visit(state);
      }
      commit = true;
    } finally {
      currentProgram.endTransaction(tx, commit);
    }
    for (String root : roots) println("FLOW_ROOT " + root);
    for (String edge : edges) println("FLOW_EDGE " + edge);
    for (String selector : selectors) println("FLOW_SELECTOR " + selector);
    for (String gap : unknown) println("FLOW_UNKNOWN " + gap);
    println("FLOW_OK states=" + seen.size() + " roots=" + roots.size()
        + " edges=" + edges.size() + " selectors=" + selectors.size()
        + " unresolved=" + unknown.size());
  }
}"#;

// Ghidra's scripting API requires JVM glue. This mapper is authored for
// static analysis; it does not execute ROM code. Board layout follows MAME
// revision 89185ed286c7830b4e6daa647d843a929077ecd2, simpsons.cpp.
const SIMPSONS: &str = r#"// Generated contributor-only Ghidra integration.
// @category Games
import ghidra.app.script.GhidraScript;
import ghidra.program.model.mem.MemoryBlock;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.HexFormat;

public class MapKonamiBoard extends GhidraScript {
    private static final int PAGE = 0x2000;
    private static final int FIXED = 0x78000;
    private static final int SIZE = 0x80000;
    private static final int BANKS = SIZE / PAGE;

    private byte[] readImage(String path) throws Exception {
        try (var input = Files.newInputStream(Path.of(path))) {
            byte[] image = input.readNBytes(SIZE + 1);
            if (image.length != SIZE) {
                throw new IllegalArgumentException("expected 512 KiB ROM");
            }
            return image;
        }
    }

    private void permissions(MemoryBlock block, boolean writable,
            boolean executable, boolean device) {
        block.setRead(true);
        block.setWrite(writable);
        block.setExecute(executable);
        block.setVolatile(device);
    }

    private MemoryBlock fixedBlock(byte[] image) throws Exception {
        var memory = currentProgram.getMemory();
        var blocks = memory.getBlocks();
        if (blocks.length != 1) {
            throw new IllegalStateException("expected fixed-only program");
        }
        var fixed = blocks[0];
        byte[] existing = new byte[SIZE - FIXED];
        if (!fixed.getStart().equals(toAddr("8000")) ||
                fixed.getSize() != existing.length ||
                memory.getBytes(fixed.getStart(), existing) !=
                    existing.length ||
                !Arrays.equals(existing, 0, existing.length,
                    image, FIXED, SIZE)) {
            throw new IllegalStateException("fixed ROM mismatch");
        }
        return fixed;
    }

    private void addBank(byte[] image, int bank) throws Exception {
        monitor.checkCancelled();
        var memory = currentProgram.getMemory();
        int offset = bank * PAGE;
        var block = memory.createInitializedBlock(
            String.format("rom_bank_%02d", bank), toAddr("6000"),
            new ByteArrayInputStream(image, offset, PAGE), PAGE,
            monitor, true);
        permissions(block, false, true, false);
        block.setComment(String.format("Physical 0x%05x; selector %d%s",
            offset, bank, offset >= FIXED ? "; fixed alias" : ""));
        byte[] observed = new byte[PAGE];
        if (!block.isOverlay() || block.getSize() != PAGE ||
                memory.getBytes(block.getStart(), observed) != PAGE ||
                !Arrays.equals(observed, 0, PAGE,
                    image, offset, offset + PAGE)) {
            throw new IllegalStateException("bank readback mismatch");
        }
        println("BANK=" + bank + " START=" + block.getStart() +
            " END=" + block.getEnd() + " PHYSICAL=" + offset);
    }

    @Override
    protected void run() throws Exception {
        if (getScriptArgs().length != 1 ||
                !currentProgram.getLanguageID().getIdAsString()
                    .equals("Konami053248:BE:16:partial")) {
            throw new IllegalArgumentException("expected Konami and raw ROM");
        }
        byte[] image = readImage(getScriptArgs()[0]);
        var fixed = fixedBlock(image);
        var memory = currentProgram.getMemory();
        int tx = currentProgram.startTransaction("Map Konami board");
        boolean commit = false;
        try {
            fixed.setName("fixed_rom");
            permissions(fixed, false, true, false);
            fixed.setComment("Physical 0x78000..0x7ffff");
            for (int bank = 0; bank < BANKS; bank++) {
                addBank(image, bank);
            }
            var ram = memory.createUninitializedBlock(
                "main_ram", toAddr("4000"), PAGE, false);
            permissions(ram, true, false, false);
            var video = memory.createUninitializedBlock(
                "video_device_views", toAddr("0000"), 0x4000, false);
            permissions(video, true, false, true);
            video.setComment("Banked palette, tile, sprite and I/O views");
            if (memory.getBlock(toAddr("6000")) != null) {
                throw new IllegalStateException("default-space bank");
            }
            if (memory.getBlocks().length != BANKS + 3) {
                throw new IllegalStateException("unexpected block count");
            }
            commit = true;
        } finally {
            currentProgram.endTransaction(tx, commit);
        }
        String hash = HexFormat.of().formatHex(
            MessageDigest.getInstance("SHA-256").digest(image));
        println("MAPPED_OK=" + memory.getBlocks().length +
            " UNIQUE_PHYSICAL_BYTES=" + SIZE + " SHA256=" + hash);
    }
}"#;

/// Supported board memory profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Board {
    /// Simpsons four-player Konami 053248 board.
    Simpsons,
    /// Finite selector candidate report.
    SimpsonsBounds(OsString, OsString, NonZeroU64),
    /// Read-only decoded dispatch fact export script.
    SimpsonsFacts,
    /// Synthetic 512 KiB bank traversal fixture, emitted as hex.
    SimpsonsFixture,
    /// Bounded bank-aware instruction discovery script.
    SimpsonsFlow,
    /// Reconcile proven local flow observations into a candidate ledger.
    SimpsonsReconcile(OsString, OsString, OsString, NonZeroU64),
}

impl Board {
    /// Admit a named board and optional contributor mode.
    /// # Errors
    /// Rejects unknown boards and unexpected arguments.
    pub fn parse(board: &str, args: &[OsString]) -> Result<Self, String> {
        match (board, args) {
            ("simpsons", []) => Ok(Self::Simpsons),
            ("simpsons", [mode]) if mode == "fixture" => {
                Ok(Self::SimpsonsFixture)
            },
            ("simpsons", [mode]) if mode == "flow" => Ok(Self::SimpsonsFlow),
            ("simpsons", [mode]) if mode == "facts" => Ok(Self::SimpsonsFacts),
            ("simpsons", [mode, facts, rom, limit]) if mode == "bounds" => {
                Ok(Self::SimpsonsBounds(
                    facts.clone(),
                    rom.clone(),
                    crate::parse_limit(limit)?,
                ))
            },
            ("simpsons", [mode, facts, rom, ledger, limit])
                if mode == "reconcile" =>
            {
                Ok(Self::SimpsonsReconcile(
                    facts.clone(),
                    rom.clone(),
                    ledger.clone(),
                    crate::parse_limit(limit)?,
                ))
            },
            _ => Err(String::from("invalid ghidra-board simpsons mode")),
        }
    }

    /// Emit the selected board script or synthetic fixture.
    /// # Errors
    /// Returns an output failure.
    pub fn run(self) -> Result<(), String> {
        match self {
            Self::Simpsons => write_line(SIMPSONS),
            Self::SimpsonsFixture => write_line(&fixture_hex()?),
            Self::SimpsonsFlow => write_line(FLOW),
            Self::SimpsonsFacts => write_line(facts::SCRIPT),
            Self::SimpsonsReconcile(facts, rom, ledger, limit) => {
                return bounds::reconcile(&facts, &rom, &ledger, limit);
            },
            Self::SimpsonsBounds(facts, rom, limit) => {
                return bounds::run(&facts, &rom, limit);
            },
        }
        .map_err(|error| error.to_string())
    }
}

fn fixture_hex() -> Result<String, String> {
    let image = fixture_image()?;
    let mut text = String::new();
    for chunk in image.chunks(32) {
        for byte in chunk {
            write!(text, "{byte:02x}").map_err(|error| error.to_string())?;
        }
        text.push('\n');
    }
    Ok(text)
}

fn fixture_image() -> Result<Vec<u8>, String> {
    let mut image = vec![0u8; 0x80000];
    for bank in 0..64usize {
        let page = bank.saturating_mul(0x2000);
        put_fixture(&mut image, page, &[0x8f])?;
        put_fixture(&mut image, page.saturating_add(0x1fff), &[0xae])?;
    }
    let mut program = Vec::new();
    for selector in 0..=u8::MAX {
        program.extend_from_slice(&[0x38, selector, 0xa9, 7, 0x60, 0]);
    }
    for selector in [0, 63] {
        program.extend_from_slice(&[0x38, selector, 0xa9, 7, 0x7f, 0xff]);
    }
    // Same direct-page offset permits only a conditional bank candidate.
    for offset in [0x22, 0x23] {
        program.extend_from_slice(&[
            0x11, 0xc5, 0x3b, 0xc4, 0x22, 0x39, 0xc4, offset, 0x0e, 5, 0xa9, 7,
            0x60, 0,
        ]);
    }
    program.extend_from_slice(&[0x39, 0x26, 0xa8, 7, 0x60, 0]);
    put_fixture(&mut image, 0x79000, &program)?;
    for offset in [0x7fff6, 0x7fff8, 0x7fffc, 0x7fffe] {
        put_fixture(&mut image, offset, &[0x90, 0])?;
    }
    Ok(image)
}

fn put_fixture(
    image: &mut [u8],
    offset: usize,
    bytes: &[u8],
) -> Result<(), String> {
    let end = offset.checked_add(bytes.len()).ok_or("fixture offset")?;
    image
        .get_mut(offset..end)
        .ok_or("fixture range")?
        .copy_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_board_is_admitted() -> Result<(), String> {
        if Board::parse("simpsons", &[])? != Board::Simpsons
            || Board::parse("simpsons", &[OsString::from("flow")])?
                != Board::SimpsonsFlow
            || Board::parse("simpsons", &[OsString::from("fixture")])?
                != Board::SimpsonsFixture
            || Board::parse("other", &[]).is_ok()
            || Board::parse("simpsons", &[OsString::from("extra")]).is_ok()
        {
            return Err(String::from("invalid board admission"));
        }
        Ok(())
    }

    #[test]
    fn fixture_preserves_physical_banks_and_vectors() -> Result<(), String> {
        let image = fixture_image()?;
        if image.len() != 0x80000 {
            return Err(String::from("invalid fixture length"));
        }
        for page in image.as_chunks::<0x2000>().0 {
            if page.first() != Some(&0x8f) {
                return Err(String::from("missing bank entry"));
            }
        }
        if image.get(0x7fffe..) != Some([0x90, 0].as_slice())
            || image.get(0x79000..0x79006)
                != Some([0x38, 0, 0xa9, 7, 0x60, 0].as_slice())
        {
            return Err(String::from("invalid fixture roots"));
        }
        Ok(())
    }

    #[test]
    fn mapper_preserves_bank_identity_and_transaction() -> Result<(), String> {
        let text = SIMPSONS;
        for expected in [
            "private static final int PAGE = 0x2000;",
            "private static final int FIXED = 0x78000;",
            "private static final int SIZE = 0x80000;",
            "BANKS = SIZE / PAGE;",
            "PAGE,\n            monitor, true",
            "memory.getBlock(toAddr(\"6000\")) != null",
            "currentProgram.endTransaction(tx, commit)",
            "image, FIXED, SIZE",
            "image, offset, offset + PAGE",
            "input.readNBytes(SIZE + 1)",
        ] {
            if !text.contains(expected) {
                return Err(format!("missing mapping invariant: {expected}"));
            }
        }
        Ok(())
    }
}
