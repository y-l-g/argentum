//! Tenancy via the `Cx` scoped value `Tenant(id)`.
//!
//! The framework owns the tenant *filter*, not only the tenant gate. For a
//! resource whose `requires_tenant()` is `true`, every loader runs
//! [`scoped_query`](crate::resource::scoped_query) — the resource's own
//! [`query`](crate::resource::Resource::query) with `tenant_id = <tenant>`
//! ANDed onto it — and the column is discovered from the model's schema here
//! (`derived_tenant_filter`, the default body of `Resource::tenant_scope`). A
//! gated resource therefore cannot serve unscoped rows by forgetting an
//! override, and `Resource::query` stays the app's *non-tenant* scoping seam.
//!
//! Discovery is narrow and fails closed: the model must declare a field whose
//! application name is `tenant_id` and whose type is a UUID. Anything else
//! reads as "no tenant column", and a gated resource that hits that is refused
//! by `Panel::build` at boot. A predicate that is only `None` for some tenants
//! still answers an error naming the resource rather than querying unscoped.
//!
//! The authenticated user's tenant is the production source: the auth layer
//! injects `Tenant` into the request `Cx` when the logged-in user carries one.
//! A server-set `Tenant` request extension takes precedence, so app middleware
//! and `Router::handle` tests can override it deliberately. No request header
//! supplies a tenant: learning another tenant's UUID does not make anyone that
//! tenant.

use toasty::stmt::Expr;
use topcoat::context::{Cx, try_request_context};

/// Request-scoped tenant identifier.
///
/// The auth layer sets it via `cx.with(Tenant(id))` from the logged-in user;
/// server code and tests may also carry it as a request extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tenant(pub uuid::Uuid);

/// Returns the tenant id from `cx`, if present.
///
/// Checks a `Tenant` request extension first (a server-set override — the
/// deliberate seam app middleware and `Router::handle` tests use), then the
/// `Tenant` scoped value the auth layer injects from the authenticated user.
/// No request header is consulted.
pub fn tenant_id(cx: &Cx) -> Option<uuid::Uuid> {
    if let Some(parts) = try_request_context::<http::request::Parts>(cx)
        && let Some(t) = parts.extensions.get::<Tenant>()
    {
        return Some(t.0);
    }
    try_request_context::<Tenant>(cx).map(|t| t.0)
}

/// Requires a tenant, returning an error if missing (for tenancy-gated resources).
pub fn require_tenant(cx: &Cx) -> Result<uuid::Uuid, topcoat::Error> {
    tenant_id(cx).ok_or_else(|| topcoat::router::error::forbidden().into())
}

/// The application name of the column the framework scopes a gated resource by.
///
/// The convention is public — it is the contract a model signs up to when its
/// resource declares
/// [`requires_tenant`](crate::resource::Resource::requires_tenant) — and this
/// is the one place the discovery reads it.
const TENANT_FIELD: &str = "tenant_id";

/// Position of `M`'s tenant column in its own schema, or `None` when `M`
/// declares none the framework can recognize.
///
/// Found by **name and type** over [`Model::schema`]'s field list: a primitive
/// field whose application name is `tenant_id` and whose type is
/// [`toasty::stmt::Type::Uuid`]. The index is taken from the field's own
/// [`FieldId`](toasty::schema::app::FieldId) — the index
/// [`Model::path_field`] addresses — rather than from the position in the
/// vector, so the filter and the generated `tenant_id()` accessor cannot drift.
///
/// Name-based discovery is the fragile half, so a miss is `None` and every
/// caller treats it as *fail closed*: nothing falls back to the unscoped query,
/// and nothing guesses. A `tenant_id` declared as `String`, or a tenant column
/// spelled any other way, is invisible here on purpose — comparing a UUID
/// against it would be a driver-level type error at best and a cross-tenant
/// match at worst.
pub(crate) fn tenant_field_index<M: toasty::schema::Model>() -> Option<usize> {
    M::schema()
        .fields()
        .iter()
        .find(|field| {
            field.name.app.as_deref() == Some(TENANT_FIELD)
                && field
                    .ty
                    .as_primitive()
                    .is_some_and(|primitive| primitive.ty.is_uuid())
        })
        .map(|field| field.id.index)
}

/// The **derived** `tenant_id = tenant` over `M`, or `None` when
/// [`tenant_field_index`] finds no tenant column.
///
/// This is the default body of
/// [`Resource::tenant_scope`](crate::resource::Resource::tenant_scope) — the
/// public hook a resource overrides when its rows inherit their tenant instead
/// of carrying one — so the name says *derived*: it is the name-based
/// convenience, not the only way to scope a gated resource.
///
/// Built generically on purpose: `Model::path_field` addresses the column by
/// index and `Value::Uuid` types the comparison, so no generated accessor — and
/// no per-resource copy of the filter — is needed.
pub(crate) fn derived_tenant_filter<M: toasty::schema::Model>(
    tenant: uuid::Uuid,
) -> Option<Expr<bool>> {
    let index = tenant_field_index::<M>()?;
    Some(M::path_field::<uuid::Uuid>(index).eq(tenant))
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;

    fn cx_with_header(value: &str) -> Cx {
        let mut parts = http::Request::builder()
            .uri("/")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        parts.headers.insert(
            "x-tenant-id",
            value.parse().expect("header value must parse"),
        );
        CxTestBuilder::new().request_context(parts).build()
    }

    #[test]
    fn tenant_extension_wins_over_scoped_value_and_defaults_to_none() {
        let id = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();

        // The auth layer's scoped value is the production source.
        let cx = CxTestBuilder::new().request_context(Tenant(id)).build();
        assert_eq!(tenant_id(&cx), Some(id));

        // A server-set request extension overrides it deliberately.
        let cx = CxTestBuilder::new()
            .request_context({
                let mut parts = http::Request::builder()
                    .uri("/")
                    .body(())
                    .unwrap()
                    .into_parts()
                    .0;
                parts.extensions.insert(Tenant(id));
                parts
            })
            .request_context(Tenant(other))
            .build();
        assert_eq!(tenant_id(&cx), Some(id));

        // Nothing set → None (callers must reject tenantless access).
        let cx = CxTestBuilder::new().build();
        assert_eq!(tenant_id(&cx), None);
    }

    #[test]
    fn tenant_header_is_never_trusted() {
        // GH #131: the header fallback is gone; spoofing another tenant's
        // UUID in `x-tenant-id` must not resolve a tenant.
        let id = uuid::Uuid::new_v4();
        assert_eq!(tenant_id(&cx_with_header(&id.to_string())), None);
    }

    /// A model with the conventional column, one without any, and one whose
    /// same-named column is the wrong type.
    #[derive(Debug, Clone, toasty::Model)]
    struct Scoped {
        #[key]
        #[auto]
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
        name: String,
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct Unscoped {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct WronglyTyped {
        #[key]
        #[auto]
        id: uuid::Uuid,
        tenant_id: String,
    }

    #[test]
    fn tenant_column_is_discovered_by_name_and_uuid_type() {
        // `id` is index 0, `tenant_id` index 1, `name` index 2.
        assert_eq!(tenant_field_index::<Scoped>(), Some(1));
        // No column at all, and a `tenant_id` that is not a UUID: both are
        // "cannot scope", never "scope by something else".
        assert_eq!(tenant_field_index::<Unscoped>(), None);
        assert_eq!(tenant_field_index::<WronglyTyped>(), None);
    }

    /// The derived filter is only real if it reaches SQL: the discovered index
    /// and the UUID comparison must narrow a live query.
    #[tokio::test]
    async fn derived_tenant_filter_scopes_a_live_query() {
        let mut db = toasty::Db::builder()
            .models(toasty::models!(Scoped))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let mine = uuid::Uuid::new_v4();
        let theirs = uuid::Uuid::new_v4();
        toasty::create!(Scoped {
            tenant_id: mine,
            name: "Mine"
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Scoped {
            tenant_id: theirs,
            name: "Theirs"
        })
        .exec(&mut db)
        .await
        .unwrap();

        let filter = derived_tenant_filter::<Scoped>(mine).expect("Scoped declares tenant_id");
        let rows = toasty::stmt::Query::<toasty::stmt::List<Scoped>>::all()
            .filter(filter)
            .exec(&mut db)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Mine");

        // No discoverable column → no filter → the caller must fail rather than
        // run the query.
        assert!(derived_tenant_filter::<Unscoped>(mine).is_none());
    }
}
