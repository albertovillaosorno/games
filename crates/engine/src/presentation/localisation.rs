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
//   - Locale identifier admission, catalogue limits and message fallback.
// - Must-Not:
//   - Own host locale discovery, text rendering or title translations.
// - Allows:
//   - Inputs: Locale tags, stable keys, Unicode text and explicit limits.
//   - Outputs: Canonical locale identifiers and resolved messages.
//   - Side effects: Stores admitted caller-supplied translations.
// - Split-When:
//   - Message formatting or language negotiation needs separate policy.
// - Merge-When:
//   - One shared presentation mechanism supersedes catalogue admission.
// - Summary:
//   - Bounded, platform-neutral translation catalogue.
// - Description:
//   - Keeps language preferences independent of simulation and protocols.
// - Usage:
//   - Admit locales and messages, check completeness and resolve keys.
// - Defaults:
//   - Caller chooses fallback and allocation budget.
//

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use super::message::{
    MessageArguments, MessageFormatError, render, valid_identifier,
    valid_template,
};

const CATALOG_HEADER: &str = "catalog-v1\n";

/// A case-normalized BCP 47 language, script, region and variant tag.
///
/// Extlang, extension, private-use, grandfathered and registry alias
/// processing is not supported. Admission is syntactic, not a registry lookup.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LocaleTag(String);

/// Catalogue admission failure, with no partial insertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// A caller-selected catalogue capacity would be exceeded.
    Capacity,
    /// The exact locale and key already have a message.
    DuplicateMessage,
    /// A key is empty or contains characters outside the identifier grammar.
    InvalidKey,
    /// A malformed or unsupported locale identifier.
    InvalidLocale,
    /// A versioned catalogue payload is malformed or incomplete.
    InvalidPayload,
    /// A translation is empty, malformed or contains unsupported controls.
    InvalidText,
}

/// Total admitted logical catalogue capacities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogLimits {
    /// Maximum distinct locale identifiers.
    pub locales: NonZeroUsize,
    /// Maximum distinct locale/message pairs.
    pub messages: NonZeroUsize,
    /// Maximum admitted UTF-8 bytes across locale tags, keys and messages.
    pub utf8_bytes: NonZeroUsize,
}

type LocaleMessages = BTreeMap<String, String>;

/// A bounded translation catalogue with a caller-selected fallback locale.
#[derive(Clone, Debug)]
pub struct TranslationCatalog {
    fallback: LocaleTag,
    limits: CatalogLimits,
    locales: BTreeMap<LocaleTag, LocaleMessages>,
    messages: usize,
    utf8_bytes: usize,
}

impl LocaleTag {
    /// Maximum admitted ASCII locale identifier length in bytes.
    pub const MAX_BYTES: usize = 255;

    /// Obtain the normalized, hyphen-separated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Admit and normalize a locale identifier of at most 255 ASCII bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed or unsupported subtags.
    pub fn parse(value: &str) -> Result<Self, CatalogError> {
        if value.len() > Self::MAX_BYTES || !value.is_ascii() {
            return Err(CatalogError::InvalidLocale);
        }
        let mut parts = value.split('-').peekable();
        let language = parts.next().ok_or(CatalogError::InvalidLocale)?;
        if !(2..=8).contains(&language.len()) || !alpha(language) {
            return Err(CatalogError::InvalidLocale);
        }
        let mut tag = language.to_ascii_lowercase();
        if let Some(script) =
            parts.next_if(|part| part.len() == 4 && alpha(part))
        {
            tag.push('-');
            let mut letters = script.chars();
            if let Some(first) = letters.next() {
                tag.push(first.to_ascii_uppercase());
            }
            tag.extend(letters.map(|letter| letter.to_ascii_lowercase()));
        }
        if let Some(region) = parts.next_if(|part| region(part)) {
            tag.push('-');
            tag.push_str(&region.to_ascii_uppercase());
        }
        let mut variants = BTreeSet::new();
        for part in parts {
            if !variant(part) {
                return Err(CatalogError::InvalidLocale);
            }
            let normalized = part.to_ascii_lowercase();
            if !variants.insert(normalized.clone()) {
                return Err(CatalogError::InvalidLocale);
            }
            tag.push('-');
            tag.push_str(&normalized);
        }
        Ok(Self(tag))
    }
}

impl Borrow<str> for LocaleTag {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl TranslationCatalog {
    fn admit_batch<'text, Entries>(
        &mut self,
        locale: &str,
        entries: Entries,
    ) -> Result<(), CatalogError>
    where
        Entries:
            IntoIterator<Item = Result<(&'text str, &'text str), CatalogError>>,
    {
        let tag = LocaleTag::parse(locale)?;
        let mut draft = CatalogDraft::new(self, &tag)?;
        for entry in entries {
            let (key, text) = entry?;
            draft.push(self, &tag, (key, text))?;
        }
        if draft.entries.is_empty() {
            return Err(CatalogError::InvalidPayload);
        }
        self.locales.entry(tag).or_default().extend(
            draft
                .entries
                .into_iter()
                .map(|(key, text)| (String::from(key), String::from(text))),
        );
        self.messages = draft.messages;
        self.utf8_bytes = draft.utf8_bytes;
        Ok(())
    }

    /// Check whether the exact locale provides a key without using fallback.
    #[must_use]
    pub fn contains_exact(&self, locale: &LocaleTag, key: &str) -> bool {
        self.get(locale.as_str(), key).is_some()
    }

    /// The configured final fallback locale.
    #[must_use]
    pub const fn fallback(&self) -> &LocaleTag {
        &self.fallback
    }

    /// Report the first required key missing from the exact locale.
    ///
    /// The caller chooses the key order. Fallback cannot mask incomplete
    /// translations during catalogue admission.
    #[must_use]
    pub fn first_missing<'key>(
        &self,
        locale: &LocaleTag,
        required: &[&'key str],
    ) -> Option<&'key str> {
        required
            .iter()
            .copied()
            .find(|key| !self.contains_exact(locale, key))
    }

    /// Resolve and format a message with typed named parameters.
    ///
    /// Placeholders use `{name}`; doubled braces represent literal braces.
    /// Values are substituted literally, never parsed as another template.
    ///
    /// # Errors
    /// Returns an error for missing messages or invalid parameter usage,
    /// including an exceeded output byte budget.
    pub fn format_message(
        &self,
        locale: &LocaleTag,
        key: &str,
        arguments: MessageArguments<'_>,
    ) -> Result<String, MessageFormatError> {
        let template = self
            .resolve_tag(locale, key)
            .ok_or(MessageFormatError::MissingMessage)?;
        render(template, arguments.parameters, arguments.max_utf8_bytes)
    }

    fn get(&self, locale: &str, key: &str) -> Option<&str> {
        self.locales.get(locale)?.get(key).map(String::as_str)
    }

    /// Admit one unique, nonempty message with a stable ASCII key.
    ///
    /// Keys contain dot-separated nonempty lowercase ASCII identifiers
    /// using letters, digits, underscores and hyphens.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid tags, keys, text, duplicate messages
    /// or exceeded catalogue limits. Rejection leaves existing data intact.
    pub fn insert(
        &mut self,
        locale: &str,
        key: &str,
        text: &str,
    ) -> Result<(), CatalogError> {
        self.admit_batch(locale, [Ok((key, text))])
    }

    /// Admit a versioned, UTF-8 catalogue for one explicitly named locale.
    ///
    /// The payload starts with `catalog-v1` and a newline. Each subsequent
    /// line contains a stable key, a tab and one nonempty translation,
    /// terminated by a newline. Existing locale keys cannot be replaced.
    /// The byte budget also bounds the source before parsing it.
    ///
    /// # Errors
    ///
    /// Rejects malformed UTF-8, incompatible framing, invalid entries,
    /// duplicates and exceeded limits without admitting any of the batch.
    pub fn load_locale(
        &mut self,
        locale: &str,
        bytes: &[u8],
    ) -> Result<(), CatalogError> {
        let source_limit = self
            .limits
            .utf8_bytes
            .get()
            .saturating_add(self.limits.messages.get().saturating_mul(2))
            .saturating_add(CATALOG_HEADER.len());
        if bytes.len() > source_limit {
            return Err(CatalogError::Capacity);
        }
        let text = str::from_utf8(bytes)
            .map_err(|_error| CatalogError::InvalidPayload)?;
        let rows = text
            .strip_prefix(CATALOG_HEADER)
            .and_then(|body| body.strip_suffix('\n'))
            .ok_or(CatalogError::InvalidPayload)?;
        self.admit_batch(
            locale,
            rows.split('\n').map(|line| {
                line.split_once('\t').ok_or(CatalogError::InvalidPayload)
            }),
        )
    }

    fn lookup(&self, mut locale: &str, key: &str) -> Option<&str> {
        loop {
            if let Some(text) = self.get(locale, key) {
                return Some(text);
            }
            locale = locale.rsplit_once('-')?.0;
        }
    }

    /// Create an empty catalogue with explicit logical storage limits.
    #[must_use]
    pub const fn new(fallback: LocaleTag, limits: CatalogLimits) -> Self {
        Self {
            fallback,
            limits,
            locales: BTreeMap::new(),
            messages: 0,
            utf8_bytes: 0,
        }
    }

    /// Resolve a locale string, using fallback for invalid identifiers.
    #[must_use]
    pub fn resolve(&self, locale: &str, key: &str) -> Option<&str> {
        LocaleTag::parse(locale).map_or_else(
            |_| self.lookup(self.fallback.as_str(), key),
            |tag| self.resolve_tag(&tag, key),
        )
    }

    /// Resolve an admitted locale without reparsing it on each lookup.
    ///
    /// Progressively remove rightmost subtags, then use the configured
    /// fallback tag and its parents. Return None if the key is absent.
    #[must_use]
    pub fn resolve_tag(&self, locale: &LocaleTag, key: &str) -> Option<&str> {
        self.lookup(locale.as_str(), key)
            .or_else(|| self.lookup(self.fallback.as_str(), key))
    }
}

#[derive(Debug)]
struct CatalogDraft<'text> {
    entries: BTreeMap<&'text str, &'text str>,
    messages: usize,
    utf8_bytes: usize,
}

impl<'text> CatalogDraft<'text> {
    fn new(
        catalog: &TranslationCatalog,
        tag: &LocaleTag,
    ) -> Result<Self, CatalogError> {
        let new_locale = !catalog.locales.contains_key(tag);
        if new_locale && catalog.locales.len() >= catalog.limits.locales.get() {
            return Err(CatalogError::Capacity);
        }
        let utf8_bytes = if new_locale {
            catalog
                .utf8_bytes
                .checked_add(tag.as_str().len())
                .ok_or(CatalogError::Capacity)?
        } else {
            catalog.utf8_bytes
        };
        Ok(Self {
            entries: BTreeMap::new(),
            messages: catalog.messages,
            utf8_bytes,
        })
    }

    fn push(
        &mut self,
        catalog: &TranslationCatalog,
        tag: &LocaleTag,
        (key, text): (&'text str, &'text str),
    ) -> Result<(), CatalogError> {
        if catalog.contains_exact(tag, key) || self.entries.contains_key(key) {
            return Err(CatalogError::DuplicateMessage);
        }
        if self.messages >= catalog.limits.messages.get() {
            return Err(CatalogError::Capacity);
        }
        let bytes = self
            .utf8_bytes
            .checked_add(key.len())
            .and_then(|size| size.checked_add(text.len()))
            .ok_or(CatalogError::Capacity)?;
        if bytes > catalog.limits.utf8_bytes.get() {
            return Err(CatalogError::Capacity);
        }
        if !valid_identifier(key) {
            return Err(CatalogError::InvalidKey);
        }
        if text.is_empty()
            || !valid_template(text)
            || text.chars().any(|ch| ch.is_control() && ch != '\n')
        {
            return Err(CatalogError::InvalidText);
        }
        let _previous = self.entries.insert(key, text);
        self.messages = self.messages.saturating_add(1);
        self.utf8_bytes = bytes;
        Ok(())
    }
}

fn alpha(part: &str) -> bool {
    part.bytes().all(|byte| byte.is_ascii_alphabetic())
}

fn region(part: &str) -> bool {
    (part.len() == 2 && alpha(part))
        || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn variant(part: &str) -> bool {
    ((5..=8).contains(&part.len())
        || (part.len() == 4
            && part
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_digit())))
        && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::num::NonZeroUsize;

    use super::{CatalogError, CatalogLimits, LocaleTag, TranslationCatalog};
    use crate::presentation::{
        MessageArguments, MessageFormatError, MessageParameter, MessageValue,
    };

    fn check<T, U>(actual: &T, expected: &U) -> Result<(), String>
    where
        T: Debug + PartialEq<U> + ?Sized,
        U: Debug + ?Sized,
    {
        (actual == expected)
            .then_some(())
            .ok_or_else(|| format!("expected {expected:?}, got {actual:?}"))
    }

    fn admitted<T>(result: Result<T, CatalogError>) -> Result<T, String> {
        result.map_err(|error| format!("catalogue admission: {error:?}"))
    }

    fn limits(bytes: usize) -> CatalogLimits {
        CatalogLimits {
            locales: NonZeroUsize::new(3).unwrap_or(NonZeroUsize::MIN),
            messages: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            utf8_bytes: NonZeroUsize::new(bytes).unwrap_or(NonZeroUsize::MIN),
        }
    }

    #[test]
    fn locale_casing_and_grammar_are_deterministic() -> Result<(), String> {
        for (input, expected) in [
            ("EN", "en"),
            ("eS-mX", "es-MX"),
            ("zh-hANT-tw", "zh-Hant-TW"),
            ("es-419", "es-419"),
            ("sl-ROZAJ-1994", "sl-rozaj-1994"),
        ] {
            check(admitted(LocaleTag::parse(input))?.as_str(), expected)?;
        }
        for invalid in [
            "",
            "e",
            "es_MX",
            "es--MX",
            "es-MX-",
            "es-1234-MX",
            "es-\u{d1}X",
            "en-US-u-ca-gregory",
            "x-private",
            "sl-rozaj-ROZAJ",
        ] {
            check(
                &LocaleTag::parse(invalid),
                &Err(CatalogError::InvalidLocale),
            )?;
        }
        Ok(())
    }

    #[test]
    fn locale_and_regional_fallback_preserve_messages() -> Result<(), String> {
        let mut catalog = TranslationCatalog::new(
            admitted(LocaleTag::parse("en"))?,
            limits(256),
        );
        admitted(catalog.insert("en", "start", "Start"))?;
        admitted(catalog.insert("en", "quit", "Quit"))?;
        admitted(catalog.insert("es", "start", "Comenzar"))?;
        admitted(catalog.insert("es-MX", "start", "Jugar"))?;
        check(&catalog.resolve("ES-mx", "start"), &Some("Jugar"))?;
        check(&catalog.resolve("es-ES", "start"), &Some("Comenzar"))?;
        check(&catalog.resolve("es-MX", "quit"), &Some("Quit"))?;
        check(&catalog.resolve("fr", "start"), &Some("Start"))?;
        check(&catalog.resolve("es_MX", "start"), &Some("Start"))?;
        check(&catalog.resolve("fr", "missing"), &None)?;
        check(
            &catalog
                .contains_exact(&admitted(LocaleTag::parse("es-MX"))?, "quit"),
            &false,
        )
    }

    #[test]
    fn script_and_fallback_parents_are_considered() -> Result<(), String> {
        let mut catalog = TranslationCatalog::new(
            admitted(LocaleTag::parse("en-US"))?,
            limits(256),
        );
        admitted(catalog.insert("en", "menu.start", "Start"))?;
        admitted(catalog.insert("zh", "menu.start", "\u{5f00}\u{59cb}"))?;
        admitted(catalog.insert("zh-Hant", "menu.start", "\u{958b}\u{59cb}"))?;
        let regional = admitted(LocaleTag::parse("zh-hant-tw"))?;
        check(
            &catalog.resolve_tag(&regional, "menu.start"),
            &Some("\u{958b}\u{59cb}"),
        )?;
        check(
            &catalog.resolve("zh-Hans-CN", "menu.start"),
            &Some("\u{5f00}\u{59cb}"),
        )?;
        check(&catalog.resolve("fr", "menu.start"), &Some("Start"))
    }

    #[test]
    fn rejection_does_not_consume_budget_or_replace_text() -> Result<(), String>
    {
        let mut catalog = TranslationCatalog::new(
            admitted(LocaleTag::parse("es-MX"))?,
            limits(14),
        );
        check(
            &catalog.insert("es-MX", "a..b", "x"),
            &Err(CatalogError::InvalidKey),
        )?;
        check(
            &catalog.insert("es-MX", "a", "\0"),
            &Err(CatalogError::InvalidText),
        )?;
        admitted(catalog.insert("es-MX", "a", "\u{a1}S\u{ed}!"))?;
        check(
            &catalog.insert("ES-mx", "a", "No"),
            &Err(CatalogError::DuplicateMessage),
        )?;
        check(
            &catalog.insert("en", "b", "No"),
            &Err(CatalogError::Capacity),
        )?;
        admitted(catalog.insert("es-MX", "b", "x"))?;
        check(&catalog.resolve("es-MX", "a"), &Some("\u{a1}S\u{ed}!"))?;
        check(&catalog.resolve("es-MX", "b"), &Some("x"))?;
        check(
            &catalog.insert("es-MX", "c", "y"),
            &Err(CatalogError::Capacity),
        )
    }

    #[test]
    fn exact_completeness_and_batch_admission_are_transactional()
    -> Result<(), String> {
        let en = admitted(LocaleTag::parse("en"))?;
        let es = admitted(LocaleTag::parse("es-MX"))?;
        let mut catalog = TranslationCatalog::new(en, limits(256));
        admitted(catalog.load_locale(
            "en",
            b"catalog-v1\nmenu.play\tPlay\nmenu.back\tBack\n",
        ))?;
        admitted(
            catalog.load_locale("es-MX", b"catalog-v1\nmenu.play\tJugar\n"),
        )?;
        check(
            &catalog.first_missing(&es, &["menu.play", "menu.back"]),
            &Some("menu.back"),
        )?;
        check(&catalog.resolve_tag(&es, "menu.back"), &Some("Back"))?;
        let malformed = b"catalog-v1\nmenu.back\tVolver\ninvalid\n";
        check(
            &catalog.load_locale("es-MX", malformed),
            &Err(CatalogError::InvalidPayload),
        )?;
        check(&catalog.contains_exact(&es, "menu.back"), &false)?;
        admitted(
            catalog.load_locale("es-MX", b"catalog-v1\nmenu.back\tVolver\n"),
        )?;
        check(
            &catalog.first_missing(&es, &["menu.play", "menu.back"]),
            &None,
        )?;
        check(&catalog.resolve_tag(&es, "menu.back"), &Some("Volver"))
    }

    #[test]
    fn batch_rejects_bad_framing_duplicate_and_excess_source()
    -> Result<(), String> {
        let mut catalog = TranslationCatalog::new(
            admitted(LocaleTag::parse("en"))?,
            limits(64),
        );
        for bytes in [
            b"catalog-v2\na\tb\n".as_slice(),
            b"catalog-v1\na\tb",
            b"catalog-v1\na\t\xff\n",
            b"catalog-v1\n",
        ] {
            check(
                &catalog.load_locale("en", bytes),
                &Err(CatalogError::InvalidPayload),
            )?;
        }
        check(
            &catalog.load_locale("en", b"catalog-v1\na\tb\na\tc\n"),
            &Err(CatalogError::DuplicateMessage),
        )?;
        check(
            &catalog.load_locale("en", &[b'x'; 256]),
            &Err(CatalogError::Capacity),
        )?;
        check(&catalog.resolve("en", "a"), &None)?;
        admitted(
            catalog.load_locale(
                "en",
                "catalog-v1\na\t\u{bf}S\u{ed}?\n".as_bytes(),
            ),
        )?;
        check(&catalog.resolve("en", "a"), &Some("\u{bf}S\u{ed}?"))
    }

    #[test]
    fn locale_and_message_counts_are_independently_bounded()
    -> Result<(), String> {
        let mut catalog = TranslationCatalog::new(
            admitted(LocaleTag::parse("en"))?,
            CatalogLimits {
                locales: NonZeroUsize::MIN,
                messages: NonZeroUsize::MIN.saturating_add(1),
                utf8_bytes: NonZeroUsize::MIN.saturating_add(255),
            },
        );
        admitted(catalog.insert("en", "one", "One"))?;
        check(
            &catalog.insert("es", "one", "Uno"),
            &Err(CatalogError::Capacity),
        )?;
        admitted(catalog.insert("en", "two", "Two"))?;
        check(
            &catalog.insert("en", "three", "Three"),
            &Err(CatalogError::Capacity),
        )?;
        check(&catalog.resolve("es", "one"), &Some("One"))
    }

    #[test]
    fn admitted_templates_use_locale_fallback_and_typed_arguments()
    -> Result<(), String> {
        let en = admitted(LocaleTag::parse("en"))?;
        let es = admitted(LocaleTag::parse("es-MX"))?;
        let fr = admitted(LocaleTag::parse("fr"))?;
        let mut catalog = TranslationCatalog::new(en, limits(256));
        admitted(catalog.insert("en", "menu.score", "{score} points"))?;
        admitted(catalog.insert("es-MX", "menu.score", "Puntos: {score}"))?;
        check(
            &catalog.insert("en", "menu.invalid", "{score"),
            &Err(CatalogError::InvalidText),
        )?;
        let parameters = [MessageParameter {
            name: "score",
            value: MessageValue::Unsigned(50),
        }];
        let arguments = MessageArguments {
            max_utf8_bytes: NonZeroUsize::MIN.saturating_add(31),
            parameters: &parameters,
        };
        check(
            &catalog.format_message(&es, "menu.score", arguments),
            &Ok(String::from("Puntos: 50")),
        )?;
        check(
            &catalog.format_message(&fr, "menu.score", arguments),
            &Ok(String::from("50 points")),
        )?;
        check(
            &catalog.format_message(&es, "menu.unknown", arguments),
            &Err(MessageFormatError::MissingMessage),
        )
    }
}
