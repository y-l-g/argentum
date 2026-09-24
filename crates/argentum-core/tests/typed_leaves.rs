//! Typed leaves at the form edge (GH #192): a lens whose leaf is not a
//! `String`, read and written through the type's own spelling.

use std::collections::HashMap;

use argentum_core::schema::{Schema, TextInput};
use toasty::Db;
use topcoat::{
    context::{Cx, CxTestBuilder},
    view::ViewExt,
};

#[derive(Debug, toasty::Model)]
struct Measurement {
    #[key]
    #[auto]
    id: uuid::Uuid,
    label: String,
    word_count: i64,
    recorded_at: jiff::Timestamp,
}

async fn cx() -> Cx {
    let db = Db::builder()
        .models(toasty::models!(Measurement))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    CxTestBuilder::new().app_context(db).build()
}

/// A typed field bound to a lens of the wrong type must not compile, which is
/// the guarantee `r#for` carries and `typed` has to keep. There is no way to
/// assert "does not compile" in a passing test, so the positive half is what is
/// pinned: the leaf's own type compiles.
#[tokio::test]
async fn a_typed_field_renders_the_values_display() {
    let cx = cx().await;
    let schema = Schema::new((
        TextInput::typed::<Measurement, i64>(Measurement::fields().word_count()),
        TextInput::typed::<Measurement, jiff::Timestamp>(Measurement::fields().recorded_at()),
    ));
    let values = HashMap::from([
        ("word_count".to_string(), "1240".to_string()),
        (
            "recorded_at".to_string(),
            "2024-01-02T03:04:05Z".to_string(),
        ),
    ]);
    let html = schema
        .render_with(&cx, &values, &HashMap::new())
        .await
        .unwrap()
        .single()
        .await
        .unwrap()
        .render(&cx);
    assert!(
        html.contains("value=\"1240\""),
        "an integer field renders its value: {html}"
    );
    // Not a `datetime-local`: the field type declares the control (GH #192).
    assert!(
        html.contains("type=\"text\"") && !html.contains("datetime-local"),
        "a typed timestamp is still a text input: {html}"
    );
    assert!(html.contains("Word count"), "label from the lens: {html}");
}

#[tokio::test]
async fn a_bad_submission_is_an_inline_field_error() {
    let schema = Schema::new((
        TextInput::typed::<Measurement, i64>(Measurement::fields().word_count()),
        TextInput::typed::<Measurement, jiff::Timestamp>(Measurement::fields().recorded_at()),
    ));
    let values = HashMap::from([
        ("word_count".to_string(), "lots".to_string()),
        ("recorded_at".to_string(), "2024-13-01".to_string()),
    ]);
    let errors = schema.validate(&values);
    assert_eq!(
        errors.get("word_count"),
        Some(&vec!["`lots` is not a valid whole number".to_string()]),
        "an unparseable integer names the offending input, got {errors:?}"
    );
    assert_eq!(
        errors.get("recorded_at"),
        Some(&vec!["`2024-13-01` is not a valid timestamp".to_string()]),
        "an unparseable date names the offending input, got {errors:?}"
    );
}

#[tokio::test]
async fn a_valid_submission_is_stored_in_the_types_spelling() {
    let schema = Schema::new(TextInput::typed::<Measurement, jiff::Timestamp>(
        Measurement::fields().recorded_at(),
    ));
    // A spelling the browser may send that is not what `Display` produces.
    let mut values = HashMap::from([(
        "recorded_at".to_string(),
        "2024-01-02T03:04:05+00:00[UTC]".to_string(),
    )]);
    assert!(
        schema.validate(&values).is_empty(),
        "the parser accepts what the type accepts"
    );
    schema.normalize_values(&mut values);
    let stored = values.get("recorded_at").expect("still present");
    let parsed: jiff::Timestamp = stored.parse().expect("the stored spelling parses");
    assert_eq!(
        parsed.to_string(),
        *stored,
        "what is written is the type's own Display, so a re-read is a fixpoint"
    );
}

#[tokio::test]
async fn an_empty_submission_stays_the_presence_rules_business() {
    let schema = Schema::new(
        TextInput::typed::<Measurement, i64>(Measurement::fields().word_count()).optional(),
    );
    assert!(
        schema.validate(&HashMap::new()).is_empty(),
        "an optional typed field accepts empty, as a text field does"
    );
    let required = Schema::new(TextInput::typed::<Measurement, i64>(
        Measurement::fields().word_count(),
    ));
    assert_eq!(
        required
            .validate(&HashMap::new())
            .get("word_count")
            .cloned()
            .unwrap_or_default(),
        vec!["Word count is required".to_string()],
        "a non-nullable typed field reports presence, not a parse failure"
    );
}

#[tokio::test]
async fn a_text_field_is_untouched_by_the_typed_path() {
    let schema = Schema::new(TextInput::r#for(Measurement::fields().label()));
    let mut values = HashMap::from([("label".to_string(), "  spaced  ".to_string())]);
    assert!(schema.validate(&values).is_empty());
    schema.normalize_values(&mut values);
    assert_eq!(
        values.get("label").map(String::as_str),
        Some("spaced"),
        "a text field still stores exactly what was typed, trimmed"
    );
}

/// The whole point of the typed seam (GH #192): a bad value typed into a typed
/// column is a **field error on the page**, not a 500 and not a silent default.
///
/// Pinned through the real panel rather than `Schema::validate`, because the
/// requirement is about what a user sees: the earlier tests prove the rule, this
/// proves the wiring — that the create handler reaches it and re-renders inline.
#[tokio::test]
async fn a_bad_typed_submission_re_renders_inline_and_writes_nothing() {
    use argentum_core::{Auth, Panel, Resource};
    use topcoat::router::{Body, Router};

    #[derive(Debug, toasty::Model, Clone)]
    struct Reading {
        #[key]
        #[auto]
        id: uuid::Uuid,
        word_count: i64,
    }
    struct ReadingResource;
    impl Resource for ReadingResource {
        type Model = Reading;
        fn slug() -> String {
            "readings".to_string()
        }
        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_create(_cx: &Cx) -> bool {
            true
        }
        fn table(cx: &Cx) -> argentum_core::Table<Reading> {
            argentum_core::Table::r#for(cx)
                .id(|r: &Reading| r.id.to_string())
                // The list renders the integer through a computed column: a
                // lens-bound column takes `Path<M, String>`, the same
                // compile-time rule the typed field constructor respects.
                .columns(argentum_core::TextColumn::computed(
                    "Words",
                    |r: &Reading| r.word_count.to_string(),
                ))
        }
        fn form(_cx: &Cx) -> Schema {
            Schema::new(TextInput::typed::<Reading, i64>(
                Reading::fields().word_count(),
            ))
        }
        async fn create_record(
            _cx: &Cx,
            values: HashMap<String, String>,
            ex: &mut dyn toasty::Executor,
        ) -> topcoat::Result<Reading> {
            // A create returns the row it wrote: that is what the framework
            // hands to `after_commit` (GH #112).
            toasty::create!(Reading {
                word_count: values
                    .get("word_count")
                    .expect("validated")
                    .parse::<i64>()
                    .expect("a typed field is validated before the record fn"),
            })
            .exec(ex)
            .await
            .map_err(|error| -> topcoat::Error { error.into() })
        }
    }

    let db = Db::builder()
        .models(toasty::models!(Reading))
        .connect("sqlite::memory:")
        .await
        .unwrap();
    db.push_schema().await.unwrap();
    let router: Router = Panel::new("admin")
        .app_context(db.clone())
        .resource::<ReadingResource>()
        .auth(Auth::disabled())
        .build()
        .expect("panel builds");

    let csrf = uuid::Uuid::new_v4().to_string();
    let resp = router
        .handle(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/admin/readings/create")
                .header(
                    http::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .header(
                    http::header::COOKIE,
                    format!("{}={csrf}", argentum_core::csrf::COOKIE_NAME),
                )
                .body(Body::from(format!("word_count=lots&csrf_token={csrf}")))
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        http::StatusCode::OK,
        "a bad typed value re-renders the form, it does not 500"
    );
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("`lots` is not a valid whole number"),
        "the field carries the parse error inline: {html}"
    );
    assert!(
        html.contains("value=\"lots\""),
        "the control keeps what the user typed so they can fix it: {html}"
    );

    let mut db_check = db;
    let stored = Reading::all().exec(&mut db_check).await.unwrap();
    assert!(
        stored.is_empty(),
        "a rejected submission writes nothing, got {} rows",
        stored.len()
    );
}

/// Empty is the presence rule's business, not the typed rule's (GH #192).
///
/// A typed column has no spelling for "no value" — `""` is not an `i64` and not
/// a `Timestamp` — so the panel answers empty where it answers it everywhere:
/// `.required()` refuses it inline, and an optional typed field reaches its
/// record fn as `""`, which the record fn defaults exactly as it would for any
/// other optional column. Normalisation therefore leaves an empty submission
/// alone rather than inventing a value the user never gave.
#[tokio::test]
async fn an_empty_submission_is_left_for_the_record_fn_to_default() {
    let schema = Schema::new(
        TextInput::typed::<Measurement, i64>(Measurement::fields().word_count()).optional(),
    );
    let mut values = HashMap::from([("word_count".to_string(), String::new())]);
    assert!(
        schema.validate(&values).is_empty(),
        "an optional typed field accepts an empty submit"
    );
    schema.normalize_values(&mut values);
    assert_eq!(
        values.get("word_count").map(String::as_str),
        Some(""),
        "empty stays empty: a typed column has no 'no value' spelling"
    );
}

/// Timezone and precision survive a round-trip (GH #192 gotcha).
///
/// A `Display`/`FromStr` pair that drops the offset or truncates sub-second
/// precision corrupts data on an edit the user never touched.
#[tokio::test]
async fn a_timestamp_round_trips_offset_and_subsecond_precision() {
    let schema = Schema::new(TextInput::typed::<Measurement, jiff::Timestamp>(
        Measurement::fields().recorded_at(),
    ));
    for input in [
        "2024-01-02T03:04:05.123456789Z",
        "2024-01-02T03:04:05+05:30",
        "2024-06-30T23:59:59Z",
    ] {
        let mut values = HashMap::from([("recorded_at".to_string(), input.to_string())]);
        assert!(schema.validate(&values).is_empty(), "{input} must validate");
        schema.normalize_values(&mut values);
        let stored = values.get("recorded_at").cloned().unwrap_or_default();
        let parsed: jiff::Timestamp = stored
            .parse()
            .unwrap_or_else(|e| panic!("{stored:?} must parse back: {e}"));
        assert_eq!(
            parsed.to_string(),
            stored,
            "the stored spelling is the type's fixpoint for {input}"
        );
        let original: jiff::Timestamp = input.parse().expect("input parses");
        assert_eq!(
            original, parsed,
            "no instant is lost between {input} and {stored}"
        );
    }
}
