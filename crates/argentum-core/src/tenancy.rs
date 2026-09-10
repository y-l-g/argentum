//! Tenancy via the `Cx` scoped value `Tenant(id)`.
//!
//! `Resource::query(cx)` is the single seam for tenancy (ADR-0002). Tenant-scoped
//! resources filter `tenant_id().eq(tenant_id(cx))` for every loader.
//!
//! The authenticated user's tenant is the production source: the auth layer
//! (ADR-0013) injects `Tenant` into the request `Cx` when the logged-in user
//! carries one. A `Tenant` request extension — server-set only, never a header
//! — takes precedence, so app middleware and `Router::handle` tests can
//! override it deliberately. The `x-tenant-id` header fallback was removed in
//! GH #131: learning another tenant's UUID no longer makes anyone that tenant.

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
/// No request header is consulted (GH #131).
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

#[cfg(test)]
mod tests {
    use super::*;
    use topcoat::context::CxTestBuilder;

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
}
