//! A single renderable component, proving the core crate renders through
//! Topcoat's `view!` pipeline. The real toolkit surface (Panel, Resource,
//! Table, Schema, Action) lands in later phases.

use topcoat::{Result, view::*};

/// A minimal heading component. Exercises Topcoat's component + view pipeline
/// from the core crate so the workspace proves a green vertical slice.
#[component]
pub async fn heading(title: &str) -> Result {
    view! { <h1 class="text-2xl font-bold tracking-tight text-foreground">(title)</h1> }
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

        assert!(
            html.contains("text-foreground") && html.contains("Users"),
            "heading should contain token classes and title, got {html}"
        );
    }
}
