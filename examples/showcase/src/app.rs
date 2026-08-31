use std::collections::HashMap;

use argentum_core::{NavigationItem, Panel, Resource, Schema, Table, TextColumn, TextInput};
use toasty::Db;
use topcoat::{
    Result,
    asset::AssetBundle,
    context::Cx,
    font::{Font, fontsource::fontsource_font},
    router::{Router, href, layout},
    tailwind,
};

use crate::models::User;

/// The theme's sans font, pulled from Fontsource and self-hosted as a Topcoat asset.
const GEIST: Font = fontsource_font!(GEIST, host: Asset);

// ---------------------------------------------------------------------------
// Resource — single Model → Resource, see CONTEXT.md
// ---------------------------------------------------------------------------

/// Admin resource for `User`.
///
/// Manual `Resource` impl — `#[derive(Resource)]` currently only supports
/// `model`/`query`, not `table`. A custom `Table` is needed so we implement
/// `Resource` by hand; a `#[resource(table=...)]` derive extension will
/// replace this later.
pub struct UserResource;

impl Resource for UserResource {
    type Model = User;

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }
    fn can_view(_cx: &Cx, _record: &User) -> bool {
        true
    }
    fn can_create(_cx: &Cx) -> bool {
        true
    }
    fn can_update(_cx: &Cx, _record: &User) -> bool {
        true
    }
    fn can_delete(_cx: &Cx, _record: &User) -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<User> {
        Table::r#for(cx)
            .id(|u: &User| u.id.to_string())
            .columns((
                TextColumn::r#for(User::fields().name(), |u: &User| u.name.clone())
                    .searchable()
                    .sortable(),
                TextColumn::r#for(User::fields().email(), |u: &User| u.email.clone()).searchable(),
                TextColumn::r#for(User::fields().role(), |u: &User| u.role.clone()),
                TextColumn::computed("Status", |u: &User| {
                    if u.active { "Active" } else { "Inactive" }.to_string()
                }),
                TextColumn::computed("Created", |u: &User| {
                    u.created_at.strftime("%Y-%m-%d").to_string()
                }),
            ))
            .paginate(2)
    }

    fn form(_cx: &Cx) -> Schema {
        // Canonical Resource::form seam (spec #6 solution) — typed lens → TextInput.
        // Proves both Resource entry points are wired; showcase pages use this
        // indirectly via Schema::new, but resource owners declare forms here.
        Schema::new((
            TextInput::r#for(User::fields().name()).required(),
            TextInput::r#for(User::fields().email())
                .required()
                .email()
                .unique(),
        ))
    }

    fn hydrate_form_values(record: &User) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("name".to_string(), record.name.clone());
        map.insert("email".to_string(), record.email.clone());
        map
    }

    fn create_record(
        cx: &Cx,
        values: HashMap<String, String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            let name = values
                .get("name")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let email = values
                .get("email")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let mut db = argentum_core::db::db(&cx);
            toasty::create!(User {
                name: name,
                email: email,
                role: "member",
                active: true,
                created_at: jiff::Timestamp::now(),
            })
            .exec(&mut db)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn update_record(
        cx: &Cx,
        id: String,
        values: HashMap<String, String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
            })?;
            let mut db = argentum_core::db::db(&cx);
            let mut record = Self::query(&cx)
                .filter(User::fields().id().eq(uuid))
                .first()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .ok_or_else(topcoat::router::error::not_found)?;
            let name = values
                .get("name")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let email = values
                .get("email")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            toasty::update!(record {
                name: name,
                email: email,
            })
            .exec(&mut db)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn delete_record(cx: &Cx, id: String) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
            })?;
            let mut db = argentum_core::db::db(&cx);
            let record = Self::query(&cx)
                .filter(User::fields().id().eq(uuid))
                .first()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .ok_or_else(topcoat::router::error::not_found)?;
            // Use the model's delete via query to respect tenancy.
            Self::query(&cx)
                .filter(User::fields().id().eq(record.id))
                .delete()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn bulk_delete_records(
        cx: &Cx,
        ids: Vec<String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            let mut db = argentum_core::db::db(&cx);
            // All-or-nothing: verify each exists and passes policy before deleting.
            for id in &ids {
                let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
                })?;
                let rec = Self::query(&cx)
                    .filter(User::fields().id().eq(uuid))
                    .first()
                    .exec(&mut db)
                    .await
                    .map_err(|e| -> topcoat::Error { e.into() })?
                    .ok_or_else(topcoat::router::error::not_found)?;
                if !Self::can_delete(&cx, &rec) {
                    return Err(topcoat::router::error::forbidden().into());
                }
            }
            // Perform deletes.
            for id in ids {
                let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
                })?;
                Self::query(&cx)
                    .filter(User::fields().id().eq(uuid))
                    .delete()
                    .exec(&mut db)
                    .await
                    .map_err(|e| -> topcoat::Error { e.into() })?;
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Layout — Panel shell at /admin, wraps every /admin/* page
// ---------------------------------------------------------------------------

#[layout("/admin")]
async fn admin_layout(cx: &Cx, slot: Result) -> Result {
    Panel::layout_shell(cx, slot).await
}

// ---------------------------------------------------------------------------
// Router helper (used by `main.rs` and tests)
// ---------------------------------------------------------------------------

pub fn router(db: Db) -> Router {
    build_router(db, Some(load_assets()))
}

/// Build the showcase router without filesystem assets for markup tests.
///
/// This is deliberately separate from [`router`]: the application path fails
/// loudly when its generated bundle is missing, while tests can exercise the
/// server-rendered markup without pretending an asset bundle exists.
pub fn router_for_tests(db: Db) -> Router {
    build_router(db, None)
}

fn build_router(db: Db, bundle: Option<AssetBundle>) -> Router {
    let panel = Panel::new("admin")
        .app_context(db)
        .resource::<UserResource>()
        .navigation(NavigationItem::from_href(
            "Showcase",
            href!("/admin/showcase"),
            "/admin/showcase",
        ));
    match bundle {
        Some(bundle) => panel
            .assets(bundle)
            .shell_assets(tailwind::stylesheet!(), GEIST)
            .build(),
        None => panel.build(),
    }
}

fn load_assets() -> AssetBundle {
    match AssetBundle::load() {
        Ok(bundle) => bundle,
        Err(near_executable) => {
            // Cargo places test executables in `target/*/deps`, while the
            // bundle remains beside the package binary in `target/*`.
            let test_bundle = std::env::current_exe()
                .ok()
                .and_then(|exe| {
                    let dir = exe.parent()?;
                    if dir.file_name().is_some_and(|name| name == "deps") {
                        Some(dir.parent()?.to_path_buf())
                    } else {
                        None
                    }
                })
                .map(|dir| dir.join("assets"));
            match test_bundle {
                Some(dir) => AssetBundle::load_dir(&dir).unwrap_or_else(|test_error| {
                    panic!(
                        "showcase asset bundle is unavailable: executable lookup failed ({near_executable}); tried {} ({test_error})",
                        dir.display()
                    )
                }),
                None => panic!(
                    "showcase asset bundle is unavailable: executable lookup failed ({near_executable})"
                ),
            }
        }
    }
}
