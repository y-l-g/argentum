//! Procedural macros for Argentum.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    DeriveInput, Token,
    parse::{Parse, ParseStream},
};

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

    let expanded = match &args.query {
        Some(path) => quote! {
            impl #impl_generics ::argentum_core::Resource for #ident #ty_generics #where_clause {
                type Model = #model_ty;
                fn query(cx: &::argentum_core::__macro::Cx)
                    -> ::argentum_core::__macro::stmt::Query<
                        ::argentum_core::__macro::stmt::List<Self::Model>>
                {
                    #path(cx)
                }
            }
        },
        None => quote! {
            impl #impl_generics ::argentum_core::Resource for #ident #ty_generics #where_clause {
                type Model = #model_ty;
            }
        },
    };
    TokenStream::from(expanded)
}
