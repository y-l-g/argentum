use topcoat::{Result, router::page, view::view};

#[page("/admin/showcase")]
async fn showcase_index() -> Result {
    view! {
        <div class="ac-showcase ac-showcase-index">
            <h1>"Showcase"</h1>
            <p>"Single example demonstrating every Argentum feature — minimal snippet, description, and live result."</p>

            <section class="ac-showcase-section">
                <h2>"Features"</h2>
                <ul>
                    <li><a href="/admin/showcase/panel">"Panel"</a>" — app shell, prefix, Db in app_context, Router discover"</li>
                    <li><a href="/admin/showcase/resource">"Resource"</a>" — #[derive(Resource)] model + query override, navigation"</li>
                    <li><a href="/admin/showcase/schema">"Schema"</a>" — Section / Group / Grid / Text + composition variants"</li>
                    <li><a href="/admin/showcase/db">"Db + memoize"</a>" — db(cx) glue and per-request memoization"</li>
                    <li><a href="/admin">"Admin list (/admin)"</a>" — real app page (Panel + Resource → table)"</li>
                </ul>
            </section>

            <section class="ac-showcase-section">
                <h2>"How to read each page"</h2>
                <p>"Each page shows: description → minimal code snippet → rendered result as in a real app."</p>
                <pre><code>"// snippet (copy-paste minimal)\n// rendered below is the live View from that code"</code></pre>
            </section>
        </div>
    }
}
