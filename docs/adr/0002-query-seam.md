# Resource::query(cx) is the single row-scoping seam

Tenancy, soft-deletes, and any row-level restriction flow through one overridable method: `Resource::query(cx) -> Query<List<Model>>`. Every Table, Form, Action, and shard loader starts from it. Tenancy enters as `cx.with(Tenant(id))` on the way in, not as a global scope appended elsewhere; there is no `withoutGlobalScope` footgun to forget. Day-1 single-panel/single-tenant, but the seam is what makes multi-tenant later without touching resource bodies.
