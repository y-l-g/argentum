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
#[derive(Debug, Clone)]
pub(crate) enum OptionLoadError {
    /// The related resource denies `can_view_any` (or has no tenant) for
    /// this request.
    Denied,
    /// The driver failed, or the related table overflows the option cap.
    LoadFailed,
}

/// The boxed future a relationship loader returns.
pub(crate) type RelationshipLoadFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<(String, String)>, OptionLoadError>> + Send>,
>;

#[allow(clippy::type_complexity)]
pub(crate) type RelationshipLoader =
    std::sync::Arc<dyn Fn(&Cx) -> RelationshipLoadFuture + Send + Sync>;

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
    if !R::can_view_any(cx) {
        return Err(OptionLoadError::Denied);
    }
    if R::requires_tenant() && crate::tenancy::tenant_id(cx).is_none() {
        // The panel gates every handler through `enforce_tenant::<R>`;
        // option loads must not be the one tenantless path into `R::query`.
        return Err(OptionLoadError::Denied);
    }
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
        // misleading subset. Counted before policy filtering.
        tracing::warn!(
            resource = R::slug(),
            max = MAX_RELATIONSHIP_OPTIONS,
            "relationship option table overflows the cap"
        );
        return Err(OptionLoadError::LoadFailed);
    }
    records.retain(|record| R::can_view(cx, record));
    Ok(records)
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
}
