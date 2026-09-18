use std::collections::HashMap;

use argentum_core::{
    Brand, DateFilter, FileUpload, Grid, Group, NavigationItem, Panel, Repeater, Resource, Schema,
    Section, Select, SelectFilter, Table, Tabs, TernaryFilter, TextColumn, TextInput,
    VariantFilter, Wizard, resource::HrefCheck, tenant_id,
};
use toasty::Db;
use topcoat::{
    Result,
    asset::AssetBundle,
    context::Cx,
    font::{Font, fontsource::fontsource_font},
    router::{Router, Slot, layout},
    tailwind,
    view::View,
};

use crate::models::{Author, BLOCKED_TENANT, Comment, Post, User};

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

    fn navigation_label() -> String {
        "Team".to_string()
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }
    fn can_view(_cx: &Cx, _record: &User) -> bool {
        true
    }
    fn can_create(_cx: &Cx) -> bool {
        true
    }
    fn can_update(_cx: &Cx, record: &User) -> bool {
        // Row-level rule: Ken's account is SSO-managed outside the panel,
        // so the panel never writes it (reads still flow).
        record.name != "Ken Thompson"
    }
    fn can_delete(_cx: &Cx, record: &User) -> bool {
        // Same SSO guard on the delete path: per-row Policy proven over HTTP.
        record.name != "Ken Thompson"
    }

    fn table(cx: &Cx) -> Table<User> {
        Table::r#for(cx)
            .id(|u: &User| u.id.to_string())
            .pk(|u: &User| u.id.to_string())
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
            .paginate(25)
            .live_search(true)
    }

    fn form(_cx: &Cx) -> Schema {
        // Profile as a single-step wizard: the shipped Wizard seam grouping
        // a real section, not a throwaway demo page.
        Schema::new(
            Wizard::new().schema(
                Section::new("Profile").schema((
                    TextInput::r#for(User::fields().name()).placeholder("Ada Lovelace"),
                    TextInput::r#for(User::fields().email())
                        .email()
                        .unique()
                        .placeholder("ada@example.com"),
                    // Static-options Select (the non-relationship kind): role
                    // vocabulary with presence defaulting from the column.
                    Select::r#for(User::fields().role())
                        .options(vec!["admin".to_string(), "member".to_string()])
                        .label("Role")
                        .optional(),
                    // Bool lens via static options: the shipped Field set has no
                    // checkbox, so Active renders as a Yes/No select.
                    Select::r#for(User::fields().active())
                        .options_with_labels(vec![
                            ("true".to_string(), "Active".to_string()),
                            ("false".to_string(), "Inactive".to_string()),
                        ])
                        .label("Active")
                        .optional(),
                )),
            ),
        )
    }

    fn hydrate_form_values(record: &User) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("name".to_string(), record.name.clone());
        map.insert("email".to_string(), record.email.clone());
        map.insert("role".to_string(), record.role.clone());
        map.insert(
            "active".to_string(),
            if record.active { "true" } else { "false" }.to_string(),
        );
        map
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<()> {
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
        // Optional selects fall back to member/active: the form offers them,
        // older clients omitting them still create a valid member.
        let role = match values.get("role").map(|s| s.trim().to_string()) {
            Some(r) if r == "admin" || r == "member" => r,
            _ => "member".to_string(),
        };
        let active = !matches!(
            values.get("active").map(|s| s.trim().to_string()),
            Some(a) if a == "false"
        );
        toasty::create!(User {
            name: name,
            email: email,
            role: role,
            active: active,
            created_at: jiff::Timestamp::now(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        Ok(())
    }

    async fn update_record(
        _cx: &Cx,
        mut record: User,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<()> {
        // The handler's checked snapshot (GH #86): `record` was loaded
        // inside the framework tx and policy-checked — no re-query.
        let name = match values.get("name") {
            // Absent keys keep the stored value (GH #89): an omitted
            // optional field must not silently blank the record.
            Some(v) => v.trim().to_string(),
            None => record.name.clone(),
        };
        let email = match values.get("email") {
            Some(v) => v.trim().to_string(),
            None => record.email.clone(),
        };
        let role = match values.get("role") {
            Some(v) if v.trim() == "admin" || v.trim() == "member" => v.trim().to_string(),
            Some(_) => record.role.clone(),
            None => record.role.clone(),
        };
        let active = match values.get("active") {
            Some(v) if v.trim() == "false" => false,
            Some(v) if v.trim() == "true" => true,
            _ => record.active,
        };
        toasty::update!(record {
            name: name,
            email: email,
            role: role,
            active: active,
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        Ok(())
    }

    fn delete_record(
        cx: &Cx,
        record: User,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            // Use the model's delete via query to respect tenancy.
            Self::query(&cx)
                .filter(User::fields().id().eq(record.id))
                .delete()
                .exec(&mut *ex)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn bulk_delete_records(
        cx: &Cx,
        records: Vec<User>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            // Framework-checked records (GH #84, #86): delete each inside
            // the handler's tx — any error rolls the batch back.
            for rec in &records {
                Self::query(&cx)
                    .filter(User::fields().id().eq(rec.id))
                    .delete()
                    .exec(&mut *ex)
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

    fn navigation_label() -> String {
        "Writers".to_string()
    }

    fn query(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Author>> {
        let mut q = toasty::stmt::Query::<toasty::stmt::List<Author>>::all();
        if let Some(tid) = tenant_id(cx) {
            q = q.filter(Author::fields().tenant_id().eq(tid));
        }
        q
    }

    fn can_view_any(cx: &Cx) -> bool {
        if tenant_id(cx).is_some_and(|tid| tid == BLOCKED_TENANT) {
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

    // Tenant-scoped model (GH #87): every handler fails closed without a
    // tenant instead of leaking unscoped rows or minting nil-tenant orphans.
    fn requires_tenant() -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Author> {
        Table::r#for(cx)
            .id(|a: &Author| a.id.to_string())
            .pk(|a: &Author| a.id.to_string())
            .columns((
                TextColumn::r#for(Author::fields().name(), |a: &Author| a.name.clone())
                    .searchable()
                    .sortable(),
                TextColumn::r#for(Author::fields().email(), |a: &Author| a.email.clone())
                    .searchable(),
            ))
            .paginate(25)
            .live_search(true)
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            TextInput::r#for(Author::fields().name()),
            TextInput::r#for(Author::fields().email()).email().unique(),
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
        ex: &mut dyn toasty::Executor,
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
            let tid =
                tenant_id(&cx).expect("requires_tenant handlers always set a tenant (GH #87)");
            toasty::create!(Author {
                tenant_id: tid,
                name: name,
                email: email
            })
            .exec(&mut *ex)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    async fn update_record(
        _cx: &Cx,
        mut rec: Author,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<()> {
        // The handler's checked snapshot (GH #86) — no re-query.
        let name = match values.get("name") {
            // Absent keys keep the stored value (GH #89).
            Some(v) => v.trim().to_string(),
            None => rec.name.clone(),
        };
        let email = match values.get("email") {
            Some(v) => v.trim().to_string(),
            None => rec.email.clone(),
        };
        toasty::update!(rec {
            name: name,
            email: email
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        Ok(())
    }

    fn delete_record(
        cx: &Cx,
        rec: Author,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            Self::query(&cx)
                .filter(Author::fields().id().eq(rec.id))
                .delete()
                .exec(&mut *ex)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn bulk_delete_records(
        cx: &Cx,
        records: Vec<Author>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            // Framework-checked records (GH #84, #86) — delete inside the tx.
            for rec in &records {
                Self::query(&cx)
                    .filter(Author::fields().id().eq(rec.id))
                    .delete()
                    .exec(&mut *ex)
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

    fn navigation_label() -> String {
        "Blog Posts".to_string()
    }

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
        if tenant_id(cx).is_some_and(|tid| tid == BLOCKED_TENANT) {
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

    // Tenant-scoped model (GH #87): every handler fails closed without a
    // tenant instead of leaking unscoped rows or minting nil-tenant orphans.
    fn requires_tenant() -> bool {
        true
    }

    fn table(cx: &Cx) -> Table<Post> {
        Table::r#for(cx)
            .id(|p: &Post| p.id.to_string())
            .pk(|p: &Post| p.id.to_string())
            .columns((
                TextColumn::r#for(Post::fields().title(), |p: &Post| p.title.clone())
                    .searchable()
                    .sortable(),
                TextColumn::r#for(Post::fields().status(), |p: &Post| p.status.clone()),
                TextColumn::computed("Featured", |p: &Post| {
                    if p.featured { "Yes" } else { "No" }.to_string()
                }),
                TextColumn::computed("Author", |p: &Post| {
                    // Loud on missing includes (GH #101): a silent "-" reads
                    // as data. The list/export loaders always `include`
                    // author, so this only fires if the query changes.
                    debug_assert!(
                        !p.author.is_unloaded(),
                        "Author column needs Post::query to include author"
                    );
                    if p.author.is_unloaded() {
                        "(unloaded)".to_string()
                    } else {
                        p.author.get().name.clone()
                    }
                }),
                TextColumn::computed("Comments", |p: &Post| {
                    debug_assert!(
                        !p.comments.is_unloaded(),
                        "Comments column needs Post::query to include comments"
                    );
                    if p.comments.is_unloaded() {
                        "(unloaded)".to_string()
                    } else {
                        p.comments.get().len().to_string()
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
                // Prebuilt-expression VariantFilter (no embedded enum needed):
                // the editorial spotlight facet over the featured flag.
                VariantFilter::r#for(
                    "spotlight",
                    "Spotlight",
                    vec![
                        ("Featured".to_string(), Post::fields().featured().eq(true)),
                        ("Standard".to_string(), Post::fields().featured().eq(false)),
                    ],
                ),
            ))
            .group_by("status", |p: &Post| p.status.clone())
            .paginate(25)
            .live_search(true)
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            Section::new("Content").schema((
                TextInput::r#for(Post::fields().title()).placeholder("A title editors click"),
                // Optional so quick draft stubs submit; full stories fill it.
                TextInput::r#for(Post::fields().body())
                    .placeholder("The full story…")
                    .optional(),
            )),
            // Grouped metadata: lifecycle selects beside the author picker.
            Group::new().schema((
                Grid::new(2).schema((
                    Select::r#for(Post::fields().status())
                        .options(vec!["draft".to_string(), "published".to_string()])
                        .label("Status")
                        .optional(),
                    Select::r#for(Post::fields().featured())
                        .options_with_labels(vec![
                            ("true".to_string(), "Featured".to_string()),
                            ("false".to_string(), "Regular".to_string()),
                        ])
                        .label("Spotlight")
                        .optional(),
                )),
                Select::r#for(Post::fields().author_id())
                    .relationship::<AuthorResource>(
                        AuthorResource::query,
                        |a: &Author| a.id,
                        |a: &Author| a.name.clone(),
                    )
                    .searchable()
                    .label("Author"),
            )),
            // Media as tabs: upload and tags grouped until tab JS lands.
            Tabs::new().schema((
                FileUpload::r#for(Post::fields().image_path()),
                Repeater::new("Tags").schema(TextInput::r#for(Post::fields().tags()).label("Tag")),
            )),
        ))
    }

    fn hydrate_form_values(record: &Post) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("title".to_string(), record.title.clone());
        m.insert("body".to_string(), record.body.clone());
        m.insert("status".to_string(), record.status.clone());
        m.insert(
            "featured".to_string(),
            if record.featured { "true" } else { "false" }.to_string(),
        );
        m.insert("author_id".to_string(), record.author_id.to_string());
        m.insert("image_path".to_string(), record.image_path.clone());
        m.insert("tags".to_string(), record.tags.clone());
        m
    }

    fn create_record(
        cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
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
            let author_exists = AuthorResource::query(&cx)
                .filter(Author::fields().id().eq(author_id))
                .first()
                .exec(&mut *ex)
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
            // Optional lifecycle fields with draft defaults: older clients
            // omitting them still create a valid draft.
            let body = values
                .get("body")
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
            let status = match values.get("status").map(|s| s.trim().to_string()) {
                Some(s) if s == "draft" || s == "published" => s,
                _ => "draft".to_string(),
            };
            let featured = matches!(
                values.get("featured").map(|s| s.trim().to_string()),
                Some(s) if s == "true"
            );
            let tid =
                tenant_id(&cx).expect("requires_tenant handlers always set a tenant (GH #87)");
            toasty::create!(Post {
                tenant_id: tid,
                title: title,
                body: body,
                status: status,
                featured: featured,
                created_at: jiff::Timestamp::now(),
                image_path: image_path,
                tags: tags,
                author_id: author_id,
            })
            .exec(&mut *ex)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn update_record(
        cx: &Cx,
        mut rec: Post,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            // The handler's checked snapshot (GH #86) — no re-query.
            let title = match values.get("title") {
                // Absent keys keep the stored value (GH #89).
                Some(v) => v.trim().to_string(),
                None => rec.title.clone(),
            };
            let author_id = match values.get("author_id") {
                Some(s) => s.trim().parse::<uuid::Uuid>().map_err(|e| {
                    topcoat::Error::from(std::io::Error::other(format!("invalid author_id: {e}")))
                })?,
                None => rec.author_id,
            };
            // Symmetric FK double-check (GH #91, mirrors create): validate_async
            // already checked, but the author may be cross-tenant or deleted since.
            let author_exists = AuthorResource::query(&cx)
                .filter(Author::fields().id().eq(author_id))
                .first()
                .exec(&mut *ex)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?
                .is_some();
            if !author_exists {
                return Err(topcoat::Error::from(std::io::Error::other(
                    "author not found",
                )));
            }
            let image_path = match values.get("image_path") {
                // Absent keys keep the stored value (GH #89).
                Some(v) => v.trim().to_string(),
                None => rec.image_path.clone(),
            };
            let tags = match values.get("tags") {
                Some(v) => v.trim().to_string(),
                None => rec.tags.clone(),
            };
            let body = match values.get("body") {
                Some(v) => v.trim().to_string(),
                None => rec.body.clone(),
            };
            let status = match values.get("status") {
                Some(v) if v.trim() == "draft" || v.trim() == "published" => v.trim().to_string(),
                _ => rec.status.clone(),
            };
            let featured = match values.get("featured") {
                Some(v) if v.trim() == "true" => true,
                Some(v) if v.trim() == "false" => false,
                _ => rec.featured,
            };
            toasty::update!(rec {
                title: title,
                author_id: author_id,
                image_path: image_path,
                tags: tags,
                body: body,
                status: status,
                featured: featured
            })
            .exec(&mut *ex)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn delete_record(
        cx: &Cx,
        rec: Post,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            Self::query(&cx)
                .filter(Post::fields().id().eq(rec.id))
                .delete()
                .exec(&mut *ex)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn bulk_delete_records(
        cx: &Cx,
        records: Vec<Post>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            // Framework-checked records (GH #84, #86) — delete inside the tx.
            for rec in &records {
                Self::query(&cx)
                    .filter(Post::fields().id().eq(rec.id))
                    .delete()
                    .exec(&mut *ex)
                    .await
                    .map_err(|e| -> topcoat::Error { e.into() })?;
            }
            Ok(())
        }
    }
}

/// Discussion resource over `Comment`: the moderation queue.
///
/// Comments carry no tenant of their own — they inherit visibility from their
/// post — so the query is unscoped and `requires_tenant` stays false. Deletes
/// are hidden in the panel (`deletable() == false`): removals happen through
/// the post lifecycle, never from the queue. Server policy still allows them,
/// so the override is chrome-only.
pub struct CommentResource;

impl Resource for CommentResource {
    type Model = Comment;

    fn navigation_label() -> String {
        "Discussion".to_string()
    }

    fn deletable() -> bool {
        false
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }
    fn can_view(_cx: &Cx, _record: &Comment) -> bool {
        true
    }
    fn can_create(_cx: &Cx) -> bool {
        true
    }
    fn can_update(_cx: &Cx, _record: &Comment) -> bool {
        true
    }
    fn can_delete(_cx: &Cx, _record: &Comment) -> bool {
        true
    }

    fn query(cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Comment>> {
        let _ = cx;
        let inc_post: toasty::stmt::Include<Comment, Post> = Comment::fields().post().into();
        toasty::stmt::Query::<toasty::stmt::List<Comment>>::all().include(inc_post)
    }

    fn table(cx: &Cx) -> Table<Comment> {
        Table::r#for(cx)
            .id(|c: &Comment| c.id.to_string())
            .pk(|c: &Comment| c.id.to_string())
            .columns((
                TextColumn::r#for(Comment::fields().body(), |c: &Comment| c.body.clone())
                    .searchable()
                    .sortable(),
                TextColumn::computed("Post", |c: &Comment| {
                    debug_assert!(
                        !c.post.is_unloaded(),
                        "Post column needs Comment::query to include post"
                    );
                    if c.post.is_unloaded() {
                        "(unloaded)".to_string()
                    } else {
                        c.post.get().title.clone()
                    }
                }),
            ))
            .paginate(25)
            .live_search(true)
    }

    fn form(_cx: &Cx) -> Schema {
        Schema::new((
            TextInput::r#for(Comment::fields().body()).placeholder("Write a reply…"),
            Select::r#for(Comment::fields().post_id())
                .relationship::<PostResource>(
                    PostResource::query,
                    |p: &Post| p.id,
                    |p: &Post| p.title.clone(),
                )
                .searchable()
                .label("Post"),
        ))
    }

    fn hydrate_form_values(record: &Comment) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("body".to_string(), record.body.clone());
        m.insert("post_id".to_string(), record.post_id.to_string());
        m
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<()> {
        let body = values
            .get("body")
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string();
        let post_id = values
            .get("post_id")
            .cloned()
            .unwrap_or_default()
            .trim()
            .parse::<uuid::Uuid>()
            .map_err(|e| {
                topcoat::Error::from(std::io::Error::other(format!("invalid post_id: {e}")))
            })?;
        toasty::create!(Comment {
            body: body,
            post_id: post_id,
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        Ok(())
    }

    async fn update_record(
        _cx: &Cx,
        mut record: Comment,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<()> {
        let body = match values.get("body") {
            Some(v) => v.trim().to_string(),
            None => record.body.clone(),
        };
        let post_id = match values.get("post_id") {
            Some(s) => s.trim().parse::<uuid::Uuid>().map_err(|e| {
                topcoat::Error::from(std::io::Error::other(format!("invalid post_id: {e}")))
            })?,
            None => record.post_id,
        };
        toasty::update!(record {
            body: body,
            post_id: post_id,
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        Ok(())
    }

    fn delete_record(
        cx: &Cx,
        record: Comment,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            Self::query(&cx)
                .filter(Comment::fields().id().eq(record.id))
                .delete()
                .exec(&mut *ex)
                .await
                .map_err(|e| -> topcoat::Error { e.into() })?;
            Ok(())
        }
    }

    fn bulk_delete_records(
        cx: &Cx,
        records: Vec<Comment>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        let cx = cx.clone();
        async move {
            for rec in &records {
                Self::query(&cx)
                    .filter(Comment::fields().id().eq(rec.id))
                    .delete()
                    .exec(&mut *ex)
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
async fn admin_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
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
    let mut panel = Panel::new("admin")
        .app_context(db)
        .brand(
            Brand::new("Argentum Blog").logo(
                "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20viewBox='0%200%2024%2024'%3E%3Ccircle%20cx='12'%20cy='12'%20r='10'%20fill='%236366f1'/%3E%3Ctext%20x='12'%20y='16'%20text-anchor='middle'%20font-size='12'%20fill='white'%20font-family='sans-serif'%3EA%3C/text%3E%3C/svg%3E",
            ),
        )
        .dark_mode(true)
        .resource::<UserResource>()
        .resource::<AuthorResource>()
        .resource::<PostResource>()
        .resource::<CommentResource>()
        // Saved view outside the resource set: the published queue.
        // Query-aware active state (the shell matches paths, so a bare URL
        // could never highlight): active exactly on the published filter.
        .navigation(NavigationItem {
            label: "Published".to_string(),
            url: "/admin/posts?filters=status:published".to_string(),
            href_check: Some(std::sync::Arc::new(|cx: &Cx| {
                let uri = topcoat::router::request::uri(cx);
                uri.path() == "/admin/posts"
                    && uri
                        .query()
                        .is_some_and(|q| q.contains("status:published"))
            }) as HrefCheck),
            order: 1,
        });
    // Demo credentials stay available for local development via
    // SHOWCASE_LOGIN_HINT, but the default login page is shippable with no
    // hint. Empty values install nothing (no empty hint paragraph).
    if let Ok(hint) = std::env::var("SHOWCASE_LOGIN_HINT")
        && !hint.trim().is_empty()
    {
        panel = panel.login_hint(hint);
    }
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
