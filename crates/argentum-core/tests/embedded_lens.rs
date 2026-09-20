//! Embedded lens resolution (GH #185).
//!
//! `TextInput::r#for` binds a top-level field: it resolves against the owned
//! `app::Model`, which cannot see embedded models, so a path through an
//! embedded struct is rejected as a traversal lens (GH #100). `r#for_context`
//! resolves through the request's app schema instead, so the leaf arrives as
//! its flattened storage column.
//!
//! This is the render-path proof: the flattened name has to be what the form
//! posts and what `field_names()` allow-lists, or a bound embedded field would
//! render blank and then be refused as an unknown key.

use std::collections::HashMap;

use argentum_core::{Schema, TextInput};
use topcoat::context::{Cx, CxTestBuilder};
use topcoat::view::ViewExt;

#[derive(Debug, Clone, toasty::Embed)]
struct Seo {
    title: String,
    description: String,
}

#[derive(Debug, Clone, toasty::Embed)]
struct Meta {
    seo: Seo,
    note: String,
}

#[derive(Debug, Clone, toasty::Model)]
struct Author {
    #[key]
    #[auto]
    id: uuid::Uuid,
    name: String,
}

/// A unit-or-payload enum: the payload columns are `kind_at` / `kind_url`,
/// with the variant label living in the `kind` discriminant column.
#[derive(Debug, Clone, toasty::Embed)]
enum Kind {
    Scheduled { at: String },
    Published { url: String },
}

/// A shared column: every variant declares a timestamp under the same shared
/// identifier, so they coalesce into one column named after the identifier.
#[derive(Debug, Clone, toasty::Embed)]
enum Publication {
    #[column(variant = 1)]
    Scheduled {
        #[shared(timestamp)]
        scheduled_at: String,
    },
    #[column(variant = 2)]
    Published {
        #[shared(timestamp)]
        published_at: String,
    },
}

/// A document embeds into **one** column named after the field itself.
#[derive(Debug, Clone, toasty::Embed)]
struct Extra {
    note: String,
}

#[derive(Debug, Clone, toasty::Model)]
struct Article {
    #[key]
    #[auto]
    id: uuid::Uuid,
    #[index]
    title: String,
    meta: Meta,
    kind: Kind,
    #[document]
    extra: Extra,
    publication: Publication,
    #[index]
    author_id: uuid::Uuid,
    #[belongs_to(key = author_id, references = id)]
    author: toasty::Deferred<Author>,
}

/// A `Db` built from the article model — the app schema comes with it.
async fn article_cx() -> Cx {
    let db = toasty::Db::builder()
        .models(toasty::models!(Article, Author))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    CxTestBuilder::new().app_context(db).build()
}

async fn render(schema: &Schema, cx: &Cx, values: HashMap<String, String>) -> String {
    schema
        .render_with(cx, &values, &HashMap::new())
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(cx)
}

#[tokio::test]
async fn embedded_leaf_resolves_to_its_flattened_column() {
    let cx = article_cx().await;
    // Two levels deep: Article.meta.seo.title -> meta_seo_title.
    let input = TextInput::r#for_context(&cx, Article::fields().meta().seo().title());
    assert_eq!(
        input.field_name(),
        "meta_seo_title",
        "an embedded leaf must resolve to its flattened storage column"
    );

    let mut values = HashMap::new();
    values.insert("meta_seo_title".to_string(), "Nested title".to_string());
    let html = render(&Schema::new(input), &cx, values).await;
    assert!(
        html.contains("value=\"Nested title\""),
        "the flattened value must hydrate into the control, got {html}"
    );
    assert!(
        html.contains("name=\"meta_seo_title\""),
        "the control must post the flattened column, got {html}"
    );
}

#[tokio::test]
async fn a_one_level_embedded_leaf_resolves_too() {
    let cx = article_cx().await;
    let input = TextInput::r#for_context(&cx, Article::fields().meta().note());
    assert_eq!(input.field_name(), "meta_note");
}

#[tokio::test]
async fn a_top_level_field_still_resolves_through_the_schema_path() {
    let cx = article_cx().await;
    let input = TextInput::r#for_context(&cx, Article::fields().title());
    assert_eq!(
        input.field_name(),
        "title",
        "a single-segment lens must be unchanged by the schema-aware path"
    );
}

/// A document collapses to its own column, so binding an inner path must
/// target that column rather than accumulate the document's inner steps.
#[tokio::test]
async fn a_document_leaf_resolves_to_the_document_column() {
    let cx = article_cx().await;
    let input = TextInput::r#for_context(&cx, Article::fields().extra().note());
    assert_eq!(
        input.field_name(),
        "extra",
        "a #[document] field stores as one column named after the field"
    );
}

/// An enum payload column is `{enum_field}_{field}`; the variant name lives in
/// the discriminant column and must not leak into the payload column.
#[tokio::test]
async fn an_enum_payload_leaf_uses_the_field_name_not_the_variant() {
    let cx = article_cx().await;
    let at = TextInput::r#for_context(&cx, Article::fields().kind().scheduled().at());
    assert_eq!(at.field_name(), "kind_at");

    let url = TextInput::r#for_context(&cx, Article::fields().kind().published().url());
    assert_eq!(url.field_name(), "kind_url");
}

/// A shared column is reachable through the variant-rooted accessor the field
/// carries — the interim path while the un-gated accessor is upstream (GH #140,
/// tokio-rs/toasty#1212). What matters here is the *storage* name: it is the
/// shared identifier, not the declaring variant's field name.
#[tokio::test]
async fn a_shared_column_resolves_to_the_shared_identifier() {
    let cx = article_cx().await;
    let scheduled = TextInput::r#for_context(
        &cx,
        Article::fields().publication().scheduled().scheduled_at(),
    );
    assert_eq!(
        scheduled.field_name(),
        "publication_timestamp",
        "a #[shared] column is named after the shared identifier"
    );

    // Both variants land on the same column — that is what "shared" means.
    let published = TextInput::r#for_context(
        &cx,
        Article::fields().publication().published().published_at(),
    );
    assert_eq!(published.field_name(), "publication_timestamp");
}

/// A traversal lens over a relation is not an embedded step, and this walk is
/// for embedded binding only. It must fail loudly rather than bind anything —
/// `author_id` and `name` are different columns, so a silent misbind here would
/// write the wrong one (GH #100).
#[tokio::test]
#[should_panic(expected = "only embedded steps")]
async fn a_relation_traversal_is_refused_rather_than_misbound() {
    let cx = article_cx().await;
    let _ = TextInput::r#for_context(&cx, Article::fields().author().name());
}

/// The allow-list and validation read the same name the control posts, so a
/// bound embedded field is neither rejected as unknown nor silently unvalidated.
#[tokio::test]
async fn the_flattened_name_participates_in_allow_list_and_validation() {
    let cx = article_cx().await;
    let schema = Schema::new((
        TextInput::r#for_context(&cx, Article::fields().title()),
        TextInput::r#for_context(&cx, Article::fields().meta().seo().title()),
    ));

    let mut values = HashMap::new();
    values.insert("title".to_string(), "Top".to_string());
    values.insert("meta_seo_title".to_string(), "Nested".to_string());
    assert!(
        schema.unknown_keys(&values).is_empty(),
        "declared embedded fields must be allow-listed, got {:?}",
        schema.unknown_keys(&values)
    );

    // An embedded leaf is storage-nullable, so it is optional by default: an
    // absent value must not fail the submit.
    let mut only_title = HashMap::new();
    only_title.insert("title".to_string(), "Top".to_string());
    assert!(
        schema.validate(&only_title).is_empty(),
        "an embedded leaf must not be required by default, got {:?}",
        schema.validate(&only_title)
    );

    // ...but a required one is still required when asked for explicitly.
    let required = Schema::new(
        TextInput::r#for_context(&cx, Article::fields().meta().seo().description()).required(),
    );
    assert!(
        required
            .validate(&HashMap::new())
            .contains_key("meta_seo_description"),
        "an explicitly required embedded leaf must validate presence"
    );
}

/// Without a `Db` there is no app schema, and the single-segment rule must
/// still refuse a traversal lens loudly rather than bind the wrong column.
#[tokio::test]
#[should_panic(expected = "single-field lens")]
async fn without_a_schema_a_traversal_lens_still_fails_loudly() {
    let cx = CxTestBuilder::new().build();
    let _ = TextInput::r#for_context(&cx, Article::fields().meta().note());
}
