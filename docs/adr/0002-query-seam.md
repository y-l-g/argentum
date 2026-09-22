# Resource::query is the resource's own row-scoping seam

Date: 2026-08-19 — Status: accepted — Amended: 2026-09-22

## Decision

`Resource::query(cx) -> Query<List<Model>>` is the one overridable seam for a resource's **own** row
scoping: soft deletes, row-level visibility, and the includes a page loads. Every list, form, record
and shard loader reaches its rows through it, and tenancy enters as `cx.with(Tenant(id))` on the way
in, not as a global scope appended elsewhere — there is no `withoutGlobalScope` footgun to forget. The
tenant filter itself is not stated there: the framework owns that half.

For a resource whose `requires_tenant()` is `true`, every loader — list, edit/delete load, bulk
fetch, unique pre-check, export, and the three relationship option loaders — runs
`scoped_query::<R>(cx)`, which ANDs the tenant predicate onto whatever `query` (or `export_query`)
returned. The predicate comes from `Resource::tenant_scope(tenant)`, whose default derives
`tenant_id = <request tenant>` from the model's own schema: a field named `tenant_id` whose type is
a UUID. A resource whose rows inherit their tenant — the showcase's comments, which belong to a post
that carries one — overrides `tenant_scope` with the relation path instead, so the derived default
is a convenience, not the only shape. `scoped_query` is a free function applied *after* the
resource's own override on purpose: no override can drop the tenant half by accident, and there is
deliberately no override that removes the predicate.

The gate is inseparable from the scope. A gated resource that supplies no predicate at all — no
derivable column and no `tenant_scope` override — is a **boot** failure: `Panel::build` probes
`R::tenant_scope(uuid::Uuid::nil())` through `check_resource` and returns an `Err` naming the
resource before the router exists (the probe answers by the model's shape, not the tenant value).
The request-time error in `apply_tenant_scope` stays as well: a `tenant_scope` that answers `Some`
for the probe and `None` for a particular tenant is only invalid under that context, and app code
that calls `scoped_query` outside a panel never passes `build` at all.

A resource that must serve more than one tenant keeps `requires_tenant() = false` and scopes in
`query` by hand — the one explicit, visible way to be tenant-unscoped, stated in the resource body
where a reviewer sees it, and it gives up the 403 gate with the derived filter. App code that loads
rows outside the framework's loaders must call `scoped_query` too: `query` on a gated resource is
the tenant-unscoped base by design, so the tenant-unscoped case is visible at the call site rather
than implied by an omission.

`schema` does not depend on `resource`: the option loaders are generic over `schema::OptionSource`,
whose `scoped_query` is a **required** method, and `resource` supplies the blanket impl every
`Resource` gets. For a direct `OptionSource` implementor the rule is convention rather than
structure — nothing stops that body writing `Ok(Query::all())` beside `requires_tenant() == true`,
and the compiler will not notice. `Resource` remains the only shape whose scope the framework
applies for you; an app that wants that guarantee implements `Resource`, not `OptionSource`.
