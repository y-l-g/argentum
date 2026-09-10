//! Tenancy via `Cx` scoped value `Tenant(id)` (`cx.with(Tenant(id))`).
//!
//! `Resource::query(cx)` is the single seam for tenancy (ADR-0002). Tenant-scoped
//! resources filter `tenant_id().eq(tenant_id(cx))` for every loader.
//!
//! Security note (GH #87): the `x-tenant-id` header fallback below is a
//! **test/showcase harness only**. Real apps must set `Tenant` from
//! authenticated session middleware via `cx.with(Tenant(id))` and never trust
//! the header — anyone who learns another tenant's UUID would otherwise become
//! that tenant. `Resource::query` defaults to unscoped `Query::all()`, so a
//! forgotten override leaks; tenant-scoped resources should be reviewed for
//! the override and tenantless creates rejected instead of minting nil rows.

use topcoat::context::{Cx, try_request_context};

/// Request-scoped tenant identifier. Set via `cx.with(Tenant(id))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tenant(pub uuid::Uuid);

/// Returns the tenant id from `cx`, if present.
///
/// Checks `Tenant` request context, then `http::request::Parts` extensions (for
/// `Router::handle` tests that insert `Tenant` into the request), then the
/// `x-tenant-id` header (test/showcase harness only, GH #87 — never trust in prod).
pub fn tenant_id(cx: &Cx) -> Option<uuid::Uuid> {
    if let Some(t) = try_request_context::<Tenant>(cx) {
        return Some(t.0);
    }
    if let Some(parts) = try_request_context::<http::request::Parts>(cx) {
        if let Some(t) = parts.extensions.get::<Tenant>() {
            return Some(t.0);
        }
        if let Some(v) = parts
            .headers
            .get("x-tenant-id")
            .and_then(|h| h.to_str().ok())
            && let Ok(id) = v.parse::<uuid::Uuid>()
        {
            return Some(id);
        }
    }
    None
}

/// Requires a tenant, returning an error if missing (for tenancy-gated resources).
pub fn require_tenant(cx: &Cx) -> Result<uuid::Uuid, topcoat::Error> {
    tenant_id(cx).ok_or_else(|| topcoat::router::error::forbidden().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use topcoat::context::CxTestBuilder;

    fn cx_with_header(value: Option<&str>) -> Cx {
        let mut builder = http::Request::builder()
            .uri("/")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        if let Some(v) = value {
            builder
                .headers
                .insert("x-tenant-id", v.parse().expect("header value must parse"));
        }
        CxTestBuilder::new().request_context(builder).build()
    }

    #[test]
    fn tenant_prefers_cx_over_header_and_defaults_to_none() {
        let id = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        // Header fallback (harness only) parses.
        let cx = cx_with_header(Some(&other.to_string()));
        assert_eq!(tenant_id(&cx), Some(other));
        // Missing/invalid header → None (caller must reject tenantless creates).
        assert_eq!(tenant_id(&cx_with_header(None)), None);
        assert_eq!(tenant_id(&cx_with_header(Some("not-a-uuid"))), None);
        // Extension Tenant (Router::handle tests) wins over header.
        let cx = CxTestBuilder::new()
            .request_context({
                let mut parts = http::Request::builder()
                    .uri("/")
                    .header("x-tenant-id", other.to_string())
                    .body(())
                    .unwrap()
                    .into_parts()
                    .0;
                parts.extensions.insert(Tenant(id));
                parts
            })
            .build();
        assert_eq!(tenant_id(&cx), Some(id));
    }
}
