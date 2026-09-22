//! Argentum — a server-rendered admin toolkit on Topcoat and Toasty.
//!
//! This crate holds the toolkit's foundation: the [`Resource`] trait and the
//! types that compose an admin UI (Panel, Table, Schema, Action). See the
//! workspace README and `CONTEXT.md` for the vocabulary and the roadmap.
#![doc = include_str!("../../../CONTEXT.md")]

#[doc(hidden)]
pub mod __macro {
    pub use toasty::schema::{Embed, Model};
    pub use toasty::stmt;
    pub use toasty::stmt::Path;
    pub use toasty_core::schema::app::VariantId;
    pub use topcoat::context::Cx;
}
// The toolkit surface: Panel, Resource, Table, Schema, Notification,
// Tenancy, CSRF, and the `Db` glue.
#[cfg(feature = "auth")]
pub mod auth;
pub mod csrf;
pub mod cursor;
pub mod db;
pub mod notification;
pub mod panel;
pub mod resource;
pub mod schema;
pub mod tenancy;
pub mod upload;

pub use argentum_macros::{EmbeddedForm, Resource};
#[cfg(feature = "auth")]
pub use auth::{Auth, Authenticator, CurrentUser, PasswordAuth};
pub use notification::{Notification, NotificationStatus};
pub use panel::{Brand, DarkMode, Panel};
pub use resource::{
    Committed, DateFilter, Filter, IncludeNeeds, IntoFilters, IntoRelationColumns, Mutation,
    NavTarget, NavigationItem, RelationColumn, RelationColumns, Resource, RowKey, SelectFilter,
    Sort, Table, TablePage, TableSignals, TableState, TernaryFilter, TextColumn, VariantFilter,
    render_relation,
};
pub use schema::{
    EmbeddedForm, EnumSpec, FieldLens, FileUpload, Grid, Group, IntoSchema, Repeater, Schema,
    Section, Select, Tabs, TextInput, Textarea, TypedValue, discriminant_input, enum_spec,
    leaf_key, parse_leaf, read_embedded, submitted, write_embedded,
};
pub use tenancy::{Tenant, tenant_id};
pub use upload::Uploader;
