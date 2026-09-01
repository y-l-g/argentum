# Feedback on tokio-rs/topcoat PR #373 — `feat!: streaming SSR and live! + emit! regions`

**PR:** https://github.com/tokio-rs/topcoat/pull/373 (`view-stream`, head `30d92c99`, 2026-09-01)
**Tested by:** branch `feedback/topcoat-pr-373` in argentum. Workspace deps `topcoat`, `topcoat-ui`, `topcoat-ui-registry` pinned to `branch = "view-stream"` in `Cargo.toml`, Cargo.lock re-pinned to head. `cargo check --workspace` run against it.
**What argentum is:** an admin/CMS framework built on topcoat (view + router + runtime + ui-registry). Heavy consumer of the component/view layer: ~164 `-> Result` view/component/sites, ~89 `child: View` parameter sites, plus layouts matching on `slot: Result`.

---

## TL;DR

The branch fetches, resolves and compiles cleanly as a dependency (topcoat, topcoat-ui and topcoat-ui-registry are all self-consistent on the branch). The streaming model (`live!`/`emit!`, `suspense`, `error_boundary`) is a big win for us — admin panels are full of exactly the "shell first, slow part later" shapes this enables. The lazy-`View` refactor is the right direction.

Two mechanical changes account for **all 198 compile errors** we hit (in `argentum-ui` alone — `argentum-core` depends on it and could not be reached):

| Errors | Cause | Fix shape |
|---|---|---|
| 109 × E0107 | `topcoat::Result` dropped its `T = View` default type parameter | mechanical: `-> Result` → `-> Result<View>` or `-> Result<impl View>` |
| 89 × E0782 | `View` is now a trait; `child: View` parameters no longer parse as types | mechanical: `child: View` → `child: Child` (+ `#[default]`) |

Both are large but low-thought churn. Our main ask: **keep the `Result` default type parameter** (see F1) — it removes a third of the breakage for no semantic gain we can find.

---

## F1 — Restore the default type parameter on `topcoat::Result` (109 of our 198 errors)

On `main`: `pub type Result<T = view::View, E = Error>`. On `view-stream`: `pub type Result<T, E = Error>`.

Every component/handler that returned bare `Result` (meaning `Result<View>`) now fails with E0107. If the rationale is that handlers should return `Result<impl View>` after this PR, note that `Result<View>` (a boxed/erased view) is still meaningful — e.g. components built dynamically, or code that hasn't moved to `impl View` yet — and keeping the default makes the migration opt-in per call site instead of a flag-day rewrite. Even a dedicated alias (`pub type ViewResult = Result<Box<dyn View>>`?) would soften this.

Concretely: dropping the default converted ~110 one-token edits into our migration. Restoring it would let PR #373 land for consumers *before* they re-type every handler.

## F2 — `child: View` → `child: Child` is mechanical, but deserves a migration note (89 errors)

The trait-ification of `View` is fine; the break is that `child: View` was a very common component signature and now fails with E0782 whose rustc suggestions (`use a generic`, `impl View`, `&dyn View`) are all wrong for component params — the right fix is `Child<'_>` with `#[default]`, which the errors don't hint at. A short "Migrating from eager views" section in `crates/topcoat/docs/view.md` (old → new for: `child: View`, `slot: Result`, `#[component(boxed)]`, bare `Result` returns) would make consumer migrations self-serve. The examples (`examples/live`, `examples/suspense`) are good references; they're how we figured the new signatures out.

## F3 — `error_boundary` replacing the `slot: Result` match is a clear improvement

The old pattern (layout matches on `Result` slot, sets status code, renders fallback) was awkward and made layouts non-lazy. `error_boundary` with a rethrowable fallback (`Err` from the fallback rethrows upstream) is strictly better and keeps the status-code concern in one place.

## F4 — `.boxed()` instead of `#[component(boxed)]`

More explicit and keeps the macro surface smaller; `ViewExt::boxed()` for the recursive-component type cycle reads fine. No complaints — just noting it's another mechanical migration site (we have a handful of recursive components).

## F5 — Streaming semantics: questions from a consumer's perspective

- **Multiple `emit!`s per region:** each emission *replaces* the previous one. We have use cases for *append* (log-style tables, progress lines). Is an append mode on the roadmap, or should consumers model it as N live regions?
- **Client disconnect mid-stream:** the PR notes `emit!` returning `Err` lets the body handle failure. Good. What is the recommended shape — abort the region silently, or propagate `Err` to end the stream?
- **Mid-stream redirect as `window.location.replace`:** sensible given the fixed status line, but no-JS clients get nothing. Could `emit!`/the region script degrade to a `<meta http-equiv="refresh">` fallback? Minor.
- **Per-request marker ids:** we assume ids are random per request (no caching pitfalls)? Worth stating explicitly in the docs since it affects whether live regions can sit behind CDNs.

## F6 — What we'd adopt immediately once migrated

- `suspense` for slow table/panel bodies behind the admin shell (fallback = skeleton).
- `live!`/`emit!` for form-action results without full page reloads and without us wiring a client library.
- Lazy views should also cut initial-byte latency on our heaviest panels (they currently await all data before returning a view).

---

## Migration inventory for argentum (for our own tracking)

- ~110 bare `-> Result` returns → `-> Result<impl View>` / `-> Result<View>` (unblocks if F1 lands).
- 89 `child: View` params → `child: Child` + `#[default]`.
- Layouts: `slot: Result` → `slot: Slot<'_>` + `(slot)` interpolation, wrapped in `error_boundary` where status codes were set.
- Recursive components: `#[component(boxed)]` → `.boxed()`.
- Follow-up (post-migration): adopt `suspense`/`error_boundary`/`live!` in panels; see ADR-0005 (stable slice) for the affected surfaces.
