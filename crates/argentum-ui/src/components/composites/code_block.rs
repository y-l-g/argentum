use topcoat::{
    Result,
    view::{Attributes, StaticClass, class, component, view},
};

const PRE: StaticClass =
    class!("overflow-x-auto rounded-lg border border-border bg-muted p-4 text-sm");
const CODE: StaticClass = class!("font-mono whitespace-pre text-foreground");

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
    let lang = lang.clone();
    let code = code.clone();
    // Shiki class added so tests can assert highlighting hook; actual
    // syntect highlighting will replace plain (code) with spans later.
    view! {
        <div class="relative">
            <pre
                class=(class!(PRE, "shiki", attrs.remove("class")))
                data-lang=(lang)
                (attrs)
            >
                <code class=(CODE)>(code)</code>
            </pre>
            // Copy button — Shiki T4 will add `navigator.clipboard` handler
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
