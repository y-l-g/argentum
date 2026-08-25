use argentum_core::{Grid, Group, Schema, Section, Text};
use topcoat::{Result, context::Cx, router::page, view::view};

#[page("/admin/showcase/schema")]
async fn schema_showcase(cx: &Cx) -> Result {
    // Live renders for each variant — keep code minimal and matching snippets.
    let text = Schema::new(Text::new("hello")).render(cx).await?;
    let section_empty = Schema::new(Section::new("Account")).render(cx).await?;
    let section_with_child = Schema::new(Section::new("Account").schema(Text::new("hello")))
        .render(cx)
        .await?;
    let group = Schema::new(Group::new().schema(Text::new("inside group")))
        .render(cx)
        .await?;
    let grid_1 = Schema::new(Grid::new(1).schema(Text::new("one col")))
        .render(cx)
        .await?;
    let grid_2 = Schema::new(Grid::new(2).schema((Text::new("a"), Text::new("b"))))
        .render(cx)
        .await?;
    let grid_12 = Schema::new(Grid::new(12).schema(Text::new("12 cols")))
        .render(cx)
        .await?;
    let grid_clamped = Schema::new(Grid::new(15).schema(Text::new("15 → clamped to 12")))
        .render(cx)
        .await?;
    let nested = Schema::new(
        Section::new("Outer").schema(Grid::new(2).schema((Text::new("left"), Text::new("right")))),
    )
    .render(cx)
    .await?;
    let composed = Schema::new((
        Section::new("A").schema(Text::new("a")),
        Group::new().schema(Text::new("b")),
    ))
    .render(cx)
    .await?;
    let empty = Schema::empty().render(cx).await?;

    view! {
        <div class="ac-showcase">
            <h1>"Schema"</h1>
            <p>"Unified layout primitive for forms/infolists. Text + Section/Group/Grid compose via IntoSchema tuples and render through view!."</p>

            <section class="ac-showcase-block">
                <h2>"Text (leaf)"</h2>
                <p>"Placeholder leaf — will become typed fields (TextInput, Select...)."</p>
                <pre><code>"Schema::new(Text::new(\"hello\"))"</code></pre>
                <div class="ac-showcase-result">(text)</div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Section"</h2>
                <p>"Titled container. Empty vs with child."</p>
                <pre><code>"Section::new(\"Account\")\nSection::new(\"Account\").schema(Text::new(\"hello\"))"</code></pre>
                <div class="ac-showcase-result">
                    <h3>"empty"</h3> (section_empty)
                    <h3>"with child"</h3> (section_with_child)
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Group"</h2>
                <p>"Unlabelled container."</p>
                <pre><code>"Group::new().schema(Text::new(\"inside group\"))"</code></pre>
                <div class="ac-showcase-result">(group)</div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Grid"</h2>
                <p>"Column container, cols 1..12 (clamped). Variants: 1, 2, 12, 15→12, nested in Section."</p>
                <pre><code>"Grid::new(1).schema(Text::new(\"one col\"))\nGrid::new(2).schema((Text::new(\"a\"), Text::new(\"b\")))\nGrid::new(12).schema(Text::new(\"12 cols\"))\nGrid::new(15).schema(Text::new(\"15 → clamped to 12\")) // ac-grid-cols-12\nSection::new(\"Outer\").schema(Grid::new(2).schema((Text::new(\"left\"), Text::new(\"right\"))))"</code></pre>
                <div class="ac-showcase-result">
                    <h3>"1 col"</h3> (grid_1)
                    <h3>"2 cols"</h3> (grid_2)
                    <h3>"12 cols"</h3> (grid_12)
                    <h3>"15 clamped"</h3> (grid_clamped)
                    <h3>"nested Grid in Section"</h3> (nested)
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Composition"</h2>
                <p>"Tuples of IntoSchema become one Schema. Empty renders nothing."</p>
                <pre><code>"Schema::new((Section::new(\"A\").schema(Text::new(\"a\")), Group::new().schema(Text::new(\"b\"))))\nSchema::empty() // renders \"\""</code></pre>
                <div class="ac-showcase-result">
                    <h3>"(Section, Group)"</h3> (composed)
                    <h3>"empty"</h3> (empty) <span class="ac-muted">"(empty — no ac-* classes)"</span>
                </div>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a>" | " <a href="/admin">"→ admin list"</a></p>
        </div>
    }
}
