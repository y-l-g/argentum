//! Field leaves — `TextInput`, `Textarea`, `Select`, `FileUpload`.
//!
//! Typed inputs bound to Toasty field lenses; the lens is the single
//! source of truth for the field name, label, and required default.

mod file_upload;
mod select;
mod text_input;
mod textarea;

use argentum_ui::{
    field as ui_field, field_content as ui_field_content, field_error as ui_field_error,
    field_label as ui_field_label, field_title as ui_field_title,
};
pub use file_upload::FileUpload;
pub use select::Select;
pub use text_input::TextInput;
pub use textarea::Textarea;
use topcoat::{Result, context::Cx, view::*};

/// How a read-only value is presented.
///
/// Two shapes, because the difference is content, not styling: prose wraps
/// mid-word never, and an identifier (a stored path, an address) has no spaces
/// to break at, so it breaks anywhere and sets in mono. A `bool` parameter
/// reads as validation metadata at a call site, so the two shapes are the
/// `ValueKind` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    /// Wrapping text: a title, a body, a description.
    Prose,
    /// A path, a key, an address — no spaces to break at.
    Machine,
}

/// The read-only half of a field: the label with the record's stored
/// value under it, no control and no validation slot.
///
/// Every field type renders its view through this, so a detail page reads
/// uniformly and the one place that decides "how does a value look" lives here
/// rather than in the page handler. The label is the same `field_label` the
/// form uses, inside the same `field` family, so a field is recognisable across
/// the two pages.
///
/// An absent value and an empty one render the same, deliberately: the
/// framework stores `""` rather than NULL, so a stored record cannot
/// tell them apart and the page must not pretend otherwise.
fn render_value<'a>(
    cx: &'a Cx,
    label: &str,
    value: Option<&str>,
    kind: ValueKind,
) -> Result<BoxView<'a>> {
    let text = value.unwrap_or_default().to_string();
    let value_class = match kind {
        ValueKind::Prose => "text-sm break-words whitespace-pre-wrap",
        ValueKind::Machine => "text-sm font-mono break-all whitespace-pre-wrap",
    };
    let value = view! { cx => <div class=(value_class)>(text)</div> }.boxed();
    render_value_view(cx, label, value)
}

/// The field chrome `render_value` puts around a rendered value, for
/// a field whose read-only value is not a plain string.
///
/// A `FileUpload` renders its stored path as a link and supplies that
/// view here, so the label, the `field` family and the `ac-field` marker stay
/// the ones every other read-only field renders through.
fn render_value_view<'a>(cx: &'a Cx, label: &str, value: BoxView<'a>) -> Result<BoxView<'a>> {
    let label = label.to_string();
    Ok(view! {
        cx =>
        ui_field(
            attrs: attributes! { class="ac-field" },
            ui_field_content(
                ui_field_title((label))
                (value)
            )
        )
    }
    .boxed())
}

/// The validation state a form control renders: the error id its
/// `aria-describedby` points at, the message its error slot shows, and whether
/// the field is invalid.
///
/// A field is invalid when it carries an error or when it has a `fallback`
/// message of its own — the relationship denial a `Select` surfaces on GET
/// which has no `errors` entry yet.
pub(crate) struct FieldChrome {
    name: String,
    error_id: String,
    error_text: String,
    has_error: bool,
}

impl FieldChrome {
    pub(crate) fn new(name: &str, errors: &[String], fallback: Option<String>) -> Self {
        let incoming = errors.first().cloned().unwrap_or_default();
        let has_error = !errors.is_empty() || fallback.is_some();
        let error_text = if incoming.is_empty() {
            fallback.unwrap_or_default()
        } else {
            incoming
        };
        Self {
            name: name.to_string(),
            error_id: format!("{name}-error"),
            error_text,
            has_error,
        }
    }

    /// The control's `aria-invalid`: `"true"` also colors the field's label.
    pub(crate) fn aria_invalid(&self) -> &'static str {
        if self.has_error { "true" } else { "false" }
    }

    /// The control's `aria-describedby`, pointing at the error slot while the
    /// field is invalid. Owned because a rendered view outlives this value.
    pub(crate) fn described_by(&self) -> Option<String> {
        self.has_error.then(|| self.error_id.clone())
    }
}

/// The chrome every form control renders: the `field` wrapper carrying
/// `ac-field` / `ac-field--error`, the label with the required marker, the
/// control, and the error slot.
///
/// `attributes` carries the extra wrapper attributes a control needs — the
/// `Select` option and filter hooks.
pub(crate) fn render_field<'a>(
    cx: &'a Cx,
    chrome: &FieldChrome,
    label: &str,
    required: bool,
    attributes: Attributes,
    control: BoxView<'a>,
) -> Result<BoxView<'a>> {
    let name = chrome.name.clone();
    let label_text = label.to_string();
    let has_error = chrome.has_error;
    let error_id = chrome.error_id.clone();
    let error_text = chrome.error_text.clone();
    let field_class = if has_error {
        "ac-field ac-field--error"
    } else {
        "ac-field"
    };
    Ok(view! {
        cx =>
        ui_field(
            attrs: attributes! {
                class=(field_class)
                data-invalid=(has_error.then_some("true"))
                (attributes)
            },
            ui_field_label(
                attrs: attributes! { for=(name) },
                (label_text)
                if required {
                    <span class="text-destructive" aria-hidden="true">"*"</span>
                }
            )
            (control)
            if has_error {
                ui_field_error(
                    attrs: attributes! { id=(error_id) class="ac-error" aria-live="polite" },
                    (error_text)
                )
            }
        )
    }
    .boxed())
}

#[cfg(test)]
mod test_support {
    use super::*;
    pub(super) use crate::test_support::cx;
    #[derive(Debug, toasty::Model)]
    pub(super) struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        #[unique]
        email: String,
    }

    /// The whole opening tag carrying `needle` — how a test asserts on an
    /// element whose attributes render in no guaranteed order (topcoat#122)
    /// without depending on a marker attribute nothing consumes.
    pub(super) fn tag_with<'h>(html: &'h str, needle: &str) -> &'h str {
        let at = html
            .find(needle)
            .unwrap_or_else(|| panic!("no {needle} in {html}"));
        opening_tag_at(html, html[..at].rfind('<').expect("its opening tag"))
    }

    /// The opening tag that starts at `start`, sliced up to the `>` closing it.
    ///
    /// `Attributes` renders in no guaranteed order (topcoat#122), so a test
    /// locates a tag by whichever attribute it can and asserts on the whole
    /// tag. Quoting is honoured, so a `>` inside an attribute value (Tailwind
    /// selectors carry them) does not end the slice.
    pub(super) fn opening_tag_at(html: &str, start: usize) -> &str {
        let mut quoted = false;
        for (offset, byte) in html.as_bytes()[start..].iter().enumerate() {
            match byte {
                b'"' => quoted = !quoted,
                b'>' if !quoted => return &html[start..start + offset],
                _ => {}
            }
        }
        panic!("unterminated tag at byte {start} in {html}");
    }

    /// The attributes of the opening tag carrying `needle`, sorted — quoting is
    /// honoured, so a Tailwind class value stays one token.
    ///
    /// `Attributes` renders in no guaranteed order (topcoat#122), so two
    /// renders of the same markup compare as sets of `name="value"` tokens.
    pub(super) fn attributes_of(html: &str, needle: &str) -> Vec<String> {
        let mut quoted = false;
        let mut attrs: Vec<String> = Vec::new();
        let mut current = String::new();
        for ch in tag_with(html, needle).chars() {
            match ch {
                '"' => {
                    quoted = !quoted;
                    current.push(ch);
                }
                ch if ch.is_whitespace() && !quoted => {
                    if !current.is_empty() {
                        attrs.push(std::mem::take(&mut current));
                    }
                }
                ch => current.push(ch),
            }
        }
        if !current.is_empty() {
            attrs.push(current);
        }
        attrs.remove(0); // the tag name
        attrs.sort();
        attrs
    }

    /// A nullable FK, for the optional-by-default select case.
    #[derive(Debug, toasty::Model)]
    pub(super) struct NullableRef {
        #[key]
        #[auto]
        id: uuid::Uuid,
        parent_id: Option<uuid::Uuid>,
    }

    /// A non-nullable foreign key, for the required-by-default FK select.
    #[derive(Debug, toasty::Model)]
    pub(super) struct FkRef {
        #[key]
        #[auto]
        id: uuid::Uuid,
        author_id: uuid::Uuid,
    }

    /// `Select`/`FileUpload` follow the same required-default as `TextInput`
    /// non-nullable lenses default required, `.optional` opts
    /// out, `.required()` forces it back.
    #[test]
    fn select_and_file_upload_required_defaults_follow_nullability() {
        // Non-nullable String field: bare Select/FileUpload reject "".
        let select = Select::r#for(DummyUser::fields().name());
        assert!(
            select
                .validate("")
                .iter()
                .any(|e| e.contains("is required")),
            "non-nullable"
        );
        let upload = FileUpload::r#for(DummyUser::fields().name());
        assert!(
            upload
                .validate("")
                .iter()
                .any(|e| e.contains("is required")),
            "non-nullable"
        );

        // `.optional()` opts out; `.required()` forces it back on.
        assert!(
            Select::r#for(DummyUser::fields().name())
                .optional()
                .validate("")
                .is_empty()
        );
        assert!(
            FileUpload::r#for(DummyUser::fields().name())
                .optional()
                .validate("")
                .is_empty()
        );
        assert!(
            Select::r#for(DummyUser::fields().name())
                .optional()
                .required()
                .validate("")
                .iter()
                .any(|e| e.contains("is required"))
        );

        // Nullable lenses default optional (parity).
        let select = Select::r#for(NullableRef::fields().parent_id());
        assert!(
            select.validate("").is_empty(),
            "nullable FK select defaults optional, got {:?}",
            select.validate("")
        );
        // FileUpload's lens type only binds non-nullable `String` columns
        // (a nullable field is `Option<String>`), so its required default is
        // always on; the nullability walk keeps it correct if the lens
        // widens upstream (#183).
    }
}
