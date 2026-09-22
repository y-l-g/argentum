//! Response hardening headers (GH #176).
//!
//! The panel serves one document per request, and a document that anyone can
//! frame is a clickjacking surface on every deployment by default. `Panel`
//! installs [`FrameAncestors`] unless the app opts out, so the threat is closed
//! where it lands rather than in each deployment's proxy config.

use http::header;
use topcoat::context::Cx;
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, response::Response};

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
}
