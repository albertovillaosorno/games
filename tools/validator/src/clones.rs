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
//   - Detection of substantial duplicated Rust function bodies.
// - Must-Not:
//   - Modify source or reject ambiguous resemblance as verified clones.
// - Allows:
//   - Inputs: Authored Rust files and lexical token streams.
//   - Outputs: Exact and potential near-clone diagnostics with locations.
//   - Side effects: Diagnostic messages on standard error.
// - Split-When:
//   - A compiler-grade parser becomes necessary for ambiguous syntax.
// - Merge-When:
//   - Another shared architecture checker covers clone detection.
// - Summary:
//   - Conservative normalised-token duplicate detection.
// - Description:
//   - Rejects exact substantial clones, reports structural similarities.
// - Usage:
//   - Invoked by cargo validate as an authored source gate.
// - Defaults:
//   - Exclude generated paths, marked code and trivial bodies.
//
//! Conservative detection of substantial Rust function-body duplicates.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::iter::Peekable;
use std::path::Path;
use std::str::Chars;

use crate::files;

// Tiny bodies are typically accessors, constructors or required trait glue.
// Requiring several statements also prevents matching macro boilerplate.
const MIN_TOKENS: usize = 72;
const MIN_STATEMENTS: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    line: usize,
    text: String,
}

#[derive(Debug)]
struct Function {
    exact: Vec<String>,
    line: usize,
    path: String,
    shape: Vec<String>,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Findings {
    exact: Vec<String>,
    near: Vec<String>,
}

fn string_literal(iter: &mut Peekable<Chars<'_>>, line: &mut usize) -> String {
    let mut literal = String::from("\"");
    let mut escaped = false;
    for symbol in iter.by_ref() {
        literal.push(symbol);
        if symbol == '\n' {
            *line = line.saturating_add(1);
        }
        if escaped {
            escaped = false;
        } else if symbol == '\\' {
            escaped = true;
        } else if symbol == '"' {
            break;
        }
    }
    literal
}

fn block_comment(iter: &mut Peekable<Chars<'_>>, line: &mut usize) {
    let mut depth = 1usize;
    let mut previous = '\0';
    for symbol in iter.by_ref() {
        if symbol == '\n' {
            *line = line.saturating_add(1);
        }
        match (previous, symbol) {
            ('/', '*') => {
                depth = depth.saturating_add(1);
            },
            ('*', '/') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            },
            _ => {},
        }
        previous = symbol;
    }
}

fn line_comment(iter: &mut Peekable<Chars<'_>>, line: &mut usize) {
    for symbol in iter.by_ref() {
        if symbol == '\n' {
            *line = line.saturating_add(1);
            break;
        }
    }
}

fn identifier(first: char, iter: &mut Peekable<Chars<'_>>) -> String {
    let mut word = String::from(first);
    while iter
        .peek()
        .is_some_and(|next| next.is_alphanumeric() || *next == '_')
    {
        if let Some(next) = iter.next() {
            word.push(next);
        }
    }
    word
}

fn raw_literal(
    iter: &mut Peekable<Chars<'_>>,
    line: &mut usize,
) -> Option<String> {
    let mut lookahead = iter.clone();
    let mut hashes = 0usize;
    while lookahead.peek() == Some(&'#') {
        let _mark = lookahead.next();
        hashes = hashes.saturating_add(1);
    }
    if lookahead.peek() != Some(&'"') {
        return None;
    }
    let mut literal = String::from("r");
    for _ in 0..hashes {
        let _mark = iter.next();
        literal.push('#');
    }
    let _quote = iter.next();
    literal.push('"');
    loop {
        let Some(symbol) = iter.next() else {
            return Some(literal);
        };
        literal.push(symbol);
        if symbol == '\n' {
            *line = line.saturating_add(1);
        }
        if symbol != '"' {
            continue;
        }
        let mut remaining = iter.clone();
        if (0..hashes).all(|_| {
            remaining.peek() == Some(&'#') && remaining.next().is_some()
        }) {
            for _ in 0..hashes {
                let _mark = iter.next();
                literal.push('#');
            }
            return Some(literal);
        }
    }
}

fn symbol_text(
    symbol: char,
    iter: &mut Peekable<Chars<'_>>,
    line: &mut usize,
) -> String {
    if symbol == '"' {
        string_literal(iter, line)
    } else if symbol == 'r' {
        raw_literal(iter, line).unwrap_or_else(|| identifier(symbol, iter))
    } else if symbol.is_alphanumeric() || symbol == '_' {
        identifier(symbol, iter)
    } else {
        String::from(symbol)
    }
}

fn tokens(source: &str) -> Vec<Token> {
    let mut result = Vec::new();
    let mut characters = source.chars().peekable();
    let mut line = 1usize;
    while let Some(symbol) = characters.next() {
        if symbol == '\n' {
            line = line.saturating_add(1);
            continue;
        }
        if symbol.is_whitespace() {
            continue;
        }
        if symbol == '/' && characters.peek() == Some(&'/') {
            let _slash = characters.next();
            line_comment(&mut characters, &mut line);
            continue;
        }
        if symbol == '/' && characters.peek() == Some(&'*') {
            let _asterisk = characters.next();
            block_comment(&mut characters, &mut line);
            continue;
        }
        let start_line = line;
        let text = symbol_text(symbol, &mut characters, &mut line);
        result.push(Token { line: start_line, text });
    }
    result
}

fn structural_token(value: &str) -> String {
    if value.starts_with("r\"") || value.starts_with("r#") {
        return String::from("STRING");
    }
    if value.starts_with('"') {
        return String::from("STRING");
    }
    if value.chars().next().is_some_and(char::is_numeric) {
        return String::from("NUMBER");
    }
    if value
        .chars()
        .next()
        .is_some_and(|symbol| symbol.is_alphabetic() || symbol == '_')
    {
        if [
            "if", "else", "for", "while", "loop", "let", "mut", "return",
            "break", "continue", "match", "true", "false", "async", "await",
            "move", "unsafe", "const", "self", "Self", "impl", "in", "ref",
            "as", "Some", "None", "Ok", "Err",
        ]
        .contains(&value)
        {
            return String::from(value);
        }
        return String::from("IDENT");
    }
    String::from(value)
}

fn function_body(
    path: &str,
    tokens: &[Token],
    start: usize,
) -> Option<Function> {
    let function = tokens.get(start)?;
    if function.text != "fn" {
        return None;
    }
    let signature = tokens.iter().skip(start.saturating_add(2));
    let offset = signature
        .take(160)
        .take_while(|token| token.text != ";")
        .position(|token| token.text == "{")?;
    let opening = start.saturating_add(2).saturating_add(offset);
    let mut depth = 1usize;
    let mut body = Vec::new();
    for token in tokens.iter().skip(opening.saturating_add(1)) {
        if token.text == "{" {
            depth = depth.saturating_add(1);
        } else if token.text == "}" {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
        body.push(token.text.clone());
    }
    let statements = body.iter().filter(|token| *token == ";").count();
    if depth != 0 || body.len() < MIN_TOKENS || statements < MIN_STATEMENTS {
        return None;
    }
    let shape = body.iter().map(|token| structural_token(token)).collect();
    Some(Function {
        exact: body,
        line: function.line,
        path: String::from(path),
        shape,
    })
}

fn functions(path: &str, source: &str) -> Vec<Function> {
    let stream = tokens(source);
    stream
        .iter()
        .enumerate()
        .filter_map(|(index, _)| function_body(path, &stream, index))
        .collect()
}

fn excluded(path: &Path, source: &str) -> Option<&'static str> {
    let generated = path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("generated" | "gen" | "target" | ".reference")
        )
    }) || path.to_string_lossy().ends_with(".generated.rs");
    if generated {
        return Some("generated path");
    }
    let header = source.lines().take(20);
    if header.into_iter().any(|line| {
        line.contains("@generated") || line.contains("Code generated")
    }) {
        return Some("explicit generated marker");
    }
    None
}

fn location(function: &Function) -> String {
    format!("{}:{}", function.path, function.line)
}

fn compare<'functions>(
    functions: impl IntoIterator<Item = &'functions Function>,
) -> Findings {
    let mut exact = BTreeMap::<Vec<String>, &Function>::new();
    let mut structural = BTreeMap::<Vec<String>, &Function>::new();
    let mut findings = Findings::default();
    for function in functions {
        if let Some(previous) = exact.get(&function.exact) {
            if previous.path != function.path {
                findings.exact.push(format!(
                    "exact substantial clone: {} and {}",
                    location(previous),
                    location(function)
                ));
            }
        } else {
            let _old = exact.insert(function.exact.clone(), function);
        }
        if let Some(previous) = structural.get(&function.shape) {
            if previous.path != function.path
                && previous.exact != function.exact
            {
                findings.near.push(format!(
                    "possible renamed clone: {} and {}",
                    location(previous),
                    location(function)
                ));
            }
        } else {
            let _old = structural.insert(function.shape.clone(), function);
        }
    }
    findings
}

/// Scan authored Rust functions for substantial exact and potential clones.
///
/// # Errors
///
/// Rejects matching substantial function bodies in different source files.
pub(super) fn validate(root: &Path) -> Result<(), String> {
    let mut records = Vec::new();
    for path in files::authored(root)? {
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(root.join(&path))
            .map_err(|error| error.to_string())?;
        if excluded(&path, &source).is_some() {
            continue;
        }
        records.extend(functions(&path.to_string_lossy(), &source));
    }
    let findings = compare(&records);
    let mut stderr = io::stderr().lock();
    for near in findings.near.iter().take(20) {
        writeln!(stderr, "{near} (review manually; not blocking)")
            .map_err(|error| error.to_string())?;
    }
    if !findings.exact.is_empty() {
        return Err(findings.exact.join("\n"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/validator/clones.rs"]
mod tests;
