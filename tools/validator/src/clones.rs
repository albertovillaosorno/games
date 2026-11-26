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
use std::iter::Peekable;
use std::path::Path;
use std::str::Chars;

use games_platform::ir::process::write_diagnostic;

use crate::files;

// Tiny bodies are typically accessors, constructors or required trait glue.
// Requiring several statements also prevents matching macro boilerplate.
const MIN_TOKENS: usize = 72;
const MIN_STATEMENTS: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    line: usize,
    pub(crate) text: String,
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
    while let Some(symbol) = iter.next() {
        if symbol == '\n' {
            *line = line.saturating_add(1);
        } else if symbol == '/' && iter.peek() == Some(&'*') {
            let _opening = iter.next();
            depth = depth.saturating_add(1);
        } else if symbol == '*' && iter.peek() == Some(&'/') {
            let _closing = iter.next();
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
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

fn unicode_escape(
    iter: &mut Peekable<Chars<'_>>,
    literal: &mut String,
) -> Option<()> {
    if iter.next()? != '{' {
        return None;
    }
    literal.push('{');
    let mut digits = 0u8;
    loop {
        let digit = iter.next()?;
        literal.push(digit);
        if digit == '}' {
            return (digits != 0).then_some(());
        }
        if digit.is_ascii_hexdigit() {
            digits = digits.saturating_add(1);
            if digits > 6 {
                return None;
            }
        } else if digit != '_' || digits == 0 {
            return None;
        }
    }
}

fn character_escape(
    iter: &mut Peekable<Chars<'_>>,
    literal: &mut String,
) -> Option<()> {
    let escape = iter.next()?;
    literal.push(escape);
    match escape {
        'x' => {
            for _ in 0u8..2 {
                let digit = iter.next()?;
                if !digit.is_ascii_hexdigit() {
                    return None;
                }
                literal.push(digit);
            }
        },
        'u' => unicode_escape(iter, literal)?,
        'n' | 'r' | 't' | '\\' | '\'' | '"' | '0' => {},
        _ => return None,
    }
    Some(())
}

// A valid character literal has a closing quote after one scalar or
// escape. A lifetime or label begins with the same punctuation but does
// not have that closing quote; do not consume it on failed lookahead.
fn character_literal(iter: &mut Peekable<Chars<'_>>) -> Option<String> {
    let mut lookahead = iter.clone();
    let first = lookahead.next()?;
    let mut literal = String::from("'");
    literal.push(first);
    match first {
        '\\' => character_escape(&mut lookahead, &mut literal)?,
        '\'' | '\n' | '\r' | '\t' => return None,
        _ => {},
    }
    if lookahead.next()? != '\'' {
        return None;
    }
    literal.push('\'');
    *iter = lookahead;
    Some(literal)
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
    } else if symbol == '\'' {
        character_literal(iter).unwrap_or_else(|| String::from(symbol))
    } else if matches!(symbol, 'b' | 'c') && iter.peek() == Some(&'r') {
        let mut lookahead = iter.clone();
        let _prefix = lookahead.next();
        if let Some(raw) = raw_literal(&mut lookahead, line) {
            *iter = lookahead;
            format!("{symbol}{raw}")
        } else {
            identifier(symbol, iter)
        }
    } else if symbol.is_alphanumeric() || symbol == '_' {
        identifier(symbol, iter)
    } else {
        String::from(symbol)
    }
}

pub(crate) fn tokens(source: &str) -> Vec<Token> {
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
    if ["r\"", "r#", "br\"", "br#", "cr\"", "cr#"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
    {
        return String::from("STRING");
    }
    if value.starts_with('"') {
        return String::from("STRING");
    }
    if value.starts_with('\'') && value.len() > 1 {
        return String::from("CHAR");
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

// Ignore delimiters in argument types, array lengths and const-generic
// expressions. Only a top-level opening brace starts a function body.
fn signature_opening(tokens: &[Token]) -> Option<usize> {
    let mut groups = Vec::new();
    let mut angles = 0usize;
    let mut previous = "";
    for (offset, token) in tokens.iter().take(160).enumerate() {
        let symbol = token.text.as_str();
        match symbol {
            "(" => groups.push(")"),
            "[" => groups.push("]"),
            "{" if !groups.is_empty() || angles != 0 => groups.push("}"),
            ")" | "]" | "}" if groups.pop() != Some(symbol) => return None,
            _ if !groups.is_empty() => {},
            "{" => return Some(offset),
            ";" if angles == 0 => return None,
            "<" => angles = angles.saturating_add(1),
            ">" if previous != "-" => angles = angles.saturating_sub(1),
            _ => {},
        }
        previous = symbol;
    }
    None
}

fn function_body(
    path: &str,
    tokens: &[Token],
    start: usize,
) -> Option<Function> {
    let function = tokens.get(start)?;
    let name = tokens.get(start.checked_add(1)?)?;
    if function.text != "fn"
        || !name
            .text
            .chars()
            .next()
            .is_some_and(|symbol| symbol.is_alphabetic() || symbol == '_')
    {
        return None;
    }
    let signature = start.checked_add(2)?;
    let offset = signature_opening(tokens.get(signature..)?)?;
    let opening = signature.checked_add(offset)?;
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
pub fn validate(root: &Path) -> Result<(), String> {
    let mut records = Vec::new();
    for path in files::authored(root)? {
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = files::read_text(root, &path)?;
        if excluded(&path, &source).is_some() {
            continue;
        }
        records.extend(functions(&path.to_string_lossy(), &source));
    }
    let findings = compare(&records);
    for near in findings.near.iter().take(20) {
        write_diagnostic(format_args!(
            "{near} (review manually; not blocking)"
        ))
        .map_err(|error| error.to_string())?;
    }
    if !findings.exact.is_empty() {
        return Err(findings.exact.join("\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::path::Path;

    use super::{compare, excluded, functions, tokens};

    fn fixture(name: &str, prefix: &str, operator: &str) -> String {
        let mut source = format!("fn {name}() {{\n");
        for index in 0u8..16u8 {
            let _result = writeln!(
                source,
                "let {prefix}_{index} = {index} {operator} 1;"
            );
        }
        source.push_str("}\n");
        source
    }

    #[test]
    fn exact_clones_are_rejected_with_both_locations() {
        let first = fixture("alpha", "name", "+");
        let second = fixture("beta", "name", "+");
        let mut records = functions("crates/a/src/lib.rs", &first);
        records.extend(functions("crates/b/src/lib.rs", &second));
        let findings = compare(&records);
        assert_eq!(findings.exact.len(), 1);
        assert_eq!(findings.near, Vec::<String>::new());
        assert!(findings.exact.iter().any(|message| {
            message.contains("a/src/lib.rs:1")
                && message.contains("b/src/lib.rs:1")
        }));
    }

    #[test]
    fn renamed_clones_require_manual_review() {
        let first = fixture("alpha", "first", "+");
        let second = fixture("beta", "second", "+");
        let mut records = functions("crates/a/src/lib.rs", &first);
        records.extend(functions("crates/b/src/lib.rs", &second));
        let findings = compare(&records);
        assert_eq!(findings.exact, Vec::<String>::new());
        assert_eq!(findings.near.len(), 1);
    }

    #[test]
    fn unrelated_bodies_and_trivial_glue_are_not_clones() {
        let first = fixture("alpha", "first", "+");
        let second = fixture("beta", "second", "*");
        let short = String::from("fn get() { let x = 1; x; }");
        let mut records = functions("crates/a/src/lib.rs", &first);
        records.extend(functions("crates/b/src/lib.rs", &second));
        assert!(functions("crates/c/src/lib.rs", &short).is_empty());
        let findings = compare(&records);
        assert_eq!(findings.exact, Vec::<String>::new());
        assert_eq!(findings.near, Vec::<String>::new());
    }

    #[test]
    fn ignores_comments_and_generated_inputs() {
        let first =
            String::from("fn example() { /* nested /* comment */ text */ 1; }");
        let stream = tokens(&first);
        assert!(!stream.iter().any(|token| token.text == "comment"));
        let path = Path::new("crates/example/src/generated/file.rs");
        assert_eq!(excluded(path, &first), Some("generated path"));
        let marked = String::from("// @generated\nfn manual() {}\n");
        assert_eq!(
            excluded(Path::new("crates/example/src/lib.rs"), &marked),
            Some("explicit generated marker"),
        );
    }

    #[test]
    fn normalises_whitespace_and_nested_comments() {
        let first = fixture("alpha", "entity", "+");
        let mut second = fixture("beta", "entity", "+");
        second = second.replace(" = ", "/* outer /* inner */ ok */=");
        let mut records = functions("crates/a/src/lib.rs", &first);
        records.extend(functions("crates/b/src/lib.rs", &second));
        let findings = compare(&records);
        assert_eq!(findings.exact.len(), 1);
        assert_eq!(findings.near, Vec::<String>::new());
    }

    #[test]
    fn character_literals_do_not_truncate_substantial_clones() {
        let first = fixture("alpha", "entity", "+").replacen(
            "{\n",
            "{\nlet brace = '}';\n",
            1,
        );
        let second = fixture("beta", "entity", "+").replacen(
            "{\n",
            "{\nlet brace = '}';\n",
            1,
        );
        let mut records = functions("crates/a/src/lib.rs", &first);
        records.extend(functions("crates/b/src/lib.rs", &second));
        assert_eq!(records.len(), 2);
        assert_eq!(compare(&records).exact.len(), 1);
    }

    #[test]
    fn characters_and_lifetimes_have_distinct_tokens() {
        let source = concat!(
            "fn example<'a>(x: &'a str) { ",
            r"let a = '}'; let b = '\''; let c = '\u{7b}'; ",
            r"let d = '\x7d'; let e = b'{'; }",
        );
        let stream = tokens(source);
        for literal in ["'}'", r"'\''", r"'\u{7b}'", r"'\x7d'", "'{'"] {
            assert!(stream.iter().any(|token| token.text == literal));
        }
        assert_eq!(stream.iter().filter(|token| token.text == "'").count(), 2);
        assert_eq!(stream.iter().filter(|token| token.text == "{").count(), 1);
        assert_eq!(stream.iter().filter(|token| token.text == "}").count(), 1);
    }

    #[test]
    fn nested_comment_delimiters_do_not_overlap() {
        let stream = tokens("/* outer /*/ fn fake() {} */ outer */ fn real()");
        assert!(!stream.iter().any(|token| token.text == "fake"));
        assert!(stream.iter().any(|token| token.text == "real"));
    }

    #[test]
    fn function_pointer_types_are_not_named_function_bodies() {
        let source = fixture("example", "entity", "+")
            .replace("example()", "example(callback: fn())");
        assert_eq!(functions("crates/a/src/lib.rs", &source).len(), 1);
    }

    #[test]
    fn array_and_const_generic_signatures_retain_function_bodies() {
        let body = fixture("first", "same", "+");
        for signature in [
            "fn first(items: [u8; 4]) -> [u8; 4]",
            "fn first<const N: usize>() -> [u8; N]",
            "fn first() -> Holder<{ 2 + 3 }>",
            "fn first<F: Fn() -> u8>() -> [u8; 3]",
        ] {
            let source = body.replace("fn first()", signature);
            let parsed = functions("crates/a/src/lib.rs", &source);
            let ordinary = functions("crates/b/src/lib.rs", &body);
            assert_eq!(parsed.len(), 1, "{signature}");
            assert_eq!(compare(parsed.iter().chain(&ordinary)).exact.len(), 1);
        }
    }

    #[test]
    fn raw_literals_do_not_expose_fake_functions() {
        for prefix in ["r", "br", "cr"] {
            let raw =
                format!("const EXAMPLE = {prefix}##\"fn nested() {{ \"}}\"##;");
            let stream = tokens(&raw);
            assert_eq!(
                stream.iter().filter(|token| token.text == "fn").count(),
                0,
            );
            assert_eq!(
                stream.iter().filter(|token| token.text == "{").count(),
                0,
            );
        }
    }
}
