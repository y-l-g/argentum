use std::collections::HashMap;

use argentum_core::{
    DateFilter, FileUpload, Grid, NavigationItem, Panel, Repeater, Resource, Schema, Section,
    Select, SelectFilter, Table, TernaryFilter, TextColumn, TextInput, tenant_id,
};
use toasty::Db;
use topcoat::{
    Result,
    asset::AssetBundle,
    context::Cx,
    font::{Font, fontsource::fontsource_font},
    router::{Router, href, layout},
    tailwind,
};

use crate::models::{Author, Comment, Post, User};

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

pub struct AuthorResource;

impl Resource for AuthorResource {
    type Model = Author;

    fn query(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Author>> {
        let mut q = toasty::stmt::Query::<toasty::stmt::List<Author>>::all();
        if let Some(tid) = tenant_id(cx) {
            q = q.filter(Author::fields().tenant_id().eq(tid));
        }
        q
    }

    fn can_view_any(cx: &Cx) -> bool {
        if let Some(tid) = tenant_id(cx)
            && tid == uuid::Uuid::from_u128(9999)
        {
            return false;
        }
        true
    }
    fn can_view(cx: &Cx, _record: &Author) -> bool {
        Self::can_view_any(cx)
    }
    fn can_create(cx: &Cx) -> bool {
        Self::can_view_any(cx)
    }
    fn can_update(cx: &Cx, _record: &Author) -> bool {
        Self::can_view_any(cx)
    }
    fn can_delete(cx: &Cx, _record: &Author) -> bool {
        Self::can_view_any(cx)
    }

    fn table(cx: &Cx) -> Table<Author> {
        Table::r#for(cx)
            .id(|a: &Author| a.id.to_string())
            .columns((
                TextColumn::r#for(Author::fields().name(), |a: &Author| a.name.clone())
                    .searchable()
                    .sortable(),
                TextColumn::r#for(Author::fields().email(), |a: &Author| a.email.clone())
                    .searchable(),
            ))
            .paginate(2)
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            TextInput::r#for(Author::fields().name()).required(),
            TextInput::r#for(Author::fields().email())
                .required()
                .email()
                .unique(),
        ))
    }

    fn hydrate_form_values(record: &Author) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("name".to_string(), record.name.clone());
        m.insert("email".to_string(), record.email.clone());
        m
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
            let tid = tenant_id(&cx).unwrap_or(uuid::Uuid::nil());
            let mut db = argentum_core::db::db(&cx);
            toasty::create!(Author {
                tenant_id: tid,
                name: name,
                email: email
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
            let mut rec = Self::query(&cx)
                .filter(Author::fields().id().eq(uuid))
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
            toasty::update!(rec {
                name: name,
                email: email
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
            let rec = Self::query(&cx)
                .filter(Author::fields().id().eq(uuid))
                .first()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .ok_or_else(topcoat::router::error::not_found)?;
            Self::query(&cx)
                .filter(Author::fields().id().eq(rec.id))
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
            for id in &ids {
                let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
                })?;
                let rec = Self::query(&cx)
                    .filter(Author::fields().id().eq(uuid))
                    .first()
                    .exec(&mut db)
                    .await
                    .map_err(|e| -> topcoat::Error { e.into() })?
                    .ok_or_else(topcoat::router::error::not_found)?;
                if !Self::can_delete(&cx, &rec) {
                    return Err(topcoat::router::error::forbidden().into());
                }
            }
            for id in ids {
                let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
                })?;
                Self::query(&cx)
                    .filter(Author::fields().id().eq(uuid))
                    .delete()
                    .exec(&mut db)
                    .await
                    .map_err(|e| -> topcoat::Error { e.into() })?;
            }
            Ok(())
        }
    }
}

pub struct PostResource;

impl Resource for PostResource {
    type Model = Post;

    fn query(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Post>> {
        // Tenancy + explicit includes (one round-trip, no N+1).
        let mut q = toasty::stmt::Query::<toasty::stmt::List<Post>>::all();
        if let Some(tid) = tenant_id(cx) {
            q = q.filter(Post::fields().tenant_id().eq(tid));
        }
        let inc_author: toasty::stmt::Include<Post, Author> = Post::fields().author().into();
        let inc_comments: toasty::stmt::Include<Post, toasty::stmt::List<Comment>> =
            Post::fields().comments().into();
        q.include(inc_author).include(inc_comments)
    }

    fn can_view_any(cx: &Cx) -> bool {
        if let Some(tid) = tenant_id(cx)
            && tid == uuid::Uuid::from_u128(9999)
        {
            return false;
        }
        true
    }
    fn can_view(cx: &Cx, _record: &Post) -> bool {
        Self::can_view_any(cx)
    }
    fn can_create(cx: &Cx) -> bool {
        Self::can_view_any(cx)
    }
    fn can_update(cx: &Cx, _record: &Post) -> bool {
        Self::can_view_any(cx)
    }
    fn can_delete(cx: &Cx, _record: &Post) -> bool {
        Self::can_view_any(cx)
    }

    fn table(cx: &Cx) -> Table<Post> {
        Table::r#for(cx)
            .id(|p: &Post| p.id.to_string())
            .columns((
                TextColumn::r#for(Post::fields().title(), |p: &Post| p.title.clone())
                    .searchable()
                    .sortable(),
                TextColumn::computed("Author", |p: &Post| {
                    if p.author.is_unloaded() {
                        "-".to_string()
                    } else {
                        p.author.get().name.clone()
                    }
                }),
                TextColumn::computed("Comments", |p: &Post| {
                    if p.comments.is_unloaded() {
                        "0".to_string()
                    } else {
                        p.comments.get().len().to_string()
                    }
                }),
                TextColumn::computed("Author Email", |p: &Post| {
                    if p.author.is_unloaded() {
                        String::new()
                    } else {
                        p.author.get().email.clone()
                    }
                }),
            ))
            .filters((
                SelectFilter::r#for(
                    Post::fields().status(),
                    vec!["draft".into(), "published".into()],
                ),
                TernaryFilter::r#for(Post::fields().featured()),
                DateFilter::r#for(Post::fields().created_at()),
            ))
            .paginate(2)
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            Section::new("Post Details").schema((
                TextInput::r#for(Post::fields().title()).required(),
                Select::r#for(Post::fields().author_id())
                    .relationship::<AuthorResource>(AuthorResource::query, |a: &Author| {
                        a.name.clone()
                    })
                    .required()
                    .label("Author"),
            )),
            Grid::new(2).schema((
                FileUpload::r#for(Post::fields().image_path()).required(),
                Repeater::new("Tags").schema(
                    TextInput::r#for(Post::fields().tags())
                        .required()
                        .label("Tag"),
                ),
            )),
        ))
    }

    fn hydrate_form_values(record: &Post) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("title".to_string(), record.title.clone());
        m.insert("author_id".to_string(), record.author_id.to_string());
        m.insert("image_path".to_string(), record.image_path.clone());
        m.insert("tags".to_string(), record.tags.clone());
        m
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
            let title = values
                .get("title")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let author_id_str = values
                .get("author_id")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let author_id = author_id_str.parse::<uuid::Uuid>().map_err(|e| {
                topcoat::Error::from(std::io::Error::other(format!("invalid author_id: {e}")))
            })?;
            // Verify author exists via AuthorResource::query (tenancy-aware) - existence already checked in validation but double.
            let mut db = argentum_core::db::db(&cx);
            let author_exists = AuthorResource::query(&cx)
                .filter(Author::fields().id().eq(author_id))
                .first()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .is_some();
            if !author_exists {
                return Err(topcoat::Error::from(std::io::Error::other(
                    "author not found",
                )));
            }
            let image_path = values
                .get("image_path")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let tags = values
                .get("tags")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let tid = tenant_id(&cx).unwrap_or(uuid::Uuid::nil());
            toasty::create!(Post {
                tenant_id: tid,
                title: title,
                body: String::new(),
                status: "draft".to_string(),
                featured: false,
                created_at: jiff::Timestamp::now(),
                image_path: image_path,
                tags: tags,
                author_id: author_id,
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
            let mut rec = Self::query(&cx)
                .filter(Post::fields().id().eq(uuid))
                .first()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .ok_or_else(topcoat::router::error::not_found)?;
            let title = values
                .get("title")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let author_id_str = values
                .get("author_id")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let author_id = author_id_str.parse::<uuid::Uuid>().map_err(|e| {
                topcoat::Error::from(std::io::Error::other(format!("invalid author_id: {e}")))
            })?;
            let image_path = values
                .get("image_path")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let tags = values
                .get("tags")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            toasty::update!(rec {
                title: title,
                author_id: author_id,
                image_path: image_path,
                tags: tags
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
            let rec = Self::query(&cx)
                .filter(Post::fields().id().eq(uuid))
                .first()
                .exec(&mut db)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .ok_or_else(topcoat::router::error::not_found)?;
            Self::query(&cx)
                .filter(Post::fields().id().eq(rec.id))
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
            for id in &ids {
                let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
                })?;
                let rec = Self::query(&cx)
                    .filter(Post::fields().id().eq(uuid))
                    .first()
                    .exec(&mut db)
                    .await
                    .map_err(|e| -> topcoat::Error { e.into() })?
                    .ok_or_else(topcoat::router::error::not_found)?;
                if !Self::can_delete(&cx, &rec) {
                    return Err(topcoat::router::error::forbidden().into());
                }
            }
            for id in ids {
                let uuid = id.parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid id: {e}")))
                })?;
                Self::query(&cx)
                    .filter(Post::fields().id().eq(uuid))
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
        .resource::<AuthorResource>()
        .resource::<PostResource>()
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
