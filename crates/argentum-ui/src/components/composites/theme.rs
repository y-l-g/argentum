use topcoat::{
    Result,
    view::{StaticStr, Unescaped, component, view},
};

/// Pre-paint theme application — the blocking head script that kills the
/// dark-mode flash (shadcn `theme-provider` parity: `attribute="class"` on
/// `<html>` + pre-paint apply).
///
/// `theme.js` wires the toggle buttons and persists the choice to
/// `localStorage` + a `theme` cookie, but it runs on `DOMContentLoaded`,
/// i.e. after first paint. Render this in the `<head>` — before any body
/// content — so the `dark` class is on `<html>` before the first pixel.
/// Keeping `theme.js`'s own apply as a backstop is harmless (idempotent).
const THEME_INIT: &str = "(function(){var t=null;try{t=localStorage.getItem('theme')}catch(e){}\
if(!t){var m=document.cookie.match(/theme=([^;]+)/);if(m)t=m[1]}\
if(t==='dark')document.documentElement.classList.add('dark')})();";

/// Render in the `<head>` of every layout, before the stylesheet link.
///
/// ```ignore
/// <head>
///     theme_init_script()
///     <link rel="stylesheet" href=(tailwind::stylesheet!())>
/// </head>
/// ```
#[component]
pub async fn theme_init_script() -> Result {
    view! { <script>(Unescaped::new_unchecked(StaticStr(THEME_INIT)))</script> }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;

    #[tokio::test]
    async fn renders_blocking_head_script() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! { cx_ref => theme_init_script() }.unwrap().render(&cx);
        assert!(
            html.starts_with("<script>") && html.contains("classList.add('dark')"),
            "expected raw inline script, got {html}"
        );
    }
}
