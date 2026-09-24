//! The rules a field applies to a submitted string, and their messages
//! (GH #243).
//!
//! Presence, email and the typed parse are the rules a form applies without a
//! `Cx`. `TextInput`, `Select`, `Textarea` and `FileUpload` each read their
//! errors from one [`Rules`], and the repeater walk in `Schema::validate`
//! words its own required error with [`required_error`]. The rules that need a
//! `Cx` — `check_unique` in `panel::forms` and `Select`'s option-existence
//! probe — stay with their callers.

use email_address::{EmailAddress, Options};

/// A typed column's own spelling rules, for the typed constructors (GH #192).
///
/// The form edge is text: a control submits a `String`, so a column that is not
/// a `String` needs a `Display` to render and a `FromStr` to read back. `NOUN`
/// names the type in the error a user sees (`` `2024-13-01` is not a valid
/// date ``), because "invalid" alone does not tell them what was expected.
///
/// Implemented for the types a panel actually binds rather than as a blanket
/// over `FromStr`: a blanket would let a field declare a parse only to have no
/// sensible message for it, and the set is small.
pub trait TypedValue: std::fmt::Display + std::str::FromStr {
    /// What this type is called in a validation error.
    const NOUN: &'static str;
}

/// The integer types a typed leaf can bind (GH #191 widened this from the three
/// GH #192 shipped): a derived embedded value classifies a field as a leaf by
/// its type, so the set of leaf-capable types has to be the whole integer
/// family rather than the ones the showcase happened to use.
macro_rules! typed_whole_number {
    ($($ty:ty),* $(,)?) => {
        $(
            impl TypedValue for $ty {
                const NOUN: &'static str = "whole number";
            }
        )*
    };
}

typed_whole_number!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl TypedValue for bool {
    const NOUN: &'static str = "yes/no value";
}

impl TypedValue for f32 {
    const NOUN: &'static str = "number";
}

impl TypedValue for f64 {
    const NOUN: &'static str = "number";
}

impl TypedValue for uuid::Uuid {
    const NOUN: &'static str = "identifier";
}

impl TypedValue for jiff::Timestamp {
    const NOUN: &'static str = "timestamp";
}

/// How a typed field reads a submitted string back (GH #192).
///
/// A `String` field keeps the identity parser — store what was typed — so the
/// untyped path stays byte-for-byte what it was. A typed field gets a parser
/// that validates at the form edge and normalises through `Display`.
type ValueParser = std::sync::Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// The parser a typed field binds: reject what `T` cannot parse, and store what
/// `T`'s own `Display` produces for it (GH #192).
///
/// Normalising through `Display` is the point, not a side effect: it is what
/// makes an edit that never touched the field write back a value of the same
/// shape it read, rather than an unreviewed re-spelling. A `jiff::Timestamp`
/// submitted as `2024-01-02T03:04:05Z` is stored as that type's canonical form.
fn typed_parser<T: TypedValue>() -> ValueParser {
    std::sync::Arc::new(|value: &str| match value.parse::<T>() {
        Ok(parsed) => Ok(parsed.to_string()),
        Err(_) => Err(format!("`{value}` is not a valid {}", T::NOUN)),
    })
}

/// The rules a field declares on top of presence, and the wording of every
/// message they produce (GH #243).
///
/// Presence is not one of them: whether an empty submit is refused is a
/// declaration on the field — a non-nullable column is required, a unique one
/// is never empty (GH #189) — so [`Rules::validate`] takes the caller's
/// resolved flag and a field with no other rule holds nothing at all.
#[derive(Clone, Default)]
pub(crate) struct Rules {
    email: bool,
    parser: Option<ValueParser>,
}

impl Rules {
    /// A field with no declared rule: presence alone.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add the typed parse rule for `T` (GH #192).
    pub(crate) fn typed<T: TypedValue>(mut self) -> Self {
        self.parser = Some(typed_parser::<T>());
        self
    }

    /// Turn on the email rule.
    pub(crate) fn set_email(&mut self) {
        self.email = true;
    }

    /// Whether the email rule is on — the control's `type` attribute reads it.
    pub(crate) fn is_email(&self) -> bool {
        self.email
    }

    /// Whether a typed parse rule is on.
    pub(crate) fn is_typed(&self) -> bool {
        self.parser.is_some()
    }

    /// Validate `value`, in rule order: presence, email, typed parse.
    ///
    /// `required` is the caller's resolved presence flag. An empty submit is
    /// the presence rule's business alone: the email and parse rules skip it,
    /// so an optional field accepts empty whatever else it declares.
    pub(crate) fn validate(&self, label: &str, required: bool, value: &str) -> Vec<String> {
        let v = value.trim();
        let mut errs = Vec::new();
        if required && v.is_empty() {
            errs.push(required_error(label));
        }
        if self.email && !v.is_empty() && !is_email(v) {
            errs.push(format!("{label} must be a valid email"));
        }
        // The typed rule (GH #192) runs last and only on a value the rules
        // above accepted.
        if !v.is_empty()
            && errs.is_empty()
            && let Some(parser) = &self.parser
            && let Err(message) = parser(v)
        {
            errs.push(message);
        }
        errs
    }

    /// The stored spelling of a submission the caller has already validated
    /// (GH #192).
    ///
    /// The typed parse's `Display` for a typed field, the trimmed submission
    /// for an untyped one — so a value the user left alone is written back in
    /// the shape the record fn wrote it, not in whichever spelling the browser
    /// sent. Callers that have not validated must not use this: it reports a
    /// failure rather than guessing.
    pub(crate) fn normalize(&self, value: &str) -> Result<String, String> {
        let v = value.trim();
        match &self.parser {
            Some(parser) => parser(v),
            None => Ok(v.to_string()),
        }
    }
}

/// The message for an empty submit, shared with the repeater walk in
/// `Schema::validate`.
pub(crate) fn required_error(label: &str) -> String {
    format!("{label} is required")
}

/// The longest address the rule accepts. RFC 5321 §4.5.3.1.3 carries 256
/// octets including the angle brackets, and `email_address` bounds the local
/// part and the domain separately rather than their sum.
const EMAIL_MAX_LENGTH: usize = 254;

/// Whether `value` is an address the email rule accepts.
///
/// `email_address` parses the RFC 5322 grammar. `with_required_tld` gives a
/// text domain two labels, so `a@b` and `a@b..c` are refused; a bracketed
/// literal takes the crate's other domain path and passes whatever its label
/// count, so `a@[127.0.0.1]` and `a@[IPv6:::1]` are accepted.
/// `without_display_text` refuses `Ada <ada@example.com>`, a header rather
/// than an address.
///
/// The rest of the accepted set is the crate's:
///
/// - a quoted local part, `"a b"@example.com`;
/// - a unicode local part or domain, `用户@例え.jp`;
/// - an unquoted local part refuses `(`, `)`, `,`, `:`, `;`, `<`, `>`, `[`, `]`, `\`, `"` and
///   space, so `a,b@b.com`, `a(b@b.com` and `a:b@b.com` are refused;
/// - a domain label starts and ends with a letter or digit, so `user@my_host.com` is accepted and
///   `a@b!.com` is refused;
/// - a single-character TLD, `a@b.c`.
///
/// The crate bounds the local part at 64 octets and the domain at 254;
/// [`EMAIL_MAX_LENGTH`] caps their sum.
fn is_email(value: &str) -> bool {
    value.len() <= EMAIL_MAX_LENGTH
        && EmailAddress::parse_with_options(
            value,
            Options::default()
                .with_required_tld()
                .without_display_text(),
        )
        .is_ok()
}
