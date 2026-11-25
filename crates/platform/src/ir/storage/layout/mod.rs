// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Configured persistent asset and executable locations.
// - Must-Not:
//   - Read environment variables, access paths or publish payloads.
// - Allows:
//   - Inputs: Bounded local configuration and admitted title identifiers.
//   - Outputs: Asset and executable paths independent of the source format.
//   - Side effects: None.
// - Split-When:
//   - A storage policy has a separate admission contract.
// - Merge-When:
//   - Another contract resolves the same deployment locations.
// - Summary:
//   - One persistent asset location per title, independent of target.
// - Description:
//   - Separate target binaries from shared content without copying assets.
// - Usage:
//   - Parse trusted local .env text and resolve a selected title.
// - Defaults:
//   - Reject missing, duplicate or unsafe settings.

use super::super::toolchain::Target;

/// Persistent destinations supplied explicitly by the operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    assets: String,
    output: String,
    target: Target,
}

impl Layout {
    /// Resolve the single asset directory shared by every target of a title.
    /// # Errors
    /// Rejects unsafe title identifiers.
    pub fn assets(&self, title: &str) -> Result<String, &'static str> {
        title_id(title)?;
        Ok(format!("{}/{title}/assets", self.assets))
    }

    /// Resolve the selected target's executable package directory.
    /// # Errors
    /// Rejects unsafe title identifiers.
    pub fn binaries(&self, title: &str) -> Result<String, &'static str> {
        title_id(title)?;
        Ok(format!(
            "{}/{title}/bin/{}",
            self.output,
            self.target.name()
        ))
    }

    /// Parse an explicit local configuration without shell expansion.
    /// # Errors
    /// Rejects missing keys, duplicate keys and unsupported targets.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let mut assets = None;
        let mut output = None;
        let mut target = None;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, raw_value) = line
                .split_once('=')
                .ok_or("invalid local configuration assignment")?;
            let value = raw_value.trim();
            match key.trim() {
                "GAMES_ASSET_ROOT" => set_once(&mut assets, root(value)?)?,
                "GAMES_OUTPUT_ROOT" => set_once(&mut output, root(value)?)?,
                "GAMES_TARGET" => set_once(&mut target, parse_target(value)?)?,
                _ => {},
            }
        }
        Ok(Self {
            assets: assets.ok_or("missing GAMES_ASSET_ROOT")?,
            output: output.ok_or("missing GAMES_OUTPUT_ROOT")?,
            target: target.ok_or("missing GAMES_TARGET")?,
        })
    }

    /// Target selected by the local operator.
    #[must_use]
    pub const fn target(&self) -> Target {
        self.target
    }
}

fn parse_target(value: &str) -> Result<Target, &'static str> {
    Target::portable_checks()
        .into_iter()
        .find(|target| target.name() == value)
        .ok_or("unsupported GAMES_TARGET")
}

fn root(raw_value: &str) -> Result<String, &'static str> {
    let value = raw_value.trim_end_matches('/');
    let bytes = value.as_bytes();
    let drive = matches!(bytes, [letter, b':', b'/', ..]
        if letter.is_ascii_alphabetic());
    let anchored = value.starts_with('/') || drive;
    if !anchored
        || value.is_empty()
        || value.chars().any(char::is_control)
        || value.contains(['$', '~', '"', '\'', '\\'])
        || value
            .split('/')
            .any(|part| matches!(part, "." | ".." | ".cache" | ".temp"))
    {
        return Err("invalid persistent storage root");
    }
    Ok(value.to_owned())
}

fn set_once<Value>(
    slot: &mut Option<Value>,
    value: Value,
) -> Result<(), &'static str> {
    if slot.is_some() {
        return Err("duplicate local configuration key");
    }
    *slot = Some(value);
    Ok(())
}

fn title_id(title: &str) -> Result<(), &'static str> {
    if title.is_empty()
        || title.starts_with('-')
        || title.ends_with('-')
        || !title.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
        })
    {
        return Err("invalid title identifier");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Layout, Target};

    const CONFIG: &str = concat!(
        "GAMES_ASSET_ROOT=/persistent/games\n",
        "GAMES_OUTPUT_ROOT=/exports/games\n",
        "GAMES_TARGET=linux-x86-64\n",
    );

    #[test]
    fn assets_ignore_executable_location() -> Result<(), &'static str> {
        let layout = Layout::parse(CONFIG)?;
        if layout.assets("metal-slug")? != "/persistent/games/metal-slug/assets"
            || layout.binaries("metal-slug")?
                != "/exports/games/metal-slug/bin/linux-x86-64"
        {
            return Err("incorrect shared location");
        }
        Ok(())
    }

    #[test]
    fn all_targets_share_the_same_assets() -> Result<(), &'static str> {
        for target in Target::portable_checks() {
            let text = CONFIG.replace("linux-x86-64", target.name());
            let layout = Layout::parse(&text)?;
            if layout.target() != target
                || layout.assets("metal-slug")?
                    != "/persistent/games/metal-slug/assets"
                || !layout.binaries("metal-slug")?.ends_with(target.name())
            {
                return Err("target-specific asset duplication");
            }
        }
        Ok(())
    }

    #[test]
    fn unsafe_and_ambiguous_settings_fail() -> Result<(), &'static str> {
        let layout = Layout::parse(CONFIG)?;
        for title in ["", "..", "a/b", "a\\b", "-arg", "a b"] {
            if layout.assets(title).is_ok() || layout.binaries(title).is_ok() {
                return Err("unsafe title");
            }
        }
        for suffix in [
            "GAMES_ASSET_ROOT=/other",
            "GAMES_OUTPUT_ROOT=/other",
            "GAMES_TARGET=linux-x86-64",
        ] {
            if Layout::parse(&format!("{CONFIG}{suffix}")).is_ok() {
                return Err("duplicate setting");
            }
        }
        for root in [
            "/tmp/.temp/assets",
            "/tmp/.cache/a",
            "/a/../b",
            "$HOME",
            "relative/assets",
        ] {
            if Layout::parse(&CONFIG.replace("/persistent/games", root)).is_ok()
            {
                return Err("unsafe root");
            }
        }
        Ok(())
    }
}
