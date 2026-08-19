//! Procedural macros for Argentum.
//!
//! This crate is a skeleton. The `Resource` derive (and the grammar/macro
//! trio split, mirroring Topcoat) lands in the phase that implements
//! `#[derive(Resource)]`. Until then it exposes no working macros.

use proc_macro::TokenStream;

/// Placeholder so the crate has a real entry point. Emits a compile error if
/// used before the derive is implemented, so a stray `#[derive(Resource)]`
/// fails loudly instead of silently doing nothing.
#[proc_macro_derive(Resource)]
pub fn resource(_input: TokenStream) -> TokenStream {
    "compile_error!(\"`#[derive(Resource)]` is not implemented yet; see the Argentum roadmap.\")"
        .parse()
        .unwrap()
}
