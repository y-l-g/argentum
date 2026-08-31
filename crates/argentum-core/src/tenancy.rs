//! Tenancy via `Cx` scoped value `Tenant(id)` (`cx.with(Tenant(id))`).
//!
//! `Resource::query(cx)` is the single seam for tenancy (ADR-0002). Tenant-scoped
//! resources filter `tenant_id().eq(tenant_id(cx))` for every loader.

use topcoat::context::{Cx, try_request_context};

/// Request-scoped tenant identifier. Set via `cx.with(Tenant(id))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tenant(pub uuid::Uuid);

/// Returns the tenant id from `cx`, if present.
///
/// Checks `Tenant` request context, then `http::request::Parts` extensions (for
/// `Router::handle` tests that insert `Tenant` into the request), then the
/// `x-tenant-id` header.
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
