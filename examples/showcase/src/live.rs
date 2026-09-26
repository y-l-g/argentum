//! The live feed: a Topcoat shard whose WebSocket connection keeps every open
//! page current.
//!
//! The page renders `live_feed`, a shard, so a write re-renders the list
//! without touching the page around it. The shard's body is a `live!` region
//! that calls `connected`: during the HTTP render it emits the newest users and
//! finishes, and the browser then opens a WebSocket and renders the shard
//! again. On that render the body emits the current rows and waits on the
//! board's wake channel, re-reading and emitting again after every write. The
//! connection belongs to the shard, so the rest of the shell stays on HTTP and
//! no polling script runs.
//!
//! The rows come from the request's own database, so one router's feed never
//! shows another router's writes. The wake channel is a process-global because
//! `Panel::app_context` carries only the `Db`; an app that owns a context of
//! its own keeps the sender beside it instead.

use std::sync::LazyLock;

use tablo_core::{Resource, db::db};
use topcoat::{
    Result,
    context::Cx,
    router::page,
    runtime::{connected, shard},
    view::{View, emit, live, view},
};

use crate::{app::UserResource, models::User};

/// Where the live feed lives.
pub const LIVE_PATH: &str = "/admin/live";

/// How many of the newest users the feed shows.
const FEED_ROWS: usize = 20;

/// The process-wide wake channel behind every open feed.
static BOARD: LazyLock<Board> = LazyLock::new(Board::default);

/// Wakes every connected feed after a committed write.
///
/// The board carries no event: a wake only tells each feed to re-read the rows
/// it shows, so the rows stay per-request and the channel stays small.
struct Board {
    changed: tokio::sync::broadcast::Sender<()>,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            changed: tokio::sync::broadcast::channel(16).0,
        }
    }
}

impl Board {
    /// Wake every connected feed.
    fn notify(&self) {
        // No connection listening is not a failure: the next HTTP render reads
        // the rows.
        let _ = self.changed.send(());
    }

    /// Subscribe before reading the rows, so a write between the read and the
    /// wait is not missed.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }
}

/// Wake every connected feed the process holds.
pub(crate) fn notify() {
    BOARD.notify();
}

/// Subscribe to the process-wide board.
fn subscribe() -> tokio::sync::broadcast::Receiver<()> {
    BOARD.subscribe()
}

/// `GET /admin/live` — the panel page holding the live feed.
///
/// The attribute spells the path [`LIVE_PATH`] names; the integration test
/// requests `LIVE_PATH`, so a drift between the two fails the test.
#[page("/admin/live")]
async fn live_page(cx: &Cx) -> Result<impl View> {
    Ok(view! {
        cx =>
        tablo_ui::page(
            tablo_ui::page_header(
                tablo_ui::page_title("Live activity")
                tablo_ui::page_description(
                    "The newest users, re-read after every committed write and pushed over a WebSocket."
                )
            )
            tablo_ui::page_content(
                tablo_ui::card(
                    tablo_ui::card_header(tablo_ui::card_title("Newest users"))
                    tablo_ui::card_content(live_feed())
                )
            )
        )
    })
}

/// The feed itself: a shard whose live region streams the newest users to every
/// open page.
///
/// The first emission is what the HTTP render sends; `connected` then returns
/// false and asks the browser for a connection. The connected render runs the
/// body from the top, so it emits the current rows again before it waits — a
/// write between the two renders cannot be missed.
#[shard]
async fn live_feed(cx: &Cx) -> Result<impl View> {
    // Runtime endpoints bypass page guards (Topcoat's shard contract), so the
    // shard restates the panel gate: a request without a permitted user must
    // not read the rows, over HTTP or over the connection.
    if tablo_core::auth::enforced(cx) {
        tablo_core::auth::require_authenticated(cx)?;
    }
    Ok(live! {
        let mut changed = subscribe();
        loop {
            let users = newest_users(cx).await?;
            // The emptiness check borrows `users`, and the `else` arm then moves
            // it into the view; the bool keeps the two uses apart.
            let empty = users.is_empty();
            let token = emit! {
                if empty {
                    <p class="text-sm text-muted-foreground" data-live-feed="">
                        "No users yet."
                    </p>
                } else {
                    <ul class="flex flex-col gap-2" data-live-feed="">
                        for user in users {
                            <li class="text-sm">(user.name)</li>
                        }
                    </ul>
                }
            }?;
            if !connected(cx) {
                break Ok(token);
            }
            match changed.recv().await {
                // Re-emit immediately: the next pass re-reads the rows.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                // The board lives for the process, so this arm is unreachable;
                // ending the region is the safe answer if that changes.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break Ok(token),
                Ok(()) => {}
            }
        }
    })
}

/// The newest users the panel serves, newest first.
async fn newest_users(cx: &Cx) -> Result<Vec<User>> {
    let mut db = db(cx);
    let users = UserResource::query(cx)
        .order_by(User::fields().created_at().desc())
        .limit(FEED_ROWS)
        .exec(&mut db)
        .await?;
    Ok(users)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tablo_core::Committed;

    use super::*;

    /// A wake reaches a subscriber: a feed waiting on a board resumes when the
    /// board is notified.
    #[tokio::test]
    async fn notify_wakes_a_subscriber() {
        let board = Board::default();
        let mut changed = board.subscribe();
        board.notify();

        tokio::time::timeout(Duration::from_secs(1), changed.recv())
            .await
            .expect("notify must wake a subscriber")
            .expect("the sender outlives the receiver");
    }

    /// The resource hook wakes the process-wide board, so a committed write
    /// re-runs every connected feed.
    #[tokio::test]
    async fn a_committed_write_wakes_the_feed() {
        let mut changed = subscribe();
        let user = User {
            id: uuid::Uuid::new_v4(),
            name: "Ada".to_string(),
            email: "ada@example.com".to_string(),
            role: "admin".to_string(),
            active: true,
            age: 36,
            created_at: jiff::Timestamp::now(),
        };
        UserResource::after_commit(&Cx::default(), Committed::created(user))
            .await
            .expect("the hook notifies the board");

        tokio::time::timeout(Duration::from_secs(1), changed.recv())
            .await
            .expect("a committed write must wake the feed")
            .expect("the sender outlives the receiver");
    }
}
