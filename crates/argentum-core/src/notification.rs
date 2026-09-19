//! `Notification` — transient user-visible message (CONTEXT.md).
//!
//! Produced by an `Action`'s result and rendered in the `Panel` shell's
//! top-level boundary so it survives `Table` swaps. Status + title (plus an
//! optional description), auto-dismissed after ~4s by
//! `argentum-ui/assets/notifications.js` and dismissible through the toast's
//! close button — the shadcn/Sonner toast surface (GH #151).
//!
//! The flash cookie is Topcoat's `CookieStore` (serde JSON, GH #139) instead
//! of a hand-rolled wire format; the jar defaults carry the hardened
//! attributes (HttpOnly, Secure, SameSite=Lax, Path=/ — the `__Host-` name
//! requires them, GH #149) on writes and removals alike, so set and clear
//! cannot drift again. One-time semantics ride the cookie alone: Topcoat
//! flushes `Set-Cookie` on error responses too (topcoat#408), so the mutation
//! `Err` redirects no longer need the old `?notification=` fallback.

use serde::{Deserialize, Serialize};
use topcoat::context::{Cx, try_request_context};
use topcoat::cookie::{CookieJar, CookieJarCell, Cookies, cookie_store, cookies};
use topcoat::icon::icon;
use topcoat::runtime::{Signal, shard, signal};
use topcoat::view::{Attributes, BoxView, View, ViewExt, attributes, view};

use argentum_ui::{
    icons, toast, toast_close, toast_content, toast_description, toast_icon, toast_title,
};

/// The kind of notification (status).
///
/// The serde tokens are lowercase so the JSON cookie reads naturally (GH #139).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationStatus {
    Success,
    Error,
    Info,
    Warning,
}

impl NotificationStatus {
    /// The lowercase token, shared with the flash cookie and `data-type`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Info => "info",
            Self::Warning => "warning",
        }
    }
}

/// A transient message shown after a mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub status: NotificationStatus,
    pub title: String,
    /// Optional supporting line under the title, rendered as the toast
    /// description. Absent (`None`) keeps the GH #139 cookie wire format, so
    /// cookies written before the field existed still decode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Notification {
    pub fn success(title: impl Into<String>) -> Self {
        Self::new(NotificationStatus::Success, title)
    }

    pub fn error(title: impl Into<String>) -> Self {
        Self::new(NotificationStatus::Error, title)
    }

    pub fn info(title: impl Into<String>) -> Self {
        Self::new(NotificationStatus::Info, title)
    }

    pub fn warning(title: impl Into<String>) -> Self {
        Self::new(NotificationStatus::Warning, title)
    }

    fn new(status: NotificationStatus, title: impl Into<String>) -> Self {
        Self {
            status,
            title: title.into(),
            description: None,
        }
    }

    /// Attach the supporting line under the title.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

pub(crate) const COOKIE_NAME: &str = "__Host-argentum_notification";

/// The jar defaults the flash cookie relies on (GH #139): `CookieStore`
/// commits a bare cookie, so the hardened attributes live here and apply to
/// writes *and* removals alike — the `Map` adapter transforms both. The
/// `__Host-` name requires Secure + Path=/ + no Domain (GH #149); the
/// session and CSRF cookies set their own attributes and are unaffected
/// (`default_*` only fills what is unset).
fn hardened(jar: &CookieJar) -> impl Cookies + '_ {
    jar.default_path("/")
        .default_http_only(true)
        .default_secure(true)
        .default_same_site(topcoat::cookie::SameSite::Lax)
}

/// Store a notification for the next request (flash).
pub fn set_notification(cx: &Cx, notification: Notification) {
    if try_request_context::<CookieJarCell>(cx).is_none() {
        return;
    }
    // `commit` serializes to JSON and queues the Set-Cookie — one hand-rolled
    // wire format less (GH #139). A failed commit must not fail the mutation
    // it rides on, but swallowing it retries a completed write with no toast
    // (GH #174) — log it for operators instead.
    if let Err(error) = cookie_store::<Notification, _>(hardened(cookies(cx)), COOKIE_NAME)
        .set(notification)
        .commit()
    {
        tracing::error!(error = %error, "flash notification commit failed");
    }
}

/// Flash the failure of a write that passed validation but did not land
/// (GH #174).
///
/// Every mutation handler (create, update, delete, bulk delete) routes its
/// write and commit failures through this before returning the error, so the
/// user gets "couldn't …" instead of a bare 500 that leaves them guessing
/// whether the write went through. The title names the operation, never the
/// driver's text — internals stay in the server log (GH #174 §1).
///
/// Delivery: the flash rides a `Set-Cookie`, and Topcoat's cookie layer writes
/// pending cookies on **both** paths — on `Err` it stashes them in
/// `response_headers`, which the router applies once the error response exists
/// (`topcoat-router/src/router.rs`). So the toast renders on the 500 page too,
/// which is the whole point of setting it before returning the error.
pub(crate) fn notify_write_failure(cx: &Cx, action: &str) {
    set_notification(
        cx,
        Notification::error(format!("Couldn't {action}"))
            .description("Nothing was changed — try again."),
    );
}

/// Take the notification from the request (if present) and clear it.
pub fn take_notification(cx: &Cx) -> Option<Notification> {
    try_request_context::<CookieJarCell>(cx)?;
    let unparsed = cookie_store::<Notification, _>(hardened(cookies(cx)), COOKIE_NAME);
    match unparsed.parse() {
        // Present and readable: hand it out, then expire it (the removal
        // carries the same hardened attributes through the jar defaults).
        Ok(Some(store)) => {
            let notification = store.get();
            store.remove();
            Some(notification)
        }
        // Unreadable garbage (a corrupted or foreign value): expire it, no
        // toast — same fail-open-to-none as the old decode.
        Err(_) => {
            cookie_store::<Notification, _>(hardened(cookies(cx)), COOKIE_NAME).remove();
            None
        }
        Ok(None) => None,
    }
}

/// Render one notification as the shadcn/Sonner toast (GH #151).
///
/// The status picks Sonner's icon under shadcn's theming (`circle-check`,
/// `info`, `triangle-alert`, `octagon-x`); the error icon reads
/// `text-destructive` so a failure cannot look like an info toast.
///
/// `attrs` are merged onto the toast surface. The shell's flash stack passes
/// an empty set; the live transport ([`live_toaster`]) adds a per-mount `id`
/// so re-rendering the same variant replaces the mounted toast instead of
/// re-syncing its `data-mounted` state (GH #154 §3).
pub async fn render_notification<'a>(
    cx: &'a Cx,
    notification: Notification,
    attrs: Attributes,
) -> topcoat::Result<BoxView<'a>> {
    let Notification {
        status,
        title,
        description,
    } = notification;
    let icon_data = match status {
        NotificationStatus::Success => icons::CIRCLE_CHECK,
        NotificationStatus::Error => icons::OCTAGON_X,
        NotificationStatus::Info => icons::INFO,
        NotificationStatus::Warning => icons::TRIANGLE_ALERT,
    };
    let icon_class = if status == NotificationStatus::Error {
        "size-4 text-destructive"
    } else {
        "size-4"
    };
    let status = status.as_str();
    Ok(view! {
        cx =>
        toast(
            attrs: attributes! { data-type=(status) (attrs) },
            toast_icon(icon(data: icon_data, attrs: attributes! { class=(icon_class) }))
            toast_content(
                toast_title((title))
                if let Some(description) = description {
                    toast_description((description))
                }
            )
            toast_close()
        )
    }
    .boxed())
}

/// The signals a page owns to mount a [`Notification`] in place (GH #154 §3).
///
/// [`live_toast`] creates them; a click handler writes a procedure's returned
/// `(status, title, description)` into them and bumps `serial` (a repeat of
/// the same variant must still be a change), and the shell's [`live_toaster`]
/// shard reads them and mounts the real Sonner surface — no navigation, no
/// scroll reset. Every value is client input by the time the shard reads it.
#[derive(Clone)]
pub struct LiveToast {
    /// The Sonner status token (`success`/`info`/`warning`/`error`); empty
    /// means "no toast mounted".
    pub status: Signal<String>,
    /// The toast title.
    pub title: Signal<String>,
    /// The optional supporting line; empty renders no description.
    pub description: Signal<String>,
    /// Bumped on every mount so an identical repeat still re-renders.
    pub serial: Signal<u64>,
}

/// The live toast signals for this request (call once per page).
///
/// The signals are created here, so both the page's handlers and the shell's
/// [`live_toaster`] resolve the same handles in one request.
pub fn live_toast(cx: &Cx) -> LiveToast {
    LiveToast {
        status: signal(cx, String::new),
        title: signal(cx, String::new),
        description: signal(cx, String::new),
        serial: signal(cx, || 0u64),
    }
}

/// The shell's live toaster shard (GH #154 §3): reads the page's
/// [`LiveToast`] signals and mounts the toast in place when one is set.
///
/// A shard rather than an eager read, so writing the signals re-renders only
/// this stack — the page, its scroll, and its focus stay put. The mount `id`
/// carries the serial: the browser's morph matches by id, so a new mount is a
/// fresh toast node (with `data-mounted="false"`) that `notifications.js`
/// arms, instead of rewriting the already-mounted one.
///
/// The body lives in [`render_live_toaster`]: the shard macro's generated
/// handler cannot name the request lifetime its `impl View` would capture, so
/// the helper resolves the boxed view and the shard only forwards it.
#[shard]
pub async fn live_toaster(
    cx: &Cx,
    status: Signal<String>,
    title: Signal<String>,
    description: Signal<String>,
    serial: Signal<u64>,
) -> topcoat::Result<impl View> {
    render_live_toaster(cx, &status, &title, &description, &serial).await
}

async fn render_live_toaster<'a>(
    cx: &'a Cx,
    status: &Signal<String>,
    title: &Signal<String>,
    description: &Signal<String>,
    serial: &Signal<u64>,
) -> topcoat::Result<BoxView<'a>> {
    // Runtime endpoints bypass page guards (topcoat shard contract), so the
    // shard restates the panel gate; a slot with no live toast needs no auth
    // to render empty, but a direct POST must not mount toasts unauthenticated.
    #[cfg(feature = "auth")]
    if crate::auth::enforced(cx) {
        crate::auth::require_authenticated(cx)?;
    }
    let status = status.get();
    let mount = serial.get();
    if status.is_empty() {
        // No `<span>` placeholder (GH #160): the shell mounts this slot inside
        // the toaster `<ol>`, which permits only `li`/`script`/`template`
        // children — the empty view renders nothing.
        return Ok(().boxed());
    }
    let title = title.get();
    let description = description.get();
    let notification = match status.as_str() {
        "success" => Notification::success(title),
        "warning" => Notification::warning(title),
        "error" => Notification::error(title),
        _ => Notification::info(title),
    };
    let notification = if description.is_empty() {
        notification
    } else {
        notification.description(description)
    };
    let mount_id = format!("live-toast-{mount}");
    render_notification(cx, notification, attributes! { cx => id=(mount_id) }).await
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

    /// GH #174: a failed write flashes an error the user can read — the
    /// operation, not the driver's error text.
    #[test]
    fn write_failure_notification_names_the_operation() {
        let cx = cx_with_cookie(None);
        notify_write_failure(&cx, "create the record");
        let notification = take_notification(&cx).expect("a failed write must flash");
        assert_eq!(notification.status, NotificationStatus::Error);
        assert_eq!(notification.title, "Couldn't create the record");
        assert!(
            notification.description.is_some(),
            "the toast must say the write did not land"
        );
    }

    #[test]
    fn take_notification_decodes_the_json_cookie() {
        let enc = serde_json::to_string(&Notification::success("hello")).unwrap();
        let cx = cx_with_cookie(Some(&enc));
        let n = take_notification(&cx);
        assert!(n.is_some(), "the JSON flash cookie decodes");
        assert_eq!(n.unwrap().title, "hello");
        // Clearing itself is pinned by
        // `notification_removal_header_carries_the_host_prefix_contract`.
    }

    #[test]
    fn percent_encoded_json_cookie_decodes() {
        let enc = "%7B%22status%22%3A%22success%22%2C%22title%22%3A%22Created%22%7D";
        let cx = cx_with_cookie(Some(enc));
        let n = take_notification(&cx);
        assert!(n.is_some(), "the percent-encoded flash cookie decodes");
        assert_eq!(n.unwrap().title, "Created");
    }

    /// Unreadable cookie garbage is expired, not toasted (same fail-open-to-
    /// none as the old decode), and the removal still satisfies the `__Host-`
    /// contract (GH #139).
    #[test]
    fn unreadable_flash_cookie_is_expired_silently() {
        let cx = cx_with_cookie(Some("not-json"));
        assert!(take_notification(&cx).is_none(), "garbage yields no toast");
        let mut headers = http::HeaderMap::new();
        topcoat::cookie::write_cookies(&cx, &mut headers);
        let cleared = headers
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{COOKIE_NAME}=")))
            .expect("the garbage cookie must be expired");
        assert!(
            cleared.contains("Max-Age=0")
                && cleared.contains("Secure")
                && cleared.contains("Path=/"),
            "the removal carries the __Host- contract: {cleared}"
        );
    }

    #[test]
    fn notification_description_round_trips() {
        // The description is optional and absent from the wire format when
        // unset (GH #139 compatibility); old cookies still decode.
        assert_eq!(
            serde_json::to_string(&Notification::success("hello")).unwrap(),
            r#"{"status":"success","title":"hello"}"#
        );
        let enc = serde_json::to_string(&Notification::warning("Careful").description("Low disk"))
            .unwrap();
        assert!(
            enc.contains("\"status\":\"warning\"") && enc.contains("\"description\":\"Low disk\"")
        );
        let back = take_notification(&cx_with_cookie(Some(&enc))).expect("decodes");
        assert_eq!(back.description.as_deref(), Some("Low disk"));
        let old = take_notification(&cx_with_cookie(Some(
            r#"{"status":"success","title":"hi"}"#,
        )))
        .expect("a pre-description cookie decodes");
        assert!(old.description.is_none());

        // The status token is shared with the cookie and `data-type`.
        assert_eq!(NotificationStatus::Success.as_str(), "success");
        assert_eq!(NotificationStatus::Error.as_str(), "error");
        assert_eq!(NotificationStatus::Info.as_str(), "info");
        assert_eq!(NotificationStatus::Warning.as_str(), "warning");
    }

    /// The flash cookie carries the hardened `__Host-` contract (GH #149):
    /// `Secure`, `HttpOnly`, `SameSite=Lax`, `Path=/`, no `Domain`.
    #[test]
    fn notification_cookie_is_host_prefixed_and_secure() {
        let cx = cx_with_cookie(None);
        set_notification(&cx, Notification::success("hello"));
        let cookie = cookies(&cx)
            .get(COOKIE_NAME)
            .expect("set_notification set the cookie");
        assert_eq!(cookie.name(), COOKIE_NAME);
        // The committed value is the JSON wire format (lowercase status
        // tokens, GH #139).
        assert_eq!(cookie.value(), r#"{"status":"success","title":"hello"}"#);
        assert!(cookie.secure().unwrap_or(false), "{cookie:?}");
        assert!(cookie.http_only().unwrap_or(false), "{cookie:?}");
        assert_eq!(cookie.path(), Some("/"));
        assert!(cookie.domain().is_none(), "{cookie:?}");
    }

    /// The consumed flash cookie must be cleared with a `__Host-`-conformant
    /// removal (GH #149): a `__Host-`-named `Set-Cookie` without `Secure` is
    /// ignored by browsers — `Max-Age=0` deletions included — so the flash
    /// would survive every navigation. Pinned here through topcoat's own
    /// response finalization; the create/edit flow end-to-end is covered by
    /// the panel test `mutation_redirect_carries_the_flash_cookie_instead_of_a_query`.
    #[test]
    fn notification_removal_header_carries_the_host_prefix_contract() {
        use http::header::SET_COOKIE;

        let enc = serde_json::to_string(&Notification::success("hello")).unwrap();
        let cx = cx_with_cookie(Some(&enc));
        let n = take_notification(&cx);
        assert!(n.is_some(), "the flash cookie must decode");

        let mut headers = http::HeaderMap::new();
        topcoat::cookie::write_cookies(&cx, &mut headers);
        let cleared = headers
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{COOKIE_NAME}=")))
            .expect("the consumed flash cookie must be cleared")
            .to_string();
        assert!(
            cleared.contains("Max-Age=0") || cleared.contains("Expires=Thu, 01 Jan 1970"),
            "the removal header must expire the cookie: {cleared}"
        );
        assert!(
            cleared.contains("Secure") && cleared.contains("Path=/"),
            "the removal header must satisfy the __Host- contract: {cleared}"
        );
        assert!(
            cleared.split(';').all(|attr| {
                let attr = attr.trim();
                !attr.starts_with("Domain=")
            }),
            "a `__Host-` cookie must not carry a Domain: {cleared}"
        );
    }
}
