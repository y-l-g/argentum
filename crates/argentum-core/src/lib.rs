//! Argentum — a server-rendered admin toolkit on Topcoat and Toasty.
//!
//! This crate holds the toolkit's foundation: the [`Resource`] trait and the
//! types that compose an admin UI (Panel, Table, Schema, Action). See the
//! workspace README and `CONTEXT.md` for the vocabulary and the roadmap.
#![doc = include_str!("../../../CONTEXT.md")]

#[doc(hidden)]
pub mod __macro {
    pub use toasty::stmt;
    pub use topcoat::context::Cx;
}
// The full toolkit surface lands in later phases. For now, expose a single
// renderable component and the `Db` glue so the crate has vertical slices
// through Topcoat and Toasty.
pub mod cursor;
pub mod db;
pub mod notification;
pub mod panel;
pub mod policy;
pub mod resource;
pub mod schema;
pub mod view;

pub use argentum_macros::Resource;
pub use notification::{Notification, NotificationStatus};
pub use panel::{Brand, DarkMode, Panel};
pub use policy::{AllowAll, DenyAll, Policy};
pub use resource::{
    Column, NavigationItem, Pages, Resource, RowKey, Sort, Table, TablePage, TableState, TextColumn,
};
pub use schema::{FieldLens, Grid, Group, IntoSchema, Schema, Section, Select, Text, TextInput};
