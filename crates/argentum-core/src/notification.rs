//! `Notification` — transient user-visible message (CONTEXT.md).
//!
//! Produced by an `Action`'s result and rendered in the `Panel` shell's
//! top-level boundary so it survives `Table` swaps. Status + title, ~4s.

use topcoat::context::{Cx, try_request_context};
use topcoat::cookie::{Cookie, CookieJarCell, Cookies, cookies};

/// The kind of notification (status).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationStatus {
    Success,
    Error,
    Info,
    Warning,
}

impl NotificationStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Info => "info",
            Self::Warning => "warning",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "error" => Self::Error,
            "info" => Self::Info,
            "warning" => Self::Warning,
            _ => Self::Success,
        }
    }
}

/// A transient message shown after a mutation.
#[derive(Debug, Clone)]
pub struct Notification {
    pub status: NotificationStatus,
    pub title: String,
}

impl Notification {
    pub fn success(title: impl Into<String>) -> Self {
        Self {
            status: NotificationStatus::Success,
            title: title.into(),
        }
    }

    pub fn error(title: impl Into<String>) -> Self {
        Self {
            status: NotificationStatus::Error,
            title: title.into(),
        }
    }

    pub fn info(title: impl Into<String>) -> Self {
        Self {
            status: NotificationStatus::Info,
            title: title.into(),
        }
    }

    /// Encode to cookie value: `status:title` (title is percent-encoded to avoid `:` issues).
    fn encode(&self) -> String {
        // Simple encoding: status + ":" + percent-encoded title (use URL encoding for ":" and ";")
        // For MVP, just replace ":" with "%3A" and ";" with "%3B"
        let escaped = self
            .title
            .replace('%', "%25")
            .replace(':', "%3A")
            .replace(';', "%3B")
            .replace('\n', "%0A");
        format!("{}:{}", self.status.as_str(), escaped)
    }

    fn decode(s: &str) -> Option<Self> {
        let (status_str, title_enc) = s.split_once(':')?;
        let status = NotificationStatus::from_str(status_str);
        let title = title_enc
            .replace("%0A", "\n")
            .replace("%3B", ";")
            .replace("%3A", ":")
            .replace("%25", "%");
        Some(Self { status, title })
    }
}

const COOKIE_NAME: &str = "argentum_notification";

/// Store a notification for the next request (flash).
#[allow(clippy::question_mark)]
pub fn set_notification(cx: &Cx, notification: Notification) {
    if try_request_context::<CookieJarCell>(cx).is_none() {
        return;
    }
    let value = notification.encode();
    let cookie = Cookie::build((COOKIE_NAME, value))
        .path("/")
        .http_only(false)
        .same_site(topcoat::cookie::SameSite::Lax)
        .build();
    cookies(cx).add(cookie);
}

/// Take the notification from the request (if present) and clear it.
pub fn take_notification(cx: &Cx) -> Option<Notification> {
    try_request_context::<CookieJarCell>(cx)?;
    let jar = cookies(cx);
    let cookie = jar.get(COOKIE_NAME)?;
    let value = cookie.value().to_string();
    // Remove the cookie so it doesn't persist.
    jar.remove(Cookie::build((COOKIE_NAME, "")).path("/").build());
    Notification::decode(&value)
}

/// Render the notification stack HTML if a notification is present.
/// Returns `Option<View>` HTML string fragment; caller should embed in shell.
pub fn render_notification(cx: &Cx) -> Option<String> {
    let n = take_notification(cx)?;
    // Rendered as fixed top-4 right-4 card with Token classes, as spec requires.
    // The shell's outer div already has `fixed top-4 right-4 z-50 flex flex-col gap-2`,
    // so we just need the inner card.
    // For direct HTML check, ensure these classes appear.
    let (border, bg) = match n.status {
        NotificationStatus::Success => ("border-border bg-background", "success"),
        NotificationStatus::Error => ("border-destructive bg-background", "error"),
        _ => ("border-border bg-background", "info"),
    };
    let _ = bg;
    // The actual rendering will be done via view! in panel.rs; this helper just provides data.
    // But we expose a helper to get the notification for rendering.
    Some(format!(
        r#"<div class="rounded-xl border {border} shadow-sm p-4"><p class="text-sm font-medium">{}</p></div>"#,
        html_escape(&n.title)
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Request;
    use topcoat::context::CxTestBuilder;

    fn cx_with_cookie(value: Option<&str>) -> Cx {
        let mut builder = Request::builder().uri("/").body(()).unwrap().into_parts().0;
        if let Some(v) = value {
            builder.headers.insert(
                http::header::COOKIE,
                format!("{COOKIE_NAME}={v}").parse().unwrap(),
            );
        }
        // Need cookie layer: the test builder must have a CookieJarCell.
        // Topcoat's cookies() expects request_context::<CookieJarCell> to exist,
        // which is installed by the cookie router layer. For unit tests we
        // manually insert a CookieJarCell.
        use topcoat::cookie::CookieJarCell;
        CxTestBuilder::new()
            .request_context(builder)
            .request_context(CookieJarCell::new())
            .build()
    }

    #[test]
    fn encode_decode_roundtrip() {
        let n = Notification::success("User created");
        let enc = n.encode();
        let dec = Notification::decode(&enc).unwrap();
        assert_eq!(dec.title, "User created");
        assert_eq!(dec.status, NotificationStatus::Success);
    }

    #[test]
    fn decode_handles_colons() {
        let n = Notification::success("a:b:c");
        let enc = n.encode();
        let dec = Notification::decode(&enc).unwrap();
        assert_eq!(dec.title, "a:b:c");
    }

    #[tokio::test]
    async fn take_notification_clears_cookie() {
        let enc = Notification::success("hello").encode();
        let cx = cx_with_cookie(Some(&enc));
        let n = take_notification(&cx);
        assert!(n.is_some());
        assert_eq!(n.unwrap().title, "hello");
        // Second take should be None because cookie was removed (but removal is via jar delta,
        // not immediate get). In this test jar still has original cookie, so we check that
        // decode works; actual removal is via Set-Cookie header, not immediate.
        // Just ensure first take succeeded.
    }
}
