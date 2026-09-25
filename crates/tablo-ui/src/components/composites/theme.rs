use std::fmt::Write as _;

use topcoat::{
    Result,
    view::{View, component, view},
};

/// Pre-paint theme application — the blocking head script that reconciles the
/// `dark` class on `<html>` before the first pixel (shadcn `theme-provider`
/// parity: `attribute="class"` + pre-paint apply).
///
/// `theme.js` wires the toggle buttons and persists the choice to
/// `localStorage` + a `theme` cookie, but it runs on `DOMContentLoaded`, i.e.
/// after first paint. Render this in the `<head>` — before any body content —
/// so the class is right before the first pixel.
///
/// The stored preference is **authoritative in both directions**: a
/// stored `light` *removes* the server-rendered `dark` class rather than
/// leaving it in place.
///
/// The script deliberately avoids `&&`: topcoat has no DOM-safe inline-script
/// primitive, so the body goes through the normal HTML escape path, which
/// rewrites `&&` to `&amp;&amp;` — and character references are **not** decoded
/// inside `<script>` (HTML's script-data state), so the emitted script would
/// be a syntax error. Nested `if`s keep the body free of escapable characters.
fn theme_init_script_body(default_dark: bool) -> String {
    let mut script = String::from(
        "(function(){var t=null;try{t=localStorage.getItem('theme')}catch(e){}\
if(!t){var m=document.cookie.match(/theme=([^;]+)/);if(m)t=m[1]}\
if(t!=='light'){if(t!=='dark')t=",
    );
    let _ = write!(script, "'{}'", if default_dark { "dark" } else { "light" });
    script.push_str(
        ";}if(t==='dark')document.documentElement.classList.add('dark');\
else document.documentElement.classList.remove('dark')})();",
    );
    script
}

/// Render in the `<head>` of every layout, before the stylesheet link.
///
/// `default_dark` is the server's build-time preference (`Panel::dark_mode`),
/// used only when the visitor has no stored choice.
///
/// The body goes through the normal escaping path rather than `Unescaped`:
/// topcoat has no DOM-safe script primitive, and the body is ours — a fixed
/// ASCII program with one `true`/`false` interpolated — so escaping is
/// harmless as long as the program stays free of characters HTML would
/// rewrite (see `theme_init_script_body`).
///
/// ```ignore
/// <head>
///     theme_init_script(default_dark)
///     <link rel="stylesheet" href=(tailwind::stylesheet!())>
/// </head>
/// ```
#[component]
pub async fn theme_init_script(default_dark: bool) -> Result<impl View> {
    let body = theme_init_script_body(default_dark);
    Ok(view! { <script>(body)</script> })
}

#[cfg(test)]
mod tests {
    use topcoat::{context::CxTestBuilder, view::ViewExt};

    use super::*;

    async fn rendered(default_dark: bool) -> String {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        view! { cx_ref => theme_init_script(default_dark: default_dark) }
            .single()
            .await
            .unwrap()
            .render(&cx)
    }

    #[tokio::test]
    async fn renders_blocking_head_script() {
        let html = rendered(false).await;
        assert!(
            html.starts_with("<script>"),
            "expected an inline script, got {html}"
        );
        assert!(
            html.contains("classList.add") && html.contains("classList.remove"),
            "the pre-paint script must reconcile the class both ways (GH #184), got {html}"
        );
    }

    /// The inline script must survive HTML escaping intact: character
    /// references are not decoded inside `<script>`, so an escaped `&&` would
    /// ship a broken program.
    #[tokio::test]
    async fn script_body_carries_no_html_escapes() {
        for default_dark in [true, false] {
            let html = rendered(default_dark).await;
            assert!(
                !html.contains("&amp;") && !html.contains("&lt;") && !html.contains("&quot;"),
                "inline theme script must not need HTML escaping, got {html}"
            );
        }
    }

    /// GH #184: the server's build-time preference is only the fallback — it
    /// is interpolated for a visitor with no stored choice, and never widens
    /// the script's authority over a stored one.
    #[tokio::test]
    async fn default_dark_is_the_fallback_only() {
        let dark = rendered(true).await;
        assert!(
            dark.contains("t='dark'"),
            "dark default must be interpolated as the fallback, got {dark}"
        );

        let light = rendered(false).await;
        assert!(
            light.contains("t='light'"),
            "light default must be interpolated as the fallback, got {light}"
        );
        assert!(
            !light.contains("t='dark'"),
            "a light default must not interpolate dark, got {light}"
        );
    }
}
