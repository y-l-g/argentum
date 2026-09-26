//! The rules a field applies to a submitted string, and their messages.
//!
//! Presence, email and the typed parse are the rules a form applies without a
//! `Cx`. `TextInput`, `Select`, `Textarea` and `FileUpload` each read their
//! errors from one [`Rules`], and the repeater walk in `Schema::validate`
//! words its own required error with [`required_error`]. The rules that need a
//! `Cx` — `check_unique` in `panel::forms` and `Select`'s option-existence
//! probe — stay with their callers.

use email_address::{EmailAddress, Options};

/// A typed column's own spelling rules, for the typed constructors.
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

    /// Whether a successful `FromStr` is a value the form accepts.
    ///
    /// `FromStr` is the first word, not the last: `f32`/`f64` parse `NaN`,
    /// `inf` and `-inf` (and a literal that overflows, like `1e400`), none of
    /// which is a number a field can hand back — the stored spelling would be
    /// one no user typed. The default accepts whatever `FromStr` produced, so
    /// only a type with such a gap implements this.
    fn accepts(value: &Self) -> bool {
        let _ = value;
        true
    }
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

    fn accepts(value: &Self) -> bool {
        value.is_finite()
    }
}

impl TypedValue for f64 {
    const NOUN: &'static str = "number";

    fn accepts(value: &Self) -> bool {
        value.is_finite()
    }
}

impl TypedValue for uuid::Uuid {
    const NOUN: &'static str = "identifier";
}

impl TypedValue for jiff::Timestamp {
    const NOUN: &'static str = "timestamp";
}

/// Whether `T` is the timestamp type a typed field renders as `datetime-local`.
pub(crate) fn is_timestamp<T>() -> bool {
    std::any::type_name::<T>() == std::any::type_name::<jiff::Timestamp>()
}

/// Parse a timestamp submission into its stored spelling.
///
/// Accepts what the type accepts (RFC 3339) plus the `datetime-local` shapes a
/// browser sends (`YYYY-MM-DDTHH:MM`, with optional seconds and fraction),
/// assumed UTC. Returns the type's canonical `Display`, so a re-read is a
/// fixpoint.
pub(crate) fn parse_timestamp_storage(value: &str) -> Result<String, String> {
    let error = || format!("`{value}` is not a valid timestamp");
    let trimmed = value.trim();
    if let Ok(parsed) = trimmed.parse::<jiff::Timestamp>() {
        return Ok(parsed.to_string());
    }
    let normalized = normalize_datetime_local(trimmed).ok_or_else(error)?;
    match normalized.parse::<jiff::Timestamp>() {
        Ok(parsed) => Ok(parsed.to_string()),
        Err(_) => Err(error()),
    }
}

/// A `datetime-local` value as the RFC 3339 string a timestamp parses, or
/// `None` when the shape is not one the control sends.
fn normalize_datetime_local(value: &str) -> Option<String> {
    let t = value.find('T')?;
    let after_t = &value[t + 1..];
    // A zone offset after the `T` means the value already names its offset;
    // the direct parse above refused it, so it is not a valid timestamp. A
    // valid control value carries neither `+` nor `-` after the `T`.
    if after_t.contains('+') || after_t.contains('-') {
        return None;
    }
    if value.ends_with(['Z', 'z']) {
        return None;
    }
    // `YYYY-MM-DDTHH:MM` needs seconds before the zone; longer shapes carry
    // their own seconds and fraction.
    if value.len() == 16 {
        Some(format!("{value}:00Z"))
    } else {
        Some(format!("{value}Z"))
    }
}

/// A stored timestamp as the `datetime-local` value its control renders.
///
/// The control carries no zone, so the instant renders in UTC truncated to the
/// minute. Anything that is not a timestamp (empty, old free-text) renders
/// empty: there is no back-compat spelling for free-text.
pub(crate) fn format_timestamp_input(storage: &str) -> String {
    let trimmed = storage.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match trimmed.parse::<jiff::Timestamp>() {
        Ok(parsed) => parsed.strftime("%Y-%m-%dT%H:%M").to_string(),
        Err(_) => String::new(),
    }
}

/// How a typed field reads a submitted string back.
///
/// A `String` field keeps the identity parser — store what was typed — so the
/// untyped path stays byte-for-byte what it was. A typed field gets a parser
/// that validates at the form edge and normalises through `Display`.
type ValueParser = std::sync::Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// The parser a typed field binds: reject what `T` cannot parse — or parses
/// into a value it does not accept — and store what `T`'s own
/// `Display` produces for it.
///
/// Normalising through `Display` is the point, not a side effect: it is what
/// makes an edit that never touched the field write back a value of the same
/// shape it read, rather than an unreviewed re-spelling. A `jiff::Timestamp`
/// submitted as `2024-01-02T03:04:05Z` is stored as that type's canonical form.
fn typed_parser<T: TypedValue>() -> ValueParser {
    std::sync::Arc::new(|value: &str| match value.parse::<T>() {
        Ok(parsed) if T::accepts(&parsed) => Ok(parsed.to_string()),
        _ => Err(format!("`{value}` is not a valid {}", T::NOUN)),
    })
}

/// The rules a field declares on top of presence, and the wording of every
/// message they produce.
///
/// Presence is not one of them: whether an empty submit is refused is a
/// declaration on the field — a non-nullable column is required, a unique one
/// is never empty — so [`Rules::validate`] takes the caller's
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

    /// Add the typed parse rule for `T`.
    pub(crate) fn typed<T: TypedValue>(mut self) -> Self {
        if is_timestamp::<T>() {
            self.parser = Some(std::sync::Arc::new(parse_timestamp_storage));
        } else {
            self.parser = Some(typed_parser::<T>());
        }
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
        // The typed rule runs last and only on a value the rules
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

    /// The stored spelling of a submission the caller has already validated.
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

#[cfg(test)]
mod tests {
    use super::Rules;

    /// The typed rule words one message for a value the type cannot parse and
    /// for one it parses into a value it does not accept.
    fn rejected(input: &str) -> String {
        format!("`{input}` is not a valid number")
    }

    /// GH #297: `f32`/`f64` `FromStr` accepts `NaN`, `inf` and `-inf`, and the
    /// typed rule passed them through as stored values. They are refused like
    /// any other unparseable submission.
    #[test]
    fn a_float_refuses_a_non_finite_parse() {
        for input in ["NaN", "nan", "inf", "-inf", "infinity", "1e400"] {
            let f64_errs = Rules::new().typed::<f64>().validate("Amount", true, input);
            let f32_errs = Rules::new().typed::<f32>().validate("Amount", true, input);
            assert_eq!(f64_errs, vec![rejected(input)], "f64 accepted {input}");
            assert_eq!(f32_errs, vec![rejected(input)], "f32 accepted {input}");
        }
    }

    /// The same field keeps accepting a finite number, and normalises it
    /// through `f64`'s `Display` as any typed rule does.
    #[test]
    fn a_float_accepts_and_normalises_a_finite_number() {
        let rules = Rules::new().typed::<f64>();
        assert!(rules.validate("Amount", true, "-0.25").is_empty());
        assert!(rules.validate("Amount", true, "1e3").is_empty());
        assert_eq!(
            rules.normalize(" 12.50 ").expect("12.50 parses"),
            "12.5",
            "a finite value stores its own spelling"
        );
    }
}
