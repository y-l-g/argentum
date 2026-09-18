//! Sidebar navigation: [`NavigationItem`] plus the [`HrefCheck`] seam.
//!
//! Moved verbatim from `resource.rs` (GH #133): no behavior change.

use std::sync::Arc;

use topcoat::context::Cx;
use topcoat::router::{Href, HrefParams, HrefQueries, HrefTarget};

use super::Resource;

/// Predicate deciding whether a [`NavigationItem`] matches the request URI —
/// factored out of `NavigationItem` so the field signature stays readable.
pub type HrefCheck = Arc<dyn Fn(&Cx) -> bool + Send + Sync>;

/// Sidebar entry derived from a `Resource` (see `CONTEXT.md`).
#[derive(Default)]
pub struct NavigationItem {
    pub label: String,
    pub url: String,
    pub href_check: Option<HrefCheck>,
    /// Sort key for the sidebar (GH #102): items render in stable `order`
    /// order, so declaration order breaks ties. Resources declare in
    /// `Panel::resource` order (all default `0`); custom items interleave
    /// via [`.sorted()`](Self::sorted) — e.g. `.sorted(-1)` pins above the
    /// resources.
    pub order: i32,
}

impl Clone for NavigationItem {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            url: self.url.clone(),
            href_check: self.href_check.clone(),
            order: self.order,
        }
    }
}

impl std::fmt::Debug for NavigationItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationItem")
            .field("label", &self.label)
            .field("url", &self.url)
            .field("href_check", &self.href_check.is_some())
            .field("order", &self.order)
            .finish()
    }
}

impl PartialEq for NavigationItem {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label && self.url == other.url
    }
}
impl Eq for NavigationItem {}

impl NavigationItem {
    /// Derive a sidebar entry from a `Resource` type, using the given panel
    /// mount prefix.
    ///
    /// Label comes from [`Resource::navigation_label`] (the pluralized model
    /// name), URL from [`Resource::slug`] under the prefix — the same URL
    /// [`crate::panel::Panel::resource`] registers the list page at, so the
    /// sidebar and the router can never disagree.
    pub fn from_resource_with_prefix<R: Resource>(prefix: &str) -> Self {
        let trimmed = prefix.trim_matches('/').trim();
        let base = if trimmed.is_empty() {
            "/admin".to_string()
        } else {
            format!("/{trimmed}")
        };
        Self {
            label: R::navigation_label(),
            url: format!("{base}/{}", R::slug()),
            href_check: None,
            order: 0,
        }
    }

    /// Create a `NavigationItem` from a typed `Href` (e.g. `href!("/admin/showcase")`).
    ///
    /// The label is provided explicitly; `url` is the href's resolved URL
    /// (e.g. `"/admin/showcase"`), and `is_current` delegates to
    /// `Href::is_current` so query/encoding are handled per Topcoat `d273cb15`.
    /// For dynamic `Panel::prefix()` items use `from_resource_with_prefix`.
    pub fn from_href<T, P, Q, F>(
        label: impl Into<String>,
        href: Href<T, P, Q, F>,
        url: impl Into<String>,
    ) -> Self
    where
        T: HrefTarget + Send + Sync + 'static,
        P: HrefParams + Send + Sync + 'static,
        Q: HrefQueries + Send + Sync + 'static,
        F: std::fmt::Display + Send + Sync + 'static,
    {
        // Sidebar sections (e.g. Showcase) should stay active on their
        // sub-pages, while Href::is_current is exact (path + query). Use a
        // slash-boundary prefix check on the href's resolved path so
        // from_href items behave like is_current_path but still benefit from
        // href's encoding-aware path generation.
        let url_string: String = url.into();
        let prefix = url_string.clone();
        let check = Arc::new(move |cx: &Cx| {
            if href.is_current(cx) {
                return true;
            }
            let current = topcoat::router::request::uri(cx).path();
            if current == prefix {
                return true;
            }
            current
                .strip_prefix(prefix.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        }) as Arc<dyn Fn(&Cx) -> bool + Send + Sync>;
        Self {
            label: label.into(),
            url: url_string,
            href_check: Some(check),
            order: 0,
        }
    }

    /// Derive a sidebar entry from a `Resource` type.
    ///
    /// Shorthand for `from_resource_with_prefix::<R>("/admin")` — kept for
    /// single-panel Phase 1 call sites. New code should use
    /// `from_resource_with_prefix` or `Panel::nav_item`.
    pub fn from_resource<R: Resource>() -> Self {
        Self::from_resource_with_prefix::<R>("/admin")
    }

    /// Pin this item's sidebar position (GH #102): lower `order` renders
    /// first, ties keep declaration order.
    pub fn sorted(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Whether this item is current for the request in `cx`.
    ///
    /// If this item was created via `from_href`, delegates to `Href::is_current`
    /// (sorted decoded query + percent-encoding). Otherwise mirrors that
    /// semantics for string URLs: exact path match, or prefix match on a slash
    /// boundary — uniform for every item, resources and custom links alike
    /// (GH #39/#148: since resources mount at `{prefix}/{slug}`, no generated
    /// item points at the bare panel prefix, so the old root-exact special
    /// case is gone and the doc no longer promises one).
    pub fn is_current(&self, cx: &Cx) -> bool {
        if let Some(check) = &self.href_check {
            return check(cx);
        }
        let current = topcoat::router::request::uri(cx).path();
        self.is_current_path(current)
    }

    /// Whether this item is current for the given request path (without query):
    /// an exact match, or a prefix match on a slash boundary (so
    /// `/admin/users` is active on `/admin/users/create` but not on
    /// `/admin/userships`). Uniform for every item — since resources mount at
    /// `{prefix}/{slug}` (GH #39), no generated item points at the bare panel
    /// prefix that needed the old root-exact special case.
    ///
    /// Split from `is_current` so `Panel::render_shell` can stay testable
    /// without constructing a full `http::request::Parts` in `Cx`.
    pub fn is_current_path(&self, current_path: &str) -> bool {
        if current_path == self.url {
            return true;
        }
        current_path
            .strip_prefix(&self.url)
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty::stmt::List;
    use topcoat::context::CxTestBuilder;

    #[derive(Debug, Clone, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct UserResource;

    impl Resource for UserResource {
        type Model = User;

        fn query(_cx: &Cx) -> toasty::stmt::Query<List<User>> {
            // Custom scoping example: only users named Ada
            toasty::stmt::Query::<List<User>>::all().filter(User::fields().name().eq("Ada"))
        }
    }

    #[test]
    fn navigation_derives_label_and_url_from_model() {
        let item = NavigationItem::from_resource::<UserResource>();
        // Label: pluralized model name; URL: panel prefix + resource slug
        // (the same URL Panel::resource registers the list page at).
        assert_eq!(item.label, "Users");
        assert_eq!(item.url, "/admin/users");
    }

    #[test]
    fn slugs_follow_the_filament_convention() {
        // UserResource → strip "Resource" → pluralize → kebab-case
        assert_eq!(<UserResource as Resource>::slug(), "users");
        assert_eq!(UserResource::navigation_label(), "Users");
    }

    #[test]
    fn navigation_item_is_current_path() {
        let users = NavigationItem {
            label: "Users".to_string(),
            url: "/admin/users".to_string(),
            href_check: None,
            order: 0,
        };
        let showcase = NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
            href_check: None,
            order: 0,
        };
        // exact
        assert!(users.is_current_path("/admin/users"));
        assert!(showcase.is_current_path("/admin/showcase"));
        // slash-boundary — sub-pages active
        assert!(users.is_current_path("/admin/users/create"));
        assert!(showcase.is_current_path("/admin/showcase/table"));
        // slash-boundary — near-misses inactive
        assert!(!users.is_current_path("/admin/userships"));
        assert!(!showcase.is_current_path("/admin/showcases"));
        assert!(!showcase.is_current_path("/admin/showcase-table"));
        // unrelated
        assert!(!users.is_current_path("/other"));
        assert!(!showcase.is_current_path("/admin/users"));
    }

    #[test]
    fn navigation_item_is_current_via_cx() {
        let item = NavigationItem {
            label: "Showcase".to_string(),
            url: "/admin/showcase".to_string(),
            href_check: None,
            order: 0,
        };
        let (parts, ()) = http::Request::builder()
            .uri("/admin/showcase/table")
            .body(())
            .unwrap()
            .into_parts();
        let cx = CxTestBuilder::new().request_context(parts).build();
        assert!(item.is_current(&cx));
        let (parts2, ()) = http::Request::builder()
            .uri("/admin/showcases")
            .body(())
            .unwrap()
            .into_parts();
        let cx2 = CxTestBuilder::new().request_context(parts2).build();
        assert!(!item.is_current(&cx2));
    }
}
