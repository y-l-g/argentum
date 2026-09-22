//! First-class embedded values (GH #191).
//!
//! A value codec derived from the type's shape, with every key resolved from the
//! compiled app schema: the flat form map ↔ a typed embedded value, and a form
//! declaration that derives its controls instead of listing them.
//!
//! The proof is what these tests never do: spell a flattened column name, or
//! decide a variant from which payload columns happen to be non-empty.

use std::collections::HashMap;

use argentum_core::{
    EmbeddedForm, Schema, enum_spec, form_keys, leaf_key, read_embedded, submitted, write_embedded,
};
use topcoat::context::{Cx, CxTestBuilder};
use topcoat::view::ViewExt;

#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
struct Seo {
    title: String,
    description: String,
}

#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
struct Credit {
    author: String,
    licence: String,
}

#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
struct Poster {
    url: String,
    credit: Credit,
}

/// A shared column across three variants: the timestamp coalesces into one
/// `publication_timestamp` column, whichever variant declares it.
#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
enum Publication {
    #[column(variant = 1)]
    Scheduled {
        #[shared(timestamp)]
        scheduled_at: String,
        scheduled_for: String,
    },
    #[column(variant = 2)]
    Published {
        #[shared(timestamp)]
        published_at: String,
        canonical_url: String,
    },
    #[column(variant = 3)]
    Archived {
        #[shared(timestamp)]
        archived_at: String,
        reason: String,
    },
}

/// An enum carrying a nested struct inside a variant, and a typed leaf.
#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
enum Media {
    #[column(variant = 1)]
    Image { url: String, alt: String },
    #[column(variant = 2)]
    Video { video_url: String, poster: Poster },
}

/// A unit variant carries no payload: the discriminant alone is the value.
#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
enum Visibility {
    #[column(variant = 1)]
    Public,
    #[column(variant = 2)]
    Private { reason: String },
}

#[derive(Debug, Clone, Default, PartialEq, toasty::Embed, EmbeddedForm)]
struct PostStats {
    #[form(label = "Word count")]
    word_count: i64,
    read_minutes: i64,
}

#[derive(Debug, Clone, toasty::Model)]
struct Post {
    #[key]
    #[auto]
    id: uuid::Uuid,
    title: String,
    seo: Seo,
    publication: Publication,
    media: Media,
    post_stats: PostStats,
    visibility: Visibility,
}

async fn post_cx() -> Cx {
    let db = toasty::Db::builder()
        .models(toasty::models!(Post))
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    CxTestBuilder::new().app_context(db).build()
}

fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The framework names every key; the app never spells one. This pins the names
/// the resolver produces against the ones the showcase used to write by hand.
#[tokio::test]
async fn keys_come_from_the_compiled_mapping() {
    let cx = post_cx().await;

    assert_eq!(
        leaf_key(&cx, Post::fields().seo().title()),
        "seo_title",
        "an embedded struct's leaf is its flattened column"
    );
    assert_eq!(
        leaf_key(&cx, Post::fields().post_stats().word_count()),
        "post_stats_word_count"
    );

    let publication = enum_spec(&cx, Post::fields().publication()).expect("an embedded enum");
    assert_eq!(
        publication.discriminant(),
        "publication",
        "the enum's discriminant column is named after the field"
    );
    assert_eq!(
        publication.variants(),
        [
            ("Scheduled".to_string(), "1".to_string()),
            ("Published".to_string(), "2".to_string()),
            ("Archived".to_string(), "3".to_string()),
        ],
        "each variant's stored discriminant, in declaration order"
    );
    assert_eq!(publication.value_of("Published"), Some("2"));
    assert_eq!(publication.variant_of("3"), Some("Archived"));

    let keys = form_keys(&cx, Post::fields().publication());
    assert!(keys.contains(&"publication".to_string()), "got {keys:?}");
    assert!(keys.contains(&"publication_timestamp".to_string()));
    assert!(keys.contains(&"publication_canonical_url".to_string()));
    // The shared column appears once, not once per declaring variant.
    assert_eq!(
        keys.iter()
            .filter(|key| key.as_str() == "publication_timestamp")
            .count(),
        1,
        "a shared column is one column, got {keys:?}"
    );

    assert!(enum_spec(&cx, Post::fields().seo()).is_none());
}

/// A struct: leaves in, leaves out, no discriminant.
#[tokio::test]
async fn a_struct_round_trips_through_the_flat_map() {
    let cx = post_cx().await;
    let seo = Seo {
        title: "Hello".to_string(),
        description: "World".to_string(),
    };

    let mut values = HashMap::new();
    write_embedded(&cx, Post::fields().seo(), &seo, &mut values);
    assert_eq!(
        values,
        map(&[("seo_title", "Hello"), ("seo_description", "World")]),
        "a struct writes exactly its leaves"
    );

    let read: Seo = read_embedded(&cx, Post::fields().seo(), &values);
    assert_eq!(read, seo);
}

/// An enum: the active variant's leaves **plus its discriminant**, and reading
/// picks the variant from the discriminant rather than from payload emptiness.
#[tokio::test]
async fn an_enum_round_trips_with_an_explicit_discriminant() {
    let cx = post_cx().await;
    let published = Publication::Published {
        published_at: "2026-09-22T00:00:00Z".to_string(),
        canonical_url: "/hello".to_string(),
    };

    let mut values = HashMap::new();
    write_embedded(&cx, Post::fields().publication(), &published, &mut values);
    assert_eq!(
        values,
        map(&[
            ("publication", "2"),
            ("publication_timestamp", "2026-09-22T00:00:00Z"),
            ("publication_canonical_url", "/hello"),
        ]),
        "an enum writes its discriminant and the active variant's leaves"
    );

    let read: Publication = read_embedded(&cx, Post::fields().publication(), &values);
    assert_eq!(read, published);

    // The discriminating case: a submission whose *payloads* say Published but
    // whose discriminant says Archived is read as Archived. Emptiness is never
    // consulted (GH #191) — this is what the hand-written reassembly got wrong.
    let contradictory = map(&[
        ("publication", "3"),
        ("publication_timestamp", "2026-09-22T00:00:00Z"),
        ("publication_canonical_url", "/hello"),
        ("publication_reason", "superseded"),
    ]);
    let read: Publication = read_embedded(&cx, Post::fields().publication(), &contradictory);
    assert_eq!(
        read,
        Publication::Archived {
            archived_at: "2026-09-22T00:00:00Z".to_string(),
            reason: "superseded".to_string(),
        },
        "the discriminant decides the variant, not which payloads are non-empty"
    );
}

/// A submission without a discriminant — a hand-written POST, or the create
/// form before a variant control exists — reads as the first variant, and never
/// as "whichever payload happened to be filled in".
#[tokio::test]
async fn a_missing_discriminant_reads_as_the_first_variant() {
    let cx = post_cx().await;
    let values = map(&[
        ("publication_timestamp", "2026-09-22T00:00:00Z"),
        ("publication_canonical_url", "/hello"),
    ]);

    let read: Publication = read_embedded(&cx, Post::fields().publication(), &values);
    assert_eq!(
        read,
        Publication::Scheduled {
            scheduled_at: "2026-09-22T00:00:00Z".to_string(),
            scheduled_for: String::new(),
        },
        "no discriminant falls back to the first variant"
    );
}

/// Nesting: a struct inside a variant delegates to that struct's own codec, and
/// its leaves land three levels deep in the flattened column.
#[tokio::test]
async fn nested_values_delegate_to_their_own_codec() {
    let cx = post_cx().await;
    let video = Media::Video {
        video_url: "/v.mp4".to_string(),
        poster: Poster {
            url: "/p.jpg".to_string(),
            credit: Credit {
                author: "Ada".to_string(),
                licence: "CC-BY".to_string(),
            },
        },
    };

    let mut values = HashMap::new();
    write_embedded(&cx, Post::fields().media(), &video, &mut values);
    assert_eq!(
        values,
        map(&[
            ("media", "2"),
            ("media_video_url", "/v.mp4"),
            ("media_poster_url", "/p.jpg"),
            ("media_poster_credit_author", "Ada"),
            ("media_poster_credit_licence", "CC-BY"),
        ])
    );

    let read: Media = read_embedded(&cx, Post::fields().media(), &values);
    assert_eq!(read, video);
}

/// A unit variant has no payload: the discriminant is the whole value, and a
/// control set that renders nothing for it still round-trips.
#[tokio::test]
async fn a_unit_variant_round_trips_on_its_discriminant_alone() {
    let cx = post_cx().await;

    let mut values = HashMap::new();
    write_embedded(
        &cx,
        Post::fields().visibility(),
        &Visibility::Public,
        &mut values,
    );
    assert_eq!(values, map(&[("visibility", "1")]));
    let read: Visibility = read_embedded(&cx, Post::fields().visibility(), &values);
    assert_eq!(read, Visibility::Public);

    let mut values = HashMap::new();
    write_embedded(
        &cx,
        Post::fields().visibility(),
        &Visibility::Private {
            reason: "draft".to_string(),
        },
        &mut values,
    );
    assert_eq!(
        values,
        map(&[("visibility", "2"), ("visibility_reason", "draft")])
    );
    let read: Visibility = read_embedded(&cx, Post::fields().visibility(), &values);
    assert_eq!(
        read,
        Visibility::Private {
            reason: "draft".to_string()
        }
    );
}

/// A typed leaf keeps its own spelling rule (GH #192): `Display` out, `FromStr`
/// back, and an empty submit is the type's default rather than a panic.
#[tokio::test]
async fn typed_leaves_round_trip_and_default_when_empty() {
    let cx = post_cx().await;
    let stats = PostStats {
        word_count: 1200,
        read_minutes: 6,
    };

    let mut values = HashMap::new();
    write_embedded(&cx, Post::fields().post_stats(), &stats, &mut values);
    assert_eq!(
        values,
        map(&[
            ("post_stats_word_count", "1200"),
            ("post_stats_read_minutes", "6"),
        ])
    );
    let read: PostStats = read_embedded(&cx, Post::fields().post_stats(), &values);
    assert_eq!(read, stats);

    let empty = map(&[
        ("post_stats_word_count", "  "),
        ("post_stats_read_minutes", ""),
    ]);
    let read: PostStats = read_embedded(&cx, Post::fields().post_stats(), &empty);
    assert_eq!(read, PostStats::default(), "an empty typed leaf defaults");
}

/// A value the type cannot parse panics loudly: typed controls refuse it inline
/// first, so reaching the codec with one means the form was bypassed, and a
/// silent zero is the bug GH #192 fixed.
#[tokio::test]
#[should_panic(expected = "is not a valid value for `post_stats_word_count`")]
async fn an_unparseable_typed_leaf_panics() {
    let cx = post_cx().await;
    let values = map(&[("post_stats_word_count", "many")]);
    let _: PostStats = read_embedded(&cx, Post::fields().post_stats(), &values);
}

/// Presence: whether a submission mentions this value at all, which is the
/// update path's "absent means unchanged" rule (GH #89) with no app-side column
/// names.
#[tokio::test]
async fn submitted_reports_whether_a_value_was_mentioned() {
    let cx = post_cx().await;
    assert!(submitted(
        &cx,
        Post::fields().seo(),
        &map(&[("seo_title", "x")])
    ));
    assert!(!submitted(
        &cx,
        Post::fields().seo(),
        &map(&[("title", "x")])
    ));
    // The discriminant counts: a form that only posts the variant mentioned it.
    assert!(submitted(
        &cx,
        Post::fields().publication(),
        &map(&[("publication", "1")])
    ));
}

/// The generated form: every leaf control, plus the hidden discriminant, named
/// from the compiled mapping — and no control at all in view mode.
#[tokio::test]
async fn the_derived_form_renders_every_leaf_and_the_discriminant() {
    let cx = post_cx().await;
    let schema = Schema::new(Publication::form(&cx, Post::fields().publication()));
    let mut values = HashMap::new();
    write_embedded(
        &cx,
        Post::fields().publication(),
        &Publication::Archived {
            archived_at: "2026-09-22T00:00:00Z".to_string(),
            reason: "superseded".to_string(),
        },
        &mut values,
    );

    let html = schema
        .render_with(&cx, &values, &HashMap::new())
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);

    assert!(
        html.contains("type=\"hidden\"") && html.contains("name=\"publication\""),
        "the discriminant must ride the form as a hidden control, got {html}"
    );
    assert!(
        html.contains("value=\"3\""),
        "the stored variant hydrates into the discriminant, got {html}"
    );
    for name in [
        "publication_timestamp",
        "publication_scheduled_for",
        "publication_canonical_url",
        "publication_reason",
    ] {
        assert!(html.contains(name), "missing control {name} in {html}");
    }
    assert!(
        html.contains(">Canonical Url<"),
        "an unlabelled field is humanized from its name, got {html}"
    );

    // Read-only: the discriminant is not a value a reader wants, so the view
    // renders nothing for it — no control leaks into a read-only page.
    let view = schema
        .render_readonly(&cx, &values)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        !view.contains("<input"),
        "a hidden control must not render in view mode, got {view}"
    );
}
