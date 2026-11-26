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
//   - Allowlisted reference requests and revision component admission.
// - Must-Not:
//   - Contact repositories, extract archives or execute downloaded tools.
// - Allows:
//   - Inputs: Known tool names and upstream revision observations.
//   - Outputs: Admitted reference selections and safe revision components.
//   - Side effects: None.
// - Split-When:
//   - An independent reference protocol needs different admission rules.
// - Merge-When:
//   - A shared policy owns the same allowlist and revision admission.
// - Summary:
//   - Reject arbitrary repositories, arguments and revision paths.
// - Description:
//   - Admit official source snapshots and specific release artifacts.
// - Usage:
//   - Select a known reference before native download dispatch.
// - Defaults:
//   - No original game downloads and no execution of retrieved files.

//! Reference selection and revision admission.

const TOOLS: &[ReferenceTool] = &[
    ReferenceTool {
        kind: ReferenceKind::Dependencies,
        name: "dependencies",
    },
    source("craft-libs", "storytold/craft-libs"),
    source("ghidra", "NationalSecurityAgency/ghidra"),
    source("jadx", "skylot/jadx"),
    source("apktool", "iBotPeaches/Apktool"),
    source("mame", "mamedev/mame"),
    source("pcsx2", "PCSX2/pcsx2"),
    source("rpcs3", "RPCS3/rpcs3"),
    source("dolphin", "dolphin-emu/dolphin"),
    source("xemu", "xemu-project/xemu"),
    source("xenia", "xenia-project/xenia"),
    source("melonds", "melonDS-emu/melonDS"),
    release(
        "ghidra-release",
        "NationalSecurityAgency/ghidra",
        "ghidra_*_PUBLIC_*.zip",
        None,
    ),
    release("jadx-release", "skylot/jadx", "jadx-[0-9]*.zip", None),
    release(
        "apktool-release",
        "iBotPeaches/Apktool",
        "apktool_*.jar",
        None,
    ),
    release(
        "wasm-bindgen-release",
        "wasm-bindgen/wasm-bindgen",
        "wasm-bindgen-*-x86_64-unknown-linux-musl.tar.gz*",
        Some("0.2.127"),
    ),
];

/// A known reference download, admitted from the fixed catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReferenceTool {
    kind: ReferenceKind,
    name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceKind {
    Dependencies,
    Release {
        pattern: &'static str,
        repo: &'static str,
        tag: Option<&'static str>,
    },
    Source(&'static str),
}

/// A revision rejected before it becomes an argument or path component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidRevision;

/// A revision component admitted for the selected reference kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReferenceRevision<'value>(&'value str);

impl<'value> ReferenceRevision<'value> {
    /// The exact admitted revision or release identifier.
    #[must_use]
    pub const fn value(self) -> &'value str {
        self.0
    }
}

impl ReferenceTool {
    /// The fixed default catalogue, in download order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        TOOLS
    }

    pub(crate) const fn kind(self) -> ReferenceKind {
        self.kind
    }

    /// The user-facing identifier and local directory component.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Select an exact catalogue name; arbitrary URLs are never admitted.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        TOOLS.iter().copied().find(|tool| tool.name == name)
    }

    /// Admit a revision observed from the selected upstream.
    ///
    /// # Errors
    /// Rejects invalid hashes, option-like tags and path components.
    pub fn revision(
        self,
        value: &str,
    ) -> Result<ReferenceRevision<'_>, InvalidRevision> {
        let valid = match self.kind() {
            ReferenceKind::Dependencies => false,
            ReferenceKind::Source(_) => {
                value.len() == 40
                    && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            },
            ReferenceKind::Release { tag, .. } => {
                release_tag(value) && tag.is_none_or(|fixed| value == fixed)
            },
        };
        if valid {
            Ok(ReferenceRevision(value))
        } else {
            Err(InvalidRevision)
        }
    }
}

fn release_tag(value: &str) -> bool {
    if value.len() > 255 || matches!(value, "LATEST" | "LATEST.pending") {
        return false;
    }
    value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
        })
}

const fn source(name: &'static str, repo: &'static str) -> ReferenceTool {
    ReferenceTool {
        kind: ReferenceKind::Source(repo),
        name,
    }
}

const fn release(
    name: &'static str,
    repo: &'static str,
    pattern: &'static str,
    tag: Option<&'static str>,
) -> ReferenceTool {
    ReferenceTool {
        kind: ReferenceKind::Release { repo, pattern, tag },
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_only_accepts_exact_known_names() {
        let names: Vec<_> = ReferenceTool::all()
            .iter()
            .map(|tool| tool.name())
            .collect();
        assert_eq!(names, [
            "dependencies",
            "craft-libs",
            "ghidra",
            "jadx",
            "apktool",
            "mame",
            "pcsx2",
            "rpcs3",
            "dolphin",
            "xemu",
            "xenia",
            "melonds",
            "ghidra-release",
            "jadx-release",
            "apktool-release",
            "wasm-bindgen-release",
        ]);
        for tool in ReferenceTool::all() {
            assert_eq!(ReferenceTool::parse(tool.name()), Some(*tool));
        }
        for invalid in ["", "../mame", "MAME", "--repo", "https://example.org"]
        {
            assert_eq!(ReferenceTool::parse(invalid), None);
        }
    }

    #[test]
    fn revisions_are_admitted_by_reference_kind() -> Result<(), String> {
        let source = ReferenceTool::parse("mame").ok_or("source missing")?;
        let release =
            ReferenceTool::parse("ghidra-release").ok_or("release missing")?;
        let binding = ReferenceTool::parse("wasm-bindgen-release")
            .ok_or("binding release missing")?;
        if binding.revision("0.2.127").is_err()
            || binding.revision("0.2.130").is_ok()
        {
            return Err("incorrect binding release admission".into());
        }
        let hash = "0123456789abcdef0123456789abcdef01234567";
        if source.revision(hash) != Ok(ReferenceRevision(hash))
            || release.revision("Ghidra_11.4.1_build")
                != Ok(ReferenceRevision("Ghidra_11.4.1_build"))
        {
            return Err("valid revision rejected".into());
        }
        for invalid in [
            "",
            ".",
            "..",
            "../v1",
            "-option",
            "v1/tag",
            "v1 tag",
            "LATEST",
            "LATEST.pending",
        ] {
            if release.revision(invalid).is_ok()
                || source.revision(invalid).is_ok()
            {
                return Err(format!("invalid revision admitted: {invalid}"));
            }
        }
        if release.revision(&"a".repeat(256)).is_ok() {
            return Err("oversized release component admitted".into());
        }
        if source.revision("v1").is_ok()
            || source
                .revision("z123456789abcdef0123456789abcdef01234567")
                .is_ok()
        {
            return Err("invalid commit hash admitted".into());
        }
        Ok(())
    }
}
