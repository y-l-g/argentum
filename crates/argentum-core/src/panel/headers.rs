//! Response hardening headers (GH #176, GH #278).
//!
//! The panel serves one document per request, and a document that anyone can
//! frame is a clickjacking surface on every deployment by default. `Panel`
//! installs [`FrameAncestors`] unless the app opts out, so the threat is closed
//! where it lands rather than in each deployment's proxy config.
//!
//! A served directory shares the panel's origin, so `Panel` also installs
//! [`ServedFileHeaders`] on each one (GH #278): the files an app accepts from
//! its users are inert, whatever their extension.

use http::{StatusCode, header};
use topcoat::{
    context::Cx,
    router::{Body, Layer, LayerFuture, Next, Path, PathBuf, response::Response},
};

/// Response header carrying the policy.
const CSP: header::HeaderName = header::CONTENT_SECURITY_POLICY;

/// The panel's default directive: only the panel may frame itself.
pub(crate) const DEFAULT_FRAME_ANCESTORS: &str = "'self'";

/// Emits `Content-Security-Policy: frame-ancestors <directive>` on every
/// response that does not already carry a `Content-Security-Policy` header.
///
/// `frame-ancestors` is the one CSP directive a `<meta>` tag cannot express, so
/// it has to ride the response — which is also why it belongs here and not in
/// [`render_document`](super::shell::Panel::render_document)'s markup.
///
/// An app that sets its own policy (its own layer or route) wins: this layer
/// only fills the gap, so a full `Content-Security-Policy` never fights a
/// second one.
#[derive(Debug, Clone)]
pub(crate) struct FrameAncestors {
    directive: String,
}

impl FrameAncestors {
    pub(crate) fn new(directive: impl Into<String>) -> Self {
        Self {
            directive: directive.into(),
        }
    }

    /// `frame-ancestors 'self'` — the default the panel ships.
    #[cfg(test)]
    pub(crate) fn same_origin() -> Self {
        Self::new(DEFAULT_FRAME_ANCESTORS)
    }
}

impl Layer for FrameAncestors {
    fn path(&self) -> Option<&Path> {
        // Every response, not just the panel prefix: a 404 or a login redirect
        // is frameable too, and a path-less layer is the only one that sees
        // unmatched routes.
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let mut response = next.run(cx, body).await?;
            insert_frame_ancestors(&mut response, &self.directive);
            Ok(response)
        })
    }
}

/// Add the directive unless the response already carries a policy.
fn insert_frame_ancestors(response: &mut Response, directive: &str) {
    if response.headers().contains_key(&CSP) {
        return;
    }
    // The directive is app-supplied text (a header value, not markup): a value
    // the header codec rejects is dropped rather than allowed to panic or
    // truncate a response mid-stream.
    if let Ok(value) = header::HeaderValue::from_str(&format!("frame-ancestors {directive}")) {
        response.headers_mut().insert(CSP, value);
    }
}

/// The one policy every served file carries.
///
/// `frame-ancestors 'self'` rides along because [`FrameAncestors`] only fills a
/// gap: it runs outside this layer and skips a response that already has a
/// policy, so this one has to speak for itself.
const SERVED_FILE_CSP: &str = "default-src 'none'; img-src 'self'; media-src 'self'; \
     style-src 'unsafe-inline'; sandbox; frame-ancestors 'self'";

/// The content types a served file may render inline.
///
/// Everything here is passive: no script, no markup, no plugin. The list is the
/// policy's source of truth, so widening it is the one way to re-open a type.
const INLINE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "video/mp4",
    "video/webm",
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "text/plain",
];

/// Emits `X-Content-Type-Options: nosniff`, a fixed sandboxing
/// `Content-Security-Policy` and `Content-Disposition: attachment` on every file
/// response the directory route serves (GH #278).
///
/// A served directory shares the panel's origin (ADR-0017 makes it public by
/// decision), and Topcoat derives `Content-Type` from the file extension, so a
/// user who uploads an `.html` or `.svg` document otherwise runs script with the
/// admin's session. The policy is fixed rather than configurable: an app that
/// serves active documents mounts them on its own origin. A disposition that
/// already downloads is kept, so a route that names a file keeps its filename.
///
/// The directory route's own failures (404, 405) leave through `Err` and skip
/// this layer; they render Topcoat's plain error response, which carries no file
/// content.
#[derive(Debug, Clone)]
pub(crate) struct ServedFileHeaders {
    path: PathBuf,
}

impl ServedFileHeaders {
    /// Wraps the directory route mounted at `pattern`, the same path
    /// [`Panel::serve_dir`](super::Panel::serve_dir) registers.
    pub(crate) fn new(pattern: &str) -> Self {
        Self {
            path: super::route_path(pattern),
        }
    }
}

impl Layer for ServedFileHeaders {
    fn path(&self) -> Option<&Path> {
        // A route's own path is a prefix of itself, so this wraps exactly the
        // served directory: the panel's own pages keep their policy.
        Some(&self.path)
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let mut response = next.run(cx, body).await?;
            harden_served_file(&mut response);
            Ok(response)
        })
    }
}

/// Make one file response the directory route served inert.
///
/// The policy overwrites whatever the directory route sent: a served file never
/// needs a scriptable policy. `nosniff` and the policy apply to every file
/// response. The disposition depends on the content type, so it is skipped on a
/// `304 Not Modified`, which carries no content type and no body to render.
fn harden_served_file(response: &mut Response) {
    let not_modified = response.status() == StatusCode::NOT_MODIFIED;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(CSP, header::HeaderValue::from_static(SERVED_FILE_CSP));
    if not_modified || is_inline(headers.get(header::CONTENT_TYPE)) {
        return;
    }
    let already_an_attachment = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("attachment"));
    if !already_an_attachment {
        headers.insert(
            header::CONTENT_DISPOSITION,
            header::HeaderValue::from_static("attachment"),
        );
    }
}

/// Whether `content_type` may render inline: the allow-list entry, with any
/// parameters (`; charset=..`) stripped and the type lowercased. A missing or
/// unreadable type downloads rather than rendering.
fn is_inline(content_type: Option<&header::HeaderValue>) -> bool {
    let Some(mime) = content_type.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let mime = mime.split(';').next().unwrap_or_default().trim();
    INLINE_TYPES.contains(&mime.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Response {
        Response::builder().body(Body::empty()).unwrap()
    }

    #[test]
    fn default_directive_is_self() {
        // The exact literal the browser receives. Comparing against a
        // `#[cfg(test)]` re-implementation of the same `format!` cannot fail
        // (GH #216): both sides would change together.
        let mut response = response();
        insert_frame_ancestors(&mut response, &FrameAncestors::same_origin().directive);
        assert_eq!(
            response.headers().get(&CSP).unwrap(),
            "frame-ancestors 'self'"
        );
    }

    #[test]
    fn an_existing_policy_wins() {
        let mut response = response();
        response
            .headers_mut()
            .insert(CSP, header::HeaderValue::from_static("default-src 'none'"));
        insert_frame_ancestors(&mut response, "'self'");
        assert_eq!(
            response.headers().get(&CSP).unwrap(),
            "default-src 'none'",
            "an app policy must not be overwritten (or duplicated)"
        );
    }

    #[test]
    fn an_invalid_directive_is_dropped_not_panicked() {
        // A newline would split the header; the layer must not panic or emit a
        // truncated value.
        let mut response = response();
        insert_frame_ancestors(&mut response, "'self'\r\nX-Evil: 1");
        assert!(response.headers().get(&CSP).is_none());
    }

    /// A served file's response with `content_type`, or none at all.
    fn file_response(content_type: Option<&str>) -> Response {
        let mut response = response();
        if let Some(content_type) = content_type {
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_str(content_type).unwrap(),
            );
        }
        response
    }

    fn policy(response: &Response) -> &str {
        response.headers().get(&CSP).unwrap().to_str().unwrap()
    }

    fn disposition(response: &Response) -> Option<&str> {
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .map(|value| value.to_str().unwrap())
    }

    #[test]
    fn the_allow_list_renders_inline() {
        for content_type in [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/avif",
            "video/mp4",
            "video/webm",
            "audio/mpeg",
            "audio/ogg",
            "audio/wav",
            "text/plain",
            // The type is case-insensitive and parameters are not part of it.
            "TEXT/PLAIN; charset=utf-8",
        ] {
            let mut response = file_response(Some(content_type));
            harden_served_file(&mut response);
            assert_eq!(
                response
                    .headers()
                    .get(header::X_CONTENT_TYPE_OPTIONS)
                    .unwrap(),
                "nosniff",
                "{content_type} must not be sniffed"
            );
            assert!(
                policy(&response).contains("sandbox"),
                "{content_type} must carry the sandbox policy"
            );
            assert_eq!(
                disposition(&response),
                None,
                "{content_type} renders inline"
            );
        }
    }

    #[test]
    fn everything_else_downloads() {
        // `image/svg+xml` can script, `text/html` is a document, and a PDF is a
        // plugin surface; the browser must save rather than render them.
        for content_type in [
            "image/svg+xml",
            "text/html; charset=utf-8",
            "application/pdf",
        ] {
            let mut response = file_response(Some(content_type));
            harden_served_file(&mut response);
            assert_eq!(
                disposition(&response),
                Some("attachment"),
                "{content_type} must download"
            );
            assert!(
                policy(&response).contains("sandbox"),
                "{content_type} must carry the sandbox policy"
            );
        }
    }

    #[test]
    fn a_missing_content_type_downloads() {
        let mut response = file_response(None);
        harden_served_file(&mut response);
        assert_eq!(disposition(&response), Some("attachment"));
    }

    #[test]
    fn an_existing_disposition_is_kept_only_when_it_downloads() {
        let mut response = file_response(Some("text/html"));
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            header::HeaderValue::from_static("attachment; filename=\"report.html\""),
        );
        harden_served_file(&mut response);
        assert_eq!(
            disposition(&response),
            Some("attachment; filename=\"report.html\""),
            "the directory route's own filename survives"
        );

        let mut response = file_response(Some("text/html"));
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            header::HeaderValue::from_static("inline"),
        );
        harden_served_file(&mut response);
        assert_eq!(
            disposition(&response),
            Some("attachment"),
            "an inline disposition is replaced, never left to render"
        );
    }

    #[test]
    fn a_not_modified_response_carries_the_policy_and_no_disposition() {
        // A 304 has no `Content-Type`, so the disposition rule cannot see what
        // is being revalidated; an inline image must not flip to a download.
        let mut response = file_response(None);
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        harden_served_file(&mut response);
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .unwrap(),
            "nosniff"
        );
        assert!(policy(&response).contains("sandbox"));
        assert_eq!(disposition(&response), None);
    }

    #[test]
    fn an_existing_policy_is_overwritten() {
        // A served file never needs a scriptable policy, so unlike
        // `FrameAncestors` this layer does not defer to what is there. The
        // literal is the directive the browser must receive: `frame-ancestors`
        // is here because `FrameAncestors` skips a response that already has a
        // policy (GH #216: comparing against the implementation constant would
        // change both sides together).
        let mut response = file_response(Some("text/html"));
        response
            .headers_mut()
            .insert(CSP, header::HeaderValue::from_static("script-src *"));
        harden_served_file(&mut response);
        assert_eq!(
            policy(&response),
            "default-src 'none'; img-src 'self'; media-src 'self'; \
             style-src 'unsafe-inline'; sandbox; frame-ancestors 'self'"
        );
    }
}
