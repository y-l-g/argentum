//! Relationship option loading — bounded, memoized, policy-checked.
//!
//! Every relationship `Select` over one source shares a single bounded
//! load per `(request, tenant)`; policy (`can_view_any`/`can_view` plus
//! the tenant gate) fails the load closed instead of leaking labels.
//!
//! The loaders are generic over [`OptionSource`] — the source surface they
//! read — rather than over `Resource`, so the dependency runs one way:
//! `resource` depends on `schema`, and the blanket impl in
//! [`crate::resource`] makes every `Resource` an option source.

use toasty::stmt::{Expr, List, OrderByExpr, Query};
use topcoat::{Result, context::Cx};

/// Everything the relationship option loaders read from the thing they load
/// options from.
///
/// The loaders ask for the **tenant-scoped** seed query, the narrowed
/// option-load seed, the policy predicates, the tenant declaration, a name for
/// the log fields, and the source's search and ordering expressions. It is a
/// *source* surface rather than a second resource trait, so `schema` names no
/// part of `Resource`: for a `Resource`, the policy and declaration methods
/// forward straight through and the two query methods compose the framework's
/// tenant-scoped query (see the blanket impl in [`crate::resource`]).
///
/// [`scoped_query`](Self::scoped_query) is **required**: a source states its own
/// scope, so a gated source cannot end up unscoped by omission. The rest have
/// the default that keeps a source declaring nothing honest — the policy
/// predicates deny, the search and ordering expressions answer `None`,
/// [`requires_tenant`](Self::requires_tenant) is `false`, and
/// [`slug`](Self::slug) falls back to the type name.
///
/// It is public because [`Select::relationship`](crate::schema::Select::relationship)'s
/// bound names it, and a `pub(crate)` trait there is a `private_bounds` warning.
/// It is not re-exported at the crate root, because its method names are
/// `Resource`'s and a glob import would collide; it is reachable as
/// `schema::OptionSource`. Every [`Resource`](crate::resource::Resource) is one
/// through the blanket impl, so a real resource keeps the tenant gate and
/// derived filter on every option load; implement it directly only for a source
/// that is not a resource, and state the scope.
pub trait OptionSource: Sized + Send + Sync + 'static {
    /// The model whose rows become options.
    type Model: toasty::schema::Model + Send + Sync + 'static;

    /// The **tenant-scoped** seed query every option load starts from.
    ///
    /// Required, and stated by every implementor, so a source cannot be
    /// unscoped by omission. A source that scopes nothing says so in the body —
    /// `Ok(Query::all())`, what a resource's default
    /// [`query`](crate::resource::Resource::query) returns — and a source whose
    /// rows are tenant-owned ANDs the tenant predicate here, which is what
    /// [`Self::requires_tenant`] declares. A `Resource` never writes this by
    /// hand: the blanket impl forwards to the framework's `scoped_query`.
    ///
    /// An `Err` is a **permanent** misdeclaration — a source that declared a
    /// tenant gate the framework cannot satisfy — reported as
    /// `OptionLoadError::Misdeclared` rather than a retryable failure.
    fn scoped_query(cx: &Cx) -> Result<Query<List<Self::Model>>>;

    /// The seed query an **option load** runs.
    ///
    /// An option load renders a value and a label per row. Both projections are
    /// opaque closures the framework cannot inspect, and the loaders read no
    /// relation of their own, so the query only has to carry the related
    /// record's own columns. The default returns [`Self::scoped_query`]
    /// unchanged — a source states its scope once — and narrowing is the
    /// blanket impl's job: for a
    /// [`Resource`](crate::resource::Resource) it forwards to the resource's
    /// needs-aware base query with an empty set, so a resource that overrides
    /// [`query_with`](crate::resource::Resource::query_with) narrows option
    /// loads too, while a resource that overrides nothing keeps its full base
    /// query.
    ///
    /// The contract this states: an option label projects the related record's
    /// own columns. A source whose option label reads a relation cannot declare
    /// that here, and a narrowed source that does so panics in `Deferred::get`.
    fn options_query(cx: &Cx) -> Result<Query<List<Self::Model>>> {
        Self::scoped_query(cx)
    }

    /// Whether the current user may see the source's records at all: `false`
    /// fails the whole option load closed, never an empty set that
    /// validates as "invalid".
    fn can_view_any(_cx: &Cx) -> bool {
        false
    }

    /// Whether the current user may view one loaded row: `false` keeps it out
    /// of the options — and out of validation — before its label renders.
    fn can_view(_cx: &Cx, _record: &Self::Model) -> bool {
        false
    }

    /// Whether the source's rows are tenant-owned: `true` fails a
    /// tenantless request closed, and is the declaration
    /// [`Self::scoped_query`] is expected to scope for.
    fn requires_tenant() -> bool {
        false
    }

    /// A short name for the loaders' log fields. A `Resource` forwards its
    /// route slug.
    fn slug() -> String {
        std::any::type_name::<Self>().to_string()
    }

    /// The related source's search predicate for `term`, or `None` when it
    /// declares no searchable column (D1). A resource answers from its
    /// declared `searchable()` columns.
    ///
    /// `None` on a non-blank term is the documented fallback: the option
    /// search runs the bounded head instead and overflows past the cap (D1a)
    /// rather than scanning the table.
    fn search_expr(_cx: &Cx, _term: &str) -> Option<Expr<bool>> {
        None
    }

    /// The related source's declared default ordering — its first sortable
    /// column, ascending, or `None`. A resource answers from its
    /// declared `sortable()` columns.
    ///
    /// The option search applies it so a narrowed result keeps the list's
    /// ordering; deliberately not a list-mode resolution, which would add a
    /// primary-key fallback this endpoint never had.
    fn order_by(_cx: &Cx) -> Option<OrderByExpr> {
        None
    }
}

/// Why a relationship option load produced no options.
///
/// Distinguishes a policy denial from a structural/transient failure so
/// `validate_async` can say "not available" instead of "retry", and so the
/// render path never re-labels a value the user may not view.
///
/// `Overflow` is distinct from `LoadFailed`: the related table
/// exceeds the option cap. A searchable `Select` degrades to "type to
/// search" instead of a retry error, while a genuine DB failure stays
/// retryable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OptionLoadError {
    /// The related resource denies `can_view_any` (or has no tenant) for
    /// this request.
    Denied,
    /// The driver failed.
    LoadFailed,
    /// The related table overflows the option cap.
    Overflow,
    /// The related resource's tenancy cannot be scoped at all: it requires a
    /// tenant, its model has no derivable `tenant_id`, and it declares no
    /// `tenant_scope`.
    ///
    /// Distinct from [`Self::LoadFailed`] because retrying cannot fix a broken
    /// declaration: the option UI must not offer a retry, and the search
    /// endpoint answers a 500 that names the misdeclaration.
    Misdeclared,
}

/// The boxed future a relationship loader returns — the bounded load and the
/// server-side *search* hand back the same shape, so they share one
/// alias.
pub(crate) type RelationshipLoadFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<(String, String)>, OptionLoadError>> + Send>,
>;

#[allow(clippy::type_complexity)]
pub(crate) type RelationshipLoader =
    std::sync::Arc<dyn Fn(&Cx) -> RelationshipLoadFuture + Send + Sync>;

#[allow(clippy::type_complexity)]
pub(crate) type RelationshipSearchLoader =
    std::sync::Arc<dyn Fn(&Cx, String) -> RelationshipLoadFuture + Send + Sync>;

/// The boxed future a targeted existence check returns (D4).
pub(crate) type RelationshipCheckFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<RelatedCheck, OptionLoadError>> + Send>,
>;

#[allow(clippy::type_complexity)]
pub(crate) type RelationshipChecker =
    std::sync::Arc<dyn Fn(&Cx, String) -> RelationshipCheckFuture + Send + Sync>;

/// Whether the option load may proceed: related `can_view_any` plus the
/// related tenant gate fail closed (`Denied`). Shared by the bounded load,
/// the server-side search, and the targeted check so tenancy isolation never
/// rides on scope resolution in one place and not the others.
fn ensure_option_access<R>(cx: &Cx) -> Result<(), OptionLoadError>
where
    R: OptionSource,
{
    if !R::can_view_any(cx) {
        return Err(OptionLoadError::Denied);
    }
    if R::requires_tenant() && crate::tenancy::tenant_id(cx).is_none() {
        // The panel gates every handler through `enforce_tenant::<R>`;
        // option loads must not be the one tenantless path into the related
        // resource's scoped query.
        return Err(OptionLoadError::Denied);
    }
    Ok(())
}

/// The relationship option loaders' seed query: [`OptionSource::options_query`]
/// with the load's own error kind.
///
/// Every loader below starts here rather than at an unscoped base so option
/// loads inherit the framework's tenant scope (`ensure_option_access` above
/// already answered the tenantless case with `Denied`) and ask for only the
/// relations an option projection reads. The one error left for this step is a
/// source the framework cannot scope at all, which is a **permanent**
/// misdeclaration and therefore
/// [`Misdeclared`](OptionLoadError::Misdeclared) rather than a retryable load
/// failure.
fn option_query<R>(cx: &Cx) -> Result<Query<List<R::Model>>, OptionLoadError>
where
    R: OptionSource,
{
    R::options_query(cx).map_err(|error| {
        tracing::error!(
            resource = R::slug(),
            error = %error,
            "relationship option load cannot scope the related resource (GH #223)"
        );
        OptionLoadError::Misdeclared
    })
}

/// Max options a relationship `Select` will load: the loader carries
/// `limit(Self + 1)` and fails past the cap instead of scanning a 10k-row
/// table per select per submit.
pub const MAX_RELATIONSHIP_OPTIONS: usize = 200;

/// The related model's primary key type — the identity a relationship option
/// stores. Fully qualified because the `Model` trait is a bound of
/// `OptionSource::Model`, not a supertrait of `OptionSource`.
pub(crate) type RelatedPrimaryKey<R> =
    <<R as OptionSource>::Model as toasty::schema::Model>::PrimaryKey;

/// Option records for one related resource, memoized per request.
///
/// Every relationship `Select` over the same `R` shares one bounded load per
/// `(request, tenant)` instead of scanning the table per select per validate
/// plus re-render scans. `tenant` is an explicit cache key: memoize tracking
/// alone cannot distinguish header-tenanted callers sharing one `Parts`, so
/// tenancy isolation never rides on scope resolution. The tenant-scoped query
/// stays the only data seam; value and label mapping stay in the
/// caller so selects with different projections share the hit.
///
/// Policy is part of the load: `can_view_any` (and the related
/// resource's tenant gate) denies the whole load — fail closed, never an
/// empty set that validates as "invalid"; loaded rows are filtered through
/// `can_view` before any label is rendered.
///
/// The cap is checked on the **raw** bounded fetch, before `can_view`
/// filtering: counting filtered rows would let one hidden record
/// defeat the cap and silently truncate a larger table, misreporting
/// legitimate FKs as "invalid".
#[topcoat::context::memoize(as_ref)]
pub(crate) async fn related_records<R>(
    cx: &Cx,
    _tenant: Option<uuid::Uuid>,
) -> Result<Vec<R::Model>, OptionLoadError>
where
    R: OptionSource,
{
    ensure_option_access::<R>(cx)?;
    bounded_options::<R>(
        cx,
        option_query::<R>(cx)?,
        "relationship option load failed",
        "relationship option table overflows the cap",
    )
    .await
}

/// The bounded option load shared by [`related_records`] and
/// [`related_records_search`]: fetch one row past the cap, map a query failure
/// to `LoadFailed`, refuse a set over the cap with `Overflow`, then drop the
/// rows the caller cannot view.
///
/// The cap is checked on the **raw** bounded fetch, before `can_view`
/// filtering: counting filtered rows would let one hidden record
/// defeat the cap and silently truncate a larger table, misreporting
/// legitimate FKs as "invalid". `failed` and `overflow` are the warning
/// messages the caller's load reports.
async fn bounded_options<R>(
    cx: &Cx,
    query: Query<List<R::Model>>,
    failed: &str,
    overflow: &str,
) -> Result<Vec<R::Model>, OptionLoadError>
where
    R: OptionSource,
{
    let mut db = crate::db::db(cx);
    let mut records = query
        .limit(MAX_RELATIONSHIP_OPTIONS + 1)
        .exec(&mut db)
        .await
        .map_err(|e| {
            tracing::warn!(
                resource = R::slug(),
                error = %e,
                "{failed}"
            );
            OptionLoadError::LoadFailed
        })?;
    if records.len() > MAX_RELATIONSHIP_OPTIONS {
        // Fail visibly: validating against a silent truncation
        // would reject legitimate FKs as "invalid" while rendering a
        // misleading subset. Counted before policy filtering. Distinct
        // `Overflow` so searchable selects degrade to type-to-
        // search instead of a retry error.
        tracing::warn!(
            resource = R::slug(),
            max = MAX_RELATIONSHIP_OPTIONS,
            "{overflow}"
        );
        return Err(OptionLoadError::Overflow);
    }
    records.retain(|record| R::can_view(cx, record));
    Ok(records)
}

/// Bounded server-side option search.
///
/// Reuses the related table's declared `searchable()` columns via
/// `R::search_expr(cx, q)` (D1): documented as "option search searches
/// the related resource's declared searchable columns". No option-specific
/// hook until a real caller needs it.
///
/// * Empty/blank `q` → bounded head (same cap as [`related_records`]).
/// * `q` non-empty but `search_expr` is `None` (no searchable columns) → fallback to the hard-cap
///   path (D1a): unfiltered bounded load, `Overflow` when over the cap. Non-searchable selects use
///   the hard-cap path.
/// * Filtered fetch carries `limit(MAX+1)` and fails with `Overflow` past the cap instead of
///   scanning the table — one bounded round-trip per keystroke burst, never the whole table.
/// * Policy mirrors the base load: `can_view_any` + tenant gate fail closed (`Denied`), rows filter
///   through `can_view` before labels.
/// * `q` is clamped to [`crate::query_term::MAX_QUERY_TERM`] chars (same bound as `?q=`), trimmed.
/// * One bounded round-trip per call, never the whole table; not memoized (`q` is unbounded per
///   keystroke, and the endpoint serves one field and one term per request, so sharing would only
///   grow the per-request cache).
pub(crate) async fn related_records_search<R>(
    cx: &Cx,
    q: String,
) -> Result<Vec<R::Model>, OptionLoadError>
where
    R: OptionSource,
{
    ensure_option_access::<R>(cx)?;
    let term = crate::query_term::clamp_query_term(&q);
    let mut query = option_query::<R>(cx)?;
    if !term.is_empty() {
        // D1: reuse the related table's declared searchable columns. When it
        // declares none, `search_expr` is None and we fall through unfiltered
        // to the capped exec below (D1a fallback: hard-cap path, `Overflow`
        // on large tables). Non-searchable selects use the hard-cap path.
        if let Some(expr) = R::search_expr(cx, &term) {
            query = query.filter(expr);
        }
    }
    // The declared default ordering only — the first sortable column, asc, or
    // nothing (GH #210 removed `order_bys()`, so this is the column-level
    // helper that replaced it). Deliberately not a list-mode resolution: the
    // option search has no table state, and the PK fallback a paginated table
    // would add is an ordering change this endpoint never had.
    if let Some(ord) = R::order_by(cx) {
        query = query.order_by(ord);
    }
    bounded_options::<R>(
        cx,
        query,
        "relationship option search failed",
        "relationship option search overflows the cap",
    )
    .await
}

/// Outcome of the targeted existence check for overflowed selects (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelatedCheck {
    /// PK parses and resolves through the tenant-scoped query and passes
    /// `can_view`.
    FoundViewable,
    /// PK resolves but `can_view` denies it (maps to "invalid", never leaks).
    FoundHidden,
    /// PK does not parse or no row matches (maps to "invalid").
    NotFound,
}

/// Targeted FK existence check for overflowed sets (D4).
///
/// Membership in the bounded set cannot validate overflowed selects (the full
/// set exceeds the cap), so validate the submitted value directly: parse via
/// `pk_eq_expr`, fetch through the tenant-scoped query, then `can_view`.
/// * `Denied` when `can_view_any` fails or tenant is missing (maps to "not available").
/// * `LoadFailed` on driver failure (maps to retry).
/// * `Ok(FoundViewable/FoundHidden/NotFound)` otherwise.
/// * Single targeted round-trip per call, not memoized (one value per validation; sharing would
///   only grow the per-request cache).
pub(crate) async fn related_record_check<R>(
    cx: &Cx,
    value: String,
) -> Result<RelatedCheck, OptionLoadError>
where
    R: OptionSource,
{
    ensure_option_access::<R>(cx)?;
    let trimmed = value.trim();
    let Some(expr) = crate::schema::pk_eq_expr::<R::Model>(trimmed) else {
        return Ok(RelatedCheck::NotFound);
    };
    let mut db = crate::db::db(cx);
    let row = option_query::<R>(cx)?
        .filter(expr)
        .first()
        .exec(&mut db)
        .await
        .map_err(|e| {
            tracing::warn!(
                resource = R::slug(),
                error = %e,
                "relationship option check failed"
            );
            OptionLoadError::LoadFailed
        })?;
    match row {
        None => Ok(RelatedCheck::NotFound),
        Some(record) => {
            if R::can_view(cx, &record) {
                Ok(RelatedCheck::FoundViewable)
            } else {
                Ok(RelatedCheck::FoundHidden)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use toasty::stmt::{List, Query};
    use topcoat::{
        context::{Cx, CxTestBuilder},
        view::*,
    };

    use super::*;
    use crate::schema::{FieldLens, Mode, Select};
    /// Related-source fixtures shared by the option-policy tests.
    ///
    /// Each one implements only the [`OptionSource`] surface its test reads —
    /// no `Resource`, no `Table`. `tenant_id` is optional so the
    /// fixtures that do not care about tenancy keep creating rows without one;
    /// `TenantScopedAuthors` needs a discoverable `tenant_id` column
    /// for the framework to derive its scope from, and the test that uses it
    /// creates its row with a tenant.
    #[derive(Debug, toasty::Model, Clone)]
    struct PolicyAuthor {
        #[key]
        #[auto]
        id: uuid::Uuid,
        tenant_id: Option<uuid::Uuid>,
        name: String,
    }

    /// The search fixtures' one declared search expression: a substring `LIKE`
    /// over the model's `name` column, which is all the loaders ask a source
    /// for. A real resource answers it from its `Table`'s `searchable()`
    /// columns; that spelling is `Table::search_expr`'s own test.
    fn name_search_expr<M: toasty::schema::Model>(
        path: FieldLens<M, String>,
        term: &str,
    ) -> Option<Expr<bool>> {
        Some(path.like_with_escape(format!("%{term}%"), '\\'))
    }

    /// Denies every request: the whole load fails closed (the trait's
    /// `can_view_any` default).
    struct DenyAllAuthors;
    impl OptionSource for DenyAllAuthors {
        type Model = PolicyAuthor;
        fn scoped_query(_cx: &Cx) -> Result<Query<List<PolicyAuthor>>> {
            Ok(Query::all())
        }
    }

    struct HideOneAuthor;
    impl OptionSource for HideOneAuthor {
        type Model = PolicyAuthor;
        fn scoped_query(_cx: &Cx) -> Result<Query<List<PolicyAuthor>>> {
            Ok(Query::all())
        }
        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_view(_cx: &Cx, record: &PolicyAuthor) -> bool {
            record.name != "Hidden"
        }
    }

    struct TenantScopedAuthors;
    impl OptionSource for TenantScopedAuthors {
        type Model = PolicyAuthor;

        /// Mirrors `Resource`'s blanket impl (`resource::scoped_query`), which
        /// `schema` cannot call without re-creating the cycle.
        fn scoped_query(cx: &Cx) -> Result<Query<List<PolicyAuthor>>> {
            let tenant = crate::tenancy::require_tenant(cx)?;
            let filter = crate::tenancy::derived_tenant_filter::<PolicyAuthor>(tenant)
                .expect("PolicyAuthor declares a tenant_id column");
            Ok(Query::all().filter(filter))
        }

        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_view(_cx: &Cx, _record: &PolicyAuthor) -> bool {
            true
        }
        fn requires_tenant() -> bool {
            true
        }
    }

    #[tokio::test]
    async fn relationship_loader_fails_past_option_cap() {
        #[derive(Debug, toasty::Model, Clone)]
        struct RefAuthor {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct RefAuthorSource;
        impl OptionSource for RefAuthorSource {
            type Model = RefAuthor;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<RefAuthor>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &RefAuthor) -> bool {
                true
            }
        }
        #[derive(Debug, toasty::Model)]
        struct RefPost {
            #[key]
            #[auto]
            id: uuid::Uuid,
            author_id: uuid::Uuid,
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(RefAuthor, RefPost))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..(MAX_RELATIONSHIP_OPTIONS + 1) {
            toasty::create!(RefAuthor {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(RefPost::fields().author_id()).relationship::<RefAuthorSource>(
            |_cx| Query::all(),
            |a: &RefAuthor| a.id,
            |a: &RefAuthor| a.name.clone(),
        );
        // Over the cap: bounded work, visible retry error — never an
        // empty-options passthrough.
        let errs = select.validate_async(&cx, "whatever").await;
        assert!(
            errs.iter().any(|e| e.contains("could not load options")),
            "overflow must surface retry error, got {errs:?}"
        );
        // An overflowed load keeps the stored FK selectable: a
        // failed load must not blank the relation into a required-error.
        let html = select
            .render_with(&cx, Some("stored-fk"), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("value=\"stored-fk\""),
            "over-cap render must keep the stored value: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_option_values_are_primary_keys_not_table_ids() {
        // `Table::id` is a display projection — option
        // values must come from the record's typed PK, or a display string
        // silently stores a label in the FK column. The source surface has no
        // table row-key projection to reach for at all, so the
        // caller's typed projection is the only option-value seam.
        #[derive(Debug, toasty::Model, Clone)]
        struct RefAuthor {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct RefAuthorSource;
        impl OptionSource for RefAuthorSource {
            type Model = RefAuthor;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<RefAuthor>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &RefAuthor) -> bool {
                true
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(RefAuthor))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(RefAuthor {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let pk = row.id.to_string();
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(RefAuthor::fields().name()).relationship::<RefAuthorSource>(
            |_cx| Query::all(),
            |a: &RefAuthor| a.id,
            |a: &RefAuthor| a.name.clone(),
        );
        // The PK validates; the label never does.
        assert!(select.validate_async(&cx, &pk).await.is_empty());
        assert_eq!(
            select.validate_async(&cx, "Ada").await,
            vec!["Name is invalid".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&pk), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains(&format!("value=\"{pk}\"")),
            "option value must be the PK, got {html}"
        );
        assert!(
            !html.contains("value=\"Ada\""),
            "the label leaked into option values: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_load_fails_closed_when_can_view_any_denies() {
        // a related source that denies `can_view_any` must not
        // leak labels or ids through a dependent form, and the error must be
        // "not available" — retrying cannot fix a permission decision.

        let mut db = toasty::Db::builder()
            .models(toasty::models!(PolicyAuthor))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(PolicyAuthor {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let pk = row.id.to_string();
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(PolicyAuthor::fields().id()).relationship::<DenyAllAuthors>(
            |_cx| Query::all(),
            |a: &PolicyAuthor| a.id,
            |a: &PolicyAuthor| a.name.clone(),
        );
        assert_eq!(
            select.validate_async(&cx, &pk).await,
            vec!["Id is not available".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&pk), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("Ada"),
            "denied labels must not render: {html}"
        );
        assert!(
            !html.contains(&pk),
            "denied values must not render either: {html}"
        );
        // The empty select must explain itself on GET (no incoming error):
        // otherwise the user sees an unrequireable field with no reason.
        assert!(
            html.contains("Id is not available"),
            "denied render must show the form-level error: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_load_denies_tenantless_requests_for_tenant_scoped_targets() {
        // option loads are another path into the related
        // resource's rows; a tenant-scoped related resource must not serve
        // unscoped options just because the parent form is reachable without a
        // tenant, and the derived filter must narrow the load to the request
        // tenant's rows.

        let mut db = toasty::Db::builder()
            .models(toasty::models!(PolicyAuthor))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let tenant = uuid::Uuid::new_v4();
        let row = toasty::create!(PolicyAuthor {
            tenant_id: Some(tenant),
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let pk = row.id.to_string();
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(PolicyAuthor::fields().id())
            .relationship::<TenantScopedAuthors>(
                |_cx| Query::all(),
                |a: &PolicyAuthor| a.id,
                |a: &PolicyAuthor| a.name.clone(),
            );
        assert_eq!(
            select.validate_async(&cx, &pk).await,
            vec!["Id is not available".to_string()]
        );
        // A resolved tenant that owns the row loads normally (a separate
        // memoize key too).
        let tenanted = cx.with(crate::tenancy::Tenant(tenant));
        assert!(select.validate_async(&tenanted, &pk).await.is_empty());
        // Another tenant's request sees nothing: the option load runs the
        // source's `scoped_query` — the fixture spells it as the framework's
        // derived tenant filter, and `Resource`'s blanket impl spells it as
        // `scoped_query::<R>` — never an unscoped base. The load itself
        // succeeds (the source is viewable), so the row is *absent from the
        // options* rather than denied — "invalid", the empty-set answer, not
        // the "not available" the gate gives.
        let foreign = cx.with(crate::tenancy::Tenant(uuid::Uuid::new_v4()));
        assert_eq!(
            select.validate_async(&foreign, &pk).await,
            vec!["Id is invalid".to_string()]
        );
    }

    #[tokio::test]
    async fn relationship_load_filters_rows_by_can_view() {
        // `can_view`-denied rows are absent from options and
        // validation — a value outside the viewable set is invalid, not
        // merely unlisted.

        let mut db = toasty::Db::builder()
            .models(toasty::models!(PolicyAuthor))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let visible = toasty::create!(PolicyAuthor {
            name: "Visible".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let hidden = toasty::create!(PolicyAuthor {
            name: "Hidden".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(PolicyAuthor::fields().id()).relationship::<HideOneAuthor>(
            |_cx| Query::all(),
            |a: &PolicyAuthor| a.id,
            |a: &PolicyAuthor| a.name.clone(),
        );
        assert!(
            select
                .validate_async(&cx, &visible.id.to_string())
                .await
                .is_empty()
        );
        assert_eq!(
            select.validate_async(&cx, &hidden.id.to_string()).await,
            vec!["Id is invalid".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&visible.id.to_string()), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(html.contains("Visible"), "viewable row must render: {html}");
        assert!(
            !html.contains("Hidden"),
            "can_view-denied row must not render: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_cap_counts_raw_rows_not_viewable_ones() {
        // GH #91 + #108: the cap is checked on the raw bounded fetch. If it
        // counted post-`can_view` rows, a single hidden record would defeat
        // it and silently truncate a larger table, misreporting viewable FKs
        // as "invalid" — the exact failure the cap exists to prevent.

        let mut db = toasty::Db::builder()
            .models(toasty::models!(PolicyAuthor))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let mut hidden_pk = String::new();
        // One past the cap, with one hidden row: the raw fetch overflows
        // even though the filtered count would fit.
        for i in 0..=MAX_RELATIONSHIP_OPTIONS {
            let name = if i == 0 {
                "Hidden".to_string()
            } else {
                format!("author-{i}")
            };
            let row = toasty::create!(PolicyAuthor { name })
                .exec(&mut db)
                .await
                .unwrap();
            if i == 0 {
                hidden_pk = row.id.to_string();
            }
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(PolicyAuthor::fields().id()).relationship::<HideOneAuthor>(
            |_cx| Query::all(),
            |a: &PolicyAuthor| a.id,
            |a: &PolicyAuthor| a.name.clone(),
        );
        // The raw fetch sees MAX+1 rows: overflow fails visibly instead of
        // rendering the 200 viewable rows as if they were the whole table.
        assert_eq!(
            select.validate_async(&cx, &hidden_pk).await,
            vec!["Id could not load options, retry".to_string()]
        );
    }

    #[tokio::test]
    async fn relationship_can_view_filtering_out_every_row_yields_invalid() {
        // `can_view` filtering happens before labels render, so a
        // row the user may not view is absent from options and does not
        // validate — and the stored value is not re-rendered on the form.

        let mut db = toasty::Db::builder()
            .models(toasty::models!(PolicyAuthor))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let hidden = toasty::create!(PolicyAuthor {
            name: "Hidden".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let pk = hidden.id.to_string();
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(PolicyAuthor::fields().id()).relationship::<HideOneAuthor>(
            |_cx| Query::all(),
            |a: &PolicyAuthor| a.id,
            |a: &PolicyAuthor| a.name.clone(),
        );
        assert_eq!(
            select.validate_async(&cx, &pk).await,
            vec!["Id is invalid".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&pk), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            !html.contains("Hidden") && !html.contains(&pk),
            "filtered-out stored value must not render: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_options_share_one_load_per_request_and_tenant() {
        // selects over one source share a single bounded load per
        // (request, tenant) — validate and re-render share the one load.
        use std::sync::atomic::{AtomicUsize, Ordering};

        static OPTION_LOADS: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug, Clone, toasty::Model)]
        struct Ref {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct CountingSource;
        impl OptionSource for CountingSource {
            type Model = Ref;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<Ref>>> {
                OPTION_LOADS.fetch_add(1, Ordering::SeqCst);
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Ref) -> bool {
                true
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(Ref))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Ref {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let id = row.id.to_string();
        let cx = CxTestBuilder::new().app_context(db).build();

        // Two selects, different labels, same source.
        let s1 = Select::r#for(Ref::fields().name()).relationship::<CountingSource>(
            |_cx| Query::all(),
            |r: &Ref| r.id,
            |r: &Ref| r.name.clone(),
        );
        let s2 = Select::r#for(Ref::fields().name()).relationship::<CountingSource>(
            |_cx| Query::all(),
            |r: &Ref| r.id,
            |r: &Ref| format!("{}!", r.name),
        );

        OPTION_LOADS.store(0, Ordering::SeqCst);
        assert!(s1.validate_async(&cx, &id).await.is_empty());
        assert!(s2.validate_async(&cx, &id).await.is_empty());
        let _ = s1
            .render_with(&cx, Some(&id), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert_eq!(
            OPTION_LOADS.load(Ordering::SeqCst),
            1,
            "two selects + re-render must share one load"
        );

        // Same cache, other tenant → separate load (no cross-tenant sharing).
        let cx_b = cx.with(crate::tenancy::Tenant(uuid::Uuid::new_v4()));
        assert!(s1.validate_async(&cx_b, &id).await.is_empty());
        assert_eq!(
            OPTION_LOADS.load(Ordering::SeqCst),
            2,
            "a second tenant must not reuse the first tenant's options"
        );
    }

    #[tokio::test]
    async fn relationship_overflow_is_distinct_from_load_failed() {
        // GH #150 D3: over-cap is `Overflow`, not `LoadFailed`, so searchable
        // selects degrade to type-to-search while DB errors stay retryable.

        #[derive(Debug, toasty::Model, Clone)]
        struct BigRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct BigRefSource;
        impl OptionSource for BigRefSource {
            type Model = BigRef;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<BigRef>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &BigRef) -> bool {
                true
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(BigRef))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..=MAX_RELATIONSHIP_OPTIONS {
            toasty::create!(BigRef {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let tenant = crate::tenancy::tenant_id(&cx);
        let err = super::related_records::<BigRefSource>(&cx, tenant)
            .await
            .unwrap_err();
        assert_eq!(err, &super::OptionLoadError::Overflow);
        // Non-searchable keeps the retry message.
        let plain = Select::r#for(BigRef::fields().name()).relationship::<BigRefSource>(
            |_cx| Query::all(),
            |r: &BigRef| r.id,
            |r: &BigRef| r.name.clone(),
        );
        assert_eq!(
            plain.validate_async(&cx, "whatever-not-a-uuid").await,
            vec!["Name could not load options, retry".to_string()]
        );
    }

    #[tokio::test]
    async fn relationship_search_narrows_past_the_cap() {
        // GH #150 D1: `related_records_search` reuses the source's declared
        // search expression — a 201-row table overflows unfiltered but a
        // distinctive term returns its bounded match.

        #[derive(Debug, toasty::Model, Clone)]
        struct SearchRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct SearchRefSource;
        impl OptionSource for SearchRefSource {
            type Model = SearchRef;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<SearchRef>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &SearchRef) -> bool {
                true
            }
            fn search_expr(_cx: &Cx, term: &str) -> Option<Expr<bool>> {
                name_search_expr(SearchRef::fields().name(), term)
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(SearchRef))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..MAX_RELATIONSHIP_OPTIONS {
            toasty::create!(SearchRef {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let unique = toasty::create!(SearchRef {
            name: "Zebra Unique".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let tenant = crate::tenancy::tenant_id(&cx);
        // Unfiltered overflows (201 rows).
        let err = super::related_records::<SearchRefSource>(&cx, tenant)
            .await
            .unwrap_err();
        assert_eq!(err, &super::OptionLoadError::Overflow);
        // Distinctive term narrows to one.
        let rows = super::related_records_search::<SearchRefSource>(&cx, "Zebra".to_string())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Zebra Unique");
        // Empty q is the bounded head → still overflows on this table.
        let err = super::related_records_search::<SearchRefSource>(&cx, "".to_string())
            .await
            .unwrap_err();
        assert_eq!(err, super::OptionLoadError::Overflow);
        // `Select::search_options` shares the same seam.
        let select = Select::r#for(SearchRef::fields().name())
            .searchable()
            .relationship::<SearchRefSource>(
                |_cx| Query::all(),
                |r: &SearchRef| r.id,
                |r: &SearchRef| r.name.clone(),
            );
        let opts = select.search_options(&cx, "Zebra").await.unwrap();
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].1, "Zebra Unique");
        assert_eq!(opts[0].0, unique.id.to_string());
    }

    #[tokio::test]
    async fn relationship_search_without_searchable_falls_back_to_cap() {
        // GH #150 D1a: no searchable columns → unfiltered bounded load, which
        // overflows large tables instead of silently truncating.

        #[derive(Debug, toasty::Model, Clone)]
        struct PlainRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        /// Declares no search expression (the trait's `None` default), which is
        /// what a resource with no `searchable()` column answers.
        struct PlainRefSource;
        impl OptionSource for PlainRefSource {
            type Model = PlainRef;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<PlainRef>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &PlainRef) -> bool {
                true
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(PlainRef))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..=MAX_RELATIONSHIP_OPTIONS {
            toasty::create!(PlainRef {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let err = super::related_records_search::<PlainRefSource>(&cx, "author-1".to_string())
            .await
            .unwrap_err();
        assert_eq!(err, super::OptionLoadError::Overflow);
    }

    #[tokio::test]
    async fn relationship_overflowed_searchable_validates_via_targeted_check() {
        // GH #150 D4: searchable selects over overflowed tables validate
        // legitimate FKs via the targeted PK check, not membership.

        #[derive(Debug, toasty::Model, Clone)]
        struct CheckRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct CheckRefSource;
        impl OptionSource for CheckRefSource {
            type Model = CheckRef;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<CheckRef>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &CheckRef) -> bool {
                record.name != "Hidden"
            }
            fn search_expr(_cx: &Cx, term: &str) -> Option<Expr<bool>> {
                name_search_expr(CheckRef::fields().name(), term)
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(CheckRef))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let mut visible_pk = String::new();
        for i in 0..=MAX_RELATIONSHIP_OPTIONS {
            let name = if i == 0 {
                "Hidden".to_string()
            } else {
                format!("author-{i}")
            };
            let row = toasty::create!(CheckRef { name })
                .exec(&mut db)
                .await
                .unwrap();
            if i == 1 {
                visible_pk = row.id.to_string();
            }
        }
        let hidden = CheckRef::filter(CheckRef::fields().name().eq("Hidden"))
            .first()
            .exec(&mut db.clone())
            .await
            .unwrap()
            .unwrap();
        let hidden_pk = hidden.id.to_string();
        let cx = CxTestBuilder::new().app_context(db).build();
        let searchable = Select::r#for(CheckRef::fields().name())
            .searchable()
            .relationship::<CheckRefSource>(
                |_cx| Query::all(),
                |r: &CheckRef| r.id,
                |r: &CheckRef| r.name.clone(),
            );
        // Legitimate FK beyond the cap passes via targeted check.
        assert!(searchable.validate_async(&cx, &visible_pk).await.is_empty());
        // Hidden row → invalid (not leaked), unknown → invalid.
        assert_eq!(
            searchable.validate_async(&cx, &hidden_pk).await,
            vec!["Name is invalid".to_string()]
        );
        assert_eq!(
            searchable
                .validate_async(&cx, &uuid::Uuid::new_v4().to_string())
                .await,
            vec!["Name is invalid".to_string()]
        );
        // Non-searchable over the same source keeps the retry error.
        let plain = Select::r#for(CheckRef::fields().name()).relationship::<CheckRefSource>(
            |_cx| Query::all(),
            |r: &CheckRef| r.id,
            |r: &CheckRef| r.name.clone(),
        );
        assert_eq!(
            plain.validate_async(&cx, &visible_pk).await,
            vec!["Name could not load options, retry".to_string()]
        );
    }

    #[tokio::test]
    async fn relationship_overflowed_searchable_renders_hint_and_keeps_value() {
        // GH #150 D6: over-cap searchable renders stored value + search input
        // + hint, with server data-attributes for the fetch.

        #[derive(Debug, toasty::Model, Clone)]
        struct HintRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct HintRefSource;
        impl OptionSource for HintRefSource {
            type Model = HintRef;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<HintRef>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &HintRef) -> bool {
                true
            }
            fn search_expr(_cx: &Cx, term: &str) -> Option<Expr<bool>> {
                name_search_expr(HintRef::fields().name(), term)
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(HintRef))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for i in 0..=MAX_RELATIONSHIP_OPTIONS {
            toasty::create!(HintRef {
                name: format!("author-{i}"),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(HintRef::fields().name())
            .searchable()
            .relationship::<HintRefSource>(
                |_cx| Query::all(),
                |r: &HintRef| r.id,
                |r: &HintRef| r.name.clone(),
            );
        let html = select
            .render_with(&cx, Some("stored-fk"), &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("value=\"stored-fk\""),
            "overflow must keep stored value: {html}"
        );
        assert!(
            html.contains("Too many options — type to search"),
            "overflow searchable must hint: {html}"
        );
        assert!(
            html.contains("data-options-server"),
            "overflow searchable must flag server fetch: {html}"
        );
        assert!(
            html.contains("data-options-field"),
            "server fetch needs the field name: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_bounded_searchable_keeps_client_filter() {
        // GH #150 + #91: bounded searchable sets narrow by label substring in
        // the browser — the server flag is overflow-only, or every small
        // table pays a debounced round-trip per keystroke.

        #[derive(Debug, toasty::Model, Clone)]
        struct SmallRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct SmallRefSource;
        impl OptionSource for SmallRefSource {
            type Model = SmallRef;
            fn scoped_query(_cx: &Cx) -> Result<Query<List<SmallRef>>> {
                Ok(Query::all())
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &SmallRef) -> bool {
                true
            }
            fn search_expr(_cx: &Cx, term: &str) -> Option<Expr<bool>> {
                name_search_expr(SmallRef::fields().name(), term)
            }
        }

        let mut db = toasty::Db::builder()
            .models(toasty::models!(SmallRef))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(SmallRef {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let select = Select::r#for(SmallRef::fields().name())
            .searchable()
            .relationship::<SmallRefSource>(
                |_cx| Query::all(),
                |r: &SmallRef| r.id,
                |r: &SmallRef| r.name.clone(),
            );
        let html = select
            .render_with(&cx, None, &[], Mode::Form)
            .await
            .unwrap()
            .single()
            .await
            .unwrap()
            .render(&cx);
        assert!(
            html.contains("data-options-filter"),
            "bounded searchable keeps the client filter input: {html}"
        );
        assert!(
            !html.contains("data-options-server"),
            "bounded searchable must not flag server fetch: {html}"
        );
        assert!(
            !html.contains("Too many options"),
            "bounded searchable must not hint: {html}"
        );
    }
}
