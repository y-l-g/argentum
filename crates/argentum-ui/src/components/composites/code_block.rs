use std::fmt::Write as _;
use std::sync::OnceLock;

use syntect::highlighting::{FontStyle, Highlighter, Style, ThemeSet};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};
use syntect::util::LinesWithEndings;
use topcoat::{
    Result,
    view::{Attributes, StaticClass, Unescaped, View, ViewExt, class, component, view},
};

// Subtle code surface, slightly lighter than the plain `bg-muted` panels
// (shadcn-docs-like): `bg-muted/50` is the base for both themes, and dark
// mixes a touch of white into `--muted` so its surface sits clearly above the
// page background rather than settling back toward it (GH #151 §4).
const PRE: StaticClass = class!(
    "overflow-x-auto rounded-lg border border-border bg-muted/50 p-4 text-sm \
     dark:bg-[color-mix(in_oklab,var(--muted),white_6%)]"
);
const CODE: StaticClass = class!("font-mono whitespace-pre text-foreground");

/// The theme-aware color comments are rendered with (GH #151 §4).
///
/// syntect's light/dark themes both pick low-contrast greys for comments,
/// which disappear into the code surface (`grey on grey`). The token color is
/// legible on both and swaps with the theme like every other component.
const COMMENT_COLOR: &str = "var(--muted-foreground)";

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// The `comment` scope, resolved once against syntect's global scope
/// repository (a global mutex, not a thread-local, so one `Scope` is valid
/// everywhere).
static COMMENT_SCOPE: OnceLock<syntect::parsing::Scope> = OnceLock::new();

/// Whether the current scope stack is inside a comment.
///
/// Scope prefixes rather than the theme's font style: an inline note is
/// `comment.line.double-slash.rust`, `///` is
/// `comment.line.documentation.rust`, and block/doc-block forms are
/// `comment.block.*` — all prefixed by `comment`.
fn in_comment(stack: &[syntect::parsing::Scope]) -> bool {
    let comment = COMMENT_SCOPE
        .get_or_init(|| syntect::parsing::Scope::new("comment").expect("valid scope name"));
    stack.iter().any(|scope| comment.is_prefix_of(*scope))
}

/// Collects highlighted runs and merges adjacent runs that share a style.
///
/// syntect's own HTML renderer merges equal-style ranges, keeping identifiers
/// such as `Text::new` as one span instead of one span per parser op; without
/// the merge, substrings break apart and the markup balloons. Comments use
/// [`COMMENT_COLOR`]; everything else keeps the syntect theme's foreground,
/// and font styles (bold/italic/underline) are emitted exactly as syntect
/// emits them. Every span carries an inline `color:` declaration, so the CSS
/// variables resolve per theme inside both highlighted `<pre>` branches.
#[derive(Default)]
struct Spans {
    html: String,
    pending: String,
    style: Style,
    comment: bool,
    open: bool,
}

impl Spans {
    fn push(&mut self, text: &str, style: Style, comment: bool) {
        if text.is_empty() {
            return;
        }
        if self.open && self.style == style && self.comment == comment {
            escape_into(&mut self.pending, text);
            return;
        }
        self.flush();
        self.style = style;
        self.comment = comment;
        self.open = true;
        escape_into(&mut self.pending, text);
    }

    fn flush(&mut self) {
        if !self.open {
            return;
        }
        self.html.push_str("<span style=\"");
        if self.style.font_style.contains(FontStyle::UNDERLINE) {
            self.html.push_str("text-decoration:underline;");
        }
        if self.style.font_style.contains(FontStyle::BOLD) {
            self.html.push_str("font-weight:bold;");
        }
        if self.style.font_style.contains(FontStyle::ITALIC) {
            self.html.push_str("font-style:italic;");
        }
        self.html.push_str("color:");
        if self.comment {
            self.html.push_str(COMMENT_COLOR);
        } else {
            let _ = write!(
                self.html,
                "#{:02x}{:02x}{:02x}",
                self.style.foreground.r, self.style.foreground.g, self.style.foreground.b
            );
        }
        self.html.push_str("\">");
        self.html.push_str(&self.pending);
        self.html.push_str("</span>");
        self.pending.clear();
        self.open = false;
    }

    fn finish(mut self) -> String {
        self.flush();
        self.html
    }
}

/// Escape `text` for element content.
///
/// `'` is left alone: it is only special inside attribute values, and every
/// span here puts text in element content.
fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Highlights Rust code with one syntect theme, returning span HTML with
/// inline colors and no background (the token surface shows through).
///
/// Walks the parser's scope changes by hand instead of going through
/// `styled_line_to_highlighted_html` so comment ranges can be re-colored to
/// the theme token (GH #151 §4); everything else is the theme's own color.
/// Adjacent equal-style runs merge back together, matching syntect's own
/// `unify_style` behavior so identifiers stay contiguous.
fn highlight_rust(code: &str, theme_name: &str) -> Option<String> {
    let ps = syntax_set();
    let ts = theme_set();
    let syntax = ps.find_syntax_by_extension("rs")?;
    let theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.values().next())?;
    let highlighter = Highlighter::new(theme);
    let mut stack = ScopeStack::new();
    let mut parse_state = ParseState::new(syntax);
    let mut spans = Spans::default();
    for line in LinesWithEndings::from(code) {
        let ops = parse_state.parse_line(line, ps).ok()?;
        let mut pos = 0usize;
        for (end, op) in ops.iter() {
            if *end > pos {
                let style = highlighter.style_for_stack(stack.as_slice());
                spans.push(&line[pos..*end], style, in_comment(stack.as_slice()));
                pos = *end;
            }
            stack.apply(op).ok()?;
        }
        if pos < line.len() {
            let style = highlighter.style_for_stack(stack.as_slice());
            spans.push(&line[pos..], style, in_comment(stack.as_slice()));
        }
    }
    Some(spans.finish())
}

/// Highlights Rust code for both color schemes: InspiredGitHub (dark ink on
/// the light surface) for light and base16-ocean.dark (pastel on dark) for
/// dark. The server cannot know the theme — `html.dark` is client state — so
/// both renders ship and CSS picks one.
fn highlight_pair(code: &str) -> Option<(String, String)> {
    Some((
        highlight_rust(code, "InspiredGitHub")?,
        highlight_rust(code, "base16-ocean.dark")?,
    ))
}

/// Code block — `overflow-x-auto` + a subtle `bg-muted/50` surface +
/// `border-border` + `font-mono` + `whitespace-pre`.
///
/// Long lines never overflow the parent `card`. Server-side highlighting via
/// `syntect` (not Shiki) renders Rust spans with inline colors for light
/// (`InspiredGitHub`) and dark (`base16-ocean.dark`); `html.dark` is client
/// state so both variants ship and CSS picks one. Comments ignore the themes'
/// low-contrast greys and render with `var(--muted-foreground)` instead, so
/// they stay legible on both surfaces (GH #151 §4).
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
) -> Result<impl View> {
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
    let copy_button_class = "absolute right-2 top-2 rounded-md border border-border bg-background px-2 py-1 text-xs text-muted-foreground hover:bg-foreground/5";
    if let Some((light, dark)) = themes {
        // `Unescaped` is the sanctioned path for trusted pre-rendered markup
        // (syntect span HTML) — an owned `String`, no leak per render.
        // Caller `class` is merged onto both pres (the plain branch merges
        // onto its single pre); remaining attrs (id/data-*/aria-*) are
        // spread onto the outer container so they survive in both branches,
        // fixing the dropped-attrs bug when highlighting succeeds.
        let extra_class = attrs.remove("class");
        Ok(view! {
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
        .boxed())
    } else {
        Ok(view! {
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
        .boxed())
    }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;
    use topcoat::view::{ViewExt, attributes};

    use super::*;

    #[tokio::test]
    async fn rust_highlight_renders_unescaped_span_html() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! { cx_ref => code_block(lang: "rust", code: "fn main() {}") }
            .single()
            .await
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
    async fn comments_render_with_the_theme_token_color() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! {
            cx_ref =>
            code_block(lang: "rust", code: "// note\nfn main() {}")
        }
        .single()
        .await
        .unwrap()
        .render(&cx);
        // Both shipped color schemes re-color the whole comment run to the
        // token (`// note` is one merged span per branch).
        assert!(html.matches("color:var(--muted-foreground)").count() >= 2);
        assert!(
            html.matches("color:var(--muted-foreground)\">// note")
                .count()
                >= 2
        );
        // The theme's own emphasis survives (InspiredGitHub marks keywords
        // bold); re-coloring must not drop font styles.
        assert!(html.contains("font-weight:bold"));
    }

    /// Multi-line snippets mixing comments and code keep every token: the
    /// comment re-coloring must not swallow the following lines.
    #[tokio::test]
    async fn multiline_snippets_keep_every_token() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let code = "// POST /admin/showcase/dialog/notify\nset_notification(\n    cx,\n    Notification::success(\"User created\").description(\"Ada Lovelace was added successfully.\"),\n);\nErr(see_other(\"/admin/showcase/dialog\").into()) // PRG";
        let html = view! { cx_ref => code_block(lang: "rust", code: (code)) }
            .single()
            .await
            .unwrap()
            .render(&cx);
        for needle in [
            "set_notification",
            "Notification::success",
            "Ada Lovelace",
            "see_other",
            "PRG",
        ] {
            assert!(html.contains(needle), "lost {needle}: {html}");
        }
    }

    #[tokio::test]
    async fn equal_style_runs_stay_contiguous() {
        // Parser ops split `Schema::new(Text::new(..))` into many ranges; the
        // renderer must merge equal-style neighbours so the identifiers stay
        // contiguous in both theme branches (GH #151 §4).
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! {
            cx_ref =>
            code_block(lang: "rust", code: "Schema::new(Text::new(\"hello\"))")
        }
        .single()
        .await
        .unwrap()
        .render(&cx);
        assert!(html.matches("Text::new").count() >= 2, "{html}");
        assert!(html.matches("Schema::new").count() >= 2, "{html}");
    }

    #[tokio::test]
    async fn caller_attrs_survive_highlighted_branch() {
        let cx = CxTestBuilder::new().build();
        let cx_ref = &cx;
        let html = view! {
            cx_ref =>
            code_block(
                lang: "rust",
                code: "fn main() {}",
                attrs: attributes! { id="my-id" class="my-class" data-x="1" }
            )
        }
        .single()
        .await
        .unwrap()
        .render(&cx);
        // custom class should be merged onto pres
        assert!(
            html.contains("my-class"),
            "missing custom class in highlighted {html}"
        );
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
        let html = view! {
            cx_ref =>
            code_block(
                lang: "text",
                code: "hello",
                attrs: attributes! { id="plain-id" class="plain-class" }
            )
        }
        .single()
        .await
        .unwrap()
        .render(&cx);
        assert!(
            html.contains("plain-class"),
            "missing class in plain {html}"
        );
        assert!(html.contains("plain-id"), "missing id in plain {html}");
    }
}
