//! Relationship option loading — bounded, memoized, policy-checked.
//!
//! Every relationship `Select` over one resource shares a single bounded
//! load per `(request, tenant)`; policy (`can_view_any`/`can_view` plus
//! the tenant gate) fails the load closed instead of leaking labels.

use topcoat::{Result, context::Cx};

/// Why a relationship option load produced no options (GH #108).
///
/// Distinguishes a policy denial from a structural/transient failure so
/// `validate_async` can say "not available" instead of "retry", and so the
/// render path never re-labels a value the user may not view.
///
/// `Overflow` (GH #150) is distinct from `LoadFailed`: the related table
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
    /// The related table overflows the option cap (GH #150).
    Overflow,
}

/// The boxed future a relationship loader returns.
pub(crate) type RelationshipLoadFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<(String, String)>, OptionLoadError>> + Send>,
>;

#[allow(clippy::type_complexity)]
pub(crate) type RelationshipLoader =
    std::sync::Arc<dyn Fn(&Cx) -> RelationshipLoadFuture + Send + Sync>;

/// The boxed future a relationship *search* loader returns (GH #150).
pub(crate) type RelationshipSearchFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<(String, String)>, OptionLoadError>> + Send>,
>;

#[allow(clippy::type_complexity)]
pub(crate) type RelationshipSearchLoader =
    std::sync::Arc<dyn Fn(&Cx, String) -> RelationshipSearchFuture + Send + Sync>;

/// The boxed future a targeted existence check returns (GH #150 D4).
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
    R: crate::resource::Resource + 'static,
{
    if !R::can_view_any(cx) {
        return Err(OptionLoadError::Denied);
    }
    if R::requires_tenant() && crate::tenancy::tenant_id(cx).is_none() {
        // The panel gates every handler through `enforce_tenant::<R>`;
        // option loads must not be the one tenantless path into `R::query`.
        return Err(OptionLoadError::Denied);
    }
    Ok(())
}

/// Max options a relationship `Select` will load (GH #91): the loader carries
/// `limit(Self + 1)` and fails past the cap instead of scanning a 10k-row
/// table per select per submit.
pub const MAX_RELATIONSHIP_OPTIONS: usize = 200;

/// The related model's primary key type — the identity a relationship option
/// stores (GH #108). Fully qualified because the `Model` trait is a bound of
/// `Resource::Model`, not a supertrait of `Resource`.
pub(crate) type RelatedPrimaryKey<R> =
    <<R as crate::resource::Resource>::Model as toasty::schema::Model>::PrimaryKey;

/// Option records for one related resource, memoized per request (GH #91).
///
/// Every relationship `Select` over the same `R` shares one bounded load per
/// `(request, tenant)` instead of scanning the table per select per validate
/// plus re-render scans. `tenant` is an explicit cache key: memoize tracking
/// alone cannot distinguish header-tenanted callers sharing one `Parts`, so
/// tenancy isolation never rides on scope resolution. `R::query` stays the
/// only data seam (tenancy preserved); value and label mapping stay in the
/// caller so selects with different projections share the hit.
///
/// Policy is part of the load (GH #108): `can_view_any` (and the related
/// resource's tenant gate) denies the whole load — fail closed, never an
/// empty set that validates as "invalid"; loaded rows are filtered through
/// `can_view` before any label is rendered.
///
/// The cap is checked on the **raw** bounded fetch, before `can_view`
/// filtering (GH #91): counting filtered rows would let one hidden record
/// defeat the cap and silently truncate a larger table, misreporting
/// legitimate FKs as "invalid".
#[topcoat::context::memoize(as_ref)]
pub(crate) async fn related_records<R>(
    cx: &Cx,
    _tenant: Option<uuid::Uuid>,
) -> Result<Vec<R::Model>, OptionLoadError>
where
    R: crate::resource::Resource + 'static,
    R::Model: Send + Sync + 'static,
{
    ensure_option_access::<R>(cx)?;
    let mut db = crate::db::db(cx);
    let mut records = R::query(cx)
        .limit(MAX_RELATIONSHIP_OPTIONS + 1)
        .exec(&mut db)
        .await
        .map_err(|e| {
            tracing::warn!(
                resource = R::slug(),
                error = %e,
                "relationship option load failed"
            );
            OptionLoadError::LoadFailed
        })?;
    if records.len() > MAX_RELATIONSHIP_OPTIONS {
        // Fail visibly (GH #91): validating against a silent truncation
        // would reject legitimate FKs as "invalid" while rendering a
        // misleading subset. Counted before policy filtering. Distinct
        // `Overflow` (GH #150) so searchable selects degrade to type-to-
        // search instead of a retry error.
        tracing::warn!(
            resource = R::slug(),
            max = MAX_RELATIONSHIP_OPTIONS,
            "relationship option table overflows the cap"
        );
        return Err(OptionLoadError::Overflow);
    }
    records.retain(|record| R::can_view(cx, record));
    Ok(records)
}

/// Bounded server-side option search (GH #150).
///
/// Reuses the related `Table`'s declared `searchable()` columns via
/// `R::table(cx).search_expr(q)` (D1): documented as "option search searches
/// the related resource's declared searchable columns". No option-specific
/// hook until a real caller needs it.
///
/// * Empty/blank `q` → bounded head (same cap as [`related_records`]).
/// * `q` non-empty but `search_expr` is `None` (no searchable columns) →
///   fallback to the hard-cap path (D1a): unfiltered bounded load, `Overflow`
///   when over the cap. Non-searchable selects keep today's behavior.
/// * Filtered fetch carries `limit(MAX+1)` and fails with `Overflow` past the
///   cap instead of scanning the table — one bounded round-trip per keystroke
///   burst, never the whole table.
/// * Policy mirrors the base load: `can_view_any` + tenant gate fail closed
///   (`Denied`), rows filter through `can_view` before labels.
/// * `q` is clamped to [`crate::resource::MAX_QUERY_TERM`] chars (same bound
///   as `?q=`), trimmed.
/// * One bounded round-trip per call, never the whole table; not memoized
///   (`q` is unbounded per keystroke, and the endpoint serves one field and
///   one term per request, so sharing would only grow the per-request cache).
pub(crate) async fn related_records_search<R>(
    cx: &Cx,
    q: String,
) -> Result<Vec<R::Model>, OptionLoadError>
where
    R: crate::resource::Resource + 'static,
    R::Model: Send + Sync + 'static,
{
    ensure_option_access::<R>(cx)?;
    let term = crate::resource::clamp_query_term(&q);
    let mut query = R::query(cx);
    if !term.is_empty() {
        let table = R::table(cx);
        // D1: reuse the related table's declared searchable columns. When it
        // declares none, `search_expr` is None and we fall through unfiltered
        // to the capped exec below (D1a fallback: hard-cap path, `Overflow`
        // on large tables). Non-searchable selects keep today's behavior.
        if let Some(expr) = table.search_expr(&term) {
            query = query.filter(expr);
        }
    }
    for ord in R::table(cx).order_bys() {
        query = query.order_by(ord);
    }
    let mut db = crate::db::db(cx);
    let mut records = query
        .limit(MAX_RELATIONSHIP_OPTIONS + 1)
        .exec(&mut db)
        .await
        .map_err(|e| {
            tracing::warn!(
                resource = R::slug(),
                error = %e,
                "relationship option search failed"
            );
            OptionLoadError::LoadFailed
        })?;
    if records.len() > MAX_RELATIONSHIP_OPTIONS {
        tracing::warn!(
            resource = R::slug(),
            max = MAX_RELATIONSHIP_OPTIONS,
            "relationship option search overflows the cap"
        );
        return Err(OptionLoadError::Overflow);
    }
    records.retain(|record| R::can_view(cx, record));
    Ok(records)
}

/// Outcome of the targeted existence check for overflowed selects (GH #150 D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelatedCheck {
    /// PK parses and resolves through `R::query` and passes `can_view`.
    FoundViewable,
    /// PK resolves but `can_view` denies it (maps to "invalid", never leaks).
    FoundHidden,
    /// PK does not parse or no row matches (maps to "invalid").
    NotFound,
}

/// Targeted FK existence check for overflowed sets (GH #150 D4).
///
/// Membership in the bounded set cannot validate overflowed selects (the full
/// set exceeds the cap), so validate the submitted value directly: parse via
/// `pk_eq_expr`, fetch through tenancy-scoped `R::query`, then `can_view`.
/// * `Denied` when `can_view_any` fails or tenant is missing (maps to
///   "not available").
/// * `LoadFailed` on driver failure (maps to retry).
/// * `Ok(FoundViewable/FoundHidden/NotFound)` otherwise.
/// * Single targeted round-trip per call, not memoized (one value per
///   validation; sharing would only grow the per-request cache).
pub(crate) async fn related_record_check<R>(
    cx: &Cx,
    value: String,
) -> Result<RelatedCheck, OptionLoadError>
where
    R: crate::resource::Resource + 'static,
    R::Model: Send + Sync + 'static,
{
    ensure_option_access::<R>(cx)?;
    let trimmed = value.trim();
    let Some(expr) = crate::schema::pk_eq_expr::<R::Model>(trimmed) else {
        return Ok(RelatedCheck::NotFound);
    };
    let mut db = crate::db::db(cx);
    let row = R::query(cx)
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
    use topcoat::context::{Cx, CxTestBuilder};

    use crate::schema::Select;

    use super::*;
    use topcoat::view::*;
    /// Related-resource fixtures shared by the option-policy tests (GH #108).
    #[derive(Debug, toasty::Model)]
    struct PolicyAuthor {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    fn policy_author_table(cx: &Cx) -> crate::resource::Table<PolicyAuthor> {
        crate::resource::Table::r#for(cx)
            .id(|a: &PolicyAuthor| a.id.to_string())
            .columns(crate::resource::TextColumn::r#for(
                PolicyAuthor::fields().name(),
                |a: &PolicyAuthor| a.name.clone(),
            ))
    }

    struct DenyAllAuthors;
    impl crate::resource::Resource for DenyAllAuthors {
        type Model = PolicyAuthor;
        fn can_view_any(_cx: &Cx) -> bool {
            false
        }
        fn table(cx: &Cx) -> crate::resource::Table<PolicyAuthor> {
            policy_author_table(cx)
        }
    }

    struct HideOneAuthor;
    impl crate::resource::Resource for HideOneAuthor {
        type Model = PolicyAuthor;
        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_view(_cx: &Cx, record: &PolicyAuthor) -> bool {
            record.name != "Hidden"
        }
        fn table(cx: &Cx) -> crate::resource::Table<PolicyAuthor> {
            policy_author_table(cx)
        }
    }

    struct TenantScopedAuthors;
    impl crate::resource::Resource for TenantScopedAuthors {
        type Model = PolicyAuthor;
        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_view(_cx: &Cx, _record: &PolicyAuthor) -> bool {
            true
        }
        fn requires_tenant() -> bool {
            true
        }
        fn table(cx: &Cx) -> crate::resource::Table<PolicyAuthor> {
            policy_author_table(cx)
        }
    }

    #[tokio::test]
    async fn relationship_loader_fails_past_option_cap() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct RefAuthor {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct RefAuthorResource;
        impl Resource for RefAuthorResource {
            type Model = RefAuthor;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &RefAuthor) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<RefAuthor> {
                crate::resource::Table::r#for(cx)
                    .id(|a: &RefAuthor| a.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        RefAuthor::fields().name(),
                        |a: &RefAuthor| a.name.clone(),
                    ))
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
        let select = Select::r#for(RefPost::fields().author_id())
            .relationship::<RefAuthorResource>(
                RefAuthorResource::query,
                |a: &RefAuthor| a.id,
                |a: &RefAuthor| a.name.clone(),
            );
        // Over the cap: bounded work, visible retry error — never an
        // empty-options passthrough (GH #91).
        let errs = select.validate_async(&cx, "whatever").await;
        assert!(
            errs.iter().any(|e| e.contains("could not load options")),
            "overflow must surface retry error, got {errs:?}"
        );
        // An overflowed load keeps the stored FK selectable (GH #91): a
        // failed load must not blank the relation into a required-error.
        let html = select
            .render_with(&cx, Some("stored-fk"), &[])
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
        // GH #108: `Table::id` is a display projection (GH #85) — option
        // values must come from the record's typed PK, or a non-canonical
        // table key silently stores a label in the FK column.
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct RefAuthor {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct RefAuthorResource;
        impl Resource for RefAuthorResource {
            type Model = RefAuthor;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &RefAuthor) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<RefAuthor> {
                crate::resource::Table::r#for(cx)
                    // Deliberately non-canonical: display key != PK.
                    .id(|a: &RefAuthor| format!("display:{}", a.name))
                    .columns(crate::resource::TextColumn::r#for(
                        RefAuthor::fields().name(),
                        |a: &RefAuthor| a.name.clone(),
                    ))
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
        let select = Select::r#for(RefAuthor::fields().name()).relationship::<RefAuthorResource>(
            RefAuthorResource::query,
            |a: &RefAuthor| a.id,
            |a: &RefAuthor| a.name.clone(),
        );
        // The PK validates; the display key never does.
        assert!(select.validate_async(&cx, &pk).await.is_empty());
        assert_eq!(
            select.validate_async(&cx, "display:Ada").await,
            vec!["Name is invalid".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&pk), &[])
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
            !html.contains("display:Ada"),
            "display key leaked into option values: {html}"
        );
    }

    #[tokio::test]
    async fn relationship_load_fails_closed_when_can_view_any_denies() {
        // GH #108: a related resource that denies `can_view_any` must not
        // leak labels or ids through a dependent form, and the error must be
        // "not available" — retrying cannot fix a permission decision.
        use crate::resource::Resource;

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
            DenyAllAuthors::query,
            |a: &PolicyAuthor| a.id,
            |a: &PolicyAuthor| a.name.clone(),
        );
        assert_eq!(
            select.validate_async(&cx, &pk).await,
            vec!["Id is not available".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&pk), &[])
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
        // GH #108: option loads are another path into `R::query`; a
        // tenant-scoped related resource must not serve unscoped options
        // just because the parent form is reachable without a tenant.
        use crate::resource::Resource;

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
        let select = Select::r#for(PolicyAuthor::fields().id())
            .relationship::<TenantScopedAuthors>(
                TenantScopedAuthors::query,
                |a: &PolicyAuthor| a.id,
                |a: &PolicyAuthor| a.name.clone(),
            );
        assert_eq!(
            select.validate_async(&cx, &pk).await,
            vec!["Id is not available".to_string()]
        );
        // A resolved tenant loads normally (a separate memoize key too).
        let tenanted = cx.with(crate::tenancy::Tenant(uuid::Uuid::new_v4()));
        assert!(select.validate_async(&tenanted, &pk).await.is_empty());
    }

    #[tokio::test]
    async fn relationship_load_filters_rows_by_can_view() {
        // GH #108: `can_view`-denied rows are absent from options and
        // validation — a value outside the viewable set is invalid, not
        // merely unlisted.
        use crate::resource::Resource;

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
            HideOneAuthor::query,
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
            .render_with(&cx, Some(&visible.id.to_string()), &[])
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
        use crate::resource::Resource;

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
            HideOneAuthor::query,
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
        // GH #108: `can_view` filtering happens before labels render, so a
        // row the user may not view is absent from options and does not
        // validate — and the stored value is not re-rendered on the form.
        use crate::resource::Resource;

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
            HideOneAuthor::query,
            |a: &PolicyAuthor| a.id,
            |a: &PolicyAuthor| a.name.clone(),
        );
        assert_eq!(
            select.validate_async(&cx, &pk).await,
            vec!["Id is invalid".to_string()]
        );
        let html = select
            .render_with(&cx, Some(&pk), &[])
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
        // GH #91: selects over one resource share a single bounded load per
        // (request, tenant) — validate + re-render no longer rescan.
        use crate::resource::Resource;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static OPTION_LOADS: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug, Clone, toasty::Model)]
        struct Ref {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct CountingResource;
        impl Resource for CountingResource {
            type Model = Ref;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &Ref) -> bool {
                true
            }
            fn query(_cx: &Cx) -> toasty::stmt::Query<toasty::stmt::List<Ref>> {
                OPTION_LOADS.fetch_add(1, Ordering::SeqCst);
                toasty::stmt::Query::<toasty::stmt::List<Ref>>::all()
            }
            fn table(cx: &Cx) -> crate::resource::Table<Ref> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &Ref| r.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Ref::fields().name(),
                        |r: &Ref| r.name.clone(),
                    ))
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

        // Two selects, different labels, same resource.
        let s1 = Select::r#for(Ref::fields().name()).relationship::<CountingResource>(
            CountingResource::query,
            |r: &Ref| r.id,
            |r: &Ref| r.name.clone(),
        );
        let s2 = Select::r#for(Ref::fields().name()).relationship::<CountingResource>(
            CountingResource::query,
            |r: &Ref| r.id,
            |r: &Ref| format!("{}!", r.name),
        );

        OPTION_LOADS.store(0, Ordering::SeqCst);
        assert!(s1.validate_async(&cx, &id).await.is_empty());
        assert!(s2.validate_async(&cx, &id).await.is_empty());
        let _ = s1
            .render_with(&cx, Some(&id), &[])
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
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct BigRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct BigRefResource;
        impl Resource for BigRefResource {
            type Model = BigRef;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &BigRef) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<BigRef> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &BigRef| r.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        BigRef::fields().name(),
                        |r: &BigRef| r.name.clone(),
                    ))
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
        let err = super::related_records::<BigRefResource>(&cx, tenant)
            .await
            .unwrap_err();
        assert_eq!(err, &super::OptionLoadError::Overflow);
        // Non-searchable keeps the retry message (today's behavior).
        let plain = Select::r#for(BigRef::fields().name()).relationship::<BigRefResource>(
            BigRefResource::query,
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
        // GH #150 D1: `related_records_search` reuses the related table's
        // searchable columns — a 201-row table overflows unfiltered but a
        // distinctive term returns its bounded match.
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct SearchRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct SearchRefResource;
        impl Resource for SearchRefResource {
            type Model = SearchRef;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &SearchRef) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<SearchRef> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &SearchRef| r.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(
                            SearchRef::fields().name(),
                            |r: &SearchRef| r.name.clone(),
                        )
                        .searchable(),
                    )
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
        let err = super::related_records::<SearchRefResource>(&cx, tenant)
            .await
            .unwrap_err();
        assert_eq!(err, &super::OptionLoadError::Overflow);
        // Distinctive term narrows to one.
        let rows = super::related_records_search::<SearchRefResource>(&cx, "Zebra".to_string())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Zebra Unique");
        // Empty q is the bounded head → still overflows on this table.
        let err = super::related_records_search::<SearchRefResource>(&cx, "".to_string())
            .await
            .unwrap_err();
        assert_eq!(err, super::OptionLoadError::Overflow);
        // `Select::search_options` shares the same seam.
        let select = Select::r#for(SearchRef::fields().name())
            .searchable()
            .relationship::<SearchRefResource>(
                SearchRefResource::query,
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
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct PlainRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct PlainRefResource;
        impl Resource for PlainRefResource {
            type Model = PlainRef;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &PlainRef) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<PlainRef> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &PlainRef| r.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        PlainRef::fields().name(),
                        |r: &PlainRef| r.name.clone(),
                    ))
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
        let err = super::related_records_search::<PlainRefResource>(&cx, "author-1".to_string())
            .await
            .unwrap_err();
        assert_eq!(err, super::OptionLoadError::Overflow);
    }

    #[tokio::test]
    async fn relationship_overflowed_searchable_validates_via_targeted_check() {
        // GH #150 D4: searchable selects over overflowed tables validate
        // legitimate FKs via the targeted PK check, not membership.
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct CheckRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct CheckRefResource;
        impl Resource for CheckRefResource {
            type Model = CheckRef;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, record: &CheckRef) -> bool {
                record.name != "Hidden"
            }
            fn table(cx: &Cx) -> crate::resource::Table<CheckRef> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &CheckRef| r.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(
                            CheckRef::fields().name(),
                            |r: &CheckRef| r.name.clone(),
                        )
                        .searchable(),
                    )
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
            .relationship::<CheckRefResource>(
                CheckRefResource::query,
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
        // Non-searchable over the same table keeps the retry error.
        let plain = Select::r#for(CheckRef::fields().name()).relationship::<CheckRefResource>(
            CheckRefResource::query,
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
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct HintRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct HintRefResource;
        impl Resource for HintRefResource {
            type Model = HintRef;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &HintRef) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<HintRef> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &HintRef| r.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(
                            HintRef::fields().name(),
                            |r: &HintRef| r.name.clone(),
                        )
                        .searchable(),
                    )
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
            .relationship::<HintRefResource>(
                HintRefResource::query,
                |r: &HintRef| r.id,
                |r: &HintRef| r.name.clone(),
            );
        let html = select
            .render_with(&cx, Some("stored-fk"), &[])
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
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct SmallRef {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct SmallRefResource;
        impl Resource for SmallRefResource {
            type Model = SmallRef;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_view(_cx: &Cx, _record: &SmallRef) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<SmallRef> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &SmallRef| r.id.to_string())
                    .columns(
                        crate::resource::TextColumn::r#for(
                            SmallRef::fields().name(),
                            |r: &SmallRef| r.name.clone(),
                        )
                        .searchable(),
                    )
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
            .relationship::<SmallRefResource>(
                SmallRefResource::query,
                |r: &SmallRef| r.id,
                |r: &SmallRef| r.name.clone(),
            );
        let html = select
            .render_with(&cx, None, &[])
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
