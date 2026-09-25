use std::{collections::HashMap, path::PathBuf};

use argentum_core::{
    Brand, ColumnWidth, Committed, DateFilter, FileUpload, Grid, Group, IncludeNeeds, Panel,
    RelationColumn, RelationColumns, Repeater, Resource, Schema, Section, Select, SelectFilter,
    Table, Tabs, TernaryFilter, TextColumn, TextInput, Textarea, Uploader, VariantFilter,
    read_embedded, render_relation, require_tenant, scoped_query, submitted, tenant_id,
    write_embedded,
};
use toasty::Db;
use topcoat::{
    Result,
    asset::AssetBundle,
    context::Cx,
    font::{Font, fontsource::fontsource_font},
    router::{Router, Slot, layout},
    tailwind,
    view::{View, ViewExt, view},
};

use crate::models::{
    Author, BLOCKED_TENANT, Comment, Media, Post, PostStats, Publication, REMOVED_COMMENT_BODY,
    Seo, User,
};

/// The theme's sans font, pulled from Fontsource and self-hosted as a Topcoat
/// asset.
///
/// `pub(crate)` so the public blog's layout links the same font as the admin
/// shell (GH #247): one document contract, one typeface.
pub(crate) const GEIST: Font = fontsource_font!(GEIST, host: Asset);

// ---------------------------------------------------------------------------
// Resource — single Model → Resource, see CONTEXT.md
// ---------------------------------------------------------------------------

/// Admin resource for `User`.
///
/// A hand-written `Resource` impl: this one declares a custom `Table`, and the
/// hooks are the declaration, so there is nothing for a derive to fill in.
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
    fn can_update(_cx: &Cx, record: &User) -> bool {
        // Row-level rule: Ken's account is SSO-managed outside the panel,
        // so the panel never writes it (reads still flow).
        record.name != "Ken Thompson"
    }
    fn can_delete(_cx: &Cx, record: &User) -> bool {
        // Same SSO guard on the delete path: per-row Policy proven over HTTP.
        record.name != "Ken Thompson"
    }

    // GH #226: chrome is opt-in. The flags are declared next to the predicates
    // above that honour them — `can_view` + `can_update` for the Edit link,
    // `can_delete` for the row and bulk Delete.
    fn editable() -> bool {
        true
    }
    fn deletable() -> bool {
        true
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
        // One section, one card: a single-child `Tabs` wrapper would be a
        // layout container with nothing to lay out (GH #239). `Tabs` earns its
        // place on `posts/create`, where it groups the upload and tags blocks.
        Schema::new(
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
        )
    }

    fn hydrate_form_values(_cx: &Cx, record: &User) -> HashMap<String, String> {
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

    /// Wake the live feed after a committed write, so an open page re-reads the
    /// users it shows.
    async fn after_commit(_cx: &Cx, _committed: Committed<User>) -> Result<()> {
        crate::live::notify();
        Ok(())
    }

    async fn create_record(
        _cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<User> {
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
        // The created row goes back to the framework: it is what
        // `after_commit` names for this write (GH #112).
        toasty::create!(User {
            name: name,
            email: email,
            role: role,
            active: active,
            created_at: jiff::Timestamp::now(),
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })
    }

    async fn update_record(
        _cx: &Cx,
        mut record: User,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<User> {
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
        // The updated row goes back to the framework (GH #112): it is what
        // `after_commit` names, and it is already the committed state.
        toasty::update!(record {
            name: name,
            email: email,
            role: role,
            active: active,
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        // The instance update reloads `record` from the database's returned
        // values, so this is the committed row — what `after_commit` names
        // (GH #112).
        Ok(record)
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

    // No `query` override (GH #223): the framework ANDs `tenant_id = <tenant>`
    // onto the default query for a `requires_tenant` resource, derived from
    // `Author`'s own schema, so the filter cannot be forgotten here.
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

    // GH #226: chrome is opt-in, declared beside the predicates above.
    fn editable() -> bool {
        true
    }
    fn deletable() -> bool {
        true
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

    fn hydrate_form_values(_cx: &Cx, record: &Author) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("name".to_string(), record.name.clone());
        m.insert("email".to_string(), record.email.clone());
        m
    }

    fn create_record(
        cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<Author>> + Send
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
            // `requires_tenant` makes the handler answer 403 before this runs
            // (GH #87), so `require_tenant` never panics on a tenantless submit
            // and a nil-tenant orphan stays impossible (GH #223).
            let tid = require_tenant(&cx)?;
            // The created row goes back to the framework (GH #112).
            toasty::create!(Author {
                tenant_id: tid,
                name: name,
                email: email
            })
            .exec(&mut *ex)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })
        }
    }

    async fn update_record(
        _cx: &Cx,
        mut rec: Author,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<Author> {
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
        // The updated row goes back to the framework (GH #112).
        toasty::update!(rec {
            name: name,
            email: email
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        // The committed row, reloaded by the instance update (GH #112).
        Ok(rec)
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

impl PostResource {
    /// The posts base query with the two relations the table can render loaded
    /// only when `needs` asks (GH #177).
    ///
    /// No tenant filter (GH #223): `requires_tenant` is `true`, so the
    /// framework scopes every loader — list, edit, delete, bulk, export — by
    /// ANDing the filter it derives from `Post`'s own `tenant_id` column onto
    /// whatever this returns. Writing it by hand here was the GH #87 hole: one
    /// override that forgot the filter served every tenant's rows.
    ///
    /// `query` is the list/detail half and loads both — the Comments column
    /// renders the count and the detail page reads `view_relations` — while
    /// [`query_with`](Resource::query_with) narrows to the includes a loader
    /// declared (GH #298), which is what the list and the export pass. Both go
    /// through this one function so the includes cannot drift apart.
    /// It takes no `Cx` because there is nothing left to resolve from the
    /// request: the scope belongs to the framework now.
    fn base(needs: &IncludeNeeds) -> toasty::stmt::Query<toasty::stmt::List<Post>> {
        let mut q = toasty::stmt::Query::<toasty::stmt::List<Post>>::all();
        if needs.wants("author") {
            let inc_author: toasty::stmt::Include<Post, Author> = Post::fields().author().into();
            q = q.include(inc_author);
        }
        if needs.wants("comments") {
            let inc_comments: toasty::stmt::Include<Post, toasty::stmt::List<Comment>> =
                Post::fields().comments().into();
            q = q.include(inc_comments);
        }
        q
    }
}

impl Resource for PostResource {
    type Model = Post;

    fn navigation_label() -> String {
        "Blog Posts".to_string()
    }

    fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Post>> {
        Self::base(&IncludeNeeds::from(["author", "comments"]))
    }

    /// The list and export load the includes their columns declared
    /// (GH #177, GH #298); the edit, delete, bulk and option loaders ask for
    /// none, so a Comment form's post options carry the posts' own columns and
    /// not every post's comments. The export inherits this branch through its
    /// default [`export_query`](Resource::export_query).
    fn query_with(_cx: &Cx, needs: &IncludeNeeds) -> toasty::stmt::Query<toasty::stmt::List<Post>> {
        Self::base(needs)
    }

    /// The post's title, so the detail heading names the post rather than its
    /// record key (GH #241).
    fn record_label(_cx: &Cx, record: &Post) -> Option<String> {
        Some(record.title.clone())
    }

    /// One post, read-only (GH #187). Each entry binds the same storage name
    /// the form posts — flattened embedded columns included — so a field means
    /// the same thing on both pages. The *list* of fields is still written
    /// twice: the schema seam has no way to derive one declaration from the
    /// other, and a page that shows a subset is the normal case.
    ///
    /// What is absent, and why: the **author key** (`Uuid`), because binding a
    /// non-`String` lens is the GH #192 seam; and the **comments**, which are a
    /// relation and so render through [`Self::view_relations`] below rather
    /// than as a field here. The shared publication timestamp *is* bound, which
    /// is why `models.rs` declares it `String` (GH #185).
    fn view(cx: &Cx) -> Schema {
        Schema::new((
            Section::new("Post").schema((
                TextInput::r#for(Post::fields().title()),
                Textarea::r#for(Post::fields().body()).rows(6),
            )),
            Section::new("Details").schema(
                Group::new().schema((
                    Grid::new(2).schema((
                        Select::r#for(Post::fields().status())
                            .options(vec!["draft".to_string(), "published".to_string()])
                            .label("Status"),
                        Select::r#for(Post::fields().featured())
                            .options_with_labels(vec![
                                ("true".to_string(), "Featured".to_string()),
                                ("false".to_string(), "Regular".to_string()),
                            ])
                            .label("Featured"),
                    )),
                    TextInput::r#for(Post::fields().image_path()).label("Cover image"),
                    TextInput::r#for(Post::fields().tags()).label("Tags"),
                )),
            ),
            Section::new("SEO").schema((
                TextInput::r#for_context(cx, Post::fields().seo().title()),
                Textarea::r#for_context(cx, Post::fields().seo().description()).rows(3),
            )),
            Section::new("Publication").schema(
                Textarea::r#for_context(
                    cx,
                    Post::fields().publication().published().published_at(),
                )
                .label("Published at")
                .optional(),
            ),
        ))
    }

    /// The post's comments, from the rows `query` already included (GH #187).
    ///
    /// `record.comments.get()` reads the included relation — no query, no
    /// per-row load — which is the point #66's criterion made. `is_unloaded` is
    /// the guard the list columns use: drop the include from `query` and this
    /// says so instead of panicking inside `Deferred::get`, so
    /// `detail_relation_check` fails on a message rather than a stack trace.
    ///
    /// The table names `CommentResource`, so the related resource's `can_view`
    /// decides which loaded comments render (GH #296).
    fn view_relations<'a>(cx: &'a Cx, record: &Post) -> Option<topcoat::view::BoxView<'a>> {
        if record.comments.is_unloaded() {
            return Some(
                view! {
                    cx =>
                    <p class="text-sm text-destructive">
                        "Comments were not loaded by this query — add them to Resource::query's include."
                    </p>
                }
                .boxed(),
            );
        }
        let columns = RelationColumns::columns((
            RelationColumn::computed("Comment", |c: &Comment| c.body.clone()),
            RelationColumn::computed("Post", |c: &Comment| c.post_id.to_string()),
        ));
        // `CommentResource` is named at the relation, so the table applies the
        // related resource's `can_view` and the policy cannot drift from the
        // comments queue's (GH #296).
        Some(render_relation::<CommentResource>(
            cx,
            "Comments",
            columns,
            record.comments.get(),
        ))
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

    // GH #226: chrome is opt-in, declared beside the predicates above.
    fn editable() -> bool {
        true
    }
    fn deletable() -> bool {
        true
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
                // GH #240: a status is narrow by content, not by kind — `r#for`
                // binds a `String` field, which the framework cannot tell from
                // a title. Featured and Comments below keep the `computed`
                // default (narrow).
                TextColumn::r#for(Post::fields().status(), |p: &Post| p.status.clone())
                    .width(ColumnWidth::Narrow),
                TextColumn::computed("Featured", |p: &Post| {
                    if p.featured { "Yes" } else { "No" }.to_string()
                }),
                // The other override direction: a computed column that holds a
                // name is body text, so it takes a share of the free width.
                TextColumn::computed("Author", |p: &Post| {
                    // Loud on missing includes (GH #101): a silent "-" reads
                    // as data. The list/export loaders include author when
                    // this column declares it (GH #177), so this only fires
                    // if the declaration and the query disagree.
                    debug_assert!(
                        !p.author.is_unloaded(),
                        "Author column needs Post::query to include author"
                    );
                    if p.author.is_unloaded() {
                        "(unloaded)".to_string()
                    } else {
                        p.author.get().name.clone()
                    }
                })
                .width(ColumnWidth::Wide)
                .needs(["author"]),
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
                })
                .needs(["comments"]),
            ))
            .filters((
                SelectFilter::r#for(
                    Post::fields().status(),
                    vec!["draft".into(), "published".into()],
                ),
                TernaryFilter::r#for(Post::fields().featured()),
                DateFilter::r#for(Post::fields().created_at()),
                // Prebuilt-expression VariantFilter (no embedded enum needed):
                // the editorial facet, where each option pairs the featured
                // flag with the lifecycle status — `Promoted` is featured and
                // published, `Backlog` is a draft that is not featured.
                VariantFilter::r#for(
                    "promoted",
                    "Promoted",
                    vec![
                        (
                            "Promoted".to_string(),
                            Post::fields()
                                .featured()
                                .eq(true)
                                .and(Post::fields().status().eq("published".to_string())),
                        ),
                        (
                            "Backlog".to_string(),
                            Post::fields()
                                .featured()
                                .eq(false)
                                .and(Post::fields().status().eq("draft".to_string())),
                        ),
                    ],
                ),
            ))
            .group_by("status", |p: &Post| p.status.clone())
            .paginate(25)
            .live_search(true)
    }

    fn form(cx: &Cx) -> Schema {
        Schema::new((
            Section::new("Content").schema((
                TextInput::r#for(Post::fields().title()).placeholder("A title editors click"),
                // Prose, so a textarea rather than a one-line input (GH #184).
                // Optional so quick draft stubs submit; full stories fill it.
                Textarea::r#for(Post::fields().body())
                    .placeholder("The full story…")
                    .rows(6)
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
                        .label("Featured")
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
            // Upload and tags as tabs: grouped until tab JS lands.
            Tabs::new().schema((
                FileUpload::r#for(Post::fields().image_path()).label("Cover image"),
                Repeater::new("Tags").schema(TextInput::r#for(Post::fields().tags()).label("Tag")),
            )),
            // Embedded **values** (GH #191). One declaration per value: the
            // controls, their flattened names, and the enum's discriminant all
            // come from the app schema and the type's own shape — nothing here
            // spells `seo_title`, and no variant is recovered from which
            // payload columns happen to be filled in.
            Group::new().schema((
                Section::new("SEO").schema(
                    Seo::form(cx, Post::fields().seo())
                        .extend(PostStats::form(cx, Post::fields().post_stats())),
                ),
                Section::new("Publication")
                    .schema(Publication::form(cx, Post::fields().publication())),
                Section::new("Attachment").schema(Media::form(cx, Post::fields().media())),
            )),
        ))
    }

    fn hydrate_form_values(cx: &Cx, record: &Post) -> HashMap<String, String> {
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
        // Embedded values (GH #191): each writes the columns the app schema
        // resolves for it — the flattened leaves, the enum's discriminant, and
        // the active variant's payload. No column name is spelled here, and no
        // "which payload is non-empty" decision is made: the stored variant is
        // what the form carries back.
        write_embedded(cx, Post::fields().seo(), &record.seo, &mut m);
        write_embedded(
            cx,
            Post::fields().publication(),
            &record.publication,
            &mut m,
        );
        write_embedded(cx, Post::fields().media(), &record.media, &mut m);
        write_embedded(cx, Post::fields().post_stats(), &record.post_stats, &mut m);
        m
    }
    fn create_record(
        cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<Post>> + Send
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
            // Verify the author exists *in this tenant*: `scoped_query` is the
            // framework's tenancy-scoped entry point (GH #223) — plain
            // `AuthorResource::query` is the tenant-unscoped base now that the
            // framework applies the tenant filter at every loader.
            let author_exists = scoped_query::<AuthorResource>(&cx)?
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
            // The same fail-closed check as the author create above: the 403 is
            // already answered (GH #87), so a nil-tenant post cannot be minted
            // (GH #223).
            let tid = require_tenant(&cx)?;
            // Embedded values (GH #191): the codec reads each one back from the
            // submission, choosing an enum's variant from the discriminant the
            // form posted rather than from which payloads are non-empty.
            let seo = read_embedded(&cx, Post::fields().seo(), &values);
            let publication = read_embedded(&cx, Post::fields().publication(), &values);
            let media = read_embedded(&cx, Post::fields().media(), &values);
            let post_stats = read_embedded(&cx, Post::fields().post_stats(), &values);
            // The created row goes back to the framework (GH #112).
            toasty::create!(Post {
                tenant_id: tid,
                title: title,
                body: body,
                status: status,
                featured: featured,
                created_at: jiff::Timestamp::now(),
                image_path: image_path,
                tags: tags,
                seo: seo,
                publication: publication,
                media: media,
                post_stats: post_stats,
                author_id: author_id,
            })
            .exec(&mut *ex)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })
        }
    }

    fn update_record(
        cx: &Cx,
        mut rec: Post,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> impl std::future::Future<Output = Result<Post>> + Send
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
            // already checked, but the author may be cross-tenant or deleted
            // since — so the check runs through the tenant-scoped query
            // (GH #223), exactly as the create above does.
            let author_exists = scoped_query::<AuthorResource>(&cx)?
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
            // Embedded values (GH #191): an absent value keeps the stored one,
            // exactly like the scalar fields above (GH #89) — the submit may
            // omit a section the form did not render. "Absent" is decided by
            // the keys the app schema resolves, not by a name spelled here.
            let seo = if submitted(&cx, Post::fields().seo(), &values) {
                read_embedded(&cx, Post::fields().seo(), &values)
            } else {
                rec.seo.clone()
            };
            let publication = if submitted(&cx, Post::fields().publication(), &values) {
                read_embedded(&cx, Post::fields().publication(), &values)
            } else {
                rec.publication.clone()
            };
            let media = if submitted(&cx, Post::fields().media(), &values) {
                read_embedded(&cx, Post::fields().media(), &values)
            } else {
                rec.media.clone()
            };
            let post_stats = if submitted(&cx, Post::fields().post_stats(), &values) {
                read_embedded(&cx, Post::fields().post_stats(), &values)
            } else {
                rec.post_stats.clone()
            };
            toasty::update!(rec {
                title: title,
                author_id: author_id,
                image_path: image_path,
                tags: tags,
                body: body,
                status: status,
                featured: featured,
                seo: seo,
                publication: publication,
                media: media,
                post_stats: post_stats
            })
            .exec(&mut *ex)
            .await
            .map_err(|e| -> topcoat::Error { e.into() })?;
            // The instance update reloads `rec`, so this is the committed row
            // (GH #112).
            Ok(rec)
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

/// Comments resource over `Comment`: the moderation queue.
///
/// Comments carry no tenant of their own — they inherit visibility from their
/// post (GH #169) — so the request tenant is required like any other gated
/// resource, and the scope is the parent post's tenant, declared in
/// [`tenant_scope`](Resource::tenant_scope) (GH #223).
///
/// That declaration is what the framework's default cannot supply: the default
/// derives the filter from a `tenant_id` column on the model, and `Comment` has
/// none. Leaving `requires_tenant` false and writing the filter inside `query`
/// was the GH #223 review's leak — a tenantless request silently *skipped* the
/// filter instead of being refused, and with `can_view_any`/`can_view` true the
/// caller could read and moderate every tenant's comments. Gated + declared is
/// the shape that fails closed: no tenant is a 403 everywhere, and the
/// predicate is the framework's to apply.
///
/// The queue moderates: row and bulk delete are enabled (GH #184), which is
/// what `can_delete`, `delete_record` and `bulk_delete_records` were already
/// written for. A resource that wants a read-only queue overrides
/// [`Resource::deletable`] to `false` instead (GH #96).
pub struct CommentResource;

/// Re-resolve a comment's parent post through the tenant-scoped
/// [`scoped_query::<PostResource>`] inside the caller's open transaction
/// (GH #178, GH #223).
///
/// `Schema::validate_async` / `Select::validate_async` already reject a
/// `post_id` outside the tenant-scoped option set before the tx opens, but that
/// is a pre-write check in a different window: a policy or tenant change between
/// the two would slip through, and a direct `create_record` / `update_record`
/// caller never ran it at all. Mirroring `PostResource`'s author double-check,
/// this is the defense-in-depth half — one query on the seam that owns tenancy.
///
/// The miss is a 404, not the 500 `PostResource` uses for a missing author: a
/// parent in another tenant is an authorization boundary, and "wrong tenant
/// looks exactly like unknown id" is this panel's contract everywhere else
/// (GH #86/#169).
async fn ensure_post_in_tenant(
    cx: &Cx,
    post_id: uuid::Uuid,
    ex: &mut dyn toasty::Executor,
) -> Result<()> {
    let in_tenant = scoped_query::<PostResource>(cx)?
        .filter(Post::fields().id().eq(post_id))
        .first()
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?
        .is_some();
    if !in_tenant {
        return Err(topcoat::router::error::not_found().into());
    }
    Ok(())
}

impl CommentResource {
    /// The comments base query with the post loaded only when `needs` asks
    /// (GH #177, GH #298).
    ///
    /// No tenant filter (GH #223): the scope is declared once, in
    /// [`tenant_scope`](Resource::tenant_scope), and the framework ANDs it onto
    /// whatever this returns — for the list, the edit load, the bulk fetch, the
    /// export and the relationship option loads alike.
    ///
    /// The list half loads the post — the Post column renders its title — and
    /// the detail page, were one declared, would read it through
    /// [`view_relations`](Resource::view_relations). Every other loader asks
    /// for nothing: the edit page, the delete paths and the option loads read
    /// only the comment's own columns.
    fn base(needs: &IncludeNeeds) -> toasty::stmt::Query<toasty::stmt::List<Comment>> {
        let mut q = toasty::stmt::Query::<toasty::stmt::List<Comment>>::all();
        if needs.wants("post") {
            let inc_post: toasty::stmt::Include<Comment, Post> = Comment::fields().post().into();
            q = q.include(inc_post);
        }
        q
    }
}

impl Resource for CommentResource {
    type Model = Comment;

    fn navigation_label() -> String {
        // "Comments", not "Discussion" (GH #184): the entity is a comment, the
        // route and model say so, and a discussion — if it means anything here
        // — would be the set of comments on one post, which is not a record the
        // panel can list or moderate.
        "Comments".to_string()
    }

    /// Tenant-scoped from the parent post, and gated like every other
    /// tenant-owned resource (GH #169, GH #223).
    ///
    /// `true` is what makes a tenantless request a 403 here instead of a read
    /// that quietly dropped the filter; the predicate below replaces the
    /// framework's name-based derivation, which finds no `tenant_id` on
    /// `Comment`.
    fn requires_tenant() -> bool {
        true
    }

    /// Inherit-through-the-relation (GH #169): scope through the parent post's
    /// tenant. Toasty rewrites the relation-path comparison into a foreign-key
    /// subquery, and the framework ANDs the result onto `query`/`export_query`
    /// exactly as it ANDs the derived `tenant_id` filter elsewhere.
    fn tenant_scope(tenant: uuid::Uuid) -> Option<toasty::stmt::Expr<bool>> {
        Some(Comment::fields().post().tenant_id().eq(tenant))
    }

    fn can_view_any(_cx: &Cx) -> bool {
        true
    }
    /// A removed comment keeps its row as a placeholder (GH #296): there is no
    /// content to read, so this refuses it and the post's relation omits it.
    /// The framework withholds the queue's row actions from a record this
    /// refuses (GH #235), which is the same answer — a placeholder has nothing
    /// to edit or delete.
    fn can_view(_cx: &Cx, record: &Comment) -> bool {
        record.body != REMOVED_COMMENT_BODY
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

    // GH #226: chrome is opt-in. The moderation queue wants both, and the
    // predicates above answer for every row (GH #184).
    fn editable() -> bool {
        true
    }
    fn deletable() -> bool {
        true
    }

    fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Comment>> {
        Self::base(&IncludeNeeds::from(["post"]))
    }

    /// The list and export load the includes their columns declared
    /// (GH #177, GH #298) — here the Post column's `post` — while the edit,
    /// delete, bulk and option loaders ask for none.
    fn query_with(
        _cx: &Cx,
        needs: &IncludeNeeds,
    ) -> toasty::stmt::Query<toasty::stmt::List<Comment>> {
        Self::base(needs)
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
                })
                // GH #240: a post title is body text, not the narrow badge a
                // computed column defaults to.
                .width(ColumnWidth::Wide)
                .needs(["post"]),
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

    fn hydrate_form_values(_cx: &Cx, record: &Comment) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("body".to_string(), record.body.clone());
        m.insert("post_id".to_string(), record.post_id.to_string());
        m
    }

    async fn create_record(
        cx: &Cx,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<Comment> {
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
        // Tenancy double-check inside the tx (GH #178): the pre-tx option-set
        // validation is not a write-time guarantee.
        ensure_post_in_tenant(cx, post_id, ex).await?;
        // The created row goes back to the framework (GH #112).
        toasty::create!(Comment {
            body: body,
            post_id: post_id,
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })
    }

    async fn update_record(
        cx: &Cx,
        mut record: Comment,
        values: HashMap<String, String>,
        ex: &mut dyn toasty::Executor,
    ) -> Result<Comment> {
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
        // An update can re-point the comment at another post (GH #178), which
        // is exactly the move the pre-tx check cannot be trusted to catch.
        ensure_post_in_tenant(cx, post_id, ex).await?;
        toasty::update!(record {
            body: body,
            post_id: post_id,
        })
        .exec(&mut *ex)
        .await
        .map_err(|e| -> topcoat::Error { e.into() })?;
        // The committed row, reloaded by the instance update (GH #112).
        Ok(record)
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

#[layout("/admin")]
async fn admin_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    Panel::layout_shell(cx, slot).await
}

pub fn router(db: Db) -> Router {
    build_router(db, Some(load_assets()), Some(upload_dir()))
}

/// Build the showcase router without filesystem assets for markup tests.
///
/// This is deliberately separate from [`router`]: the application path fails
/// loudly when its generated bundle is missing, while tests can exercise the
/// server-rendered markup without pretending an asset bundle exists.
///
/// It installs **no uploader** either, which pins the framework's default: a
/// `FileUpload` with no store keeps the sanitized client filename (GH #188).
/// A test that wants the demo store uses [`router_with_uploads`].
pub fn router_for_tests(db: Db) -> Router {
    build_router(db, None, None)
}

/// Build the showcase router with uploads enabled against `dir` (GH #188).
///
/// Assets are left out, like [`router_for_tests`]: the upload tests assert on
/// markup and on the served bytes, not on the stylesheet. Used by the upload
/// tests, which need a directory of their own — the application's is shared
/// state on disk.
pub fn router_with_uploads(db: Db, dir: impl Into<PathBuf>) -> Router {
    build_router(db, None, Some(dir.into()))
}

/// Build the showcase router with uploads at the directory the application
/// itself uses (GH #248).
///
/// [`router_with_uploads`] takes a directory so the framework's upload tests
/// can own theirs; this one is the configuration the app runs with, which is
/// what the media library's page writes through — the panel's `serve_dir` mount
/// and the store are two ends of one directory.
pub fn router_with_app_uploads(db: Db) -> Router {
    build_router(db, None, Some(upload_dir()))
}

/// Where the showcase writes uploaded bytes: `SHOWCASE_UPLOAD_DIR`, or
/// `target/showcase-uploads` so a local run works with no configuration.
pub(crate) fn upload_dir() -> PathBuf {
    std::env::var_os("SHOWCASE_UPLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/showcase-uploads"))
}

/// The URL prefix uploads are served under.
///
/// It matches the `serve_dir` route below, and the app owns both ends: the
/// store decides the path it returns, so the framework never has to guess a
/// URL convention (GH #188).
pub const UPLOAD_URL_PREFIX: &str = "/uploads";

/// The showcase's own uploader: write the bytes into the served directory and
/// return the URL they are served at (GH #188).
///
/// A demo, not a framework default — the trait is the seam and drivers are the
/// app's business. The UUID prefix keeps two uploads of `cover.png` apart, and
/// the framework hands this store a name already sanitized to a basename
/// (GH #90). Writing the file is this app's job; swapping in an object store
/// means replacing this type and nothing else.
///
/// The media library's page builds this store too (GH #248): it parses its own
/// multipart body, so the sanitized name is not the framework's to guarantee
/// there — the store applies [`basename`] itself rather than trusting every
/// caller to have done it.
pub(crate) struct DirUploader {
    dir: PathBuf,
}

impl DirUploader {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Uploader for DirUploader {
    async fn store(&self, filename: &str, bytes: &[u8]) -> Result<String, String> {
        let name = format!("{}-{}", uuid::Uuid::new_v4(), basename(filename));
        // Failure reasons are rendered to the user, so they say what the user
        // can act on and never leak the path that failed.
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|_| "the upload directory is not writable".to_string())?;
        tokio::fs::write(self.dir.join(&name), bytes)
            .await
            .map_err(|_| "the upload could not be written".to_string())?;
        // The caller stores this string and renders it verbatim as the file's
        // URL (GH #242), so it has to be one: the segment is percent-encoded,
        // or `cover #1.png` would be served as `cover ` plus a fragment, and a
        // `%22` the browser sent in the filename would decode to a quote the
        // file on disk does not carry.
        Ok(format!("{UPLOAD_URL_PREFIX}/{}", url_segment(&name)))
    }
}

/// The longest client filename the showcase keeps, in bytes.
///
/// 255 bytes is the common per-component limit on Linux filesystems, and the
/// store's `{uuid}-` prefix takes 37 of them, so the basename is capped to what
/// is left. The media library records the name it stores, so the row's
/// `filename` and the file on disk cannot disagree.
const MAX_BASENAME_BYTES: usize = 218;

/// Reduce a client-supplied filename to the basename this app stores and shows
/// (GH #90).
///
/// Strips directory components (`../../etc/passwd` → `passwd`), drops control
/// characters, trims the ends, and caps the byte length preserving the tail, so
/// the extension survives. The framework sanitizes the names its own form
/// parser hands an [`Uploader`]; the media library's page parses its own
/// multipart body (GH #248), and this is the one rule both callers apply.
pub(crate) fn basename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = clean.trim();
    if trimmed.len() <= MAX_BASENAME_BYTES {
        return trimmed.to_string();
    }
    // Walk the cut point forward to a char boundary: slicing a multibyte
    // character would panic on an attacker-controlled filename (GH #90).
    let mut start = trimmed.len() - MAX_BASENAME_BYTES;
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    trimmed[start..].to_string()
}

/// `name` as one URL path segment: everything outside the unreserved set is
/// percent-encoded (RFC 3986 §2.3).
///
/// The store's contract is that the string it returns is fetchable verbatim
/// (GH #242), and a client filename is arbitrary: a space must not become the
/// end of the URL, a `#` must not start a fragment, and a `%` must not decode
/// to something else.
fn url_segment(name: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

fn build_router(db: Db, bundle: Option<AssetBundle>, uploads: Option<PathBuf>) -> Router {
    let mut panel = Panel::new("admin")
        .app_context(db)
        .brand(
            Brand::new("Argentum Blog").logo(
                "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20viewBox='0%200%2024%2024'%3E%3Ccircle%20cx='12'%20cy='12'%20r='10'%20fill='%236366f1'/%3E%3Ctext%20x='12'%20y='16'%20text-anchor='middle'%20font-size='12'%20fill='white'%20font-family='sans-serif'%3EA%3C/text%3E%3C/svg%3E",
            ),
        )
        // Light by default (GH #184): the header toggle is the only thing that
        // turns dark on. `Panel::dark_mode` stays available for an app that
        // wants a dark-first panel.
        .resource::<UserResource>()
        .resource::<AuthorResource>()
        .resource::<PostResource>()
        .resource::<CommentResource>();
    // No "Published" saved-view entry (GH #184): it would point at
    // `/admin/posts?filters=status:published`, i.e. the Blog Posts table with a
    // filter — the same page twice in the sidebar, and the one arrangement the
    // shell's path matching highlights twice at once.
    // Demo credentials stay available for local development via
    // SHOWCASE_LOGIN_HINT, but the default login page is shippable with no
    // hint. Empty values install nothing (no empty hint paragraph).
    if let Ok(hint) = std::env::var("SHOWCASE_LOGIN_HINT")
        && !hint.trim().is_empty()
    {
        panel = panel.login_hint(hint);
    }
    if let Some(dir) = uploads {
        // Both ends of the demo (GH #188): the store writes into `dir` and
        // returns `{UPLOAD_URL_PREFIX}/…`, and the panel serves exactly that
        // prefix from the same directory — which is why the stored path is
        // fetchable without the framework inventing a URL convention.
        panel = panel
            .serve_dir(format!("{UPLOAD_URL_PREFIX}/{{*file}}"), dir.clone())
            .uploads(DirUploader::new(dir));
    }
    match bundle {
        Some(bundle) => panel
            .assets(bundle)
            .shell_assets(tailwind::stylesheet!(), GEIST)
            .build()
            .expect("showcase panel builds"),
        None => panel.build().expect("showcase panel builds"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_filename_is_reduced_to_a_basename() {
        assert_eq!(basename("../../etc/passwd"), "passwd");
        assert_eq!(basename("/abs/path/cover.png"), "cover.png");
        assert_eq!(basename("C:\\fakepath\\cover.png"), "cover.png");
        assert_eq!(basename("  cover.png  "), "cover.png");
        assert_eq!(basename("cover\u{7}.png"), "cover.png");
        assert_eq!(basename("   "), "");
        // Capped by bytes, keeping the tail so the extension survives.
        let long = format!("{}{}", "a".repeat(300), ".png");
        let capped = basename(&long);
        assert_eq!(capped.len(), MAX_BASENAME_BYTES);
        assert!(capped.ends_with(".png"), "got {capped}");
    }

    #[test]
    fn a_multibyte_filename_caps_on_a_char_boundary() {
        let long = format!("{}.png", "é".repeat(300));
        let capped = basename(&long);
        assert!(capped.len() <= MAX_BASENAME_BYTES);
        assert!(capped.ends_with(".png"), "got {capped}");
        assert!(
            capped
                .chars()
                .all(|c| c == 'é' || c == '.' || c == 'p' || c == 'n' || c == 'g')
        );
    }

    #[test]
    fn a_stored_name_becomes_a_url_path_segment() {
        assert_eq!(url_segment("cover.png"), "cover.png");
        assert_eq!(url_segment("cover #1.png"), "cover%20%231.png");
        assert_eq!(
            url_segment("quote%22 onerror=%22boom.png"),
            "quote%2522%20onerror%3D%2522boom.png"
        );
        assert_eq!(url_segment("100%.png"), "100%25.png");
        assert_eq!(url_segment("a+b&c.png"), "a%2Bb%26c.png");
        // Nothing a browser sends reaches the URL as a delimiter.
        for name in ["#", "?", "\"", " ", "%", "&", "+", "/", "\\"] {
            let encoded = url_segment(name);
            assert!(
                encoded
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"%.-_~".contains(&b)),
                "{name:?} encoded to {encoded}"
            );
        }
    }
}
