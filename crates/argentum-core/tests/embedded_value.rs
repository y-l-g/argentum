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
    EmbeddedForm, Schema, enum_spec, leaf_key, read_embedded, submitted, write_embedded,
};
use topcoat::{
    context::{Cx, CxTestBuilder},
    view::ViewExt,
};

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

/// A struct holding an enum: the nested enum's discriminant is a form key of
/// the value too, not only its payloads.
#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
struct Wrapper {
    label: String,
    inner: Media,
}

/// Variant idents the schema normalises (`OK` reads `Ok`): a codec addresses
/// variants by declaration index, so no casing has to round-trip.
#[derive(Debug, Clone, PartialEq, toasty::Embed, EmbeddedForm)]
enum Casing {
    #[column(variant = 1)]
    OK { at: String },
    #[column(variant = 2)]
    Draft,
}

/// The leaf types the panel can spell (GH #192 + GH #191's widening): `bool`
/// and the whole integer family, not only the three the showcase happened to
/// use.
#[derive(Debug, Clone, Default, PartialEq, toasty::Embed, EmbeddedForm)]
struct Flags {
    featured: bool,
    level: u8,
    revision: u32,
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
    wrapper: Wrapper,
    casing: Casing,
    flags: Flags,
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
    assert_eq!(publication.len(), 3, "three variants, in declaration order");
    assert_eq!(publication.value_of_index(1), Some("2"));
    assert_eq!(
        publication.name_of_index(1),
        Some("Published"),
        "the value a variant stores and the name it is labelled with are two \
         different things"
    );
    assert_eq!(publication.name_of_index(3), None);
    assert_eq!(publication.index_of("3"), Some(2));
    assert_eq!(publication.index_of("nope"), None);

    // A value knows which keys are its own: the discriminant and every leaf,
    // the shared column included (once — it is one column).
    assert!(submitted(
        &cx,
        Post::fields().publication(),
        &map(&[("publication", "1")])
    ));
    assert!(submitted(
        &cx,
        Post::fields().publication(),
        &map(&[("publication_timestamp", "t")])
    ));
    assert!(submitted(
        &cx,
        Post::fields().publication(),
        &map(&[("publication_canonical_url", "/x")])
    ));

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

/// A submission without a discriminant — the create form, or a hand-written
/// POST — falls back to the rule the panel used before the discriminant
/// existed: the first variant (in declaration order) with a payload of its own
/// submitted. Reached only when no discriminant is named; an explicit one
/// always wins.
#[tokio::test]
async fn a_missing_discriminant_infers_the_variant_from_its_payload() {
    let cx = post_cx().await;

    // The pre-#191 showcase behaviour, preserved: filling the Published payload
    // creates a Published value.
    let published = map(&[
        ("publication_timestamp", "2026-09-22T00:00:00Z"),
        ("publication_canonical_url", "/hello"),
    ]);
    let read: Publication = read_embedded(&cx, Post::fields().publication(), &published);
    assert_eq!(
        read,
        Publication::Published {
            published_at: "2026-09-22T00:00:00Z".to_string(),
            canonical_url: "/hello".to_string(),
        },
        "a submitted Published payload must infer Published"
    );

    let archived = map(&[
        ("publication_timestamp", "2026-09-22T00:00:00Z"),
        ("publication_reason", "superseded"),
    ]);
    let read: Publication = read_embedded(&cx, Post::fields().publication(), &archived);
    assert_eq!(
        read,
        Publication::Archived {
            archived_at: "2026-09-22T00:00:00Z".to_string(),
            reason: "superseded".to_string(),
        },
        "a submitted Archived payload must infer Archived"
    );

    // A *shared* payload cannot say which variant was meant (it belongs to all
    // three), so on its own it infers nothing: the first variant.
    let shared_only = map(&[("publication_timestamp", "2026-09-22T00:00:00Z")]);
    let read: Publication = read_embedded(&cx, Post::fields().publication(), &shared_only);
    assert_eq!(
        read,
        Publication::Scheduled {
            scheduled_at: "2026-09-22T00:00:00Z".to_string(),
            scheduled_for: String::new(),
        },
        "a shared column never selects a variant"
    );
}

/// A discriminant the submission **names** but the enum does not declare is
/// refused loudly. Reading it as some other variant would store a value the
/// caller never asked for.
#[tokio::test]
#[should_panic(expected = "does not name a variant of Publication")]
async fn an_unknown_discriminant_panics() {
    let cx = post_cx().await;
    let values = map(&[
        ("publication", "99"),
        ("publication_canonical_url", "/hello"),
    ]);
    let _: Publication = read_embedded(&cx, Post::fields().publication(), &values);
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

/// A nested enum contributes its discriminant to the value's form keys: a
/// submission that names only the nested variant has mentioned the value.
#[tokio::test]
async fn a_nested_enum_contributes_its_discriminant() {
    let cx = post_cx().await;

    // The nested enum's discriminant is a key of the value: naming only it
    // mentions the wrapper.
    assert!(
        submitted(
            &cx,
            Post::fields().wrapper(),
            &map(&[("wrapper_inner", "1")])
        ),
        "naming only the nested variant mentions the value"
    );
    assert!(
        !submitted(&cx, Post::fields().wrapper(), &map(&[("title", "x")])),
        "a key outside the value does not mention it"
    );

    let wrapper = Wrapper {
        label: "w".to_string(),
        inner: Media::Image {
            url: "/i.jpg".to_string(),
            alt: "i".to_string(),
        },
    };
    let mut values = HashMap::new();
    write_embedded(&cx, Post::fields().wrapper(), &wrapper, &mut values);
    assert_eq!(
        values,
        map(&[
            ("wrapper_label", "w"),
            ("wrapper_inner", "1"),
            ("wrapper_inner_url", "/i.jpg"),
            ("wrapper_inner_alt", "i"),
        ])
    );
    let read: Wrapper = read_embedded(&cx, Post::fields().wrapper(), &values);
    assert_eq!(read, wrapper);
}

/// Variant idents the schema normalises still round-trip: the codec addresses
/// variants by declaration index, not by a name it would have to re-derive.
#[tokio::test]
async fn variant_casing_needs_no_normalisation() {
    let cx = post_cx().await;

    for casing in [
        Casing::OK {
            at: "now".to_string(),
        },
        Casing::Draft,
    ] {
        let mut values = HashMap::new();
        write_embedded(&cx, Post::fields().casing(), &casing, &mut values);
        let read: Casing = read_embedded(&cx, Post::fields().casing(), &values);
        assert_eq!(read, casing, "wrote {values:?}");
    }

    // And the derived form renders (a name mismatch would panic here).
    let html = Schema::new(Casing::form(&cx, Post::fields().casing()))
        .render_with(&cx, &HashMap::new(), &HashMap::new())
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(html.contains("name=\"casing\""), "got {html}");
    assert!(html.contains("name=\"casing_at\""), "got {html}");
    // The label is the *normalised* name — `OK` reads `Ok` — which is exactly
    // why the codec addresses variants by index and the label is never a
    // handle: a normalized name need not round-trip.
    assert_eq!(
        variant_option_labels(&html),
        vec!["-- Select --", "Ok", "Draft"],
        "got {html}"
    );
}

/// `bool` and the wider integer types are leaves too (GH #191 widened
/// `TypedValue` to the whole family the panel can spell).
#[tokio::test]
async fn typed_leaves_cover_bool_and_the_integer_family() {
    let cx = post_cx().await;
    let flags = Flags {
        featured: true,
        level: 3,
        revision: 7,
    };

    let mut values = HashMap::new();
    write_embedded(&cx, Post::fields().flags(), &flags, &mut values);
    assert_eq!(
        values,
        map(&[
            ("flags_featured", "true"),
            ("flags_level", "3"),
            ("flags_revision", "7"),
        ])
    );
    let read: Flags = read_embedded(&cx, Post::fields().flags(), &values);
    assert_eq!(read, flags);

    // A bad `bool` is refused by the typed control before a record fn runs, so
    // the codec only ever sees a spelling the type accepts.
    let bad = map(&[("flags_featured", "yes")]);
    assert!(
        Schema::new(Flags::form(&cx, Post::fields().flags()))
            .validate(&bad)
            .contains_key("flags_featured"),
        "the derived control validates its own type"
    );
}

/// The generated form: the variant `Select`, every variant's payload in its own
/// marked group, and no control at all in view mode.
#[tokio::test]
async fn the_derived_form_renders_the_variant_select_and_every_payload() {
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

    let html = render_form(&cx, &schema, &values).await;

    assert!(
        html.contains("<select") && html.contains("name=\"publication\""),
        "the discriminant must ride the form as a visible control, got {html}"
    );
    assert!(
        html.contains("data-variant-select=\"publication\""),
        "the control must name the groups it drives, got {html}"
    );
    assert!(
        html.contains("value=\"3\" selected"),
        "the stored variant hydrates as the selected option, got {html}"
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

    // Read-only: the control itself must not render, and the page names the
    // stored variant instead of printing the machine value it stores
    // (ADR-0016) — the payload rows are every variant's, so the name is what
    // says which state the record is in.
    let view = render_view(&cx, &schema, &values).await;
    assert!(
        !view.contains("<select") && !view.contains("<input"),
        "a variant control must not render in view mode, got {view}"
    );
    assert!(
        view.contains(">Publication<") && view.contains(">Archived<"),
        "the view must name the stored variant, got {view}"
    );
    assert!(
        !view.contains(">3<"),
        "the view must print the variant's name, never its discriminant, got {view}"
    );

    // A record with no stored variant has no name to show: the row is absent
    // rather than blank — the pre-#191 hidden control rendered nothing either.
    let unnamed = render_view(&cx, &schema, &HashMap::new()).await;
    assert!(
        !unnamed.contains(">Publication<"),
        "an unset variant must not render a row, got {unnamed}"
    );
}

/// The form's HTML, hydrated with `values` (empty for a create form).
async fn render_form(cx: &Cx, schema: &Schema, values: &HashMap<String, String>) -> String {
    schema
        .render_with(cx, values, &HashMap::new())
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(cx)
}

/// The form's read-only HTML, hydrated with `values`.
async fn render_view(cx: &Cx, schema: &Schema, values: &HashMap<String, String>) -> String {
    schema
        .render_readonly(cx, values)
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(cx)
}

/// Every `data-variant="…"` value in `html`, in document order.
///
/// The marker's own attribute — the `="` is what tells it from
/// `data-variant-of` and `data-variant-select`, whose prefixes it shares.
fn variant_markers(html: &str) -> Vec<String> {
    html.match_indices("data-variant=\"")
        .map(|(at, needle)| {
            let rest = &html[at + needle.len()..];
            rest[..rest.find('"').expect("a closed marker")].to_string()
        })
        .collect()
}

/// The variant `Select`'s own markup: from its hook to the first `</select>`.
fn variant_select(html: &str) -> &str {
    html.split_once("data-variant-select=")
        .and_then(|(_, rest)| rest.split_once("</select>").map(|(select, _)| select))
        .expect("a variant select")
}

/// The `value="…"` of every option in the first variant `Select` of `html`, in
/// document order — the placeholder first, then one per variant.
fn variant_options(html: &str) -> Vec<String> {
    let select = variant_select(html);
    select
        .match_indices("value=\"")
        .map(|(at, needle)| {
            let rest = &select[at + needle.len()..];
            rest[..rest.find('"').expect("a closed value")].to_string()
        })
        .collect()
}

/// The text of every option in the first variant `Select` of `html`, in
/// document order — what a person reads, the placeholder included.
fn variant_option_labels(html: &str) -> Vec<String> {
    let select = variant_select(html);
    select
        .match_indices("<option")
        .map(|(at, _)| {
            let option = &select[at..];
            let text = option.find('>').expect("a closed option tag") + 1;
            let end = option[text..].find("</option>").expect("a closed option");
            option[text..text + end].to_string()
        })
        .collect()
}

/// GH #191: one marked group per variant, and the marker set **is** the
/// schema's variant list — so a variant added to the enum cannot silently lose
/// its group, and no group can name a variant the schema does not declare.
///
/// The markers are the discriminant values the `Select` offers, which is what
/// makes `variant.js` able to compare them with the submitted value — so the
/// control's options are asserted against the same list, values **and** labels:
/// an option labelled with the value it submits (`3`) is the hidden input made
/// clickable, not a variant a person can choose.
#[tokio::test]
async fn the_variant_groups_are_exactly_the_schemas_variants() {
    let cx = post_cx().await;
    let spec = enum_spec(&cx, Post::fields().publication()).expect("an embedded enum");
    let html = render_form(
        &cx,
        &Schema::new(Publication::form(&cx, Post::fields().publication())),
        &HashMap::new(),
    )
    .await;

    let expected: Vec<String> = (0..spec.len())
        .map(|index| spec.value_of_index(index).expect("declared").to_string())
        .collect();
    assert_eq!(
        variant_markers(&html),
        expected,
        "one group per variant, in declaration order, got {html}"
    );
    assert_eq!(
        html.matches("data-variant-of=\"publication\"").count(),
        spec.len(),
        "every group must name the enum it belongs to, got {html}"
    );
    let mut offered = variant_options(&html);
    assert_eq!(
        offered.first().map(String::as_str),
        Some(""),
        "the control opens on the empty choice — the create form has no stored \
         variant — so the marker values follow it, got {html}"
    );
    offered.remove(0);
    assert_eq!(
        offered, expected,
        "the control must offer every variant, and only those, in declaration \
         order: the marker a group carries is the option that shows it, got {html}"
    );

    let expected_names: Vec<String> = (0..spec.len())
        .map(|index| spec.name_of_index(index).expect("declared").to_string())
        .collect();
    assert_eq!(
        expected_names,
        vec!["Scheduled", "Published", "Archived"],
        "the schema's variant names are what the labels are built from"
    );
    let mut labels = variant_option_labels(&html);
    assert_eq!(
        labels.first().map(String::as_str),
        Some("-- Select --"),
        "the empty choice reads as prose too, got {html}"
    );
    labels.remove(0);
    assert_eq!(
        labels, expected_names,
        "every option reads as its variant's name, got {html}"
    );
}

/// A unit variant has no payload but still gets its group: the marker set is
/// the variant list, so the one variant with nothing to show cannot be the one
/// that loses its marker. Its option is labelled like any other.
#[tokio::test]
async fn a_unit_variant_still_gets_its_group() {
    let cx = post_cx().await;
    let html = render_form(
        &cx,
        &Schema::new(Visibility::form(&cx, Post::fields().visibility())),
        &HashMap::new(),
    )
    .await;
    assert_eq!(variant_markers(&html), vec!["1", "2"], "got {html}");
    assert_eq!(
        html.matches("data-variant-of=\"visibility\"").count(),
        2,
        "got {html}"
    );
    assert_eq!(
        variant_option_labels(&html),
        vec!["-- Select --", "Public", "Private"],
        "a unit variant is still named in the chooser, got {html}"
    );
}

/// Each variant's own payload renders **inside** its own group, and a
/// `#[shared(..)]` column renders once outside every group: the marker is what
/// `variant.js` toggles, so a leaf outside it would show for the wrong variant,
/// and a shared column inside one would vanish for the other two.
#[tokio::test]
async fn each_variants_payload_sits_in_its_own_group() {
    let cx = post_cx().await;
    let html = render_form(
        &cx,
        &Schema::new(Publication::form(&cx, Post::fields().publication())),
        &HashMap::new(),
    )
    .await;

    let group_at = |marker: &str| {
        html.find(&format!("data-variant=\"{marker}\""))
            .unwrap_or_else(|| panic!("no group for variant {marker} in {html}"))
    };
    let bounds = [
        ("1", "publication_scheduled_for"),
        ("2", "publication_canonical_url"),
        ("3", "publication_reason"),
    ];
    for (index, (marker, leaf)) in bounds.iter().enumerate() {
        let start = group_at(marker);
        let end = bounds
            .get(index + 1)
            .map(|(next, _)| group_at(next))
            .unwrap_or(html.len());
        let leaf_at = html
            .find(leaf)
            .unwrap_or_else(|| panic!("no {leaf} in {html}"));
        assert!(
            start < leaf_at && leaf_at < end,
            "{leaf} must render inside variant {marker}'s group, got {html}"
        );
    }

    let shared_at = html
        .find("publication_timestamp")
        .expect("the shared column");
    assert!(
        shared_at < group_at("1"),
        "a shared column belongs to every variant, so it renders outside the \
         groups, got {html}"
    );
}
