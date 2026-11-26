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

#[derive(Default)]
struct ErrorReferences {
    imports: BTreeSet<String>,
    reads: BTreeSet<String>,
}

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

fn compile_time_host_macro(member: &str) -> Option<&'static str> {
    match member {
        "env" | "option_env" => Some("compile-time environment macro"),
        "include" | "include_bytes" | "include_str" => {
            Some("compile-time file macro")
        },
        _ => None,
    }
}

fn host_output_macro(member: &str) -> bool {
    matches!(member, "print" | "println" | "eprint" | "eprintln" | "dbg")
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
    if matches!(scope, Scope::Core | Scope::Standard)
        && !(scope == Scope::Standard && member == "env")
        && let Some(diagnostic) = compile_time_host_macro(&member)
    {
        let _previous = findings.insert(diagnostic);
        return;
    }
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
        _ if host_output_macro(member) => "host output macro",
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

fn collect_error_imports(
    path: &[TokenTree],
    scope: Option<Scope>,
    aliases: &mut BTreeSet<String>,
) {
    let tokens = if punct(path.first(), ':') && punct(path.get(1), ':') {
        path.get(2..).unwrap_or_default()
    } else {
        path
    };
    let Some(head) = tokens.first() else {
        return;
    };
    if let TokenTree::Group(group) = head {
        if group.delimiter() == Delimiter::Brace {
            let children: Vec<_> = group.stream().into_iter().collect();
            for branch in children.split(|token| punct(Some(token), ',')) {
                collect_error_imports(branch, scope, aliases);
            }
        }
        return;
    }
    let member = name(head);
    match (scope, member.as_deref()) {
        (None, Some("std")) => {
            if let Some(tail) = path_tail(tokens) {
                collect_error_imports(tail, Some(Scope::Standard), aliases);
            }
        },
        (Some(Scope::Standard), Some("io")) => {
            if let Some(tail) = path_tail(tokens) {
                collect_error_imports(tail, Some(Scope::Io), aliases);
            }
        },
        (Some(Scope::Io), Some("Error")) => {
            let alias = if tokens.get(1).and_then(name).as_deref() == Some("as")
            {
                tokens.get(2).and_then(name)
            } else {
                member
            };
            if let Some(imported) = alias {
                let _previous = aliases.insert(imported);
            }
        },
        _ => {},
    }
}

fn collect_import(tokens: &[TokenTree], aliases: &mut BTreeSet<String>) {
    let length = tokens
        .iter()
        .position(|token| punct(Some(token), ';'))
        .unwrap_or(tokens.len());
    if let Some(tree) = tokens.get(..length) {
        collect_error_imports(tree, None, aliases);
    }
}

fn inspect(
    stream: TokenStream,
    findings: &mut Findings,
    errors: &mut ErrorReferences,
) {
    let tokens: Vec<_> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            inspect(group.stream(), findings, errors);
        }
        let Some(word) = name(token) else {
            continue;
        };
        let Some(suffix) = tokens.get(index..) else {
            continue;
        };
        if punct(suffix.get(1), '!')
            && let Some(diagnostic) = compile_time_host_macro(&word)
        {
            let _previous = findings.insert(diagnostic);
        }
        if path_tail(suffix)
            .and_then(|tail| tail.first())
            .and_then(name)
            .as_deref()
            == Some("last_os_error")
        {
            let _previous = errors.reads.insert(word.clone());
        }
        match word.as_str() {
            "use" => {
                collect_import(
                    suffix.get(1..).unwrap_or_default(),
                    &mut errors.imports,
                );
            },
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
            member
                if host_output_macro(member) && punct(suffix.get(1), '!') =>
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
    let mut errors = ErrorReferences::default();
    inspect(stream, &mut found, &mut errors);
    // File-level matching is conservative; it does not resolve shadowing.
    if !errors.imports.is_disjoint(&errors.reads) {
        let _previous = found.insert(Scope::Io.diagnostic());
    }
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
    fn error_aliases_cannot_read_the_host_error_state() -> Result<(), String> {
        for source in [
            "use std::io::Error; fn read() { Error::last_os_error(); }",
            "use std::io::Error as E; fn read() { E::last_os_error(); }",
            "fn read() { E::last_os_error(); } use std::io::Error as E;",
            "use std::{io::{Error as E}}; fn read() { E::last_os_error(); }",
            "use {::std::io::Error as E}; fn read() { E::last_os_error(); }",
            concat!(
                "use std::io::Error as E; ",
                "macro_rules! m { () => { E::last_os_error() } }",
            ),
        ] {
            require(
                findings(source)?.contains(Scope::Io.diagnostic()),
                "reject aliased host error state",
            )?;
        }
        validate(
            "use std::io::Error as E; fn value() { E::other(\"message\"); }",
        )?;
        validate("fn custom() { Error::last_os_error(); }")
    }

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
    fn detects_compile_time_host_inputs() -> Result<(), String> {
        for (source, expected) in [
            (
                r#"const SECRET: &str = env!("TOKEN");"#,
                "compile-time environment macro",
            ),
            (
                r#"const VALUE: Option<&str> = option_env!("TOKEN");"#,
                "compile-time environment macro",
            ),
            (
                r#"const BYTES: &[u8] = include_bytes!("asset.zip");"#,
                "compile-time file macro",
            ),
            (
                r#"macro_rules! m { () => { include_str!("private") } }"#,
                "compile-time file macro",
            ),
            (
                r#"::core::include!("generated.rs");"#,
                "compile-time file macro",
            ),
            (
                "use std::{include_bytes as bytes, include_str as text};",
                "compile-time file macro",
            ),
            (
                "pub use core::option_env as build_env;",
                "compile-time environment macro",
            ),
        ] {
            require(findings(source)?.contains(expected), expected)?;
            require(validate(source).is_err(), "reject host input")?;
        }
        validate(
            r#"
            // include_str!("not code")
            const LABEL: &str = "option_env!(not code)";
            fn include_bytes() {}
            fn env() {}
            "#,
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
            ("use std::println as output;", "host output macro"),
            (
                "pub use ::std::{dbg as trace, eprint};",
                "host output macro",
            ),
            ("use std::r#print;", "host output macro"),
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
