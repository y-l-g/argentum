//! `Panel` — the admin application shell.
//!
//! Owns the [`Router`] and the `Db` in `app_context`. See `CONTEXT.md`.

use toasty::Db;
use topcoat::router::{Router, RouterBuilderDiscoverExt};

use crate::resource::{NavigationItem, Resource};

/// The admin application.
///
/// ```ignore
/// Panel::new("admin").app_context(db).build()
/// ```
#[derive(Debug)]
pub struct Panel {
    prefix: String,
    db: Option<Db>,
}

impl Panel {
    /// Create a `Panel` mounted at `prefix` (e.g. `"admin"` → `"/admin"`).
    pub fn new(prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        let trimmed = raw.trim_matches('/').trim().to_string();
        let prefix = if trimmed.is_empty() {
            "/admin".to_string()
        } else {
            format!("/{trimmed}")
        };
        Self { prefix, db: None }
    }

    /// Returns the mount prefix, e.g. `"/admin"`.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Register the pooled `Db` on the `app_context`.
    pub fn app_context(mut self, db: Db) -> Self {
        self.db = Some(db);
        self
    }

    /// Build the [`Router`], discovering all `#[page]` / `#[layout]` / `#[shard]`
    /// items linked into the binary and installing the `Db` on the
    /// `app_context`.
    ///
    /// Panics if no `Db` was provided via [`app_context`](Self::app_context).
    ///
    /// Also validates the Tailwind stylesheet presence and emits a diagnostic
    /// when missing (see `examples/showcase/styles.css` and `build.rs`).
    pub fn build(self) -> Router {
        let db = self.db.expect("Panel::build requires a Db via app_context");
        // Diagnostic for per-app Tailwind contract — see ADR-0006.
        // The stylesheet is generated at `$OUT_DIR/tailwind.css` via
        // `argentum_ui::tailwind_build()` in the app's `build.rs`. In tests
        // or when the build hasn't run, the file is absent; we only warn in
        // non-test builds to keep `cargo test` green.
        #[cfg(not(test))]
        {
            let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
            let css = std::path::Path::new(&out_dir).join("tailwind.css");
            if !css.exists() && std::env::var("ARGENTUM_SKIP_TAILWIND_CHECK").is_err() {
                eprintln!(
                    "warning: Tailwind stylesheet not found at {css:?}. \
                    Ensure `styles.css` (with `@import \"tailwindcss\"` + tokens + `@source` for app and `argentum-ui`) \
                    and `build.rs` (`argentum_ui::tailwind_build()`) are present, and layout injects \
                    `tailwind::stylesheet!()` + `fontsource_font!(GEIST)`. See `examples/showcase` as reference (ADR-0006)."
                );
            }
        }
        Router::builder().discover().app_context(db).build()
    }

    /// Derive a [`NavigationItem`] for `R` using this panel's mount prefix.
    ///
    /// This is the panel-aware counterpart to `NavigationItem::from_resource`.
    /// The URL respects `self.prefix()` so `Panel::new("backoffice")` yields
    /// `"/backoffice"` instead of hard-coded `"/admin"`.
    pub fn navigation_item<R: Resource>(&self) -> NavigationItem {
        NavigationItem::from_resource_with_prefix::<R>(&self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty::Db;

    #[test]
    fn panel_normalizes_prefix() {
        assert_eq!(Panel::new("admin").prefix(), "/admin");
        assert_eq!(Panel::new("/admin").prefix(), "/admin");
        assert_eq!(Panel::new("admin/").prefix(), "/admin");
        assert_eq!(Panel::new("/admin/").prefix(), "/admin");
        assert_eq!(Panel::new("").prefix(), "/admin");
    }

    #[tokio::test]
    async fn panel_builds_router_with_db() {
        let db = Db::builder().connect("sqlite::memory:").await.unwrap();
        let router = Panel::new("admin").app_context(db).build();
        // Router built without panic — the real serving test lives in the
        // admin example's integration test.
        drop(router);
    }

    #[test]
    #[should_panic(expected = "Panel::build requires a Db")]
    fn panel_build_panics_without_db() {
        let _router = Panel::new("admin").build();
    }

    #[test]
    fn panel_navigation_item_respects_prefix() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
        }

        let panel = Panel::new("backoffice");
        let item = panel.navigation_item::<DummyResource>();
        assert_eq!(item.label, "Dummys");
        assert_eq!(item.url, "/backoffice");

        let default = Panel::new("admin").navigation_item::<DummyResource>();
        assert_eq!(default.url, "/admin");
        // Also verify the prefix-aware constructor normalises slashes
        let via_prefix = crate::resource::NavigationItem::from_resource_with_prefix::<DummyResource>(
            "/backoffice/",
        );
        assert_eq!(via_prefix.url, "/backoffice");
    }
}
