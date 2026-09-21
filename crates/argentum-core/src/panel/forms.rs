//! Form decoding (urlencoded + streamed multipart) and create/edit handlers.
//!
//! Decoding helpers stay pure and request-free where possible so the size
//! caps and filename sanitization are unit-testable at the boundary.

use std::collections::HashMap;

use topcoat::view::internal::ThenView;
use topcoat::{
    Result,
    context::Cx,
    router::{
        Body,
        error::{forbidden, see_other},
        request::{Bytes, FromRequest},
    },
    view::{BoxView, HoistView, ViewExt, attributes, view},
};

use super::actions::find_by_key;
use super::{enforce_auth, enforce_tenant, list_url};
use crate::db::db;
use crate::notification::{Notification, notify_write_failure, set_notification};

/// Failure-toast wording for the create/update handlers (GH #174): one place,
/// so the two paths cannot drift.
const WRITE_CREATE: &str = "create the record";
const WRITE_UPDATE: &str = "save the changes";
use crate::resource::Resource;

/// Helper: parse form bodies into a map — `application/x-www-form-urlencoded`
/// buffered, plus `multipart/form-data` streamed when a `FileUpload` is present
/// (GH #73).
///
/// Decoding for urlencoded is delegated to `form_urlencoded` (already in the
/// tree via topcoat): it splits pairs, decodes `+` as space, assembles
/// multi-byte UTF-8 from `%XX` sequences (`%C3%A9` → `é`, not `Ã©`), and keeps
/// encoded separators (`%26` → `&`) intact — the hand-rolled `percent_decode`
/// it replaced pushed each decoded byte through `byte as char`, corrupting
/// every non-ASCII value (GH #75 item 6). Invalid UTF-8 degrades per-value
/// (lossy) instead of discarding the whole form.
///
/// Multipart (file) parts stream through Topcoat's multer-based extractor
/// (GH #90): file bytes are drained in chunks and discarded — v1 stores the
/// sanitized filename as the `String` value, never the bytes (see
/// `FileUpload` storage contract) — so a 2 GB "upload" never materializes.
/// Text parts store their content. Unknown content types fall back to
/// urlencoded so existing tests/clients keep working.
pub(crate) async fn parse_form_values(
    cx: &Cx,
    body: Body,
) -> Result<HashMap<String, String>, topcoat::Error> {
    let content_type =
        topcoat::context::try_request_context::<http::request::Parts>(cx).and_then(|parts| {
            parts
                .headers
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
        });
    if content_type
        .as_deref()
        .is_some_and(is_multipart_content_type)
    {
        return parse_multipart_values(cx, body).await;
    }
    let bytes = Bytes::from_request(cx, body)
        .await
        .map_err(|_| topcoat::router::error::bad_request("cannot read form body"))?;
    form_values_from_request_parts(content_type.as_deref(), bytes.as_ref())
}

/// Whether a content type is `multipart/form-data` (parameters ignored).
fn is_multipart_content_type(ct: &str) -> bool {
    ct.split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
}

/// Streamed multipart half of [`parse_form_values`] (GH #90).
///
/// Fields stream one at a time with constant memory: text fields buffer
/// (bounded by the request body limit), file fields drain-and-discard while
/// only the sanitized filename is kept. Duplicate part names are last-wins;
/// nameless parts are skipped. A missing boundary is a 400, an over-limit
/// body a 413 — both classified by the extractor, never silent fallbacks.
/// Every byte the stream carries is also counted against [`MAX_FORM_BYTES`]
/// (GH #149): file drains and skipped parts go through the counter chunk by
/// chunk, text fields join it after their (extractor-bounded) read, so a
/// large upload cannot be drained chunk-by-chunk holding the handler even if
/// the extractor's limit stops wrapping the stream.
async fn parse_multipart_values(
    cx: &Cx,
    body: Body,
) -> Result<HashMap<String, String>, topcoat::Error> {
    use topcoat::router::content::multipart::Multipart;
    use topcoat::router::request::FromRequest;

    let mut out = HashMap::new();
    let mut bytes_seen = 0usize;
    let mut multipart = Multipart::from_request(cx, body).await?;
    while let Some(mut field) = multipart.next_field().await? {
        let Some(name) = field.name().map(str::to_string) else {
            // Nameless parts carry bytes too: drain them through the counter
            // so the accounting covers the whole request stream (GH #149).
            drain_bounded(&mut field, &mut bytes_seen).await?;
            continue;
        };
        if name.is_empty() {
            drain_bounded(&mut field, &mut bytes_seen).await?;
            continue;
        }
        // RFC 6266: `filename*=` (decoded) takes precedence over `filename=`.
        // Multer surfaces the plain `filename=` first, so the raw header is
        // read for `filename*=` before falling back (GH #90).
        let filename =
            filename_star_from_headers(&field).or_else(|| field.file_name().map(str::to_string));
        match filename {
            Some(f) if !f.is_empty() => {
                // v1 stores the sanitized basename, not the bytes
                // (FileUpload contract, GH #90): drain to advance the stream.
                drain_bounded(&mut field, &mut bytes_seen).await?;
                out.insert(name, sanitize_filename(&f));
            }
            Some(_) => {
                // Empty filename (no file chosen) → empty value so `required`
                // validation fires instead of treating it as missing.
                drain_bounded(&mut field, &mut bytes_seen).await?;
                out.insert(name, String::new());
            }
            None => {
                // Text fields buffer (extractor-bounded); the read joins the
                // same counter so the backstop sees the per-request total.
                let text = field.text().await?;
                count_form_bytes(&mut bytes_seen, text.len())?;
                out.insert(name, text);
            }
        }
    }
    Ok(out)
}

/// Drain one multipart field chunk-by-chunk, accounting every byte against
/// [`MAX_FORM_BYTES`] (GH #149): enforcement normally happens in the
/// extractor (`BodyLimit` wraps the multipart stream), but the drain owns its
/// own counter so an over-cap upload 413s here too instead of holding the
/// handler.
async fn drain_bounded(
    field: &mut topcoat::router::content::multipart::Field<'_>,
    bytes_seen: &mut usize,
) -> Result<(), topcoat::Error> {
    while let Some(chunk) = field.chunk().await? {
        count_form_bytes(bytes_seen, chunk.len())?;
    }
    Ok(())
}

/// Account one drained chunk against the form-body cap (GH #149). Extracted
/// so the 413 mapping is testable at the boundary without building a
/// multipart body — through the router the extractor's own limit classifies
/// the same body first, so the counter only answers when that limit stops
/// applying (defense-in-depth alongside `BodyLimit`, never a competing cap).
fn count_form_bytes(bytes_seen: &mut usize, chunk_len: usize) -> Result<(), topcoat::Error> {
    // checked_add: the accumulator cannot overflow a usize at real chunk
    // sizes, but wrapping would silently disable the cap in release builds.
    let Some(total) = bytes_seen.checked_add(chunk_len) else {
        return Err(topcoat::router::error::content_too_large().into());
    };
    *bytes_seen = total;
    if *bytes_seen > MAX_FORM_BYTES {
        return Err(topcoat::router::error::content_too_large().into());
    }
    Ok(())
}

/// RFC 5987 `filename*=` from a field's raw `Content-Disposition` header.
fn filename_star_from_headers(
    field: &topcoat::router::content::multipart::Field<'_>,
) -> Option<String> {
    let raw = field
        .headers()
        .get(http::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    raw.split(';').find_map(|seg| {
        let seg = seg.trim();
        seg.get(..10)
            .filter(|h| h.eq_ignore_ascii_case("filename*="))
            .and_then(|_| decode_rfc5987(seg[10..].trim()))
    })
}

/// Pure urlencoded half of [`parse_form_values`] (GH #90) — testable without
/// a request. Rejects bodies over `MAX_FORM_BYTES` with 413. Multipart
/// never reaches here: it streams via [`parse_multipart_values`], where a
/// missing boundary is a 400 and an over-limit body a 413 (both classified
/// by the extractor).
///
/// Kept as a second layer on purpose (GH #134, defense in depth): through the
/// router this branch is unreachable — `Bytes::from_request` buffers via
/// `to_bytes(body, body_limit(cx))`, which enforces the
/// `BodyLimit::max(MAX_FORM_BYTES)` layer [`Panel::build`](crate::panel::Panel::build)
/// installs before these bytes can exist — but the explicit check is the
/// urlencoded symmetric backstop to the multipart [`count_form_bytes`]
/// counter (GH #149), and the only pin of the 10 MiB urlencoded contract at
/// unit level (a bare `CxTestBuilder` carries no `BodyLimitKind`, so
/// `body_limit(cx)` falls back to Topcoat's 2 MiB default and cannot pin this
/// cap). One length comparison; do not collapse into [`form_values_from_bytes`]
/// without restoring both properties.
fn form_values_from_request_parts(
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<HashMap<String, String>, topcoat::Error> {
    if bytes.len() > MAX_FORM_BYTES {
        return Err(topcoat::router::error::content_too_large().into());
    }
    debug_assert!(
        content_type.is_none_or(|ct| !is_multipart_content_type(ct)),
        "multipart must stream via parse_multipart_values, not buffer here (GH #90)"
    );
    Ok(form_values_from_bytes(bytes))
}

/// Max form/multipart body accepted (GH #90): 10 MiB. v1 keeps filenames only.
pub(crate) const MAX_FORM_BYTES: usize = 10 * 1024 * 1024;

/// Sanitize a client-supplied filename to a basename (GH #90).
///
/// Strips directory components (`../../etc/passwd` → `passwd`,
/// `/abs/path` → `path`, `C:\fakepath\x` → `x`), trims whitespace, drops
/// control chars, and caps length at 255 bytes. Empty stays empty so
/// `required` validation fires.
///
/// Names that could never be a safely persisted file are rejected to empty
/// (GH #149, before persistence lands): `.` and `..`, and Windows reserved
/// device names (`con`, `nul`, `com1` — also with an extension, and
/// case-insensitive). v1 stores only the basename `String` and never touches
/// the filesystem, so today this is latent; the required validation then
/// surfaces the empty value as an inline form error (on edit, the
/// untouched-file backfill preserves the stored value instead — an
/// explicitly rejected name falls back to "keep", GH #90).
fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "." || trimmed == ".." || is_windows_reserved_name(trimmed) {
        return String::new();
    }
    // Cap at 255 bytes (common filename limit), preserving the tail. The cut
    // point is walked forward to a char boundary: slicing a multibyte char
    // would panic (a >255-byte non-ASCII filename is attacker-controlled).
    if trimmed.len() > 255 {
        let mut start = trimmed.len() - 255;
        while !trimmed.is_char_boundary(start) {
            start += 1;
        }
        trimmed[start..].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Windows reserved device names (GH #149): the stem before the first dot is
/// reserved case-insensitively — `con`, `nul`, `aux`, `prn`, `com1`–`com9`,
/// `lpt1`–`lpt9` — so `con.txt` cannot become a persisted basename either.
fn is_windows_reserved_name(name: &str) -> bool {
    let stem = match name.split_once('.') {
        Some((stem, _)) => stem,
        None => name,
    };
    let stem = stem.to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let Some(n) = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
    else {
        return false;
    };
    n.parse::<u8>().is_ok_and(|n| (1..=9).contains(&n))
}

/// Decode an RFC 5987/6266 `filename*=UTF-8''...` value (GH #90).
///
/// Only UTF-8 is supported; other charsets yield `None` so the caller falls
/// back to `filename=`. Malformed percent sequences fail the whole value
/// rather than lossy-mangling the stored name.
fn decode_rfc5987(value: &str) -> Option<String> {
    let (charset, rest) = value.split_once('\'')?;
    let (_lang, encoded) = rest.split_once('\'')?;
    if !charset.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Pure half of [`parse_form_values`] — testable without a request.
fn form_values_from_bytes(bytes: &[u8]) -> HashMap<String, String> {
    form_urlencoded::parse(bytes).into_owned().collect()
}

/// Shared create/edit page shell (GH #73 multipart enctype, CSRF hidden
/// input, inline error slot). Title and submit label are the only deltas.
async fn render_form_page<'a, R: Resource>(
    cx: &'a Cx,
    title: String,
    submit_label: &'static str,
    values: &HashMap<String, String>,
    errors: &HashMap<String, Vec<String>>,
) -> Result<BoxView<'a>> {
    let schema = R::form(cx);
    let form_html = schema.render_with(cx, values, errors).await?;
    let action = topcoat::router::request::uri(cx).path().to_string();
    // Browsers only send `<input type="file">` content as multipart (GH #73).
    let enctype: Option<String> = schema
        .has_file_upload()
        .then(|| "multipart/form-data".to_string());
    let csrf = crate::csrf::current_token(cx);
    Ok(view! {
        cx =>
        argentum_ui::page(
            argentum_ui::page_header(argentum_ui::page_title((title.clone())))
            argentum_ui::page_content(
                <form
                    method="post"
                    action=(action)
                    enctype=(enctype)
                    class="flex flex-col gap-4"
                >
                    <input type="hidden" name="csrf_token" value=(csrf)>
                    (form_html)
                    <div class="flex gap-2">
                        argentum_ui::button(
                            variant: argentum_ui::ButtonVariant::Primary,
                            attrs: attributes! { type="submit" },
                            (submit_label)
                        )
                        <a
                            href=(list_url(cx, &R::slug()))
                            class=(argentum_ui::button_variants(
                                argentum_ui::ButtonVariant::Outline,
                                argentum_ui::ButtonSize::Md,
                            ))
                        >
                            "Cancel"
                        </a>
                    </div>
                </form>
            )
        )
    }
    .boxed())
}

/// Create page GET.
pub(crate) fn resource_create<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_create(cx) {
            return Err(forbidden().into());
        }
        crate::csrf::ensure_token(cx);
        let html = render_form_page::<R>(
            cx,
            format!("Create {}", R::navigation_label()),
            "Create",
            &HashMap::new(),
            &HashMap::new(),
        )
        .await?;
        Ok(html)
    })))
}

/// Reject POST keys no declared Schema input owns (GH #89 mass-assignment
/// allow-list). `csrf_token` is a handler key, not a field, so it is filtered
/// before the check, as are `clear_<field>` flags for declared `FileUpload`
/// fields (GH #90 explicit-clear convention — `truthy`, GH #148); absent keys are fine
/// (present-keys-only updates), unknown keys are a 400 — silently ignoring
/// `role`/`tenant_id` smuggling is what the old code did, and a generic
/// record fn iterating `values` would promote them to client-controlled
/// writes.
fn reject_unknown_form_keys(
    schema: &crate::schema::Schema,
    values: &HashMap<String, String>,
) -> Result<(), topcoat::Error> {
    // One transport-key vocabulary (GH #148): the same `strip_transport_keys`
    // the record fns benefit from defines which keys the framework owns, so
    // the allow-list and the strip cannot drift apart.
    let mut filtered = values.clone();
    strip_transport_keys(schema, &mut filtered);
    let unknown = schema.unknown_keys(&filtered);
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(topcoat::router::error::bad_request(format!(
            "unknown field(s): {}",
            unknown.join(", ")
        ))
        .into())
    }
}

/// The one boolean-vocabulary check for framework form flags (GH #148):
/// `confirm=1|true`, `clear_<field>=1|true`. `yes` used to be a delete-only
/// extra; one vocabulary instead of two per-handler sets.
pub(crate) fn truthy(v: &str) -> bool {
    v == "1" || v == "true"
}

/// Strip framework transport keys from the submitted values before any
/// record fn sees them (GH #148): `csrf_token` and the `clear_<field>`
/// flags are handler keys, not writable fields — a generic `Resource` impl
/// iterating `values` (the exact threat model in the `unknown_keys` docs)
/// must not receive them as writes. The framework strips once here, not
/// per-app convention.
fn strip_transport_keys(schema: &crate::schema::Schema, values: &mut HashMap<String, String>) {
    let declared: std::collections::HashSet<String> = schema.field_names().into_iter().collect();
    // A schema field literally named `csrf_token` (or `clear_<upload>`) is a
    // misconfiguration that would silently swallow its own value here — the
    // declared-name check keeps such a field's value flowing (the collision
    // is a build-time bug, not a transport key).
    values.retain(|k, _| {
        if k == crate::csrf::FIELD_NAME {
            return declared.contains(k.as_str());
        }
        match k.strip_prefix("clear_") {
            Some(field) if schema.file_uploads().contains_key(field) => {
                declared.contains(k.as_str())
            }
            _ => true,
        }
    });
}

/// Create page POST.
/// App-side uniqueness check over the form's `unique()`-marked text inputs.
///
/// Generic over every marked field — the previous version was hard-coded to
/// `email` with a dead full-table query behind it (GH #75 residue). Queries
/// through `Resource::query` (the tenancy seam, ADR-0002) and returns
/// `field_name → ["<Label> has already been taken"]` per duplicated value.
///
/// `current` holds the record's own hydrated values on edit: a field whose
/// submitted value is unchanged belongs to this record and is skipped.
///
/// Empty submits are never probed (GH #189): a `unique()` field is required
/// (see [`crate::schema::TextInput::unique`]), so `validate` has already
/// answered `"<Label> is required"` and this check has nothing left to say — no
/// query, and no `""` written past an index that admits one.
///
/// Known limits (GH #88, upstream gap #117): races with concurrent
/// inserts (only a driver predicate closes it); the check is tenant-scoped via
/// `R::query` while DB `#[unique]` is global, so cross-tenant duplicates 500 —
/// closing that needs a scoped constraint, not field metadata, which `unique`
/// now derives (GH #183); composite uniques are not covered, and `unique`
/// exists on `TextInput` only.
async fn check_unique<R: Resource>(
    cx: &Cx,
    schema: &crate::schema::Schema,
    values: &HashMap<String, String>,
    current: &HashMap<String, String>,
    ex: &mut dyn toasty::Executor,
) -> Result<HashMap<String, Vec<String>>, topcoat::Error> {
    let mut errors: HashMap<String, Vec<String>> = HashMap::new();
    // Absent Repeater groups are not checked (GH #167): `validate` treats an
    // all-empty group as untouched via the same classification, so a stored
    // `""` must not flag a group the user never touched.
    let skip = schema.absent_repeater_fields(values);
    for (name, input) in schema.text_inputs() {
        if !input.is_unique() || skip.contains(&name) {
            continue;
        }
        let Some(submitted) = values.get(&name).map(|s| s.trim().to_string()) else {
            continue;
        };
        // Empty values are never probed (GH #189): a `unique()` field is
        // required, so validation has already refused this submit — and `""` is
        // still a value the framework stores (never NULL, GH #89), so a probe
        // would only rediscover the constraint the form just enforced.
        if submitted.is_empty() {
            continue;
        }
        // Unchanged on edit → this record's own value, not a duplicate.
        if current.get(&name).map(|s| s.trim().to_string()) == Some(submitted.clone()) {
            continue;
        }
        // Inside the handler's tx (GH #84): the check observes the same
        // snapshot as the write that follows. A failing probe fails the
        // submit (GH #167) — swallowing it would write past a check that
        // never ran.
        let rows = R::query(cx)
            .filter(input.eq_filter::<R::Model>(submitted))
            .limit(1)
            .exec(&mut *ex)
            .await
            .map_err(crate::db::unavailable)?;
        if !rows.is_empty() {
            errors.insert(
                name,
                vec![format!("{} has already been taken", input.label_str())],
            );
        }
    }
    Ok(errors)
}

/// Shared create/edit POST error tail (GH #134): re-render the form with inline
/// errors. Takes the open framework transaction by value and drops it before
/// rendering (GH #84) — the re-rendered form reloads relationship options on
/// its own handle, which would block on the pool while the tx holds it —
/// so the drop is enforced here rather than trusted at each call site.
/// Rendered output is identical to the two inlined copies this replaces.
async fn rerender_invalid_form<'a, R: Resource>(
    cx: &'a Cx,
    tx: toasty::Transaction<'_>,
    title: String,
    submit_label: &'static str,
    values: &HashMap<String, String>,
    errors: &HashMap<String, Vec<String>>,
) -> Result<BoxView<'a>> {
    drop(tx);
    render_form_page::<R>(cx, title, submit_label, values, errors).await
}

/// Shared create/edit POST success tail (GH #134): Post/Redirect/Get with a
/// flash notification. The browser follows with a GET, and the flash cookie
/// rides the error response (Topcoat flushes `Set-Cookie` on `Err` too,
/// topcoat#408). Redirect target and notification are the caller's only
/// deltas, so the redirect behavior cannot drift between create and edit.
fn redirect_after_write<R: Resource>(cx: &Cx, note: &'static str) -> topcoat::Error {
    set_notification(cx, Notification::success(note));
    see_other(list_url(cx, &R::slug())).into()
}

pub(crate) fn resource_create_post<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        if !R::can_create(cx) {
            return Err(forbidden().into());
        }
        let mut values = parse_form_values(cx, body).await?;
        crate::csrf::verify(cx, &values)?;
        let schema = R::form(cx);
        reject_unknown_form_keys(&schema, &values)?;
        // Transport keys never reach the record fn (GH #148): a generic impl
        // iterating `values` must not see `csrf_token`/`clear_*` as writable
        // fields — the framework strips them once, not per-app convention.
        strip_transport_keys(&schema, &mut values);
        let mut errors = schema.validate_async(cx, &values).await;
        // Framework-owned transaction (GH #84): opened only after
        // validation — `validate_async` relationship loaders run on their
        // own handle, which would block on the pool while the tx holds it
        // (see `db` pool discipline). The unique check and the write then
        // observe one snapshot and commit atomically. Dropping `tx`
        // without commit (validation errors, policy denials) rolls back.
        let mut db = db(cx);
        let mut tx = db.transaction().await.map_err(crate::db::unavailable)?;
        // App-side unique check over every `unique()`-marked input — the only
        // error layer until toasty exposes a unique-violation predicate
        // (upstream gap #117; never string-match driver error messages).
        for (name, errs) in
            check_unique::<R>(cx, &schema, &values, &HashMap::new(), &mut tx).await?
        {
            errors.entry(name).or_default().extend(errs);
        }
        if !errors.is_empty() {
            return rerender_invalid_form::<R>(
                cx,
                tx,
                format!("Create {}", R::navigation_label()),
                "Create",
                &values,
                &errors,
            )
            .await;
        }
        // Attempt creation via Resource hook, inside the tx.
        match R::create_record(cx, values.clone(), &mut tx).await {
            Ok(()) => match tx.commit().await {
                Ok(()) => Err(redirect_after_write::<R>(cx, "Created")),
                Err(error) => {
                    notify_write_failure(cx, WRITE_CREATE);
                    Err(crate::db::unavailable(error))
                }
            },
            // A unique violation that slipped past the app-side check (a
            // concurrent insert) surfaces as an error, not a string-matched
            // inline message (upstream gap #117) — and now with a toast.
            Err(e) => {
                notify_write_failure(cx, WRITE_CREATE);
                Err(e)
            }
        }
    })))
}

/// Edit page GET — hydrates form from model via Resource::query seam.
pub(crate) fn resource_edit<R: Resource>(cx: &Cx, _body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        let mut db = db(cx);
        let record = find_by_key::<R>(cx, &id, &mut db).await?;
        if !R::can_view(cx, &record) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &record) {
            return Err(forbidden().into());
        }
        crate::csrf::ensure_token(cx);
        let values = R::hydrate_form_values(&record);
        let html = render_form_page::<R>(
            cx,
            format!("Edit {}", R::navigation_label()),
            "Save",
            &values,
            &HashMap::new(),
        )
        .await?;
        Ok(html)
    })))
}

/// Edit page POST — validates, checks `can_view` + `can_update`, mutates via Update projection.
///
/// Requires both `can_view` and `can_update` (matching GET, GH #86 deny-by-default):
/// a view-denied but writable record must not be mutable by direct POST.
pub(crate) fn resource_edit_post<R: Resource>(cx: &Cx, body: Body) -> BoxView<'_> {
    Box::pin(HoistView::new(ThenView::new(async move {
        enforce_auth(cx)?;
        enforce_tenant::<R>(cx)?;
        let mut values = parse_form_values(cx, body).await?;
        crate::csrf::verify(cx, &values)?;
        let id = topcoat::router::path_param_segment(cx, "id").to_string();
        // Advisory load on a pooled handle (GH #86): feeds hydration and the
        // pre-validation file backfill below. The body is already parsed and
        // CSRF-verified (GH #144), so the load never runs for a forged POST.
        // The authoritative load + policy check happens inside the framework
        // transaction — validation (`validate_async` relationship loaders)
        // runs on its own handle and must never execute while the tx holds
        // the pool (see `db` pool discipline).
        let mut db0 = db(cx);
        let advisory = find_by_key::<R>(cx, &id, &mut db0).await?;
        if !R::can_view(cx, &advisory) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &advisory) {
            return Err(forbidden().into());
        }
        let schema = R::form(cx);
        reject_unknown_form_keys(&schema, &values)?;
        // Unique check excludes this record's own unchanged values.
        let current = R::hydrate_form_values(&advisory);
        // Untouched file inputs preserve the stored path (GH #90): the edit
        // form renders an empty file input (browsers never pre-fill it), so
        // an empty submit means "keep", not "clear" — without this the
        // required check rejects untouched edits and optional uploads get
        // blanked. An explicit `clear_<field>=1` opts back into clearing
        // (apps render their own checkbox; a first-class control is future
        // work).
        for name in schema.file_uploads().keys() {
            let cleared = values
                .get(&format!("clear_{name}"))
                .is_some_and(|v| truthy(v));
            let empty = values
                .get(name)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true);
            if !cleared && empty && current.get(name).is_some_and(|v| !v.trim().is_empty()) {
                values.insert(name.clone(), current[name].clone());
            }
        }
        // Transport keys never reach the record fn (GH #148) — see create.
        strip_transport_keys(&schema, &mut values);
        let mut errors = schema.validate_async(cx, &values).await;
        // Authoritative load inside the framework transaction (GH #84, #86):
        // policy is checked on this snapshot and the same record flows into
        // the write — never a silent re-load outside the checked snapshot.
        let mut db = db(cx);
        let mut tx = db.transaction().await.map_err(crate::db::unavailable)?;
        let record = find_by_key::<R>(cx, &id, &mut tx).await?;
        if !R::can_view(cx, &record) {
            return Err(forbidden().into());
        }
        if !R::can_update(cx, &record) {
            return Err(forbidden().into());
        }
        for (name, errs) in check_unique::<R>(cx, &schema, &values, &current, &mut tx).await? {
            errors.entry(name).or_default().extend(errs);
        }
        if !errors.is_empty() {
            return rerender_invalid_form::<R>(
                cx,
                tx,
                format!("Edit {}", R::navigation_label()),
                "Save",
                &values,
                &errors,
            )
            .await;
        }
        match R::update_record(cx, record, values.clone(), &mut tx).await {
            Ok(()) => match tx.commit().await {
                Ok(()) => Err(redirect_after_write::<R>(cx, "Updated")),
                Err(error) => {
                    notify_write_failure(cx, WRITE_UPDATE);
                    Err(crate::db::unavailable(error))
                }
            },
            // A unique violation that slipped past the app-side check (a
            // concurrent update) surfaces as an error, not a string-matched
            // inline message (upstream gap #117) — and now with a toast.
            Err(e) => {
                notify_write_failure(cx, WRITE_UPDATE);
                Err(e)
            }
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::super::Panel;
    use super::*;
    use toasty::Db;

    #[tokio::test]
    async fn edit_post_requires_can_view_as_well_as_can_update() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct ViewDeniedResource;
        impl Resource for ViewDeniedResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
                false
            }
            fn can_update(_cx: &Cx, _record: &Dummy) -> bool {
                true
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
            async fn update_record(
                _cx: &Cx,
                _record: Dummy,
                _values: HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
        }

        let mut db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let row = toasty::create!(Dummy {
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<ViewDeniedResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let url = format!("/admin/dummies/{}/edit", row.id);
        // GET already required both; POST must match (GH #86).
        let get = router
            .handle(
                http::Request::builder()
                    .uri(&url)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(get.status(), http::StatusCode::FORBIDDEN);
        // Valid CSRF token still 403 on policy (not on CSRF).
        let token = uuid::Uuid::new_v4().to_string();
        let post = router
            .handle(
                http::Request::builder()
                    .uri(&url)
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("name=Ada&csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            post.status(),
            http::StatusCode::FORBIDDEN,
            "view-denied edit POST must not mutate"
        );
        // Missing token is 403 even before policy (GH #99).
        let no_token = router
            .handle(
                http::Request::builder()
                    .uri(&url)
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(Body::from("name=Ada"))
                    .unwrap(),
            )
            .await;
        assert_eq!(no_token.status(), http::StatusCode::FORBIDDEN);
    }

    /// One boolean vocabulary for framework form flags (GH #148): `1` and
    /// `true` are truthy everywhere (`confirm`, `clear_<field>`); `yes` was a
    /// delete-only extra and is gone.
    #[test]
    fn truthy_accepts_one_vocabulary() {
        assert!(truthy("1") && truthy("true"));
        assert!(!truthy("yes") && !truthy("") && !truthy("on") && !truthy("TRUE"));
    }

    /// Record fns never see framework transport keys (GH #148): the create
    /// POST carries `csrf_token` (and, for file schemas, `clear_<field>`) —
    /// the framework strips them before `create_record`, so a generic impl
    /// iterating `values` cannot treat them as writable fields.
    #[tokio::test]
    async fn create_record_receives_no_transport_keys() {
        use crate::schema::{FileUpload, Schema, TextInput};
        use std::sync::Mutex;

        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
            title: String,
        }

        static RECEIVED: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
        struct CapturingResource;
        impl crate::resource::Resource for CapturingResource {
            type Model = Doc;
            fn slug() -> String {
                "docs".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Doc> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Doc| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Doc::fields().title(),
                        |d: &Doc| d.title.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> Schema {
                Schema::new((
                    TextInput::r#for(Doc::fields().title()),
                    FileUpload::r#for(Doc::fields().path()),
                ))
            }
            async fn create_record(
                _cx: &Cx,
                values: HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> topcoat::Result<()> {
                let mut keys = values.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                RECEIVED.lock().unwrap().push(keys);
                Ok(())
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Doc))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<CapturingResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let csrf = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .method(http::Method::POST)
                    .uri("/admin/docs/create")
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={csrf}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!(
                        "title=x&path=/tmp/a.bin&csrf_token={csrf}&clear_path=1"
                    )))
                    .unwrap(),
            )
            .await;
        assert!(
            resp.status().is_redirection(),
            "create succeeds, got {} {}",
            resp.status(),
            String::from_utf8_lossy(
                &http_body_util::BodyExt::collect(resp.into_body())
                    .await
                    .unwrap()
                    .to_bytes()
            )
        );
        let received = RECEIVED.lock().unwrap();
        let keys = received.last().expect("create_record ran");
        assert!(
            !keys.contains(&"csrf_token".to_string()) && !keys.contains(&"clear_path".to_string()),
            "transport keys must be stripped before the record fn, got {keys:?}"
        );
        assert_eq!(keys.len(), 2, "declared fields only, got {keys:?}");
    }

    /// Post/Redirect/Get (GH #97, #126): a mutation answers 303, the flash
    /// cookie rides the error response (Topcoat flushes `Set-Cookie` on `Err`,
    /// topcoat#408), and nothing rides the `Location` query. Following the
    /// redirect consumes the cookie, so a reload does not replay the toast.
    #[tokio::test]
    async fn mutation_redirect_carries_the_flash_cookie_instead_of_a_query() {
        use crate::resource::Resource;
        use std::collections::HashMap;

        const COOKIE_NAME: &str = crate::notification::COOKIE_NAME;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct NotifyingResource;
        impl Resource for NotifyingResource {
            type Model = Dummy;
            fn slug() -> String {
                "dummies".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                // A real field, optional so the test's csrf-only POST still
                // passes validation — `Schema::empty()` is what GH #138's
                // build check refuses for a resource that allows create.
                crate::schema::Schema::new(
                    crate::schema::TextInput::r#for(Dummy::fields().name()).optional(),
                )
            }
            async fn create_record(
                _cx: &Cx,
                _values: HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> Result<()> {
                Ok(())
            }
            fn hydrate_form_values(_record: &Dummy) -> HashMap<String, String> {
                HashMap::new()
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|r: &Dummy| r.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |r: &Dummy| r.name.clone(),
                    ))
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Dummy))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<NotifyingResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let token = uuid::Uuid::new_v4().to_string();
        let resp = router
            .handle(
                http::Request::builder()
                    .uri("/admin/dummies/create")
                    .method(http::Method::POST)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(
                        http::header::COOKIE,
                        format!("{}={token}", crate::csrf::COOKIE_NAME),
                    )
                    .body(Body::from(format!("csrf_token={token}")))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            resp.status(),
            http::StatusCode::SEE_OTHER,
            "a completed mutation is a 303 Post/Redirect/Get"
        );
        let location = resp
            .headers()
            .get(http::header::LOCATION)
            .expect("the redirect names its target")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            !location.contains("notification"),
            "the toast must not ride the query, got {location}"
        );
        let set_cookie = resp
            .headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{COOKIE_NAME}=")))
            .expect("the flash cookie flushes on the Err redirect")
            .to_string();
        assert!(
            set_cookie.contains("success") && set_cookie.contains("Created"),
            "the cookie carries the toast status and title: {set_cookie}"
        );
        assert!(
            set_cookie.contains("Secure") && set_cookie.contains("HttpOnly"),
            "the flushed cookie keeps the __Host- contract: {set_cookie}"
        );
    }

    #[tokio::test]
    async fn unique_check_flags_duplicates_for_marked_fields() {
        use crate::schema::{Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
        }

        let mut db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Subscriber { email: "a@b.c" })
            .exec(&mut db)
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        let schema = Schema::new(TextInput::r#for(Subscriber::fields().email()).unique());
        let mut values = HashMap::new();
        values.insert("email".to_string(), "a@b.c".to_string());

        // Create: duplicate → inline error on the field, label-derived.
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &HashMap::new(), &mut ex)
                .await
                .unwrap();
        assert_eq!(
            errors.get("email"),
            Some(&vec!["Email has already been taken".to_string()]),
            "duplicate must be flagged, got {errors:?}"
        );

        // Fresh value → no error.
        let mut fresh = HashMap::new();
        fresh.insert("email".to_string(), "other@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &fresh, &HashMap::new(), &mut ex)
                .await
                .unwrap();
        assert!(errors.is_empty(), "fresh value must pass, got {errors:?}");

        // Edit: the record's own unchanged value is not a duplicate.
        let mut current = HashMap::new();
        current.insert("email".to_string(), "a@b.c".to_string());
        let errors = check_unique::<SubscriberResource>(&cx, &schema, &values, &current, &mut ex)
            .await
            .unwrap();
        assert!(
            errors.is_empty(),
            "own unchanged value must be skipped, got {errors:?}"
        );

        // Edit: changed to someone else's value → flagged again.
        let mut changed_current = HashMap::new();
        changed_current.insert("email".to_string(), "old@b.c".to_string());
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &values, &changed_current, &mut ex)
                .await
                .unwrap();
        assert_eq!(
            errors.get("email"),
            Some(&vec!["Email has already been taken".to_string()]),
            "changed-to-duplicate must be flagged, got {errors:?}"
        );

        // Empty submits are never probed (GH #189): a `unique()` field is
        // required, so validation has already refused the submit — on a field
        // whose `.optional()` was overridden, too, in either call order.
        let mut empty = HashMap::new();
        empty.insert("email".to_string(), "   ".to_string());
        let optional_schema = Schema::new(
            TextInput::r#for(Subscriber::fields().email())
                .optional()
                .unique(),
        );
        let errors = check_unique::<SubscriberResource>(
            &cx,
            &optional_schema,
            &empty,
            &HashMap::new(),
            &mut ex,
        )
        .await
        .unwrap();
        assert!(
            errors.is_empty(),
            "an empty unique submit must not be probed, got {errors:?}"
        );
    }

    /// GH #189, at the layer below the handler: an explicitly `unique()` field
    /// is required even when `.optional()` follows it, validation says so, and
    /// the probe stays out of the empty case. What the two submits *write* is
    /// pinned end to end by
    /// [`two_empty_submits_on_a_unique_field_re_render_and_write_nothing`].
    #[tokio::test]
    async fn unique_field_is_required_however_it_is_marked() {
        use crate::schema::{Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
        }

        let db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let cx = CxTestBuilder::new().app_context(db.clone()).build();
        let mut ex = crate::db::db(&cx);

        // Declared `.optional()` and still required: uniqueness implies
        // presence, so the declaration cannot promise an empty value the index
        // refuses to hold twice.
        let schema = Schema::new(
            TextInput::r#for(Subscriber::fields().email())
                .unique()
                .optional(),
        );
        let mut first = HashMap::new();
        first.insert("email".to_string(), "   ".to_string());
        assert_eq!(
            schema.validate(&first).get("email"),
            Some(&vec!["Email is required".to_string()]),
            "an empty unique field must fail validation as required"
        );

        // Validation owns the empty case, so the probe adds nothing and no
        // query runs — this is what keeps the second empty submit off the
        // unique index (GH #189).
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &first, &HashMap::new(), &mut ex)
                .await
                .unwrap();
        assert!(
            errors.is_empty(),
            "an empty unique submit must not be probed, got {errors:?}"
        );

        // The submit never reaches the write, so the stored table stays empty
        // and the second empty submit cannot collide with the first.
        let mut db_check = db;
        let stored = Subscriber::all().exec(&mut db_check).await.unwrap();
        assert!(
            stored.is_empty(),
            "an empty unique submit must not write, got {} rows",
            stored.len()
        );
    }

    /// GH #189: uniqueness comes from the lens as well as the builder
    /// (`#[unique]` → `lens_field_unique`), so a field that was never marked by
    /// hand is required too — the rule is a property of the field, not of the
    /// declaration style.
    #[tokio::test]
    async fn lens_derived_unique_is_required_without_a_unique_call() {
        use crate::schema::{Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
        }

        let db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        let input = TextInput::r#for(Subscriber::fields().email());
        assert!(
            input.is_unique(),
            "the index must be recognized without a `.unique()` call (GH #183)"
        );
        assert!(input.is_required(), "derived uniqueness implies presence");

        let schema = Schema::new(input);
        let mut empty = HashMap::new();
        empty.insert("email".to_string(), "".to_string());
        assert_eq!(
            schema.validate(&empty).get("email"),
            Some(&vec!["Email is required".to_string()]),
            "an empty submit must be refused inline, not probed"
        );
        let errors =
            check_unique::<SubscriberResource>(&cx, &schema, &empty, &HashMap::new(), &mut ex)
                .await
                .unwrap();
        assert!(
            errors.is_empty(),
            "validation owns the empty case; the probe must add nothing, got {errors:?}"
        );
    }

    /// GH #189 acceptance, through the real panel: two submits with an empty
    /// `unique()` field re-render inline and write nothing. Before the fix the
    /// first empty submit *succeeded* — it stored `""` — so the panel had
    /// already broken the promise its own unique index makes, and the second
    /// empty submit met the constraint instead of the form rule: 500 when the
    /// record fn stores the value as submitted, or a misleading "has already
    /// been taken" when it trims first.
    #[tokio::test]
    async fn two_empty_submits_on_a_unique_field_re_render_and_write_nothing() {
        use crate::resource::{Resource, Table, TextColumn};
        use crate::schema::{Schema, TextInput};

        #[derive(Debug, toasty::Model)]
        struct Subscriber {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct SubscriberResource;
        impl Resource for SubscriberResource {
            type Model = Subscriber;
            fn slug() -> String {
                "subscribers".to_string()
            }
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> Table<Subscriber> {
                Table::r#for(cx)
                    .id(|s: &Subscriber| s.id.to_string())
                    .columns(TextColumn::r#for(
                        Subscriber::fields().email(),
                        |s: &Subscriber| s.email.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> Schema {
                // `.optional()` is the declaration that used to make an empty
                // submit probe instead of failing: uniqueness wins.
                Schema::new(
                    TextInput::r#for(Subscriber::fields().email())
                        .unique()
                        .optional(),
                )
            }
            async fn create_record(
                _cx: &Cx,
                values: HashMap<String, String>,
                ex: &mut dyn toasty::Executor,
            ) -> topcoat::Result<()> {
                // Writes what the panel would: the record fns trim, and the
                // framework's probe trims too, so the stored `""` is exactly
                // what the next probe looks for.
                toasty::create!(Subscriber {
                    email: values
                        .get("email")
                        .map(|v| v.trim().to_string())
                        .unwrap_or_default(),
                })
                .exec(ex)
                .await?;
                Ok(())
            }
        }

        let db = Db::builder()
            .models(toasty::models!(Subscriber))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        let router = Panel::new("admin")
            .app_context(db.clone())
            .resource::<SubscriberResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");

        let csrf = uuid::Uuid::new_v4().to_string();
        // `+` decodes to a space and an empty pair to `""`: both are empty
        // submits, and under the old rule the first stored `""` — so the second
        // met the index via a trimmed probe match while a second space met it
        // again at the driver. Neither is a duplicate, and neither may write.
        for (attempt, submitted) in ["+", ""].into_iter().enumerate() {
            let attempt = attempt + 1;
            let resp = router
                .handle(
                    http::Request::builder()
                        .method(http::Method::POST)
                        .uri("/admin/subscribers/create")
                        .header(
                            http::header::CONTENT_TYPE,
                            "application/x-www-form-urlencoded",
                        )
                        .header(
                            http::header::COOKIE,
                            format!("{}={csrf}", crate::csrf::COOKIE_NAME),
                        )
                        .body(Body::from(format!("email={submitted}&csrf_token={csrf}")))
                        .unwrap(),
                )
                .await;
            assert_eq!(
                resp.status(),
                http::StatusCode::OK,
                "empty submit {attempt} must re-render, not redirect or fail"
            );
            let body = http_body_util::BodyExt::collect(resp.into_body())
                .await
                .unwrap()
                .to_bytes();
            let html = String::from_utf8_lossy(&body);
            assert!(
                html.contains("Email is required"),
                "empty submit {attempt} must carry the presence error, got {html}"
            );
        }

        let mut db_check = db;
        let stored = Subscriber::all().exec(&mut db_check).await.unwrap();
        assert!(
            stored.is_empty(),
            "two empty submits must write nothing, got {} rows",
            stored.len()
        );
    }

    #[tokio::test]
    async fn unique_check_propagates_probe_errors() {
        use crate::schema::{Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Probe {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            email: String,
        }
        struct ProbeResource;
        impl Resource for ProbeResource {
            type Model = Probe;
        }

        // Schema never pushed: the probe query cannot run, so the check must
        // fail the submit instead of silently passing it (GH #167).
        let db = Db::builder()
            .models(toasty::models!(Probe))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        let schema = Schema::new(TextInput::r#for(Probe::fields().email()).unique());
        let mut values = HashMap::new();
        values.insert("email".to_string(), "a@b.c".to_string());
        let result =
            check_unique::<ProbeResource>(&cx, &schema, &values, &HashMap::new(), &mut ex).await;
        assert!(
            result.is_err(),
            "a failing probe must fail the submit, got {result:?}"
        );
    }

    #[tokio::test]
    async fn unique_check_ignores_absent_repeater_groups() {
        use crate::schema::{Repeater, Schema, TextInput};
        use topcoat::context::CxTestBuilder;

        #[derive(Debug, toasty::Model)]
        struct Tagged {
            #[key]
            #[auto]
            id: uuid::Uuid,
            #[unique]
            nickname: String,
        }
        struct TaggedResource;
        impl Resource for TaggedResource {
            type Model = Tagged;
        }

        let mut db = Db::builder()
            .models(toasty::models!(Tagged))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(Tagged {
            nickname: "".to_string()
        })
        .exec(&mut db)
        .await
        .unwrap();
        let cx = CxTestBuilder::new().app_context(db).build();
        let mut ex = crate::db::db(&cx);

        let schema = Schema::new(
            Repeater::new("Tags").schema(
                TextInput::r#for(Tagged::fields().nickname())
                    .unique()
                    .optional(),
            ),
        );

        // Absent group (all-inner-empty) with a stored `""`: validation calls
        // it clean (GH #147), so the unique check must agree (GH #167).
        let mut absent = HashMap::new();
        absent.insert("nickname".to_string(), "".to_string());
        assert!(
            schema.validate(&absent).is_empty(),
            "absent group must validate clean"
        );
        let errors =
            check_unique::<TaggedResource>(&cx, &schema, &absent, &HashMap::new(), &mut ex)
                .await
                .unwrap();
        assert!(
            errors.is_empty(),
            "absent group must not be unique-checked, got {errors:?}"
        );

        // Present group still checks: a taken value flags inline.
        let mut present = HashMap::new();
        present.insert("nickname".to_string(), "taken".to_string());
        toasty::create!(Tagged {
            nickname: "taken".to_string()
        })
        .exec(&mut ex)
        .await
        .unwrap();
        let errors =
            check_unique::<TaggedResource>(&cx, &schema, &present, &HashMap::new(), &mut ex)
                .await
                .unwrap();
        assert_eq!(
            errors.get("nickname"),
            Some(&vec!["Nickname has already been taken".to_string()]),
            "present group must still be unique-checked, got {errors:?}"
        );
    }

    #[test]
    fn reject_unknown_form_keys_allows_declared_plus_csrf() {
        use crate::schema::{Schema, TextInput};

        #[derive(Debug, toasty::Model)]
        struct Member {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        let schema = Schema::new(TextInput::r#for(Member::fields().name()));

        // Declared keys + csrf_token pass.
        let values = HashMap::from([
            ("name".to_string(), "Ada".to_string()),
            (
                crate::csrf::FIELD_NAME.to_string(),
                "some-token".to_string(),
            ),
        ]);
        assert!(reject_unknown_form_keys(&schema, &values).is_ok());

        // Absent keys are fine (present-keys-only updates, GH #89).
        let values = HashMap::from([(
            crate::csrf::FIELD_NAME.to_string(),
            "some-token".to_string(),
        )]);
        assert!(reject_unknown_form_keys(&schema, &values).is_ok());

        // role/tenant_id smuggling is a 400.
        let values = HashMap::from([
            ("name".to_string(), "Ada".to_string()),
            ("role".to_string(), "admin".to_string()),
            ("tenant_id".to_string(), "victim".to_string()),
        ]);
        assert!(reject_unknown_form_keys(&schema, &values).is_err());
    }

    #[test]
    fn form_values_decode_utf8_plus_and_encoded_separators() {
        // Multi-byte UTF-8: %C3%A9 must assemble to é (the old hand-rolled
        // decoder pushed each byte through `byte as char` → "Ã©", GH #75
        // item 6).
        let got = form_values_from_bytes(b"name=R%C3%A9mi");
        assert_eq!(got.get("name").map(String::as_str), Some("Rémi"));

        // `+` is a space; a literal plus is %2B — not double-decoded to space.
        let got = form_values_from_bytes(b"q=a+b&p=C%2B%2B");
        assert_eq!(got.get("q").map(String::as_str), Some("a b"));
        assert_eq!(got.get("p").map(String::as_str), Some("C++"));

        // Encoded separators survive as values.
        let got = form_values_from_bytes(b"a=1%262%3D3");
        assert_eq!(got.get("a").map(String::as_str), Some("1&2=3"));

        // Empty / blank input → empty map.
        assert!(form_values_from_bytes(b"").is_empty());
    }

    /// Build a request context carrying `content_type` and run the streaming
    /// multipart parser over `body` (GH #90).
    async fn multipart_values(
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<HashMap<String, String>, topcoat::Error> {
        let (parts, ()) = http::Request::builder()
            .uri("/admin/users/create")
            .header(http::header::CONTENT_TYPE, content_type)
            .body(())
            .unwrap()
            .into_parts();
        let cx = topcoat::context::CxTestBuilder::new()
            .request_context(parts)
            .build();
        parse_multipart_values(&cx, Body::from(body)).await
    }

    fn multipart_type(boundary: &str) -> String {
        format!("multipart/form-data; boundary={boundary}")
    }

    /// The multipart drain's byte accounting 413s one byte past the cap
    /// (GH #149 tripwire). Unit-tested at the boundary because through the
    /// router the extractor's `BodyLimit` classifies the same body first —
    /// the counter is the backstop for the day that limit stops wrapping the
    /// stream, not a competing cap.
    #[test]
    fn multipart_drain_counts_bytes_and_413s_one_past_the_cap() {
        let mut seen = 0usize;
        count_form_bytes(&mut seen, MAX_FORM_BYTES / 2).unwrap();
        count_form_bytes(&mut seen, MAX_FORM_BYTES / 2).unwrap();
        assert_eq!(seen, MAX_FORM_BYTES);
        let err = count_form_bytes(&mut seen, 1).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "one byte past the cap must map to content-too-large (413), got {err}"
        );
        // A single chunk past the cap fires without a prior accumulation.
        let mut seen = 0usize;
        let err = count_form_bytes(&mut seen, MAX_FORM_BYTES + 1).unwrap_err();
        assert!(
            err.downcast_ref::<topcoat::router::error::ContentTooLargeError>()
                .is_some(),
            "a single over-cap chunk must 413, got {err}"
        );
    }

    /// An 11 MiB multipart upload 413s end to end through the panel (GH #149
    /// acceptance). The installed `BodyLimit::max(MAX_FORM_BYTES)` layer and
    /// the drain's own counter share the same threshold, so the body is over
    /// both at once — the e2e pins the streaming path answers 413 rather than
    /// draining; the counter's own accounting (which only answers if the
    /// extractor's limit ever stops wrapping the stream — `BodyLimitKind` is
    /// private, so no public configuration can disable it) is pinned by
    /// [`count_form_bytes`]'s unit test above.
    #[tokio::test]
    async fn multipart_over_the_form_cap_413s_through_the_router() {
        use crate::resource::Resource;

        #[derive(Debug, toasty::Model)]
        struct Dummy {
            #[key]
            #[auto]
            id: uuid::Uuid,
            name: String,
        }
        struct DummyResource;
        impl Resource for DummyResource {
            type Model = Dummy;
            fn can_view_any(_cx: &Cx) -> bool {
                true
            }
            fn can_create(_cx: &Cx) -> bool {
                true
            }
            fn table(cx: &Cx) -> crate::resource::Table<Dummy> {
                crate::resource::Table::r#for(cx)
                    .id(|d: &Dummy| d.id.to_string())
                    .columns(crate::resource::TextColumn::r#for(
                        Dummy::fields().name(),
                        |d: &Dummy| d.name.clone(),
                    ))
            }
            fn form(_cx: &Cx) -> crate::schema::Schema {
                crate::schema::Schema::new(crate::schema::FileUpload::r#for(Dummy::fields().name()))
            }
            async fn create_record(
                _cx: &Cx,
                _values: std::collections::HashMap<String, String>,
                _ex: &mut dyn toasty::Executor,
            ) -> topcoat::Result<()> {
                Ok(())
            }
        }

        let db = Db::builder().connect("sqlite::memory:").await.unwrap();
        let router = Panel::new("admin")
            .app_context(db)
            .resource::<DummyResource>()
            .auth(crate::Auth::disabled())
            .build()
            .expect("panel builds");
        let boundary = "----Boundary123";
        let payload = "x".repeat(MAX_FORM_BYTES + 1024);
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"; filename=\"big.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n{payload}\r\n--{boundary}--\r\n"
        );
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("/admin/dummies/create")
            .header(
                http::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let resp = router.handle(request).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "an 11 MiB multipart upload must 413 through the router, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn multipart_stream_stores_text_and_filenames() {
        let boundary = "----Boundary123";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHello\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nBINARYBYTES\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nrust,async\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nsecond-wins\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let got = multipart_values(&multipart_type(boundary), body.into_bytes())
            .await
            .unwrap();
        assert_eq!(got.get("title").map(String::as_str), Some("Hello"));
        // v1 stores the filename, not the bytes (FileUpload contract).
        assert_eq!(got.get("image_path").map(String::as_str), Some("photo.jpg"));
        assert_eq!(got.get("tags").map(String::as_str), Some("second-wins"));

        // Empty filename → empty value so `required` fires.
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n--{b}--\r\n",
            b = boundary
        );
        let got = multipart_values(&multipart_type(boundary), body.into_bytes())
            .await
            .unwrap();
        assert_eq!(got.get("image_path").map(String::as_str), Some(""));
    }

    #[tokio::test]
    async fn multipart_stream_sanitizes_traversal_and_filename_star() {
        // Traversal filename lands sanitized (GH #90).
        let body = "--B\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"../../../etc/passwd\"\r\nContent-Type: application/octet-stream\r\n\r\nBYTES\r\n--B--\r\n";
        let got = multipart_values(&multipart_type("B"), body.as_bytes().to_vec())
            .await
            .unwrap();
        assert_eq!(got.get("image_path").map(String::as_str), Some("passwd"));

        // RFC 5987 filename* decodes and wins over filename= (both present).
        let body = "--B\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"plain.jpg\"; filename*=UTF-8''%E2%82%ACphoto.jpg\r\nContent-Type: image/jpeg\r\n\r\nBYTES\r\n--B--\r\n";
        let got = multipart_values(&multipart_type("B"), body.as_bytes().to_vec())
            .await
            .unwrap();
        assert_eq!(
            got.get("image_path").map(String::as_str),
            Some("€photo.jpg"),
            "filename*=UTF-8 must decode and win, got {got:?}"
        );
        // Non-UTF-8 charset falls back to plain filename=.
        let body = "--B\r\nContent-Disposition: form-data; name=\"image_path\"; filename=\"plain.jpg\"; filename*=latin-1''%E9.jpg\r\nContent-Type: image/jpeg\r\n\r\nBYTES\r\n--B--\r\n";
        let got = multipart_values(&multipart_type("B"), body.as_bytes().to_vec())
            .await
            .unwrap();
        assert_eq!(
            got.get("image_path").map(String::as_str),
            Some("plain.jpg"),
            "unsupported charset must fall back, got {got:?}"
        );
    }

    #[tokio::test]
    async fn multipart_stream_rejects_missing_boundary() {
        // Bare multipart without boundary is a 400, not a silent urlencoded
        // fallback that turns binary bytes into confusing required-errors.
        assert!(
            multipart_values("multipart/form-data", b"name=x".to_vec())
                .await
                .is_err()
        );
    }

    #[test]
    fn filenames_sanitize_to_basename_and_dispatch_guards_size() {
        assert_eq!(sanitize_filename("upload.jpg"), "upload.jpg");
        assert_eq!(sanitize_filename("../../../etc/cron.d/x"), "x");
        assert_eq!(sanitize_filename("/abs/path"), "path");
        assert_eq!(sanitize_filename("C:\\fakepath\\x"), "x");
        assert_eq!(sanitize_filename(""), "");
        // Names that could never be a safe persisted file are rejected to
        // empty (GH #149): dot/dot-dot, and Windows reserved device names —
        // case-insensitively and with an extension too.
        assert_eq!(sanitize_filename("."), "");
        assert_eq!(sanitize_filename(".."), "");
        assert_eq!(sanitize_filename("../.."), "");
        assert_eq!(sanitize_filename("..."), "...");
        assert_eq!(sanitize_filename("con"), "");
        assert_eq!(sanitize_filename("NUL"), "");
        assert_eq!(sanitize_filename("Com1.txt"), "");
        assert_eq!(sanitize_filename("lpt9"), "");
        assert_eq!(sanitize_filename("console.txt"), "console.txt");
        assert_eq!(sanitize_filename("companion"), "companion");
        assert_eq!(sanitize_filename("...."), "....");
        // Cap keeps the tail without splitting a multibyte char: a naive
        // `[len - 255..]` slice panics here (the cut lands inside `é`).
        let multibyte = format!("{}{}", "é".repeat(200), "a".repeat(200));
        let capped = sanitize_filename(&multibyte);
        assert!(
            capped.len() <= 255,
            "cap must bound bytes, got {}",
            capped.len()
        );
        assert!(
            capped.ends_with('a'),
            "tail must be preserved, got {capped:?}"
        );
        // Over-cap body is rejected before buffering into maps.
        let big = vec![b'a'; MAX_FORM_BYTES + 1];
        assert!(
            form_values_from_request_parts(Some("application/x-www-form-urlencoded"), &big)
                .is_err()
        );
        // Normal urlencoded still parses.
        let ok =
            form_values_from_request_parts(Some("application/x-www-form-urlencoded"), b"name=Ada")
                .unwrap();
        assert_eq!(ok.get("name").map(String::as_str), Some("Ada"));
    }

    #[test]
    fn sanitize_filename_invariants_hold() {
        // GH #136 §5 property candidates: no `/` or `\`, ≤255 bytes, never
        // panics on multibyte input.
        for raw in [
            "a/b\\c".to_string(),
            "é".repeat(300),
            "../..".to_string(),
            "con".to_string(),
            " normal.jpg ".to_string(),
            "a".repeat(500),
            "\u{0}bad\nname\"".to_string(),
        ] {
            let out = sanitize_filename(&raw);
            assert!(
                !out.contains('/') && !out.contains('\\'),
                "separators must be gone, got {out:?} from {raw:?}"
            );
            assert!(
                out.len() <= 255,
                "cap must bound bytes, got {} from {raw:?}",
                out.len()
            );
            assert!(
                out.chars().all(|c| !c.is_control()),
                "controls must be stripped, got {out:?}"
            );
        }
    }

    #[test]
    fn create_form_multipart_predicate_follows_file_upload() {
        // GH #136 layer rule: core owns the `has_file_upload` predicate
        // (see also `has_file_upload_detects_nested` for nested containers);
        // the showcase (`posts_create_form_is_multipart` /
        // `users_create_form_stays_urlencoded`) owns the HTTP enctype wiring
        // (`render_form_page` maps this predicate to
        // `enctype="multipart/form-data"` one-to-one).
        use crate::schema::{FileUpload, Schema, TextInput};

        #[derive(Debug, toasty::Model)]
        struct Doc {
            #[key]
            #[auto]
            id: uuid::Uuid,
            path: String,
            title: String,
        }
        struct WithFile;
        impl Resource for WithFile {
            type Model = Doc;
            fn form(_cx: &Cx) -> Schema {
                Schema::new(FileUpload::r#for(Doc::fields().path()))
            }
        }
        struct WithoutFile;
        impl Resource for WithoutFile {
            type Model = Doc;
            fn form(_cx: &Cx) -> Schema {
                Schema::new(TextInput::r#for(Doc::fields().title()))
            }
        }

        let cx = topcoat::context::CxTestBuilder::new().build();
        assert!(
            WithFile::form(&cx).has_file_upload(),
            "file schema must report an upload"
        );
        assert!(
            !WithoutFile::form(&cx).has_file_upload(),
            "plain schema must report no upload"
        );
    }
}
