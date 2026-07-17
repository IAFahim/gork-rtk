//! `gork-telegram` — native phone remote for Gork Build.
//!
//! Env:
//!   TELEGRAM_BOT_TOKEN   (required)
//!   ALLOWED_USER_IDS     (required, comma-separated)
//!   GORK_TELEGRAM_CWD    (optional working directory)
//!   GORK_BIN / GROK_BINARY (optional agent path)
//!   ORCHESTRATOR_HOST_ID (optional label)

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use xai_gork_telegram::{run_telegram_remote, RuntimeConfig};

#[derive(Debug, Parser)]
#[command(
    name = "gork-telegram",
    about = "Native Telegram remote for Gork Build (ACP + phone full control)"
)]
struct Args {
    /// Telegram bot token (or TELEGRAM_BOT_TOKEN).
    #[arg(long, env = "TELEGRAM_BOT_TOKEN")]
    token: String,

    /// Comma-separated allowed Telegram user ids (or ALLOWED_USER_IDS).
    #[arg(long, env = "ALLOWED_USER_IDS")]
    allowed_users: String,

    /// Working directory for the agent session.
    #[arg(long, env = "GORK_TELEGRAM_CWD", default_value = ".")]
    cwd: PathBuf,

    /// Path to gork/grok binary (default: same dir as this binary, or PATH).
    #[arg(long, env = "GORK_BIN")]
    agent: Option<PathBuf>,

    /// Optional existing session id to load.
    #[arg(long, env = "GORK_TELEGRAM_SESSION")]
    session: Option<String>,

    /// Host label shown on /start.
    #[arg(long, env = "ORCHESTRATOR_HOST_ID", default_value = "this-pc")]
    host_id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let cwd = args.cwd.canonicalize().unwrap_or(args.cwd);

    run_telegram_remote(RuntimeConfig {
        bot_token: args.token,
        allowed_user_ids: args.allowed_users,
        cwd,
        agent_bin: args.agent,
        session_id: args.session,
        host_id: args.host_id,
    })
    .await
}
