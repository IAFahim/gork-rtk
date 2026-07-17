//! Native Telegram phone remote for Gork Build.
//!
//! Architecture: long-poll Bot API + ACP client over `gork agent stdio`.
//! See `docs/TELEGRAM-NATIVE.md` in the gork-rtk repo.

pub mod acp_client;
pub mod auth;
pub mod runtime;
pub mod telegram;
pub mod ui;

pub use auth::AuthPolicy;
pub use runtime::{RuntimeConfig, run_telegram_remote};
