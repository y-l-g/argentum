use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::html::{IncludeBackground, styled_line_to_highlighted_html};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use topcoat::{
    Result,
    view::{Attributes, StaticClass, Unescaped, class, component, view},
};

const PRE: StaticClass =
    class!("overflow-x-auto rounded-lg border border-border bg-muted p-4 text-sm");
const CODE: StaticClass = class!("font-mono whitespace-pre text-foreground");

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Highlights Rust code with one syntect theme, returning span HTML with
/// inline colors and no background (the `bg-muted` token shows through).
fn highlight_rust(code: &str, theme_name: &str) -> Option<String> {
    let ps = syntax_set();
    let ts = theme_set();
    let syntax = ps.find_syntax_by_extension("rs")?;
    let theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.values().next())?;
    let mut h = HighlightLines::new(syntax, theme);
    let mut html = String::new();
    for line in LinesWithEndings::from(code) {
        let ranges = h.highlight_line(line, ps).ok()?;
        let line_html = styled_line_to_highlighted_html(&ranges, IncludeBackground::No).ok()?;
        html.push_str(&line_html);
    }
    Some(html)
}

/// Highlights Rust code for both color schemes: InspiredGitHub (dark ink on
/// the light `--muted`) for light and base16-ocean.dark (pastel on dark) for
/// dark. The server cannot know the theme — `html.dark` is client state — so
/// both renders ship and CSS picks one.
fn highlight_pair(code: &str) -> Option<(String, String)> {
    Some((
        highlight_rust(code, "InspiredGitHub")?,
        highlight_rust(code, "base16-ocean.dark")?,
    ))
}

/// Code block — `overflow-x-auto` + `bg-muted` + `border-border` + `font-mono` + `whitespace-pre`.
///
/// Long lines never overflow the parent `card`. Server-side highlighting via
/// `syntect` (not Shiki) renders Rust spans with inline colors for light
/// (`InspiredGitHub`) and dark (`base16-ocean.dark`); `html.dark` is client
/// state so both variants ship and CSS picks one.
///
/// ```ignore
/// code_block(lang: "rust", code: "fn main() {}")
/// ```
#[component]
pub async fn code_block(
    #[into]
    #[default]
    lang: String,
    #[into]
    #[default]
    code: String,
    #[default] mut attrs: Attributes,
) -> Result {
    // Server-side highlighting via syntect: when lang is rust we emit spans
    // with inline `style="color:..."` for each color scheme (light first,
    // `dark:` second), falling back to plain mono when highlighting fails or
    // lang != rust. The copy button is shared and sits above both renders.
    let themes = if lang == "rust" {
        highlight_pair(&code)
    } else {
        None
    };
    // Deduplicated copy button — shared between highlighted + plain branches.
    // `data-copy-button` is handled by `assets/code_block.js` (clipboard write).
    let copy_button_class =
        "absolute right-2 top-2 rounded-md border border-border bg-background px-2 py-1 text-xs text-muted-foreground hover:bg-foreground/5";
    if let Some((light, dark)) = themes {
        // `Unescaped` is the sanctioned path for trusted pre-rendered markup
        // (syntect span HTML) — an owned `String`, no leak per render.
        // Caller `class` is merged onto both pres (plain branch merges onto
        // its single pre at :127); remaining attrs (id/data-*/aria-*) are
        // spread onto the outer container so they survive in both branches,
        // fixing the dropped-attrs bug when highlighting succeeds.
        let extra_class = attrs.remove("class");
        view! {
            <div class="relative" (attrs)>
                <pre
                    class=(class!(PRE, "shiki dark:hidden", extra_class.clone()))
                    data-lang=(lang.clone())
                >
                    <code class=(CODE)>(Unescaped::new_unchecked(light))</code>
                </pre>
                <pre
                    class=(class!(PRE, "shiki hidden dark:block", extra_class))
                    data-lang=(lang.clone())
                >
                    <code class=(CODE)>(Unescaped::new_unchecked(dark))</code>
                </pre>
                <button
                    class=(copy_button_class)
                    aria-label="Copy code"
                    data-copy-button=""
                >
                    "Copy"
                </button>
            </div>
        }
    } else {
        view! {
            <div class="relative">
                <pre
                    class=(class!(PRE, "shiki", attrs.remove("class")))
                    data-lang=(lang)
                    (attrs)
                >
                    <code class=(CODE)>(code)</code>
                </pre>
                <button
                    class=(copy_button_class)
                    aria-label="Copy code"
                    data-copy-button=""
                >
                    "Copy"
                </button>
            </div>
        }
    }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;
    use topcoat::view::attributes;

    use super::*;

    #[tokio::test]
    async fn rust_highlight_renders_unescaped_span_html() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! { cx_ref => code_block(lang: "rust", code: "fn main() {}") }
            .unwrap()
            .render(&cx);
        // syntect span markup must pass through raw, not escaped
        assert!(
            html.contains("<span style=\"color"),
            "highlighted spans should render unescaped, got {html}"
        );
        // both color schemes ship
        assert!(html.matches("<span style=\"color").count() >= 2);
    }

    #[tokio::test]
    async fn caller_attrs_survive_highlighted_branch() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! { cx_ref => code_block(lang: "rust", code: "fn main() {}", attrs: attributes!{ id="my-id" class="my-class" data-x="1" }) }
            .unwrap()
            .render(&cx);
        // custom class should be merged onto pres
        assert!(html.contains("my-class"), "missing custom class in highlighted {html}");
        // id/data-* should survive via outer div
        assert!(
            html.contains("my-id") && html.contains("data-x"),
            "caller attrs dropped in highlighted branch {html}"
        );
    }

    #[tokio::test]
    async fn caller_attrs_survive_plain_branch() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! { cx_ref => code_block(lang: "text", code: "hello", attrs: attributes!{ id="plain-id" class="plain-class" }) }
            .unwrap()
            .render(&cx);
        assert!(html.contains("plain-class"), "missing class in plain {html}");
        assert!(html.contains("plain-id"), "missing id in plain {html}");
    }
}
