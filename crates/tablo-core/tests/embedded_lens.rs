//! Embedded lens resolution through the request's app schema.
//!
//! `TextInput::r#for` binds a top-level field: it resolves against the owned
//! `app::Model`, which cannot see embedded models, so a path through an
//! embedded struct is rejected as a traversal lens. `r#for_context`
//! resolves through the request's app schema instead, so the leaf arrives as
//! its flattened storage column.
//!
//! This is the render-path proof: the flattened name is what the form posts
//! and what `field_names()` allow-lists, or a bound embedded field would
//! render blank and then be refused as an unknown key. The resolver walk
//! itself is covered by `schema::lenses`'s own tests; the two refusals at the
//! bottom drive the panicking entry a form binding uses, which that module
//! does not.

use std::collections::HashMap;

use tablo_core::{Schema, TextInput};
use topcoat::{
    context::{Cx, CxTestBuilder},
    view::ViewExt,
};

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

#[derive(Debug, Clone, toasty::Model)]
struct Article {
    #[key]
    #[auto]
    id: uuid::Uuid,
    #[index]
    title: String,
    meta: Meta,
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

    // An embedded leaf is never required by default (binding policy: the
    // resolver reports `nullable=true` even though a required embedded
    // struct's flattened column is `NOT NULL`): an absent value must not fail
    // the submit.
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

/// A traversal lens over a relation is not an embedded step, and this walk is
/// for embedded binding only. It must fail loudly rather than bind anything —
/// `author_id` and `name` are different columns, so a silent misbind here would
/// write the wrong one. No `lenses.rs` test reaches this branch: its
/// panic rows come from the missing/foreign-root model check, not from a
/// relation hop in the walk.
#[tokio::test]
#[should_panic(expected = "only embedded steps")]
async fn a_relation_traversal_is_refused_rather_than_misbound() {
    let cx = article_cx().await;
    let _ = TextInput::r#for_context(&cx, Article::fields().author().name());
}

/// Without a `Db` there is no app schema, and the single-segment rule must
/// still refuse a traversal lens loudly rather than bind the wrong column.
/// `lenses.rs` covers `resolve_embedded_value` against a bare `Cx`, which
/// returns `None`; this pins the panicking `resolve` entry a form binding uses.
#[tokio::test]
#[should_panic(expected = "single-field lens")]
async fn without_a_schema_a_traversal_lens_still_fails_loudly() {
    let cx = CxTestBuilder::new().build();
    let _ = TextInput::r#for_context(&cx, Article::fields().meta().note());
}
