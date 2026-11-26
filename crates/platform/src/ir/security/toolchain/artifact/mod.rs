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
//   - Structural executable admission for authored distribution targets.
// - Must-Not:
//   - Execute binaries, access storage or claim complete ABI compatibility.
// - Allows:
//   - Inputs: Explicit target and bounded executable bytes.
//   - Outputs: Structural admission or typed rejection.
//   - Side effects: None.
// - Split-When:
//   - Distinct executable formats need independent structural validators.
// - Merge-When:
//   - Another shared contract owns identical format admission.
// - Summary:
//   - Check authored executable structure before package publication.
// - Description:
//   - Require a file-backed executable entry in an admitted image.
// - Usage:
//   - Admit an artifact snapshot for the requested distribution target.
// - Defaults:
//   - Reject unknown formats, invalid images and unsupported targets.

//! Host-independent structural admission of authored build artifacts.

/// Authored structural fixtures, never executable programs.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixtures {
    /// Construct a minimal structurally admitted x86-64 ELF snapshot.
    /// # Errors
    /// Reports an invalid fixture field.
    pub fn executable_fixture() -> Result<[u8; 128], String> {
        let mut elf = [0u8; 128];
        write_field(&mut elf, 0, *b"\x7fELF\x02\x01\x01")?;
        write_field(&mut elf, 16, 3u16.to_le_bytes())?;
        write_field(&mut elf, 18, 62u16.to_le_bytes())?;
        write_field(&mut elf, 20, 1u32.to_le_bytes())?;
        write_field(&mut elf, 24, 0x1078u64.to_le_bytes())?;
        write_field(&mut elf, 32, 64u64.to_le_bytes())?;
        write_field(&mut elf, 52, 64u16.to_le_bytes())?;
        write_field(&mut elf, 54, 56u16.to_le_bytes())?;
        write_field(&mut elf, 56, 1u16.to_le_bytes())?;
        write_field(&mut elf, 64, 1u32.to_le_bytes())?;
        write_field(&mut elf, 68, 5u32.to_le_bytes())?;
        write_field(&mut elf, 72, 120u64.to_le_bytes())?;
        write_field(&mut elf, 80, 0x1078u64.to_le_bytes())?;
        write_field(&mut elf, 96, 8u64.to_le_bytes())?;
        write_field(&mut elf, 104, 16u64.to_le_bytes())?;
        write_field(&mut elf, 112, 4096u64.to_le_bytes())?;
        Ok(elf)
    }

    pub(super) fn write_field<const N: usize>(
        bytes: &mut [u8],
        offset: usize,
        value: [u8; N],
    ) -> Result<(), String> {
        bytes
            .get_mut(offset..offset.saturating_add(N))
            .ok_or("invalid ELF fixture field")?
            .copy_from_slice(&value);
        Ok(())
    }
}

use std::fmt;

use crate::ir::toolchain::Target;

const ELF_HEADER: usize = 64;
const ELF_PROGRAM_HEADER: usize = 56;
type ProgramHeaders<'file> = (u64, &'file [[u8; ELF_PROGRAM_HEADER]]);

/// Rejection of an authored executable artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutableAdmissionError {
    /// The executable lacks the required structural properties.
    InvalidStructure,
    /// The target's executable format is not yet admitted.
    UnsupportedTarget,
}

impl fmt::Display for ExecutableAdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget => {
                f.write_str("executable format unavailable for target")
            },
            Self::InvalidStructure => {
                f.write_str("expected an x86-64 ELF executable")
            },
        }
    }
}

/// Check the structure of an already bounded authored executable snapshot.
///
/// This is neither an executable loader nor an ABI compatibility audit.
/// # Errors
/// Rejects unsupported targets and structurally invalid executables.
pub fn admit_executable(
    target: Target,
    bytes: &[u8],
) -> Result<(), ExecutableAdmissionError> {
    match target {
        Target::LinuxX86_64Gnu => native_entry(bytes)
            .ok_or(ExecutableAdmissionError::InvalidStructure),
        Target::AndroidArm64
        | Target::LinuxArm64Gnu
        | Target::WebAssembly
        | Target::WindowsArm64Gnullvm
        | Target::WindowsX86_64Gnu => {
            Err(ExecutableAdmissionError::UnsupportedTarget)
        },
    }
}

fn elf_field<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
}

fn elf_program_table(bytes: &[u8]) -> Option<ProgramHeaders<'_>> {
    if !bytes.starts_with(b"\x7fELF\x02\x01\x01")
        || !matches!(bytes.get(16..18), Some([2 | 3, 0]))
        || bytes.get(18..24) != Some(&[62, 0, 1, 0, 0, 0])
        || usize::from(u16::from_le_bytes(elf_field(bytes, 52)?)) != ELF_HEADER
        || usize::from(u16::from_le_bytes(elf_field(bytes, 54)?))
            != ELF_PROGRAM_HEADER
    {
        return None;
    }
    let entry = u64::from_le_bytes(elf_field(bytes, 24)?);
    let offset =
        usize::try_from(u64::from_le_bytes(elf_field(bytes, 32)?)).ok()?;
    let count = u16::from_le_bytes(elf_field(bytes, 56)?);
    if entry == 0 || offset < ELF_HEADER || matches!(count, 0 | u16::MAX) {
        return None;
    }
    let end = offset
        .checked_add(usize::from(count).checked_mul(ELF_PROGRAM_HEADER)?)?;
    let programs = bytes.get(offset..end)?.as_chunks::<ELF_PROGRAM_HEADER>().0;
    Some((entry, programs))
}

fn load_contains_entry(
    program: &[u8],
    entry: u64,
    length: u64,
) -> Option<bool> {
    let kind = u32::from_le_bytes(elf_field(program, 0)?);
    if kind != 1 {
        return Some(false);
    }
    let flags = u32::from_le_bytes(elf_field(program, 4)?);
    let file = u64::from_le_bytes(elf_field(program, 8)?);
    let address = u64::from_le_bytes(elf_field(program, 16)?);
    let size = u64::from_le_bytes(elf_field(program, 32)?);
    let memory = u64::from_le_bytes(elf_field(program, 40)?);
    let alignment = u64::from_le_bytes(elf_field(program, 48)?);
    if size > memory
        || file.checked_add(size)? > length
        || address.checked_add(memory).is_none()
        || (alignment > 1
            && (!alignment.is_power_of_two()
                || file.checked_rem(alignment)
                    != address.checked_rem(alignment)))
    {
        return None;
    }
    Some(
        flags & 1 != 0
            && entry >= address
            && entry < address.checked_add(size)?,
    )
}

// This is a structural build-output check, not an ELF loader or ABI audit.
fn native_entry(bytes: &[u8]) -> Option<()> {
    let (entry, programs) = elf_program_table(bytes)?;
    let length = u64::try_from(bytes.len()).ok()?;
    let mut executable_entry = false;
    for program in programs {
        executable_entry |= load_contains_entry(program, entry, length)?;
    }
    executable_entry.then_some(())
}

#[cfg(test)]
mod tests {
    use super::fixtures::{executable_fixture, write_field};
    use super::*;

    fn native_elf(bytes: &[u8]) -> bool {
        admit_executable(Target::LinuxX86_64Gnu, bytes).is_ok()
    }

    #[test]
    fn executable_admission_requires_a_file_backed_entry() -> Result<(), String>
    {
        let elf = executable_fixture()?;
        require(native_elf(&elf))?;
        for length in 0..elf.len() {
            require(!native_elf(elf.get(..length).ok_or("fixture prefix")?))?;
        }
        let mut static_elf = elf;
        write_field(&mut static_elf, 16, 2u16.to_le_bytes())?;
        require(native_elf(&static_elf))?;
        for (offset, value) in [
            (24, 0u64),
            (24, 0x1080), // Entry in the zero-filled tail.
            (32, 63),
            (32, u64::MAX),
            (72, 121), // Payload extends beyond the file.
            (80, u64::MAX),
            (96, 17),
            (104, 7),
            (112, 3),
            (112, 8192), // File offset and virtual address disagree.
        ] {
            let mut invalid = elf;
            write_field(&mut invalid, offset, value.to_le_bytes())?;
            require(!native_elf(&invalid))?;
        }
        for (offset, value) in
            [(18, 183u16), (52, 0), (54, 55), (56, 0), (56, u16::MAX)]
        {
            let mut invalid = elf;
            write_field(&mut invalid, offset, value.to_le_bytes())?;
            require(!native_elf(&invalid))?;
        }
        for (offset, value) in [(20, 2u32), (64, 0), (68, 4)] {
            let mut invalid = elf;
            write_field(&mut invalid, offset, value.to_le_bytes())?;
            require(!native_elf(&invalid))?;
        }
        Ok(())
    }

    #[test]
    fn unsupported_targets_fail_explicitly() -> Result<(), String> {
        require(
            admit_executable(Target::LinuxX86_64Gnu, &[])
                == Err(ExecutableAdmissionError::InvalidStructure),
        )?;
        let elf = executable_fixture()?;
        for target in Target::portable_checks() {
            if target != Target::LinuxX86_64Gnu {
                require(
                    admit_executable(target, &elf)
                        == Err(ExecutableAdmissionError::UnsupportedTarget),
                )?;
            }
        }
        Ok(())
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| String::from("artifact admission contract failed"))
    }
}
