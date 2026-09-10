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
// The toolkit surface: Panel, Resource, Table, Schema, Notification, Policy,
// Tenancy, CSRF, and the `Db` glue.
#[cfg(feature = "auth")]
pub mod auth;
pub mod csrf;
pub mod cursor;
pub mod db;
pub mod notification;
pub mod panel;
pub mod policy;
pub mod resource;
pub mod schema;
pub mod tenancy;

pub use argentum_macros::Resource;
#[cfg(feature = "auth")]
pub use auth::{Auth, Authenticator, CurrentUser, PasswordAuth};
pub use notification::{Notification, NotificationStatus};
pub use panel::{Brand, DarkMode, Panel};
pub use policy::{AllowAll, DenyAll, Policy};
pub use resource::{
    Column, DateFilter, Filter, IntoFilters, NavigationItem, Pages, Resource, RowKey, SelectFilter,
    Sort, Table, TablePage, TableState, TernaryFilter, TextColumn, VariantFilter,
};
pub use schema::{
    FieldLens, FileUpload, Grid, Group, IntoSchema, Repeater, Schema, Section, Select, Tabs, Text,
    TextInput, Wizard, lens_field_is_nullable,
};
pub use tenancy::{Tenant, tenant_id};
