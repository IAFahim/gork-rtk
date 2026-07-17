//! Native Telegram phone remote for Gork Build.
//!
//! Complete phone control: sessions, notes queue, media, live ACP, holds.
//! See `docs/TELEGRAM-NATIVE.md`.

pub mod acp_client;
pub mod auth;
pub mod notes;
pub mod runtime;
pub mod sessions;
pub mod telegram;
pub mod ui;

pub use auth::AuthPolicy;
pub use runtime::{run_telegram_remote, RuntimeConfig};
