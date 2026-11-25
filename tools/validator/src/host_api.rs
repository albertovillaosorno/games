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
//   - Direct standard-library host API admission for portable crate roles.
// - Must-Not:
//   - Resolve arbitrary macro expansions or Rust name bindings.
// - Allows:
//   - Inputs: Authored Rust token trees.
//   - Outputs: Diagnostics for prohibited host operations and imports.
//   - Side effects: None.
// - Split-When:
//   - Semantic name resolution needs an independent validator.
// - Merge-When:
//   - A stronger parsed policy covers equivalent host API checks.
// - Summary:
//   - Prevent new host access in pure shared, runtime and importer crates.
// - Description:
//   - Inspect paths, grouped imports, aliases and nested macro tokens.
// - Usage:
//   - Invoked by the portability inventory for host-independent roles.
// - Defaults:
//   - Admit explicit host-free IO and duration types, not host modules.

use std::collections::BTreeSet;

use proc_macro2::{Delimiter, TokenStream, TokenTree};

type Findings = BTreeSet<&'static str>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    Core,
    Io,
    Standard,
    Time,
}

impl Scope {
    const fn diagnostic(self) -> &'static str {
        match self {
            Self::Core => "core library alias or glob",
            Self::Io => "std::io host or module access",
            Self::Standard => "standard library alias or glob",
            Self::Time => "std::time host or module access",
        }
    }
}

fn name(token: &TokenTree) -> Option<String> {
    match token {
        TokenTree::Ident(ident) => {
            Some(ident.to_string().trim_start_matches("r#").to_owned())
        },
        TokenTree::Group(_) | TokenTree::Literal(_) | TokenTree::Punct(_) => {
            None
        },
    }
}

fn punct(token: Option<&TokenTree>, symbol: char) -> bool {
    matches!(token, Some(TokenTree::Punct(mark)) if mark.as_char() == symbol)
}

fn path_tail(tokens: &[TokenTree]) -> Option<&[TokenTree]> {
    if punct(tokens.get(1), ':') && punct(tokens.get(2), ':') {
        tokens.get(3..)
    } else {
        None
    }
}

fn io_value(member: &str) -> bool {
    matches!(
        member,
        "BufRead"
            | "BufReader"
            | "BufWriter"
            | "Bytes"
            | "Chain"
            | "Cursor"
            | "Empty"
            | "Error"
            | "ErrorKind"
            | "IoSlice"
            | "IoSliceMut"
            | "Lines"
            | "Read"
            | "Repeat"
            | "Result"
            | "Seek"
            | "SeekFrom"
            | "Sink"
            | "Split"
            | "Take"
            | "Write"
            | "copy"
            | "empty"
            | "repeat"
            | "sink"
    )
}

fn inspect_branch(tokens: &[TokenTree], scope: Scope, findings: &mut Findings) {
    let Some(head) = tokens.first() else {
        return;
    };
    if let TokenTree::Group(group) = head {
        if group.delimiter() == Delimiter::Brace {
            let children: Vec<_> = group.stream().into_iter().collect();
            for branch in children.split(|token| punct(Some(token), ',')) {
                inspect_branch(branch, scope, findings);
            }
        }
        return;
    }
    if punct(Some(head), '*') {
        let _previous = findings.insert(scope.diagnostic());
        return;
    }
    let Some(member) = name(head) else {
        return;
    };
    match scope {
        Scope::Core => {
            let diagnostic = match member.as_str() {
                "arch" => "core::arch",
                "self" => scope.diagnostic(),
                _ => return,
            };
            let _previous = findings.insert(diagnostic);
        },
        Scope::Standard => inspect_standard(&member, tokens, findings),
        Scope::Io => {
            let last_error = member == "Error"
                && path_tail(tokens)
                    .and_then(|tail| tail.first())
                    .and_then(name)
                    .as_deref()
                    == Some("last_os_error");
            if !io_value(&member) || last_error {
                let _previous = findings.insert(scope.diagnostic());
            }
        },
        Scope::Time => {
            if !matches!(member.as_str(), "Duration" | "TryFromFloatSecsError")
            {
                let _previous = findings.insert(scope.diagnostic());
            }
        },
    }
}

fn inspect_standard(
    member: &str,
    tokens: &[TokenTree],
    findings: &mut Findings,
) {
    let diagnostic = match member {
        "arch" => "std::arch",
        "env" => "std::env",
        "fs" => "std::fs",
        "net" => "std::net",
        "os" => "std::os",
        "path" => "std::path",
        "process" => "std::process",
        "self" => Scope::Standard.diagnostic(),
        "thread" => "std::thread",
        "io" | "time" => {
            let scope = if member == "io" {
                Scope::Io
            } else {
                Scope::Time
            };
            if let Some(tail) = path_tail(tokens) {
                inspect_branch(tail, scope, findings);
            } else {
                let _previous = findings.insert(scope.diagnostic());
            }
            return;
        },
        _ => return,
    };
    let _previous = findings.insert(diagnostic);
}

fn inspect(stream: TokenStream, findings: &mut Findings) {
    let tokens: Vec<_> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            inspect(group.stream(), findings);
        }
        let Some(word) = name(token) else {
            continue;
        };
        let Some(suffix) = tokens.get(index..) else {
            continue;
        };
        match word.as_str() {
            "std" | "core" => {
                let scope = if word == "std" {
                    Scope::Standard
                } else {
                    Scope::Core
                };
                if let Some(tail) = path_tail(suffix) {
                    inspect_branch(tail, scope, findings);
                } else if suffix.get(1).and_then(name).as_deref() == Some("as")
                {
                    let _previous = findings.insert(scope.diagnostic());
                }
            },
            "print" | "println" | "eprint" | "eprintln" | "dbg"
                if punct(suffix.get(1), '!') =>
            {
                let _previous = findings.insert("host output macro");
            },
            _ => {},
        }
    }
}

/// Enumerate every distinct forbidden host API family in parsed source.
pub(crate) fn findings(source: &str) -> Result<Findings, String> {
    let stream: TokenStream = source
        .parse()
        .map_err(|error| format!("Rust token parsing: {error}"))?;
    let mut found = Findings::new();
    inspect(stream, &mut found);
    Ok(found)
}

/// Reject direct host APIs in a portable crate's authored source.
///
/// This checks parsed token groups, not expanded macros or resolved names.
/// # Errors
/// Reports token parsing failures or prohibited host API references.
pub(crate) fn validate(source: &str) -> Result<(), String> {
    let found = findings(source)?;
    if found.is_empty() {
        Ok(())
    } else {
        let names: Vec<_> = found.into_iter().collect();
        Err(format!("host API outside adapter: {}", names.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::require;

    #[test]
    fn host_free_values_and_literals_are_admitted() -> Result<(), String> {
        validate(
            r##"
            use std::{collections::BTreeMap, time::Duration};
            use std::io::{Cursor, Read as _, Write};
            use core::time::Duration as FixedDuration;
            // std::fs::read("not code");
            const NOTE: &str = r#"std::env::var("test")"#;
            fn measure() { let _ = std::time::Duration::ZERO; }
            "##,
        )
    }

    #[test]
    fn detects_grouped_paths_aliases_and_macro_tokens() -> Result<(), String> {
        for (source, expected) in [
            ("pub use ::std::{fs as files, net};", "std::fs"),
            (
                "use std::{time::{Duration, Instant}};",
                Scope::Time.diagnostic(),
            ),
            (
                "use std::{io::{self as host, Read}};",
                Scope::Io.diagnostic(),
            ),
            ("use std::io::*;", Scope::Io.diagnostic()),
            ("use std::{self as library};", Scope::Standard.diagnostic()),
            ("extern crate std as other;", Scope::Standard.diagnostic()),
            (
                "macro_rules! m { () => { ::std::fs::read(\"x\") } }",
                "std::fs",
            ),
            ("fn run() { std::io::stdout(); }", Scope::Io.diagnostic()),
            ("fn run() { println!(\"x\"); }", "host output macro"),
            ("use std::r#process;", "std::process"),
            ("use core::arch::x86_64;", "core::arch"),
            ("use core::{arch as intrinsics};", "core::arch"),
        ] {
            require(findings(source)?.contains(expected), expected)?;
            require(validate(source).is_err(), "reject host source")?;
        }
        Ok(())
    }
}
