//! Fixtures shared by the panel's `#[cfg(test)]` modules.
//!
//! Every module's resource declares the same two-column model and the same
//! table over it, and mounts the same panel; one copy lives here.

use toasty::Db;
use topcoat::context::Cx;

use crate::{
    Panel,
    resource::{Resource, Table, TextColumn},
};

/// The two-column model a panel test's resource renders.
#[derive(Debug, Clone, toasty::Model)]
pub(crate) struct Dummy {
    #[key]
    #[auto]
    pub(crate) id: uuid::Uuid,
    pub(crate) name: String,
}

/// [`Dummy`]'s canonical table: display key, record key, one name column.
pub(crate) fn dummy_table(cx: &Cx) -> Table<Dummy> {
    Table::<Dummy>::r#for(cx)
        .id(|d: &Dummy| d.id.to_string())
        .pk(|d: &Dummy| d.id.to_string())
        .columns(TextColumn::r#for(Dummy::fields().name(), |d: &Dummy| {
            d.name.clone()
        }))
}

/// A panel mounted at `/admin` with one resource and the auth gate off.
pub(crate) fn panel_for<R: Resource>(db: Db) -> Panel {
    Panel::new("admin")
        .app_context(db)
        .resource::<R>()
        .auth(crate::Auth::disabled())
}
