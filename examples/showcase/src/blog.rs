//! The public blog: `/blog` and `/blog/{id}`, served with no session.
//!
//! The pages are app-level `#[page]`s under a `#[layout("/blog")]`, so they are
//! public by construction. The auth gate installs exactly two layers — the
//! panel prefix and `/_topcoat/runtime` — and a Topcoat layer wraps only the
//! routes under its path prefix, so nothing under `/blog` is gated. Route
//! discovery is link-time over the binary, so the `Router::builder().discover()`
//! in `Panel::build` picks these up with no router change.
//!
//! Both pages query the model directly. The panel's `Table` and its resource
//! loaders are panel-scoped (auth, tenancy, chrome) and would drag the admin
//! shell's assumptions into a page that has no session to resolve them from.

use tablo_core::db::db;
use toasty::stmt::Include;
use topcoat::{
    Result,
    asset::AssetConfig,
    context::{Cx, try_app_context},
    router::{
        Slot,
        error::{RouterErrorExt, not_found},
        href, layout, page, path_param,
    },
    tailwind,
    view::{View, class, view},
};

use crate::{
    app::GEIST,
    models::{Author, MediaAsset, Post},
};

// The status a post carries once it is visible to the public.
const PUBLISHED: &str = "published";

// The record key in `/blog/{id}`: the post's `Uuid`, parsed from the segment.
path_param!(pub id: uuid::Uuid);

/// The public shell: a complete document, rendered by the layout itself.
///
/// The panel's `render_document` is `pub(crate)` and its `layout_shell` is the
/// admin chrome, so a public page renders its own document from public APIs —
/// the shape the Topcoat demo's root shell uses.
///
/// The path is explicit (`/blog`) rather than `/`: layout paths are prefixes,
/// so a root layout would wrap `/admin` too and nest a document inside the
/// admin's own document.
#[layout("/blog")]
async fn blog_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    // The stylesheet, the font and the browser runtime are `Asset` URLs, and an
    // `Asset` renders only where an asset config is registered. The test router
    // builds without one (`router_for_tests`), so the head degrades to the dev
    // script alone instead of panicking — the same fallback the panel's shell
    // takes when it has no `ShellAssets`.
    let assets = try_app_context::<AssetConfig>(cx).is_some();

    Ok(view! {
        <!DOCTYPE html>
        <html>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>"Tablo Blog"</title>
                topcoat::dev::script()
                if assets {
                    topcoat::runtime::script()
                    topcoat::font::link(font: GEIST)
                    <link rel="stylesheet" href=(tailwind::stylesheet!())>
                }
            </head>
            <body class="flex min-h-screen flex-col bg-background text-foreground">
                <header class="border-b border-border">
                    <nav
                        class="mx-auto flex w-full max-w-3xl items-center gap-6 px-6 py-4"
                    >
                        <a
                            let href = href!(page);
                            let current = href.is_current(cx);
                            href=(href)
                            aria-current=(current.then_some("page"))
                            class=(class!(
                                "font-semibold",
                                "text-foreground" if current,
                                "text-muted-foreground hover:text-foreground" if !current,
                            ))
                        >
                            "Tablo Blog"
                        </a>
                        <a
                            href="/admin"
                            class="ml-auto text-sm text-muted-foreground hover:text-foreground"
                        >
                            "Admin"
                        </a>
                    </nav>
                </header>

                <main class="mx-auto w-full max-w-3xl flex-1 px-6 py-10">(slot)</main>

                <footer class="border-t border-border">
                    <p
                        class="mx-auto w-full max-w-3xl px-6 py-4 text-sm text-muted-foreground"
                    >
                        "Published with Tablo."
                    </p>
                </footer>
            </body>
        </html>
    })
}

/// The list: published posts only, newest first, each linking to its page.
#[page("/blog")]
async fn page(cx: &Cx) -> Result<impl View> {
    let mut db = db(cx);
    // The author is included, not lazily read: an un-included `Deferred` panics
    // in `get()`, and a per-row load would be one query per listed post.
    let include_author: Include<Post, Author> = Post::fields().author().into();
    let posts = Post::filter(Post::fields().status().eq(PUBLISHED.to_string()))
        .order_by(Post::fields().created_at().desc())
        .include(include_author)
        .exec(&mut db)
        .await?;

    Ok(view! {
        <h1 class="text-3xl font-bold tracking-tight">"Blog"</h1>

        if posts.is_empty() {
            <p class="mt-6 text-muted-foreground">
                "No posts have been published yet."
            </p>
        } else {
            <ul class="mt-8 flex flex-col gap-8">
                for post in &posts {
                    <li>
                        <article>
                            <h2 class="text-xl font-semibold tracking-tight">
                                <a
                                    href=(href!(post_page, Id(post.id)))
                                    class="hover:underline"
                                >
                                    (&post.title)
                                </a>
                            </h2>
                            <p class="mt-1 text-sm text-muted-foreground">
                                (author_name(post))
                                " · "
                                (post.created_at.strftime("%Y-%m-%d").to_string())
                            </p>
                            // Guarded like the detail page's: a post with no
                            // description renders no empty paragraph.
                            if !post.seo.description.is_empty() {
                                <p class="mt-3 text-muted-foreground">
                                    (&post.seo.description)
                                </p>
                            }
                        </article>
                    </li>
                }
            </ul>
        }
    })
}

/// The detail page: the body, the cover, and the SEO description.
///
/// A draft 404s rather than rendering a preview: the list is the only index of
/// what is public, and an unpublished id must not answer 200.
#[page("/blog/{id}")]
async fn post_page(cx: &Cx) -> Result<impl View> {
    // A malformed id is a not-found, not a 400: the URL names a post, and no
    // post has that key.
    let id = path_param::<Id>(cx).map_err(|_| not_found())?.to_owned();

    let mut db = db(cx);
    let include_author: Include<Post, Author> = Post::fields().author().into();
    let post = Post::filter(
        Post::fields()
            .id()
            .eq(id)
            .and(Post::fields().status().eq(PUBLISHED.to_string())),
    )
    .include(include_author)
    .first()
    .exec(&mut db)
    .await?
    .ok_or_not_found()?;

    // The post's cover, when the picker names one: a single library row the
    // post's `cover_id` points at.
    let cover = match post.cover_id {
        Some(cover_id) => {
            MediaAsset::filter(MediaAsset::fields().id().eq(cover_id))
                .first()
                .exec(&mut db)
                .await?
        }
        None => None,
    };

    Ok(view! {
        <a
            href=(href!(page))
            class="text-sm text-muted-foreground hover:text-foreground"
        >
            "← All posts"
        </a>

        <article class="mt-4">
            <h1 class="text-3xl font-bold tracking-tight">(&post.title)</h1>
            <p class="mt-2 text-sm text-muted-foreground">
                (author_name(&post))
                " · "
                (post.created_at.strftime("%Y-%m-%d").to_string())
            </p>

            if !post.seo.description.is_empty() {
                <p class="mt-6 text-lg text-muted-foreground">
                    (&post.seo.description)
                </p>
            }

            if let Some(asset) = cover.as_ref().and_then(|asset| cover_url(asset)) {
                <img
                    src=(asset.to_string())
                    alt=(post.title.clone())
                    class="mt-8 w-full rounded-lg border border-border"
                >
            }

            <div class="mt-8 leading-7">(&post.body)</div>
        </article>
    })
}

/// The author's display name, read from the include the page's query declared.
///
/// The `is_unloaded` guard keeps a dropped include a visible placeholder rather
/// than a panic inside `Deferred::get`, matching the admin table's columns.
fn author_name(post: &Post) -> String {
    if post.author.is_unloaded() {
        "(unknown author)".to_string()
    } else {
        post.author.get().name.clone()
    }
}

/// The cover URL for a picked library row, or `None` when it names no servable
/// path.
///
/// The row stores the served URL the uploader returned — a rooted path this
/// app serves — so a row whose path is not one renders no image rather than a
/// broken one.
fn cover_url(asset: &MediaAsset) -> Option<&str> {
    let path = asset.path.trim();
    path.starts_with('/').then_some(path)
}
