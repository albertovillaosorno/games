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
//   - Stable message identifiers and bounded, typed template formatting.
// - Must-Not:
//   - Execute translation content or perform locale-specific game rules.
// - Allows:
//   - Inputs: Admitted text, named parameters and an output byte budget.
//   - Outputs: A rendered Unicode message or a typed rejection.
//   - Side effects: Bounded in-memory message construction.
// - Split-When:
//   - Plural or grammatical selection requires independent contracts.
// - Merge-When:
//   - Another shared message formatter provides the same typed rules.
// - Summary:
//   - Named, reorderable message parameters with literal substitution.
// - Description:
//   - Translators may reorder or repeat named values without executable format
//     strings, and parameter values are never parsed as templates.
// - Usage:
//   - Resolve a catalogue key and render with an explicit output limit.
// - Defaults:
//   - Reject malformed syntax and missing, duplicate or unused values.
//

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

/// A typed value supplied by the caller, never by translation content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageValue<'text> {
    /// A signed integer in decimal notation.
    Integer(i64),
    /// Literal Unicode text.
    Text(&'text str),
    /// An unsigned integer in decimal notation.
    Unsigned(u64),
}

/// One named value for a localised message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageParameter<'text> {
    /// Stable identifier referenced by the translated template.
    pub name: &'text str,
    /// The typed replacement value.
    pub value: MessageValue<'text>,
}

/// Named parameters and the logical UTF-8 output capacity.
#[derive(Clone, Copy, Debug)]
pub struct MessageArguments<'text> {
    /// Maximum rendered UTF-8 bytes, without truncation.
    pub max_utf8_bytes: NonZeroUsize,
    /// Typed substitutions supplied by the caller.
    pub parameters: &'text [MessageParameter<'text>],
}

/// Formatting failure; no partial message is returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageFormatError {
    /// The caller's output byte budget or allocation would be exceeded.
    Capacity,
    /// A caller-provided name occurs more than once.
    DuplicateParameter,
    /// A name or textual parameter is invalid.
    InvalidParameter,
    /// The template contains malformed placeholders or braces.
    MalformedTemplate,
    /// The requested catalogue key is unavailable.
    MissingMessage,
    /// The template references a value not provided by the caller.
    MissingParameter,
    /// A provided value is not referenced by the translation.
    UnusedParameter,
}

#[derive(Clone, Copy, Debug)]
enum Part<'text> {
    Literal(&'text str),
    Placeholder(&'text str),
}

type NextPart<'text> = Option<(Part<'text>, &'text str)>;
type ParameterMap<'text> = BTreeMap<&'text str, MessageValue<'text>>;

impl MessageValue<'_> {
    fn append(
        self,
        output: &mut String,
        limit: NonZeroUsize,
    ) -> Result<(), MessageFormatError> {
        match self {
            Self::Integer(value) => append(output, &value.to_string(), limit),
            Self::Text(value) => {
                if value.chars().any(|ch| ch.is_control() && ch != '\n') {
                    return Err(MessageFormatError::InvalidParameter);
                }
                append(output, value, limit)
            },
            Self::Unsigned(value) => append(output, &value.to_string(), limit),
        }
    }
}

fn append(
    output: &mut String,
    value: &str,
    limit: NonZeroUsize,
) -> Result<(), MessageFormatError> {
    let size = output
        .len()
        .checked_add(value.len())
        .ok_or(MessageFormatError::Capacity)?;
    if size > limit.get() {
        return Err(MessageFormatError::Capacity);
    }
    output
        .try_reserve_exact(value.len())
        .map_err(|_error| MessageFormatError::Capacity)?;
    output.push_str(value);
    Ok(())
}

fn next_part(input: &str) -> Result<NextPart<'_>, MessageFormatError> {
    if input.is_empty() {
        return Ok(None);
    }
    let Some(index) = input.find(['{', '}']) else {
        return Ok(Some((Part::Literal(input), "")));
    };
    if index != 0 {
        let (literal, rest) = input
            .split_at_checked(index)
            .ok_or(MessageFormatError::MalformedTemplate)?;
        return Ok(Some((Part::Literal(literal), rest)));
    }
    if let Some(rest) = input.strip_prefix("{{") {
        return Ok(Some((Part::Literal("{"), rest)));
    }
    if let Some(rest) = input.strip_prefix("}}") {
        return Ok(Some((Part::Literal("}"), rest)));
    }
    let (name, rest) = input
        .strip_prefix('{')
        .and_then(|tail| tail.split_once('}'))
        .filter(|(name, _rest)| valid_identifier(name))
        .ok_or(MessageFormatError::MalformedTemplate)?;
    Ok(Some((Part::Placeholder(name), rest)))
}

fn parameters<'text>(
    input: &[MessageParameter<'text>],
) -> Result<ParameterMap<'text>, MessageFormatError> {
    let mut values = BTreeMap::new();
    for parameter in input {
        if !valid_identifier(parameter.name) {
            return Err(MessageFormatError::InvalidParameter);
        }
        if values.insert(parameter.name, parameter.value).is_some() {
            return Err(MessageFormatError::DuplicateParameter);
        }
    }
    Ok(values)
}

pub(super) fn render(
    mut input: &str,
    values: &[MessageParameter<'_>],
    limit: NonZeroUsize,
) -> Result<String, MessageFormatError> {
    let provided = parameters(values)?;
    let mut used = BTreeSet::new();
    let mut output = String::new();
    while let Some((part, rest)) = next_part(input)? {
        match part {
            Part::Literal(text) => append(&mut output, text, limit)?,
            Part::Placeholder(name) => {
                let value = provided
                    .get(name)
                    .ok_or(MessageFormatError::MissingParameter)?;
                value.append(&mut output, limit)?;
                let _new = used.insert(name);
            },
        }
        input = rest;
    }
    if used.len() != provided.len() {
        return Err(MessageFormatError::UnusedParameter);
    }
    Ok(output)
}

pub(super) fn valid_identifier(key: &str) -> bool {
    key.split('.').all(|part| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'-'
                    || byte == b'_'
            })
    })
}

pub(super) fn valid_template(mut input: &str) -> bool {
    loop {
        match next_part(input) {
            Ok(Some((_part, rest))) => input = rest,
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::{
        MessageFormatError, MessageParameter, MessageValue, render,
        valid_template,
    };

    fn limit(bytes: usize) -> NonZeroUsize {
        NonZeroUsize::new(bytes).unwrap_or(NonZeroUsize::MIN)
    }

    const fn text<'text>(
        name: &'text str,
        value: &'text str,
    ) -> MessageParameter<'text> {
        MessageParameter {
            name,
            value: MessageValue::Text(value),
        }
    }

    #[test]
    fn parameters_are_reordered_repeated_and_never_executed() {
        let values = [text("name", "A{not-a-parameter}"), MessageParameter {
            name: "score",
            value: MessageValue::Integer(-5),
        }];
        assert_eq!(
            render("{score}: {name}, {score}", &values, limit(64)),
            Ok(String::from("-5: A{not-a-parameter}, -5"))
        );
        let value = MessageParameter {
            name: "count",
            value: MessageValue::Unsigned(u64::MAX),
        };
        assert_eq!(
            render("{{{count}}}", &[value], limit(22)),
            Ok(String::from("{18446744073709551615}"))
        );
    }

    #[test]
    fn malformed_templates_and_arguments_are_not_silently_accepted() {
        for template in ["{", "}", "{Name}", "{a b}", "{}", "{a{b}"] {
            assert!(!valid_template(template));
        }
        assert!(valid_template("{{plain}}: {menu.count}"));
        assert_eq!(
            render("{a}", &[], limit(10)),
            Err(MessageFormatError::MissingParameter)
        );
        assert_eq!(
            render("{a}", &[text("a", "x"), text("a", "y")], limit(10)),
            Err(MessageFormatError::DuplicateParameter)
        );
        assert_eq!(
            render("{a}", &[text("a", "x"), text("b", "y")], limit(10)),
            Err(MessageFormatError::UnusedParameter)
        );
        assert_eq!(
            render("{a}", &[text("a", "\0")], limit(10)),
            Err(MessageFormatError::InvalidParameter)
        );
    }

    #[test]
    fn unicode_output_budget_is_measured_without_truncation() {
        let parameter = [text("value", "\u{a1}S\u{ed}!")];
        assert_eq!(
            render("{value}", &parameter, limit(6)),
            Ok(String::from("\u{a1}S\u{ed}!"))
        );
        assert_eq!(
            render("{value}", &parameter, limit(5)),
            Err(MessageFormatError::Capacity)
        );
        assert_eq!(
            render("{value}{value}", &parameter, limit(11)),
            Err(MessageFormatError::Capacity)
        );
    }
}
