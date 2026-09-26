use tablo_ui::input as ui_input;
use topcoat::{Result, context::Cx, view::*};

use super::{
    super::{
        lenses::{FieldResolver, lens_field, lens_field_unique, lens_label},
        tree::Mode,
        validation::{
            Rules, TypedValue, format_timestamp_input, is_timestamp, parse_timestamp_storage,
        },
    },
    FieldChrome, ValueKind, render_field, render_value,
};

/// The equality expression a typed leaf's unique probe binds.
///
/// Built where the declared type is known — the lens constructor — so a typed
/// field compares as its declared type rather than as its text. `None` means
/// the submitted value does not parse into that type: validation has already
/// refused it, and the probe has nothing to compare.
type EqProbe = std::sync::Arc<dyn Fn(&str) -> Option<toasty::stmt::Expr<bool>> + Send + Sync>;

/// The equality probe a typed leaf binds: the submission is parsed into the
/// declared type and compared through that type's own path, so a value that is
/// unique as text but not as the type (or the reverse) is checked for what the
/// record will store.
///
/// The index resolves inside the closure rather than here: the name is the
/// leaf's **app field name**, and for a context-bound leaf that is its flattened
/// storage column (`seo_title`), which the leaf's own lens root does not name —
/// toasty matches app field names — so resolving it eagerly would panic for an
/// embedded typed leaf. Only a `unique()` marker reaches the closure.
///
/// `IntoExpr` is what lets the comparison name the value: it is implemented for
/// every scalar toasty stores, and the panel's typed constructors require it
/// for the same reason they require `TypedValue` — a value that cannot become an
/// expression cannot be compared against its column.
fn eq_probe_typed<M, T>(name: &str) -> EqProbe
where
    M: toasty::schema::Model,
    T: TypedValue + toasty::stmt::IntoExpr<T> + 'static,
{
    let name = name.to_string();
    let timestamp = is_timestamp::<T>();
    std::sync::Arc::new(move |value: &str| {
        // A timestamp submission may be the control's zone-less shape;
        // validation normalises it to storage spelling first.
        if timestamp {
            let storage = parse_timestamp_storage(value).ok()?;
            let parsed = storage.parse::<T>().ok().filter(T::accepts)?;
            let index = M::field_name_to_id(&name).index;
            return Some(M::path_field::<T>(index).eq(parsed));
        }
        let parsed = value.parse::<T>().ok().filter(T::accepts)?;
        let index = M::field_name_to_id(&name).index;
        Some(M::path_field::<T>(index).eq(parsed))
    })
}

/// Typed text field bound to a Toasty field lens. The lens is the single
/// source of truth for the field name and type, so `TextInput::for(User::fields().name())`
/// fails to compile if the column does not exist (ADR-0001).
#[derive(Clone)]
pub struct TextInput {
    name: String,
    label: String,
    required: bool,
    unique: bool,
    placeholder: Option<String>,
    /// The email and typed-parse rules, with their messages.
    rules: Rules,
    /// The typed leaf's unique probe, absent on a text leaf: a text
    /// leaf's comparison is built from the model the handler queries.
    typed_probe: Option<EqProbe>,
    /// The control's `type`: `datetime-local` for a timestamp leaf, `text`
    /// otherwise (`email` overrides at render). A timestamp submits a
    /// zone-less `datetime-local` string the typed rule reads as UTC.
    input_type: &'static str,
}

impl std::fmt::Debug for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The parser is a closure with no useful Debug; everything a reader
        // needs is the field's identity and which rules it declares.
        f.debug_struct("TextInput")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("required", &self.required)
            .field("is_email", &self.rules.is_email())
            .field("unique", &self.unique)
            .field("placeholder", &self.placeholder)
            .field("typed", &self.rules.is_typed())
            .field("input_type", &self.input_type)
            .finish()
    }
}

impl TextInput {
    /// Create a `TextInput` bound to the given field lens.
    ///
    /// Only `String` lenses compile: binding a non-text field (a `Uuid` key,
    /// a `bool`, …) fails at compile time, mirroring `TextColumn`.
    ///
    /// `required` and `unique` default from the field's metadata (GH #100,
    /// GH #183): a non-nullable column is required, and a field backed by a
    /// single-field unique index is unique — so neither has to be restated by
    /// hand. Both stay overridable with `.optional()` / `.unique()`.
    pub fn r#for<M>(path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let model = M::schema();
        let field = lens_field(path, &model);
        let label_str = lens_label(&field);
        let unique = lens_field_unique(&field, model.as_root_unwrap());
        let name = field.name.app_unwrap().to_string();
        Self {
            name,
            label: label_str,
            // Non-nullable columns are required by default: an
            // empty submit would die at the driver instead of failing
            // inline. Override with `.optional()` for nullable columns.
            required: !field.nullable(),
            unique,
            placeholder: None,
            rules: Rules::new(),
            typed_probe: None,
            input_type: "text",
        }
    }

    /// Create a `TextInput` bound to a lens inside an embedded struct or a
    /// `#[document]`.
    ///
    /// The plain `Self::r#for` resolves a lens against the model alone, which
    /// is why it can only bind a top-level field: the owned `app::Model` cannot
    /// see the embedded models, so a path like `Post::fields().seo().title()`
    /// is rejected as a traversal lens. This resolves through the request's app
    /// schema instead, so the leaf arrives as its **flattened storage column**
    /// (`seo_title`) — the name the form posts and the record fn reads.
    ///
    /// An embedded leaf is never `required` by default: the resolver reports
    /// `nullable=true` by binding policy, since only the matching enum variant
    /// writes a variant payload column. That is the binding default, not a
    /// storage fact — the flattened column of a required embedded struct is
    /// `NOT NULL`. Opt in with [`.required()`](Self::required).
    ///
    /// Without a `Db` in context (a bare `CxTestBuilder`) this behaves exactly
    /// like `Self::r#for` and rejects the traversal lens loudly, so a test
    /// cannot silently bind the wrong column.
    pub fn r#for_context<M>(cx: &Cx, path: toasty::stmt::Path<M, String>) -> Self
    where
        M: toasty::schema::Model,
    {
        let leaf = FieldResolver::from_cx(cx).resolve(path);
        Self {
            name: leaf.name,
            label: leaf.label,
            required: !leaf.nullable,
            unique: false,
            placeholder: None,
            rules: Rules::new(),
            typed_probe: None,
            input_type: "text",
        }
    }

    /// Create a `TextInput` bound to a lens whose leaf is **not** a `String`.
    ///
    /// `Self::r#for` takes a `Path<M, String>`, which makes a wrong lens a
    /// compile error rather than a runtime mismatch (ADR-0001) — and also makes
    /// a typed column unbindable. This constructor keeps that guarantee: the
    /// lens must address one field of the model and `T` must be the leaf's
    /// actual type, so `TextInput::typed::<User, Uuid>(User::fields().name())`
    /// does not compile either. It adds the value's spelling rule:
    ///
    /// - the control renders the value's `Display`;
    /// - a submission `T` cannot parse is an **inline field error** naming the offending input (``
    ///   `2024-13-01` is not a valid timestamp ``);
    /// - what is stored is `T`'s `Display` of the parsed value, so a value re-submitted unchanged
    ///   is written back in the shape it was read.
    ///
    /// The record fn still receives `String`s: the panel's value map is
    /// text-keyed, and a typed field is a *validated* string, not a second
    /// channel.
    ///
    /// `TypedValue` is implemented for the types a panel binds — the integer
    /// types, `bool`, `f32`, `f64`, `Uuid`, `jiff::Timestamp` — rather than as a
    /// blanket over `FromStr`, because the error names what was expected; `T`
    /// must be [`toasty::stmt::IntoExpr`] of itself so the unique probe compares
    /// through the parsed value rather than its text.
    ///
    /// A `jiff::Timestamp` leaf renders `type="datetime-local"`: the control
    /// carries no zone, so the stored instant renders in UTC and a submission
    /// is read back as UTC.
    pub fn typed<M, T>(path: toasty::stmt::Path<M, T>) -> Self
    where
        M: toasty::schema::Model,
        T: TypedValue + toasty::stmt::IntoExpr<T> + 'static,
    {
        let model = M::schema();
        let field = lens_field(path, &model);
        let label_str = lens_label(&field);
        // A typed field declares no index and is not marked unique by default;
        // when the app marks it, the probe binds the declared type.
        let name = field.name.app_unwrap().to_string();
        let probe = eq_probe_typed::<M, T>(&name);
        let input_type = if is_timestamp::<T>() {
            "datetime-local"
        } else {
            "text"
        };
        Self {
            name,
            label: label_str,
            required: !field.nullable(),
            unique: false,
            placeholder: None,
            rules: Rules::new().typed::<T>(),
            typed_probe: Some(probe),
            input_type,
        }
    }

    /// [`Self::typed`] for a lens inside an embedded struct or a `#[document]`,
    /// resolving through the request's app schema exactly as
    /// `Self::r#for_context` does.
    ///
    /// A leaf under an embedded step is never required by default: the resolver
    /// reports `nullable=true` by binding policy, since only the matching enum
    /// variant writes a variant payload column. That is the binding default,
    /// not a storage fact — the flattened column of a required embedded struct
    /// is `NOT NULL`. Opt in with [`.required()`](Self::required).
    pub fn typed_context<M, T>(cx: &Cx, path: toasty::stmt::Path<M, T>) -> Self
    where
        M: toasty::schema::Model,
        T: TypedValue + toasty::stmt::IntoExpr<T> + 'static,
    {
        let leaf = FieldResolver::from_cx(cx).resolve(path);
        let probe = eq_probe_typed::<M, T>(&leaf.name);
        let input_type = if is_timestamp::<T>() {
            "datetime-local"
        } else {
            "text"
        };
        Self {
            name: leaf.name,
            label: leaf.label,
            required: !leaf.nullable,
            unique: false,
            placeholder: None,
            rules: Rules::new().typed::<T>(),
            typed_probe: Some(probe),
            input_type,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Opt out of the non-nullable default: for nullable columns
    /// where an empty submit is legitimate.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn email(mut self) -> Self {
        self.rules.set_email();
        self
    }

    /// Mark the field as backed by a unique constraint, which the app-side
    /// pre-check probes before the write.
    ///
    /// **Uniqueness implies presence**: the framework stores `""`,
    /// never NULL, so an empty value is one the index admits only
    /// once — an empty submit is refused inline as `"<Label> is required"`
    /// instead of being written, and the probe never sees it. `.optional()`
    /// does not lift that rule, whichever order the two are called in. The
    /// reasoning (and the rejected alternative) is recorded in the ADR-0010
    /// amendment of 2026-09-21.
    ///
    /// Non-`TextInput` fields declare no uniqueness (see `Textarea::r#for`),
    /// so nothing else changes.
    pub fn unique(mut self) -> Self {
        self.unique = true;
        // The marker carries presence itself, so `.optional().unique()` and
        // `.unique().optional()` mean the same thing.
        self.required = true;
        self
    }

    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = Some(p.into());
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }

    pub fn is_unique(&self) -> bool {
        self.unique
    }

    /// Whether an empty submit fails validation and the control renders as
    /// required: `required`, defaulting from column nullability per GH #100,
    /// **or** uniqueness (GH #189 — a unique field is never empty, see
    /// [`Self::unique`]). `validate` and both render paths read it, so the rule
    /// and the marker cannot disagree.
    pub(crate) fn is_required(&self) -> bool {
        self.required || self.unique
    }

    pub fn field_name(&self) -> &str {
        &self.name
    }

    /// The human label (e.g. `"Email"`) — for inline error messages.
    pub fn label_str(&self) -> &str {
        &self.label
    }

    /// The equality expression the app-side unique check probes with.
    ///
    /// A text leaf compares its submission's text through `M`'s own path — the
    /// model the handler queries, which is also the model a context-bound
    /// leaf's flattened column belongs to. A typed leaf instead
    /// parses the submission into its declared type and compares that, so the
    /// probe sees the value the record will store rather than its spelling —
    /// `01` and `1` are one value to an integer column. `None` when a typed
    /// submission does not parse: validation has already refused it, and there
    /// is nothing left to compare.
    pub(crate) fn eq_filter<M>(&self, value: &str) -> Option<toasty::stmt::Expr<bool>>
    where
        M: toasty::schema::Model,
    {
        let Some(probe) = &self.typed_probe else {
            let index = M::field_name_to_id(&self.name).index;
            return Some(M::path_field::<String>(index).eq(value.to_string()));
        };
        probe(value)
    }

    /// Validate a raw string value against the configured rules.
    pub fn validate(&self, value: &str) -> Vec<String> {
        self.rules.validate(&self.label, self.is_required(), value)
    }

    /// The stored spelling of a submission the caller has already validated.
    ///
    /// The typed parse's `Display` for a typed field, the trimmed submission
    /// for a text one — so a value the user left alone is written back in the
    /// shape the record fn wrote it, not in whichever spelling the browser
    /// sent. Callers that have not validated must not use this: it reports a
    /// failure rather than guessing.
    pub(crate) fn normalize(&self, value: &str) -> Result<String, String> {
        self.rules.normalize(value)
    }

    /// Static render: the create/edit path's control, with `value` rendered
    /// into the `value` attribute and `errors` into the error slot.
    pub(crate) async fn render_with<'a>(
        &self,
        cx: &'a Cx,
        value: Option<&str>,
        errors: &[String],
        mode: Mode,
    ) -> Result<BoxView<'a>> {
        if mode == Mode::View {
            return render_value(cx, &self.label, value, ValueKind::Machine);
        }
        let name = self.name.clone();
        // The marker reads the same predicate validation uses, so a unique
        // field is never refused for emptiness while rendering as optional.
        // `self.required` alone would do exactly that.
        let required = self.is_required();
        let placeholder = self.placeholder.clone();
        let input_type = if self.rules.is_email() {
            "email"
        } else {
            self.input_type
        };
        // A timestamp renders its UTC `datetime-local` spelling, not the
        // stored RFC 3339: the control carries no zone.
        let value_owned = value.map(|s| {
            if self.input_type == "datetime-local" {
                format_timestamp_input(s)
            } else {
                s.to_string()
            }
        });
        let chrome = FieldChrome::new(&self.name, errors, None);
        let aria_invalid = chrome.aria_invalid();
        let described_by = chrome.described_by();
        // Beautiful rendering via the upstream `field` family (topcoat#420):
        // label + control + reserved error slot, the label following the
        // field's invalid state, and `aria-invalid` driving the control's
        // error border/ring.
        let control = view! {
            cx =>
            ui_input(
                attrs: attributes! {
                    id=(name.clone())
                    type=(input_type)
                    name=(name.clone())
                    value=(value_owned.clone())
                    placeholder=(placeholder.clone())
                    required=(required)
                    aria-required=(required.then_some("true"))
                    aria-invalid=(aria_invalid)
                    aria-describedby=(described_by)
                }
            )
        }
        .boxed();
        render_field(cx, &chrome, &self.label, required, attributes! {}, control)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        super::{
            Select,
            test_support::{DummyUser, NullableRef, cx},
        },
        *,
    };
    use crate::schema::Schema;

    #[tokio::test]
    async fn text_input_renders_with_label_and_ac_field() {
        let cx = cx();
        let schema = Schema::new(TextInput::r#for(DummyUser::fields().name()));
        let html = schema
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // GH #216: the field/field-label composition is the contract; the
        // input's Token classes (`border-border`, `bg-transparent`,
        // `focus-visible:ring-ring`) are paint and belong to the showcase.
        assert!(
            html.contains("data-slot=\"field\"") && html.contains("data-slot=\"field-label\""),
            "missing field/field-label markup in {html}"
        );
        assert!(
            html.contains("name=\"name\""),
            "missing name attr in {html}"
        );
        assert!(html.contains("<input"), "missing input in {html}");
        assert!(html.contains("<label"), "missing label in {html}");
        assert!(
            html.contains("for=\"name\""),
            "missing for/id linking in {html}"
        );
        // No error → no error node: the primitive's contract is to render
        // `field_error` only when there is an error, so a valid field leaves
        // no empty `role="alert"` behind.
        assert!(
            !html.contains("role=\"alert\""),
            "a valid field must not render an error slot in {html}"
        );
        // label derived from lens: DummyUser::fields().name() → "name" → "Name"
        assert!(html.contains(">Name"), "missing label in {html}");
    }

    /// A typed field shows its stored value on a detail page.
    ///
    /// `TextInput::typed` returns a `TextInput`, so the view path is the one
    /// above: `render_readonly` sets `Mode::View` and the field renders its
    /// value instead of a control. A `Uuid` column is therefore readable, not
    /// only writable.
    #[tokio::test]
    async fn a_typed_field_renders_its_stored_value_read_only() {
        const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let cx = cx();
        let schema = Schema::new(TextInput::typed::<DummyUser, uuid::Uuid>(
            DummyUser::fields().id(),
        ));
        let mut values = HashMap::new();
        values.insert("id".to_string(), ID.to_string());
        let html = schema
            .render_readonly(&cx, &values)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains(ID),
            "a detail page must show a typed field's stored value, got {html}"
        );
        assert!(
            !html.contains("<input"),
            "and must render no control, got {html}"
        );
    }

    #[tokio::test]
    async fn text_input_error_marks_the_field_invalid() {
        // topcoat#420: `aria-invalid` drives the input's error border/ring and
        // the label's destructive color; the reserved slot carries the id the
        // control describes itself with.
        let cx = cx();
        let schema = Schema::new(TextInput::r#for(DummyUser::fields().name()).required());
        let mut errors = HashMap::new();
        errors.insert("name".to_string(), vec!["name is required".to_string()]);
        let html = schema
            .render_with(&cx, &HashMap::new(), &errors)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-invalid=\"true\"") && html.contains("ac-field--error"),
            "missing invalid field state in {html}"
        );
        assert!(
            html.contains("aria-invalid=\"true\"")
                && html.contains("aria-describedby=\"name-error\""),
            "missing aria invalid/described-by in {html}"
        );
        assert!(
            html.contains("id=\"name-error\"") && html.contains("name is required"),
            "missing error slot content in {html}"
        );
    }

    #[test]
    fn text_input_required_validates_empty() {
        let input = TextInput::r#for(DummyUser::fields().name()).required();
        assert!(
            !input.validate("").is_empty(),
            "required should reject empty"
        );
        assert!(
            input.validate("hello").is_empty(),
            "required should accept non-empty"
        );
        assert!(
            !input.validate("   ").is_empty(),
            "required should reject whitespace"
        );
        assert!(
            !TextInput::r#for(DummyUser::fields().name())
                .validate("")
                .is_empty(),
            "non-nullable columns default to required (GH #100)"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().name())
                .optional()
                .validate("")
                .is_empty(),
            "optional should accept empty"
        );
    }

    #[test]
    fn required_default_follows_lens_nullability() {
        // GH #100: `required` defaults from the DB column, with an explicit
        // `.optional()` escape hatch. Pinned through the public constructor.
        #[derive(Debug, toasty::Model)]
        struct NullableDoc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            nick: Option<String>,
        }
        assert!(
            Select::r#for(NullableDoc::fields().nick())
                .validate("")
                .is_empty(),
            "nullable columns default to optional"
        );
        assert!(
            !TextInput::r#for(DummyUser::fields().name())
                .validate("")
                .is_empty(),
            "String columns are non-nullable, empty must fail inline"
        );
    }

    #[tokio::test]
    async fn text_input_required_renders_star_and_email_type() {
        let cx = cx();
        let html_req = Schema::new(TextInput::r#for(DummyUser::fields().name()).required())
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // The required marker is the visible asterisk (GH #216: `required` /
        // `aria-required` below pin the attribute; the star is what a reader
        // sees, and `>*<` is emitted only by it).
        assert!(
            html_req.contains(">*</span>"),
            "required should render its asterisk in {html_req}"
        );
        assert!(
            html_req.contains("required"),
            "required attr missing in {html_req}"
        );
        assert!(
            html_req.contains("aria-required"),
            "aria-required missing in {html_req}"
        );
        assert!(
            html_req.contains("for=\"name\"") && html_req.contains("id=\"name\""),
            "for/id linking missing in {html_req}"
        );
        let html_email = Schema::new(TextInput::r#for(DummyUser::fields().email()).email())
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        // `r#type` would still contain the substring `type=`, so pin the
        // attribute name itself (GH #151: the raw identifier leaked into the
        // rendered HTML and made every email input a plain text input).
        assert!(
            html_email.contains("type=\"email\"") && !html_email.contains("r#type"),
            "email should render type=email, not r#type=email, in {html_email}"
        );
        let html_text = Schema::new(TextInput::r#for(DummyUser::fields().name()))
            .render(&cx)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html_text.contains("type=\"text\"") && !html_text.contains("r#type"),
            "plain should render type=text, not r#type=text, in {html_text}"
        );
        // A required-but-valid field renders no error node either.
        assert!(
            !html_req.contains("role=\"alert\""),
            "a valid required field must not render an error slot in {html_req}"
        );
    }

    #[test]
    fn text_input_email_validates() {
        let input = TextInput::r#for(DummyUser::fields().email())
            .required()
            .email();
        assert!(
            !input.validate("not-an-email").is_empty(),
            "email should reject invalid"
        );
        assert!(
            !input.validate("a@").is_empty(),
            "email should reject partial"
        );
        assert!(
            input.validate("a@b.com").is_empty(),
            "email should accept valid"
        );
        // `.optional()` still accepts an empty submit on a non-unique,
        // nullable column. `DummyUser.email` is `#[unique]`, so
        // there `.optional` cannot lift the required rule.
        assert!(
            Select::r#for(NullableRef::fields().parent_id())
                .optional()
                .validate("")
                .is_empty(),
            "an optional, non-unique field must still accept empty (GH #100)"
        );
        assert!(
            TextInput::r#for(DummyUser::fields().email())
                .email()
                .validate(" a@b.com ")
                .is_empty(),
            "email should trim"
        );
    }

    /// GH #189: the unique marker is presence, so `.optional()` cannot lift it
    /// — in the builder or from the lens.
    #[test]
    fn unique_implies_required_in_either_declaration_order() {
        let mut declarations = vec![
            TextInput::r#for(DummyUser::fields().email())
                .optional()
                .unique(),
            TextInput::r#for(DummyUser::fields().email())
                .unique()
                .optional(),
        ];
        // Derived from the lens, with no `.unique()` call at all: the rule
        // follows the column, not the declaration style.
        declarations.push(TextInput::r#for(DummyUser::fields().email()).optional());

        for (nth, input) in declarations.iter().enumerate() {
            assert!(
                input.is_unique() && input.is_required(),
                "declaration {nth} must be unique and required"
            );
            assert_eq!(
                input.validate(""),
                vec!["Email is required".to_string()],
                "declaration {nth}: an empty unique field is required, not absent"
            );
            assert_eq!(
                input.validate("   "),
                vec!["Email is required".to_string()],
                "declaration {nth}: whitespace-only counts as empty, as everywhere else"
            );
            assert!(
                input.validate("a@b.com").is_empty(),
                "declaration {nth}: a present value still validates normally"
            );
        }
    }

    /// GH #189: the marker a user sees reads the same predicate validation
    /// does, so a unique field cannot be refused for emptiness while rendering
    /// as optional — the disagreement that would have shipped had only
    /// `validate` been taught the rule.
    #[tokio::test]
    async fn unique_field_renders_the_required_marker() {
        let cx = cx();
        let html = Schema::new(
            TextInput::r#for(DummyUser::fields().email())
                .unique()
                .optional(),
        )
        .render(&cx)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
        assert!(
            html.contains("required") && html.contains("aria-required"),
            "a unique field is required in the markup too"
        );
        assert!(
            html.contains(">*</span>"),
            "the required asterisk must render"
        );
    }

    /// The email rule is `email_address`.
    #[test]
    fn text_input_email_edges() {
        let input = TextInput::r#for(DummyUser::fields().email()).email();
        for ok in [
            "a@b.com",
            "user+tag@sub.example.co",
            "Ada@Example.COM",
            // A unicode local part and domain.
            "用户@例え.jp",
            // A quoted local part and a bracketed domain literal.
            "\"a b\"@example.com",
            "a@[IPv6:::1]",
            // The crate's domain grammar, which accepts the local part's
            // atext set in a label and a single-character TLD.
            "user@my_host.com",
            "a@b.c",
        ] {
            assert!(input.validate(ok).is_empty(), "{ok} should pass");
        }
        // Rejected: a text domain without a dot, an empty label, a space, a
        // display name, and parts over their own bound.
        for bad in [
            "a@b".to_string(),
            "a@b..c".to_string(),
            "a b@c.com".to_string(),
            "Ada Lovelace <ada@example.com>".to_string(),
            "not-an-email".to_string(),
            "a@".to_string(),
            "a@b.c.".to_string(),
            ".a@b.com".to_string(),
            "a.@b.com".to_string(),
            "a@@b.com".to_string(),
            "a@-b.com".to_string(),
            "a@b-.com".to_string(),
            // An unquoted local part carrying a special, and a domain label
            // ending on one.
            "a,b@b.com".to_string(),
            "a(b@b.com".to_string(),
            "a@b!.com".to_string(),
            format!("{}@b.com", "a".repeat(65)),
            format!("a@{}.com", "b".repeat(64)),
            // 255 octets: every part fits its own bound, the address does not
            // fit RFC 5321 §4.5.3.1.3.
            format!(
                "{}@{}.{}.{}",
                "a".repeat(64),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(62)
            ),
        ] {
            assert!(!input.validate(&bad).is_empty(), "{bad} should fail");
        }
    }

    /// An empty submit is the presence rule's business: the email rule skips
    /// it, and presence reports first.
    #[test]
    fn email_rule_leaves_an_empty_value_to_presence() {
        let input = TextInput::r#for(DummyUser::fields().email())
            .required()
            .email();
        assert_eq!(input.validate(""), vec!["Email is required".to_string()]);
        assert_eq!(input.validate("   "), vec!["Email is required".to_string()]);
    }
}
