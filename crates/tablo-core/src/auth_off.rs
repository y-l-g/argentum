//! The `auth` feature compiled out (ADR-0013): no sessions, no login, no gate.
//! What remains is the one value that says so. `Panel::build` refuses a panel
//! that has not been handed it, so an ungated panel is always a line of app
//! code, never a side effect of trimming dependencies.

/// Authentication configuration when the `auth` feature is off: only the
/// explicit opt-out exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Auth(());

impl Auth {
    /// Serve the panel without authentication. Every page and mutation answers
    /// anyone who can reach it.
    #[must_use]
    pub fn disabled() -> Self {
        Self(())
    }

    /// Always `true`: the feature-off build has no other configuration.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        true
    }
}
