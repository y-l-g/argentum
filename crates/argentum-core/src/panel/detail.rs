//! The detail page (GH #187): `GET {prefix}/{slug}/{id}`, read-only.
//!
//! One record, rendered through [`Resource::view`] — the same `Schema` a form
//! uses, read the other way round. It lives beside the form handlers rather
//! than in `list.rs` because it is a record page: it loads through the same
//! `Resource::query` seam (`find_by_key`), checks the same `can_view` policy,
//! and answers the same 404 for an unknown or out-of-scope id.

use topcoat::view::internal::ThenView;
use topcoat::{
    context::Cx,
    router::{Body, error::not_found, path_param_segment},
    view::{BoxView, HoistView, ViewExt, view},
};

use super::actions::load_viewable;
use super::{enforce_auth, enforce_tenant, list_url};
use crate::db::db;
use crate::resource::Resource;

/// Detail page GET (GH #187).
///
/// A resource that declares no [`view`](Resource::view) has no detail page:
/// the handler 404s rather than rendering an empty shell, which keeps
/// "declares nothing" and "no such page" the same answer for a hand-typed URL
/// and makes the row link's absence honest.
///
/// The record loads through `R::query` (`find_by_key` — the tenancy and
/// soft-delete seam, ADR-0002), so an unknown id and an id outside the
/// request's scope get one answer, as everywhere else in the panel. `can_view`
/// on the loaded record is a 403 rather than a 404: the record exists and this
/// caller may not see it.
pub(crate) fn resource_view<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::viewed(cx) {
            return Err(not_found().into());
        }
        let id = path_param_segment(cx, "id").to_string();
        let mut db = db(cx);
        let record = load_viewable::<R>(cx, &mut db).await?;
        // Values come from the same hydration the edit form uses, so the page
        // and the form cannot disagree about what a field holds.
        let values = R::hydrate_form_values(cx, &record);
        let body = R::view(cx).render_readonly(cx, &values).await?;
        // Relations render from the record itself (GH #187): the `Schema`
        // above carries only its string projection, and the related rows are
        // already loaded by `query`'s `include`, so this adds no query.
        let relations = R::view_relations(cx, &record);
        // The title names the page and the record's key, which is what the
        // list and the URL call it (`Table::id` is the display key, `pk` the
        // record key — the URL carries the latter, GH #168).
        let title = format!("{} {id}", R::navigation_label());
        let back = list_url(cx, &R::slug());
        Ok(view! {
            cx =>
            argentum_ui::page(
                argentum_ui::page_header(
                    argentum_ui::page_title((title))
                    <a href=(back) class="text-sm text-muted-foreground underline">
                        "Back to list"
                    </a>
                )
                argentum_ui::page_content(
                    <div class="flex flex-col gap-4">
                        (body)
                        if let Some(relations) = relations {
                            (relations)
                        }
                    </div>
                )
            )
        }
        .boxed())
    })))
}
