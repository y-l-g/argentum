use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::html::{styled_line_to_highlighted_html, IncludeBackground};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use topcoat::{
    Result,
    view::{Attributes, StaticClass, View, class, component, view},
};

const PRE: StaticClass =
    class!("overflow-x-auto rounded-lg border border-border bg-muted p-4 text-sm");
const CODE: StaticClass = class!("font-mono whitespace-pre text-foreground");

fn highlight_rust(code: &str) -> Option<String> {
    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    let syntax = ps.find_syntax_by_extension("rs")?;
    // Use InspiredGitHub for light (high contrast #333 on white) as default — page is light by default.
    // base16-ocean.dark is dark pastel on light bg (muted). Dark mode will be handled via CSS variables in future.
    let theme = ts
        .themes
        .get("InspiredGitHub")
        .or_else(|| ts.themes.get("base16-ocean.dark"))
        .or_else(|| ts.themes.values().next())?;
    let mut h = HighlightLines::new(syntax, theme);
    let mut html = String::new();
    for line in LinesWithEndings::from(code) {
        let ranges = h.highlight_line(line, &ps).ok()?;
        let line_html = styled_line_to_highlighted_html(&ranges, IncludeBackground::No).ok()?;
        html.push_str(&line_html);
    }
    Some(html)
}

/// Code block — `overflow-x-auto` + `bg-muted` + `border-border` + `font-mono` + `whitespace-pre`.
///
/// Long lines never overflow the parent `card`. Shiki highlighting is wired in
/// T4; this placeholder renders plain mono text so T1 stays green.
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
    // Server Shiki via syntect — when lang is rust we emit spans with inline
    // `style="color:..."` matching neutral Tokens (base16-ocean), fallback to
    // plain mono when highlight fails or lang != rust.
    let highlighted = if lang == "rust" {
        highlight_rust(&code)
    } else {
        None
    };
    if let Some(html) = highlighted {
        let view = View::unescaped_unchecked(Box::leak(html.into_boxed_str()));
        view! {
            <div class="relative">
                <pre
                    class=(class!(PRE, "shiki", attrs.remove("class")))
                    data-lang=(lang)
                    (attrs)
                >
                    <code class=(CODE)>(view)</code>
                </pre>
                <button
                    class="absolute right-2 top-2 rounded-md border border-border bg-background px-2 py-1 text-xs text-muted-foreground hover:bg-foreground/5"
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
                    class="absolute right-2 top-2 rounded-md border border-border bg-background px-2 py-1 text-xs text-muted-foreground hover:bg-foreground/5"
                    aria-label="Copy code"
                    data-copy-button=""
                >
                    "Copy"
                </button>
            </div>
        }
    }
}
