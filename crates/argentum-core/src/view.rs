//! A single renderable component, proving the core crate renders through
//! Topcoat's `view!` pipeline. The real toolkit surface (Panel, Resource,
//! Table, Schema, Action) lands in later phases.

use topcoat::{Result, view::*};

/// A minimal heading component. Exercises Topcoat's component + view pipeline
/// from the core crate so the workspace proves a green vertical slice.
#[component]
pub async fn heading(title: &str) -> Result {
    view! { <h1 class="ac-heading">(title)</h1> }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;

    #[tokio::test]
    async fn heading_renders_through_topcoat() {
        let built = CxTestBuilder::new().build();
        let cx = &built;

        let view = view! { cx => heading(title: "Users") }.unwrap();

        let html = view.render(cx);

        assert_eq!(html, r#"<h1 class="ac-heading">Users</h1>"#);
    }
}
