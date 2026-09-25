use argentum_ui::{checkbox as ui_checkbox, input as ui_input, label as ui_label};
use topcoat::{Result, context::Cx, view::*};

use super::{
    super::{
        lenses::{lens_field, lens_label},
        tree::Mode,
        validation::Rules,
    },
    FieldChrome, ValueKind, render_field, render_value, render_value_view,
};

/// FileUpload field — stores a String path with file input handling.
///
/// Storage contract (GH #73, GH #188): the field always binds a `String` column
/// holding a *path*, never bytes. Forms containing a `FileUpload` render
/// `enctype="multipart/form-data"` (see `Panel`) and the POST parser extracts
/// the file part; where the bytes go is the app's decision, expressed by the
/// [`Uploader`](crate::Uploader) installed with
/// [`Panel::uploads`](crate::Panel::uploads). An installed uploader receives
/// the part's sanitized filename and bytes and returns the value to store; with
/// none, the sanitized basename is stored. The file input renders no `value`
/// attribute, which browsers ignore for security.
///
/// `render_with` and `validate` own the rest: the stored value as a link and
/// its `clear_<field>` checkbox (GH #188, GH #242), and the edit-time
/// `required` rule (GH #184).
#[derive(Debug, Clone)]
pub struct FileUpload {
    name: String,
    label: String,
    required: bool,
}

impl FileUpload {
    /// Create a `FileUpload` bound to the given field lens.
    ///
    /// Required defaults from the lens's nullability (GH #100, GH #147),
    /// same as `TextInput`/`Select`. The `String` lens type only binds
    /// non-nullable columns (`Option<String>` fields do not typecheck), so
    /// the default is always required and `.optional()` is the form-level
    /// opt-out; the nullability walk stays correct if the lens widens
    /// upstream (#183).
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let field = lens_field(path, &M::schema());
        Self {
            name: field.name.app_unwrap().to_string(),
            label: lens_label(&field),
            required: !field.nullable(),
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Opt out of the required default (GH #147): `r#for` only binds
    /// non-nullable `String` columns (an `Option<String>` field is
    /// `Path<M, Option<String>>` and does not typecheck), so the default is
    /// always required and this is the only way to treat a required-backed
    /// column as form-optional — an empty submit then passes validation and
    /// the record fn decides what to store.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// The label as the user sees it, for the errors the framework words
    /// (GH #188: a failed upload is reported against this label).
    ///
    /// `pub(crate)`, unlike `TextInput::label_str`: the upload seam is what
    /// needs it, and the issue's contract is that nothing about the `Schema`
    /// surface changes when an app installs a store (GH #188).
    pub(crate) fn label_str(&self) -> &str {
        &self.label
    }

    pub fn validate(&self, value: &str) -> Vec<String> {
        Rules::new().validate(&self.label, self.required, value)
    }

    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        value: Option<&str>,
        errors: &[String],
        mode: Mode,
    ) -> Result<BoxView<'a>> {
        // The detail page shows the stored path, never a file control
        // (GH #187): an empty `FileUpload` on an edit is the panel's "keep the
        // stored file" affordance, which is a statement about a form, not about
        // a record.
        if mode == Mode::View {
            return stored_upload_value(cx, &self.label, value);
        }
        let name = self.name.clone();
        // An edit hydrates the stored path; a create does not (GH #184). See
        // the type docs: the control is required only when nothing is stored,
        // since a file input cannot be pre-filled.
        let stored = stored_path(value);
        let is_edit = stored.is_some();
        let control_required = self.required && !is_edit;
        let chrome = FieldChrome::new(&name, errors, None);
        let hint_id = format!("{name}-hint");
        // The clear flag is a framework transport key, not a field (GH #148):
        // it names the stored value's owner and is stripped before any record
        // fn, so it can never be written as a field of its own.
        let clear_name = format!("clear_{name}");
        // The stored value as a link to the file it names (GH #242). Nothing
        // here guesses a URL convention — the app decides what it stores (the
        // uploader's return value) — and a value that is not a rooted path or
        // an `http(s)` URL renders as text rather than as a clickable scheme
        // (GH #277).
        let stored_display: Option<BoxView<'a>> =
            stored.map(|current| stored_upload_row(cx, current));
        let aria_invalid = chrome.aria_invalid();
        let described_by = chrome
            .described_by()
            .or_else(|| is_edit.then(|| hint_id.clone()));
        let control = view! {
            cx =>
            if let Some(row) = stored_display {
                // The stored path is visible, so "there is no file" is no
                // longer ambiguous, and the empty control reads as "leave
                // it alone" rather than "this field is broken".
                (row)
            }
            // The `input` primitive styles `type="file"` through its
            // `file:` classes and carries the `aria-invalid` error styling.
            ui_input(
                attrs: attributes! {
                    id=(name.clone())
                    type="file"
                    name=(name.clone())
                    required=(control_required)
                    aria-required=(control_required.then_some("true"))
                    aria-invalid=(aria_invalid)
                    aria-describedby=(described_by)
                }
            )
            if is_edit {
                <div class="text-xs text-muted-foreground" id=(hint_id.clone())>
                    "Leave empty to keep the current file."
                </div>
                // The one control that says "remove it" rather than "leave
                // it alone" (GH #188). It carries `value="1"` so the
                // framework's own `truthy` vocabulary reads it, and it is a
                // declared transport key, so a generic record fn never sees
                // it (GH #148).
                <div class="mt-2 flex items-center gap-2">
                    ui_checkbox(
                        attrs: attributes! {
                            id=(clear_name.clone())
                            name=(clear_name.clone())
                            value="1"
                        }
                    )
                    ui_label(
                        attrs: attributes! {
                            for=(clear_name.clone())
                            class="text-xs text-muted-foreground"
                        },
                        "Remove the current file"
                    )
                </div>
            }
        }
        .boxed();
        render_field(
            cx,
            &chrome,
            &self.label,
            control_required,
            attributes! {},
            control,
        )
    }
}

/// The stored path a `FileUpload` shows, if it has one.
///
/// A value that is absent or only whitespace is not a file: the form renders a
/// create rather than an edit with an empty "Current:" line, and the read-only
/// value renders without a link.
fn stored_path(value: Option<&str>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Whether a stored value may become an `href` (GH #277): a rooted path
/// (`/uploads/x.png`, not the scheme-relative `//host`) or an absolute
/// `http(s)` URL. Anything else — a bare basename, `javascript:`, `data:` —
/// renders as text: the framework stores what it is handed, so the render is
/// where a scheme is refused.
fn is_linkable(path: &str) -> bool {
    let lower = path.trim_start().to_ascii_lowercase();
    (lower.starts_with('/') && !lower.starts_with("//"))
        || lower.starts_with("https://")
        || lower.starts_with("http://")
}

/// The `Current: …` row a `FileUpload` shows for a stored value (GH #188).
///
/// Takes the path by value: the rendered view has to outlive the field's
/// `render_with`, and a rendering coroutine may not hold a borrow of it.
fn stored_upload_row<'a>(cx: &'a Cx, path: String) -> BoxView<'a> {
    // The link is the only way to reach the file, and the path is what it
    // says; each node owns its own copy of it.
    let text = path.clone();
    // A value that is not a safe URL renders as plain text (GH #277): the
    // wrapper and the label stay, only the anchor goes.
    let inner: BoxView<'a> = if is_linkable(&path) {
        let href = path.clone();
        view! {
            cx =>
            <a class="font-medium text-foreground underline" href=(href)>(text)</a>
        }
        .boxed()
    } else {
        view! { cx => <span class="font-medium text-foreground">(text)</span> }.boxed()
    };
    view! {
        cx =>
        <div class="text-xs text-muted-foreground" data-file-current=(path)>
            "Current: "
            (inner)
        </div>
    }
    .boxed()
}

/// A `FileUpload` read rather than edited (GH #187): the label over the stored
/// path, as a link to the file when it is one (GH #242, GH #277).
///
/// The reader asks the same "is the stored value right?" question the editor
/// asks, and following the link is how they answer it. `underline` is the
/// affordance: the value sits in body colour inside the field chrome, so
/// without it a stored path reads as text. The chrome is the one
/// `render_value_view` gives every read-only field, so a detail page stays
/// uniform.
fn stored_upload_value<'a>(cx: &'a Cx, label: &str, value: Option<&str>) -> Result<BoxView<'a>> {
    let Some(path) = stored_path(value) else {
        return render_value(cx, label, value, ValueKind::Machine);
    };
    // A value that is not a safe URL is not a link (GH #277): it renders
    // through the same machine-value path an empty value takes, so the detail
    // page shows the stored text without an `href` to click.
    if !is_linkable(&path) {
        return render_value(cx, label, Some(&path), ValueKind::Machine);
    }
    let href = path.clone();
    let link = view! {
        cx =>
        <a
            class="text-sm font-mono break-all whitespace-pre-wrap underline"
            href=(href)
        >
            (path)
        </a>
    }
    .boxed();
    render_value_view(cx, label, link)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use topcoat::context::Cx;

    use super::{
        super::test_support::{attributes_of, cx, opening_tag_at, tag_with},
        *,
    };
    use crate::schema::Schema;

    #[tokio::test]
    async fn file_upload_renders_without_value_attr() {
        let cx = cx();
        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
        }
        let schema = Schema::new(FileUpload::r#for(Doc::fields().path()));
        let mut values = HashMap::new();
        values.insert("path".to_string(), "/tmp/old.jpg".to_string());
        let html = schema
            .render_with(&cx, &values, &HashMap::new())
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("type=\"file\""),
            "missing file input in {html}"
        );
        // Browsers ignore/mask file-input value (GH #73) — must never render.
        assert!(
            !html.contains("value=\"/tmp/old.jpg\""),
            "file input must not carry value in {html}"
        );
    }

    /// The opening `<input …>` tag around the file control, so assertions do
    /// not have to care about attribute order (topcoat#122).
    fn file_input_tag(html: &str) -> String {
        let at = html.find("type=\"file\"").expect("a file input");
        let start = html[..at].rfind("<input").expect("its opening tag");
        opening_tag_at(html, start).to_string()
    }

    fn cx_and_doc_schema() -> (Cx, Schema) {
        #[derive(Debug, toasty::Model)]
        struct Upload {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
        }
        (
            cx(),
            Schema::new(FileUpload::r#for(Upload::fields().path())),
        )
    }

    async fn render_upload(schema: &Schema, cx: &Cx, value: Option<&str>) -> String {
        let mut values = HashMap::new();
        if let Some(value) = value {
            values.insert("path".to_string(), value.to_string());
        }
        schema
            .render_with(cx, &values, &HashMap::new())
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(cx)
    }

    /// The same field on a detail page (`Mode::View`).
    async fn render_readonly_upload(schema: &Schema, cx: &Cx, value: Option<&str>) -> String {
        let mut values = HashMap::new();
        if let Some(value) = value {
            values.insert("path".to_string(), value.to_string());
        }
        schema
            .render_readonly(cx, &values)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(cx)
    }

    /// GH #184: nothing stored (a create) keeps the required contract — the
    /// browser blocks an empty submit and the server reports it inline.
    #[tokio::test]
    async fn file_upload_is_required_on_create() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_upload(&schema, &cx, None).await;
        let tag = file_input_tag(&html);
        assert!(
            tag.contains("required"),
            "a create must keep the required file control, got {tag}"
        );
        assert!(
            !html.contains("data-file-current"),
            "a create has no stored path to show, got {html}"
        );
        assert!(
            !html.contains("Leave empty"),
            "the keep-current hint is an edit affordance, got {html}"
        );
    }

    /// GH #184: a stored path (an edit) makes the control optional and shows
    /// what is stored, because a file input cannot be pre-filled — otherwise
    /// the browser blocks every save and the server's untouched-value backfill
    /// never gets a request to act on.
    #[tokio::test]
    async fn file_upload_surfaces_the_stored_path_and_drops_required() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_upload(&schema, &cx, Some("/uploads/cover.jpg")).await;
        let tag = file_input_tag(&html);
        assert!(
            !tag.contains("required"),
            "an edit must not block on the empty file control, got {tag}"
        );
        assert!(
            tag.contains("aria-describedby=\"path-hint\""),
            "the control must describe itself with the hint, got {tag}"
        );
        assert!(
            html.contains("data-file-current=\"/uploads/cover.jpg\""),
            "the stored path must be surfaced, got {html}"
        );
        assert!(
            html.contains("Leave empty to keep the current file."),
            "the edit must say an empty control keeps the file, got {html}"
        );
    }

    /// A stored path that is only whitespace is not a file: it must behave as
    /// a create, not as an edit with a blank "Current:" line.
    #[tokio::test]
    async fn file_upload_treats_a_blank_stored_path_as_empty() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_upload(&schema, &cx, Some("   ")).await;
        let tag = file_input_tag(&html);
        assert!(
            tag.contains("required"),
            "a blank stored path must stay required, got {tag}"
        );
        assert!(
            !html.contains("data-file-current"),
            "a blank stored path must not render a Current line, got {html}"
        );
    }

    /// GH #242: the stored path is a link to the file it names, whatever the
    /// extension — the framework renders what the app stored, invents no URL
    /// convention, and keeps no image pipeline.
    #[tokio::test]
    async fn file_upload_links_the_stored_file() {
        let (cx, schema) = cx_and_doc_schema();
        let image = render_upload(&schema, &cx, Some("/uploads/cover.png")).await;
        assert!(
            !image.contains("<img"),
            "a stored path is never rendered as an image, got {image}"
        );
        assert!(
            tag_with(&image, "href=\"/uploads/cover.png\"").starts_with("<a"),
            "the stored path must be a link to the file, got {image}"
        );
        assert!(
            image.contains("data-file-current=\"/uploads/cover.png\""),
            "the path stays readable beside the link, got {image}"
        );

        // The stored value is opaque: a query string is part of the path the
        // app stored and reaches the link unchanged.
        let signed = render_upload(&schema, &cx, Some("/media/photo.JPG?token=abc")).await;
        assert!(
            tag_with(&signed, "href=\"/media/photo.JPG?token=abc\"").starts_with("<a"),
            "the stored path is rendered verbatim, got {signed}"
        );
    }

    /// GH #242: a *read* of the stored value links it too — a reader asks the
    /// same "is what is stored right?" question the editor asks, and following
    /// the link is how they answer it.
    #[tokio::test]
    async fn file_upload_view_mode_links_the_stored_file() {
        let (cx, schema) = cx_and_doc_schema();
        let html = render_readonly_upload(&schema, &cx, Some("/uploads/cover.png")).await;
        assert!(
            tag_with(&html, "href=\"/uploads/cover.png\"").starts_with("<a"),
            "a detail page must link the stored file, got {html}"
        );
        assert!(
            !html.contains("<img"),
            "a detail page renders no image, got {html}"
        );
        assert!(
            !html.contains("type=\"file\""),
            "and must never show a file control, got {html}"
        );

        // Nothing stored is not a link to nowhere.
        let empty = render_readonly_upload(&schema, &cx, None).await;
        assert!(
            !empty.contains("href="),
            "an empty stored value must not render a link, got {empty}"
        );
    }

    /// GH #277: a stored value becomes an `href` only when it is a rooted path
    /// or an absolute `http(s)` URL. Every other spelling — a scheme such as
    /// `javascript:` or `data:`, the scheme-relative `//host`, a bare basename
    /// — renders as text in both the edit row and the detail value: the
    /// framework stores what it is handed, so the render is where a scheme is
    /// refused.
    #[tokio::test]
    async fn file_upload_links_only_a_rooted_or_http_url() {
        let (cx, schema) = cx_and_doc_schema();
        for refused in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,x",
            "//evil.example/x.png",
            "report.pdf",
        ] {
            let edit = render_upload(&schema, &cx, Some(refused)).await;
            assert!(
                !edit.contains("href="),
                "{refused} must not become a link on the form, got {edit}"
            );
            assert!(
                edit.contains(&format!("data-file-current=\"{refused}\"")),
                "{refused} must stay visible on the form, got {edit}"
            );
            let view = render_readonly_upload(&schema, &cx, Some(refused)).await;
            assert!(
                !view.contains("href="),
                "{refused} must not become a link on the detail page, got {view}"
            );
            assert!(
                view.contains(refused),
                "{refused} must still render as text on the detail page, got {view}"
            );
        }

        for linkable in ["/uploads/a.png", "https://cdn.example/a.png"] {
            let edit = render_upload(&schema, &cx, Some(linkable)).await;
            assert!(
                tag_with(&edit, &format!("href=\"{linkable}\"")).starts_with("<a"),
                "{linkable} must stay a link on the form, got {edit}"
            );
            let view = render_readonly_upload(&schema, &cx, Some(linkable)).await;
            assert!(
                tag_with(&view, &format!("href=\"{linkable}\"")).starts_with("<a"),
                "{linkable} must stay a link on the detail page, got {view}"
            );
        }
    }

    /// GH #242: the stored path is opaque. A `.png` and a `.txt` render the
    /// same row and the same link, which is the property that fails the moment
    /// the field reads the extension again — whatever it then emits.
    #[tokio::test]
    async fn file_upload_renders_any_extension_identically() {
        let (cx, schema) = cx_and_doc_schema();
        let png = render_upload(&schema, &cx, Some("/uploads/cover.png")).await;
        let txt = render_upload(&schema, &cx, Some("/uploads/cover.txt")).await;
        for needle in ["data-file-current=", "href="] {
            assert_eq!(
                stored_attributes(&png, needle, "/uploads/cover.png"),
                stored_attributes(&txt, needle, "/uploads/cover.txt"),
                "the extension must not change the form row ({needle})"
            );
        }

        let png = render_readonly_upload(&schema, &cx, Some("/uploads/cover.png")).await;
        let txt = render_readonly_upload(&schema, &cx, Some("/uploads/cover.txt")).await;
        assert_eq!(
            stored_attributes(&png, "href=", "/uploads/cover.png"),
            stored_attributes(&txt, "href=", "/uploads/cover.txt"),
            "the extension must not change the read-only link"
        );
    }

    /// The tag carrying `needle`, with `path` normalized out of its attributes
    /// so two renders of different paths compare equal.
    fn stored_attributes(html: &str, needle: &str, path: &str) -> Vec<String> {
        attributes_of(html, needle)
            .into_iter()
            .map(|attr| attr.replace(path, "<stored>"))
            .collect()
    }

    /// GH #188: the clear control belongs to a stored value — it is the only
    /// way to say "remove the file" rather than "leave it alone".
    #[tokio::test]
    async fn file_upload_offers_the_clear_control_only_for_a_stored_value() {
        let (cx, schema) = cx_and_doc_schema();
        let edit = render_upload(&schema, &cx, Some("/uploads/cover.png")).await;
        assert!(
            edit.contains("name=\"clear_path\""),
            "an edit must post the declared transport key, got {edit}"
        );
        assert!(
            edit.contains("value=\"1\""),
            "the control must carry the truthy value the handlers read (GH #148), got {edit}"
        );
        assert!(
            edit.contains("Remove the current file"),
            "the label must say what ticking it does, got {edit}"
        );

        let create = render_upload(&schema, &cx, None).await;
        assert!(
            !create.contains("name=\"clear_path\""),
            "a create has nothing to clear, got {create}"
        );
    }
}
