//! Argentum — a server-rendered admin toolkit on Topcoat and Toasty.
//!
//! This crate holds the toolkit's foundation: the [`Resource`] trait and the
//! types that compose an admin UI (Panel, Table, Schema, Action). See the
//! workspace README and `CONTEXT.md` for the vocabulary and the roadmap.
#![doc = include_str!("../../../CONTEXT.md")]

#[doc(hidden)]
pub mod __macro {
    pub use toasty::{schema, stmt};
    pub use topcoat::context::Cx;
}
// The full toolkit surface lands in later phases. For now, expose a single
// renderable component and the `Db` glue so the crate has vertical slices
// through Topcoat and Toasty.
pub mod db;
pub mod panel;
pub mod resource;
pub mod schema;
pub mod view;

pub use argentum_macros::Resource;
pub use panel::Panel;
pub use resource::{NavigationItem, Pages, Resource, Table};
pub use schema::{FieldLens, Grid, Group, IntoSchema, Schema, Section, Text, TextInput};
