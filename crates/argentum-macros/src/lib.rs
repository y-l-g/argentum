//! Procedural macros for Argentum.

mod embedded;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    DeriveInput, Token,
    parse::{Parse, ParseStream},
};

/// Derive `EmbeddedForm` for an embedded struct or enum (GH #191).
///
/// The generated impl converts the value to and from the panel's flat form map,
/// answers whether a submission mentions it, and generates a `form(cx, parent)`
/// function returning the value's controls — all driven by the columns the app
/// schema resolves for the parent path. The framework supplies the storage
/// names, this derive supplies the Rust shape, so no flattened name is ever
/// spelled by hand.
///
/// ```ignore
/// #[derive(Debug, Clone, toasty::Embed, argentum_core::EmbeddedForm)]
/// pub enum Publication {
///     #[column(variant = 1)]
///     Scheduled {
///         #[shared(timestamp)]
///         #[form(label = "Publication timestamp")]
///         scheduled_at: String,
///         scheduled_for: String,
///     },
///     #[column(variant = 2)]
///     Published {
///         #[shared(timestamp)]
///         published_at: String,
///         canonical_url: String,
///     },
/// }
///
/// // form declaration — no field bindings written by hand
/// Section::new("Publication").schema(Publication::form(cx, Post::fields().publication()))
///
/// // hydration and the record fn
/// write_embedded(cx, Post::fields().publication(), &record.publication, &mut values);
/// let publication = read_embedded(cx, Post::fields().publication(), &values);
/// ```
///
/// # How a field is classified
///
/// A field whose type is a Rust primitive this panel can spell — `String`, the
/// integer family, `bool`, `f32`/`f64`, `Uuid`, `jiff::Timestamp` — is a
/// **leaf**: one column, read and written as text (typed leaves parse through
/// `TypedValue`, GH #192). Any other type is another **embedded value**,
/// delegated to that type's own `EmbeddedForm` impl, so nesting works by
/// deriving on each type. A relation, an `Option<T>`, a `Vec<T>`, and a
/// `#[document]` inside a value do not compile, or are refused at the schema
/// (see the `argentum-core` module docs).
///
/// # Which variant an enum reads
///
/// The discriminant column decides, in this order:
///
/// 1. a discriminant the submission **names** — always wins, and one the enum
///    does not declare is refused loudly rather than read as some other
///    variant;
/// 2. otherwise (the create form, a hand-written POST) the first variant, in
///    declaration order, with a **payload of its own** submitted — a
///    `#[shared(..)]` column belongs to several variants and so never selects
///    one;
/// 3. otherwise the first variant.
///
/// # Per-field overrides
///
/// - `#[form(label = "Canonical URL")]` — the control's label (default: the
///   field name, humanized).
/// - `#[form(textarea)]` / `#[form(textarea, rows = 3)]` — a multi-line control
///   for a `String` leaf, and its height.
///
/// Anything else in `#[form(..)]` is a compile error.
///
/// # Not covered
///
/// A `#[document]` **inside** an embedded value (its inner fields share one
/// column, so no per-field binding exists), a relation inside one, an embedded
/// enum nested **inside an enum variant** (value resolution starts at a model
/// root), and a tuple or unit struct.
#[proc_macro_derive(EmbeddedForm, attributes(form))]
pub fn embedded_form(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    embedded::expand(input)
}

struct ResourceArgs {
    model: syn::Type,
    query: Option<syn::Path>,
}

impl Parse for ResourceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut model: Option<syn::Type> = None;
        let mut query: Option<syn::Path> = None;
        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if ident == "model" {
                if model.is_some() {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        "duplicate `model` key in #[resource(...)]",
                    ));
                }
                let ty: syn::Type = input.parse()?;
                model = Some(ty);
            } else if ident == "query" {
                if query.is_some() {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        "duplicate `query` key in #[resource(...)]",
                    ));
                }
                let path = input.parse::<syn::Path>().map_err(|e| {
                    syn::Error::new(e.span(), "expected `query = path_to_function`")
                })?;
                query = Some(path);
            } else {
                return Err(syn::Error::new_spanned(
                    &ident,
                    format!("unknown key `{ident}`, expected `model` or `query`"),
                ));
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        let model = model.ok_or_else(|| {
            syn::Error::new(input.span(), "missing `model = Type` in #[resource(...)]")
        })?;

        Ok(Self { model, query })
    }
}

/// Reject anything that is not a unit struct (GH #103).
fn check_unit_struct(input: &DeriveInput) -> syn::Result<()> {
    if matches!(&input.data, syn::Data::Struct(data) if matches!(data.fields, syn::Fields::Unit)) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(Resource)] only supports unit structs",
        ))
    }
}

/// Derive `Resource` for a unit struct.
///
/// Expects `#[resource(model = Type)]` where `Type` is the Toasty `Model`.
/// Optionally `query = path` scopes the base query, where `path` is a
/// function `fn(&Cx) -> toasty::stmt::Query<toasty::stmt::List<Model>>`.
///
/// ```ignore
/// #[derive(Resource)]
/// #[resource(model = User)]
/// struct UserResource;
///
/// #[derive(Resource)]
/// #[resource(model = User, query = my_scope)]
/// struct ScopedResource;
///
/// fn my_scope(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<User>> {
///     toasty::stmt::Query::<toasty::stmt::List<User>>::all()
///         .filter(User::fields().name().eq("Ada"))
/// }
/// ```
#[proc_macro_derive(Resource, attributes(resource))]
pub fn resource(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let ident = &input.ident;
    let generics = &input.generics;

    // The derive implements `Resource` with no per-instance state, so only
    // unit structs are meaningful (GH #103): anything else silently yields an
    // impl that ignores the shape.
    if let Err(e) = check_unit_struct(&input) {
        return e.to_compile_error().into();
    }

    // Find #[resource(...)] attribute
    let attr = input.attrs.iter().find(|a| a.path().is_ident("resource"));
    let Some(attr) = attr else {
        return syn::Error::new_spanned(ident, "missing #[resource(model = Type)] attribute")
            .to_compile_error()
            .into();
    };

    let args: ResourceArgs = match attr.parse_args() {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    let model_ty = &args.model;

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Resolve the path to `argentum-core` in the consumer crate. `proc-macro-crate`
    // handles `package = "argentum-core"` renames and `as alias` (GH #52).
    // A missing dependency is a spanned `compile_error!`, not a proc-macro
    // panic (GH #103).
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
                ident,
                "argentum-core must be a dependency to #[derive(Resource)]",
            )
            .to_compile_error()
            .into();
        }
    };

    let expanded = match &args.query {
        Some(path) => quote! {
            impl #impl_generics #krate::Resource for #ident #ty_generics #where_clause {
                type Model = #model_ty;
                fn query(cx: &#krate::__macro::Cx)
                    -> #krate::__macro::stmt::Query<
                        #krate::__macro::stmt::List<Self::Model>>
                {
                    #path(cx)
                }
            }
        },
        None => quote! {
            impl #impl_generics #krate::Resource for #ident #ty_generics #where_clause {
                type Model = #model_ty;
            }
        },
    };
    TokenStream::from(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> bool {
        let input: DeriveInput = syn::parse_str(src).expect("test input must parse");
        check_unit_struct(&input).is_ok()
    }

    #[test]
    fn unit_structs_pass_fieldful_structs_and_enums_fail() {
        assert!(check("struct Foo;"));
        assert!(check("struct Foo<T>;"));
        assert!(!check("struct Foo { x: u8 }"));
        assert!(!check("struct Foo(u8);"));
        assert!(!check("enum Foo { A, B }"));
        assert!(!check("union Foo { x: u8 }"));
    }
}
