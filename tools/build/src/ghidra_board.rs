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

use std::ffi::OsString;

use games_platform::ir::toolchain::write_line;

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Board {
    /// Simpsons four-player Konami 053248 board.
    Simpsons,
}

impl Board {
    /// Admit a named board with no additional arguments.
    /// # Errors
    /// Rejects unknown boards and unexpected arguments.
    pub fn parse(board: &str, args: &[OsString]) -> Result<Self, String> {
        match (board, args) {
            ("simpsons", []) => Ok(Self::Simpsons),
            _ => Err(String::from("expected ghidra-board simpsons")),
        }
    }

    /// Emit the selected board's Ghidra integration script.
    /// # Errors
    /// Returns an output failure.
    pub fn run(self) -> Result<(), String> {
        write_line(self.source()).map_err(|error| error.to_string())
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Simpsons => SIMPSONS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_board_is_admitted() -> Result<(), String> {
        if Board::parse("simpsons", &[])? != Board::Simpsons
            || Board::parse("other", &[]).is_ok()
            || Board::parse("simpsons", &[OsString::from("extra")]).is_ok()
        {
            return Err(String::from("invalid board admission"));
        }
        Ok(())
    }

    #[test]
    fn mapper_preserves_bank_identity_and_transaction() -> Result<(), String> {
        let text = Board::Simpsons.source();
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
