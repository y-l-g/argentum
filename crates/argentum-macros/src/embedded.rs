//! `#[derive(EmbeddedForm)]` — the typed half of an embedded value (GH #191).
//!
//! The derive knows the Rust shape (which fields exist, their types, which
//! variants there are); the framework knows the storage (which column each leaf
//! occupies, what the discriminant column is called). Neither alone can bind a
//! value, so this derive calls the framework once per leaf with a typed path and
//! never spells a flattened name itself.
//!
//! See `argentum-core`'s `schema::embedded` module for the contract.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type};

/// Field types this derive binds as a **leaf** (one column, read and written as
/// text).
///
/// Everything else is another embedded value, delegated to that type's own
/// `EmbeddedForm` impl, so a relation, an `Option<T>`, or a `#[document]` fails
/// at that bound rather than binding quietly. The list is exactly the types the
/// panel can *spell*: `String`, plus every type with a
/// [`TypedValue`](argentum_core::TypedValue) impl — the integer family, `bool`,
/// `f32`/`f64`, `Uuid`, `jiff::Timestamp`. A newtype over one of them is not a
/// leaf: it needs its own `TypedValue` impl (then it is one) or is treated as an
/// embedded value.
const PRIMITIVES: &[&str] = &[
    "String",
    "bool",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "f32",
    "f64",
    "Uuid",
    "Timestamp",
];

/// How one field is bound.
enum Kind {
    /// One column: read/written as text.
    Leaf,
    /// Another embedded value: delegated to its own impl.
    Embedded,
}

pub fn expand(input: DeriveInput) -> TokenStream {
    if let Err(error) = validate_form_attrs(&input) {
        return error.to_compile_error().into();
    }
    let krate = match proc_macro_crate::crate_name("argentum-core") {
        Ok(found) => {
            let name = match found {
                proc_macro_crate::FoundCrate::Itself => "argentum_core".to_string(),
                proc_macro_crate::FoundCrate::Name(n) => n,
            };
            let ident = syn::Ident::new(&name.replace('-', "_"), proc_macro2::Span::call_site());
            quote! { ::#ident }
        }
        Err(_) => {
            return syn::Error::new_spanned(
                &input.ident,
                "argentum-core must be a dependency to #[derive(EmbeddedForm)]",
            )
            .to_compile_error()
            .into();
        }
    };

    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => expand_struct(&krate, &input, named),
            _ => unsupported(&input, "a struct with named fields"),
        },
        Data::Enum(data) => expand_enum(&krate, &input, data),
        _ => unsupported(&input, "a struct or enum"),
    }
}

fn unsupported(input: &DeriveInput, expected: &str) -> TokenStream {
    syn::Error::new_spanned(
        &input.ident,
        format!(
            "#[derive(EmbeddedForm)] supports {expected}: an embedded value's fields are read \
             and written by name (GH #191)"
        ),
    )
    .to_compile_error()
    .into()
}

/// The path to field `index` of `owner`, relative to `owner`'s type root.
///
/// The value type is left to inference: `path_field` returns a typed path whose
/// leaf type comes from how it is used, and the chain it feeds is typed by the
/// caller's `Path<M, Self>`.
///
/// `Path::chain` drops the chained path's root, so this composes under whatever
/// parent it is chained onto.
fn field_path(krate: &TokenStream2, owner: &TokenStream2, ty: &Type, index: usize) -> TokenStream2 {
    quote! {
        <#owner as #krate::__macro::Embed>::path_field::<#ty>(#index)
    }
}

/// `parent.chain(<owner>::path_field(index))`, variant-rooted when `variant` is
/// `Some`.
fn chained(
    krate: &TokenStream2,
    parent: &TokenStream2,
    owner: &TokenStream2,
    ty: &Type,
    index: usize,
    variant: Option<usize>,
) -> TokenStream2 {
    let field = field_path(krate, owner, ty, index);
    match variant {
        // A variant payload is addressed from the variant root: the enum's own
        // payload index is variant-local, so the variant step comes first and
        // `chain` rebases it onto the parent path (GH #191).
        Some(variant) => quote! {
            #parent.chain(
                <#owner as #krate::__macro::Embed>::path_root()
                    .into_variant(#krate::__macro::VariantId {
                        model: <#owner as #krate::__macro::Embed>::id(),
                        index: #variant,
                    })
                    .chain(#field)
            )
        },
        None => quote! { #parent.chain(#field) },
    }
}

/// The label a derived control renders: the Rust field name, humanized.
fn label(ident: &syn::Ident) -> String {
    let mut out = String::with_capacity(ident.to_string().len());
    for (i, part) in ident.to_string().split('_').enumerate() {
        if part.is_empty() {
            continue;
        }
        if i > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.extend(chars);
        }
    }
    out
}

/// Whether the field's type names a primitive this derive binds as a leaf.
fn is_primitive(ty: &Type) -> bool {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
    {
        return PRIMITIVES.contains(&segment.ident.to_string().as_str());
    }
    false
}

fn is_string(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "String"))
}

/// What `#[form(..)]` says about one field.
#[derive(Default)]
struct FormAttrs {
    label: Option<String>,
    /// Render a `Textarea` instead of a `TextInput` — for a multi-line `String`
    /// leaf, which is a UI choice the type cannot make.
    textarea: bool,
    /// Rows for a textarea, when the default is not what the app wants.
    rows: Option<u32>,
}

/// Every `#[form(..)]` attribute on the type, checked once up front.
///
/// An unknown key or a misplaced one is a compile error rather than a silent
/// no-op: a typo'd `#[form(text_area)]` that quietly rendered a one-line input
/// is the quiet failure this repo refuses elsewhere.
fn validate_form_attrs(input: &DeriveInput) -> syn::Result<()> {
    let fields: Vec<&syn::Field> = match &input.data {
        Data::Struct(data) => data.fields.iter().collect(),
        Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter())
            .collect(),
        Data::Union(_) => return Ok(()),
    };
    for field in fields {
        for attr in &field.attrs {
            if !attr.path().is_ident("form") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("label") {
                    let _: syn::LitStr = meta.value()?.parse()?;
                } else if meta.path.is_ident("textarea") {
                    // A flag: nothing to read.
                } else if meta.path.is_ident("rows") {
                    let _: syn::LitInt = meta.value()?.parse()?;
                } else {
                    return Err(meta.error(
                        "unknown `#[form(..)]` key: expected `label = \"…\"`, `textarea`, or \
                         `rows = N`",
                    ));
                }
                Ok(())
            })?;
        }
        let attrs = form_attrs(&field.attrs);
        if attrs.rows.is_some() && !attrs.textarea {
            return Err(syn::Error::new_spanned(
                field,
                "`#[form(rows = N)]` only means something with `#[form(textarea)]`",
            ));
        }
    }
    Ok(())
}

/// `#[form(label = "Canonical URL")]` overrides the humanized label; the rest is
/// read here. Unknown keys never reach this reader — [`validate_form_attrs`]
/// rejects them first.
fn form_attrs(attrs: &[syn::Attribute]) -> FormAttrs {
    let mut out = FormAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("form") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("label") {
                let value = meta.value()?;
                let text: syn::LitStr = value.parse()?;
                out.label = Some(text.value());
            } else if meta.path.is_ident("textarea") {
                out.textarea = true;
            } else if meta.path.is_ident("rows") {
                let value = meta.value()?;
                let rows: syn::LitInt = value.parse()?;
                out.rows = Some(rows.base10_parse()?);
            }
            Ok(())
        });
    }
    out
}

/// A field is a leaf when its type is one the panel can spell (see
/// [`PRIMITIVES`]); everything else is another embedded value.
fn kind_of(field: &syn::Field) -> Kind {
    if is_primitive(&field.ty) {
        Kind::Leaf
    } else {
        Kind::Embedded
    }
}

/// `values` carries a non-empty value for this leaf's column.
fn leaf_present(krate: &TokenStream2, path: &TokenStream2) -> TokenStream2 {
    quote! {
        values
            .get(&#krate::schema::leaf_key(cx, #path))
            .is_some_and(|value| !value.trim().is_empty())
    }
}

/// The nested value at `path` was mentioned at all.
///
/// Asked of the value itself rather than of a key list: inside an enum variant
/// the path is variant-rooted, and the nested type resolves its own leaves from
/// it (`leaf_key` follows variant roots; whole-value resolution starts at a
/// model root).
fn nested_present(krate: &TokenStream2, ty: &Type, path: &TokenStream2) -> TokenStream2 {
    quote! {
        <#ty as #krate::schema::EmbeddedForm>::any_present(cx, #path, values)
    }
}

/// The control one leaf renders: a `Textarea` when the app asks for one, a
/// `TextInput` otherwise — typed when the leaf is not a `String`.
fn leaf_control(
    krate: &TokenStream2,
    path: &TokenStream2,
    text: &str,
    ty: &Type,
    field: &syn::Field,
) -> TokenStream2 {
    let attrs = form_attrs(&field.attrs);
    if attrs.textarea {
        let rows = attrs.rows.map(|rows| quote! { .rows(#rows) });
        return quote! {
            #krate::schema::Schema::new(
                #krate::schema::Textarea::r#for_context(cx, #path).label(#text)#rows
            )
        };
    }
    if is_string(ty) {
        quote! {
            #krate::schema::Schema::new(
                #krate::schema::TextInput::r#for_context(cx, #path).label(#text)
            )
        }
    } else {
        quote! {
            #krate::schema::Schema::new(
                #krate::schema::TextInput::typed_context(cx, #path).label(#text)
            )
        }
    }
}

/// The label a derived control renders: `#[form(label = ..)]`, else the Rust
/// field name humanized.
fn field_label(field: &syn::Field, ident: &syn::Ident) -> String {
    form_attrs(&field.attrs)
        .label
        .unwrap_or_else(|| label(ident))
}

/// The hidden discriminant control plus one control per variant payload.
fn expand_struct(
    krate: &TokenStream2,
    input: &DeriveInput,
    fields: &syn::FieldsNamed,
) -> TokenStream {
    let ident = &input.ident;
    let owner = quote! { #ident };
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut writes = Vec::new();
    let mut reads = Vec::new();
    let mut controls = Vec::new();
    let mut present_checks = Vec::new();

    for (index, field) in fields.named.iter().enumerate() {
        let name = field.ident.as_ref().expect("named field");
        let ty = &field.ty;
        let parent = quote! { parent.clone() };
        match kind_of(field) {
            Kind::Leaf => {
                let path = chained(krate, &parent, &owner, ty, index, None);
                writes.push(quote! {
                    out.insert(
                        #krate::schema::leaf_key(cx, #path),
                        ::std::string::ToString::to_string(&self.#name),
                    );
                });
                present_checks.push(leaf_present(krate, &path));
                let path = chained(krate, &parent, &owner, ty, index, None);
                reads.push(quote! {
                    #name: #krate::schema::parse_leaf(&#krate::schema::leaf_key(cx, #path), values)
                });
                let text = field_label(field, name);
                controls.push(leaf_control(krate, &path, &text, ty, field));
            }
            Kind::Embedded => {
                let path = chained(krate, &parent, &owner, ty, index, None);
                writes.push(quote! {
                    #krate::schema::EmbeddedForm::write_form(&self.#name, cx, #path, out);
                });
                present_checks.push(nested_present(krate, ty, &path));
                let path = chained(krate, &parent, &owner, ty, index, None);
                reads.push(quote! {
                    #name: <#ty as #krate::schema::EmbeddedForm>::read_form(cx, #path, values)
                });
                let path = chained(krate, &parent, &owner, ty, index, None);
                controls.push(quote! { <#ty>::form(cx, #path) });
            }
        }
    }

    let expanded = quote! {
        impl #impl_generics #krate::schema::EmbeddedForm for #ident #ty_generics #where_clause {
            fn write_form<M>(
                &self,
                cx: &#krate::__macro::Cx,
                parent: #krate::__macro::Path<M, Self>,
                out: &mut ::std::collections::HashMap<String, String>,
            ) where
                M: #krate::__macro::Model,
            {
                #(#writes)*
            }

            fn read_form<M>(
                cx: &#krate::__macro::Cx,
                parent: #krate::__macro::Path<M, Self>,
                values: &::std::collections::HashMap<String, String>,
            ) -> Self
            where
                M: #krate::__macro::Model,
            {
                Self { #(#reads),* }
            }

            fn any_present<M>(
                cx: &#krate::__macro::Cx,
                parent: #krate::__macro::Path<M, Self>,
                values: &::std::collections::HashMap<String, String>,
            ) -> bool
            where
                M: #krate::__macro::Model,
            {
                false #(|| #present_checks)*
            }
        }

        impl #impl_generics #ident #ty_generics #where_clause {
            /// The form controls for this embedded value (GH #191), derived
            /// from the app schema: one per leaf column, under this value's
            /// parent path.
            ///
            /// The app composes it into a layout — `Section::new("SEO")
            /// .schema(Seo::form(cx, Post::fields().seo()))` — and declares no
            /// field bindings of its own.
            pub fn form<M>(
                cx: &#krate::__macro::Cx,
                parent: impl Into<#krate::__macro::Path<M, Self>>,
            ) -> #krate::schema::Schema
            where
                M: #krate::__macro::Model,
            {
                let parent: #krate::__macro::Path<M, Self> = parent.into();
                let mut schema = #krate::schema::Schema::empty();
                #( schema = schema.extend(#controls); )*
                schema
            }
        }
    };
    expanded.into()
}

fn expand_enum(krate: &TokenStream2, input: &DeriveInput, data: &syn::DataEnum) -> TokenStream {
    let ident = &input.ident;
    let owner = quote! { #ident };
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut write_arms = Vec::new();
    let mut read_arms = Vec::new();
    let mut controls = Vec::new();
    // A `#[shared(..)]` column is declared by several variants and is one
    // column: its control renders once, from the first variant that declares
    // it. The codec still writes and reads each variant's own spelling.
    let mut seen_shared: Vec<String> = Vec::new();
    // `(variant index, any of its own payloads submitted)` — the pre-#191
    // variant rule, reimplemented through resolved keys. Only reached when a
    // submission carries no discriminant at all.
    let mut inferred: Vec<(usize, TokenStream2)> = Vec::new();
    // `any_present`: any variant's payload, shared columns included.
    let mut variant_presence: Vec<TokenStream2> = Vec::new();

    for (variant_index, variant) in data.variants.iter().enumerate() {
        let variant_name = &variant.ident;
        // The discriminant a variant stores is addressed by declaration index:
        // that is the handle the schema itself uses, and a Rust ident is not
        // recoverable from it (Toasty normalises names, so `OK` reads `Ok`).
        let discriminant_value = quote! {
            spec.value_of_index(#variant_index)
                .expect("the app schema declares every variant this type has")
                .to_string()
        };

        match &variant.fields {
            Fields::Named(named) => {
                let mut writes = vec![quote! {
                    out.insert(spec.discriminant().to_string(), #discriminant_value);
                }];
                let mut reads = Vec::new();
                // What the *fallback* may read: a variant's own, non-shared
                // payloads.
                let mut present_checks = Vec::new();
                // What `any_present` reports for the whole enum: every payload,
                // shared ones included.
                let mut presence_checks = Vec::new();
                for (index, field) in named.named.iter().enumerate() {
                    let name = field.ident.as_ref().expect("named field");
                    let ty = &field.ty;
                    let parent = quote! { parent.clone() };
                    let shared = shared_id(&field.attrs);
                    match kind_of(field) {
                        Kind::Leaf => {
                            let path =
                                chained(krate, &parent, &owner, ty, index, Some(variant_index));
                            writes.push(quote! {
                                out.insert(
                                    #krate::schema::leaf_key(cx, #path),
                                    ::std::string::ToString::to_string(&#name),
                                );
                            });
                            // A shared column belongs to several variants, so
                            // it cannot say *which* one was meant: it never
                            // drives the fallback.
                            if shared.is_none() {
                                present_checks.push(leaf_present(krate, &path));
                            } else {
                                // Still part of the value's own presence.
                                presence_checks.push(leaf_present(krate, &path));
                            }
                            let path =
                                chained(krate, &parent, &owner, ty, index, Some(variant_index));
                            reads.push(quote! {
                                #name: #krate::schema::parse_leaf(
                                    &#krate::schema::leaf_key(cx, #path),
                                    values,
                                )
                            });
                            let text = field_label(field, name);
                            let control = leaf_control(krate, &path, &text, ty, field);
                            match shared {
                                Some(id) if seen_shared.contains(&id) => {}
                                Some(id) => {
                                    seen_shared.push(id);
                                    controls.push(control);
                                }
                                None => controls.push(control),
                            }
                        }
                        Kind::Embedded => {
                            let path =
                                chained(krate, &parent, &owner, ty, index, Some(variant_index));
                            writes.push(quote! {
                                #name.write_form(cx, #path, out);
                            });
                            // A nested value counts as submitted when any of
                            // its own keys is — its discriminant included — and
                            // it answers that itself.
                            present_checks.push(nested_present(krate, ty, &path));
                            presence_checks.push(nested_present(krate, ty, &path));
                            let path =
                                chained(krate, &parent, &owner, ty, index, Some(variant_index));
                            reads.push(quote! {
                                #name: <#ty as #krate::schema::EmbeddedForm>::read_form(cx, #path, values)
                            });
                            let path =
                                chained(krate, &parent, &owner, ty, index, Some(variant_index));
                            controls.push(quote! { <#ty>::form(cx, #path) });
                        }
                    }
                }
                if !present_checks.is_empty() {
                    inferred.push((variant_index, quote! { #(#present_checks)||* }));
                }
                if !presence_checks.is_empty() {
                    variant_presence.push(quote! { #(#presence_checks)||* });
                }
                let bindings = named.named.iter().map(|f| f.ident.as_ref().unwrap());
                write_arms.push(quote! {
                    Self::#variant_name { #(#bindings),* } => {
                        #(#writes)*
                    }
                });
                read_arms.push(quote! {
                    #variant_index => Self::#variant_name { #(#reads),* },
                });
            }
            Fields::Unit => {
                // A unit variant carries no payload, so nothing can infer it:
                // only its discriminant names it.
                write_arms.push(quote! {
                    Self::#variant_name => {
                        out.insert(spec.discriminant().to_string(), #discriminant_value);
                    }
                });
                read_arms.push(quote! { #variant_index => Self::#variant_name, });
            }
            Fields::Unnamed(_) => {
                return unsupported(input, "a struct or enum with named fields");
            }
        }
    }

    // The fallback chain, in declaration order: the first variant with a
    // submitted payload of its own, else the first variant. This is what the
    // panel did before the discriminant existed (GH #191), now driven by the
    // keys the schema resolves instead of remembered column names.
    let mut fallback = quote! { 0usize };
    for (index, check) in inferred.iter().rev() {
        fallback = quote! { if #check { #index } else { #fallback } };
    }

    let expanded = quote! {
        impl #impl_generics #krate::schema::EmbeddedForm for #ident #ty_generics #where_clause {
            fn write_form<M>(
                &self,
                cx: &#krate::__macro::Cx,
                parent: #krate::__macro::Path<M, Self>,
                out: &mut ::std::collections::HashMap<String, String>,
            ) where
                M: #krate::__macro::Model,
            {
                let spec = #krate::schema::enum_spec(cx, parent.clone())
                    .expect("an embedded enum has a discriminant column");
                match self {
                    #(#write_arms)*
                }
            }

            fn read_form<M>(
                cx: &#krate::__macro::Cx,
                parent: #krate::__macro::Path<M, Self>,
                values: &::std::collections::HashMap<String, String>,
            ) -> Self
            where
                M: #krate::__macro::Model,
            {
                let spec = #krate::schema::enum_spec(cx, parent.clone())
                    .expect("an embedded enum has a discriminant column");
                let submitted = values
                    .get(spec.discriminant())
                    .map(|value| value.trim())
                    .unwrap_or_default();
                // A discriminant the submission **names** always wins. Only a
                // submission that carries none at all falls back to the payload
                // rule; one that names an unknown variant is refused loudly
                // rather than silently read as some other variant.
                let variant: usize = if submitted.is_empty() {
                    #fallback
                } else {
                    spec.index_of(submitted).unwrap_or_else(|| {
                        panic!(
                            "submitted discriminant {submitted:?} does not name a variant of {}",
                            stringify!(#ident),
                        )
                    })
                };
                match variant {
                    #(#read_arms)*
                    other => unreachable!(
                        "variant index {other} is outside {}: the app schema and this type \
                         disagree about the variants",
                        stringify!(#ident),
                    ),
                }
            }

            fn any_present<M>(
                cx: &#krate::__macro::Cx,
                parent: #krate::__macro::Path<M, Self>,
                values: &::std::collections::HashMap<String, String>,
            ) -> bool
            where
                M: #krate::__macro::Model,
            {
                let spec = #krate::schema::enum_spec(cx, parent.clone())
                    .expect("an embedded enum has a discriminant column");
                // The discriminant is the value's own key: a submission that
                // names the variant has mentioned the value even with every
                // payload empty.
                !values
                    .get(spec.discriminant())
                    .map(|value| value.trim())
                    .unwrap_or_default()
                    .is_empty()
                    #(|| #variant_presence)*
            }
        }

        impl #impl_generics #ident #ty_generics #where_clause {
            /// The form controls for this embedded value (GH #191): the hidden
            /// discriminant plus one control per payload leaf, under this
            /// value's parent path.
            ///
            /// **Every** variant's payload renders, which is what the panel has
            /// always done by hand; choosing one variant in the UI is the
            /// follow-up on GH #191.
            pub fn form<M>(
                cx: &#krate::__macro::Cx,
                parent: impl Into<#krate::__macro::Path<M, Self>>,
            ) -> #krate::schema::Schema
            where
                M: #krate::__macro::Model,
            {
                let parent: #krate::__macro::Path<M, Self> = parent.into();
                let mut schema = #krate::schema::Schema::new(
                    #krate::schema::discriminant_input(cx, parent.clone())
                        .expect("an embedded enum has a discriminant column"),
                );
                #( schema = schema.extend(#controls); )*
                schema
            }
        }
    };
    expanded.into()
}

/// The `#[shared(ident)]` identifier a field declares, if any.
fn shared_id(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("shared") {
            continue;
        }
        let mut id = None;
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(ident) = meta.path.get_ident() {
                id = Some(ident.to_string());
            }
            Ok(())
        });
        if id.is_some() {
            return id;
        }
    }
    None
}
