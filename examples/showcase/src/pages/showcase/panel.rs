use argentum_core::Panel;
use topcoat::{Result, router::page, view::view};

#[page("/admin/showcase/panel")]
async fn panel_showcase() -> Result {
    let p_admin = Panel::new("admin").prefix().to_string();
    let p_slash = Panel::new("/admin").prefix().to_string();
    let p_slash_trail = Panel::new("admin/").prefix().to_string();
    let p_empty = Panel::new("").prefix().to_string();
    let p_custom = Panel::new("showcase").prefix().to_string();

    view! {
        <div class="ac-showcase">
            <h1>"Panel"</h1>
            <p>"Admin shell — owns Router, Db in app_context, and prefix. Single Panel in Phase 1."</p>

            <section class="ac-showcase-block">
                <h2>"Construction"</h2>
                <pre><code>"Panel::new(\"admin\").app_context(db).build() // → Router via discover + app_context(Db)\n// mounts at \"/admin\", discovers #[layout(\"/admin\")] + #[page(\"/admin...\")] "</code></pre>
                <div class="ac-showcase-result">
                    <p>"Current app mounts at: " (p_admin.clone())</p>
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Prefix normalization — variants"</h2>
                <p>"Prefixes are trimmed and normalized to /{name}. Empty → /admin."</p>
                <pre><code>"Panel::new(\"admin\").prefix()    // \"/admin\"\nPanel::new(\"/admin\").prefix()   // \"/admin\"\nPanel::new(\"admin/\").prefix()   // \"/admin\"\nPanel::new(\"\").prefix()         // \"/admin\"\nPanel::new(\"showcase\").prefix() // \"/showcase\""</code></pre>
                <div class="ac-showcase-result">
                    <table class="ac-table">
                        <thead><tr><th>"input"</th><th>"prefix()"</th></tr></thead>
                        <tbody>
                            <tr><td>"\"admin\""</td><td>(p_admin)</td></tr>
                            <tr><td>"\"/admin\""</td><td>(p_slash)</td></tr>
                            <tr><td>"\"admin/\""</td><td>(p_slash_trail)</td></tr>
                            <tr><td>"\"\""</td><td>(p_empty)</td></tr>
                            <tr><td>"\"showcase\""</td><td>(p_custom)</td></tr>
                        </tbody>
                    </table>
                </div>
            </section>

            <section class="ac-showcase-block">
                <h2>"Db in app_context"</h2>
                <pre><code>"let router = Panel::new(\"admin\").app_context(db).build();\n// later in page/shard: let mut db = db(cx); // app_context::<Db>(cx).clone()\n// Db is Arc-pooled, clone is cheap, exec needs &mut Db"</code></pre>
            </section>

            <p><a href="/admin/showcase">"← back to showcase"</a></p>
        </div>
    }
}
