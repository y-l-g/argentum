//! Procedural macros for Argentum.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    DeriveInput, Token,
    parse::{Parse, ParseStream},
};

struct ResourceArgs {
    model: syn::Type,
}

impl Parse for ResourceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut model: Option<syn::Type> = None;
        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if ident == "model" {
                let ty: syn::Type = input.parse()?;
                model = Some(ty);
            } else {
                // Unknown key — consume its value as a Type and ignore
                let _: syn::Type = input.parse()?;
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        let model = model.ok_or_else(|| {
            syn::Error::new(input.span(), "missing `model = Type` in #[resource(...)]")
        })?;
        Ok(Self { model })
    }
}

/// Derive `Resource` for a unit struct.
///
/// Expects `#[resource(model = Type)]` where `Type` is the Toasty `Model`.
///
/// ```ignore
/// #[derive(Resource)]
/// #[resource(model = User)]
/// struct UserResource;
/// ```
#[proc_macro_derive(Resource, attributes(resource))]
pub fn resource(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let ident = &input.ident;
    let generics = &input.generics;

    // Find #[resource(...)] attribute
    let attr = input.attrs.iter().find(|a| a.path().is_ident("resource"));
    let Some(attr) = attr else {
        return syn::Error::new_spanned(
            ident,
            "missing #[resource(model = Type)] attribute",
        )
        .to_compile_error()
        .into();
    };

    let args: ResourceArgs = match attr.parse_args() {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    let model_ty = &args.model;

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics ::argentum_core::Resource for #ident #ty_generics #where_clause {
            type Model = #model_ty;
        }
    };
    TokenStream::from(expanded)
}
