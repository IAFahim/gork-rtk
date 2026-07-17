//! `gork-telegram` — complete native phone remote for Gork Build.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use xai_gork_telegram::{run_telegram_remote, RuntimeConfig};

#[derive(Debug, Parser)]
#[command(
    name = "gork-telegram",
    about = "Native Telegram remote for Gork Build — full phone control (ACP)"
)]
struct Args {
    #[arg(long, env = "TELEGRAM_BOT_TOKEN")]
    token: String,

    #[arg(long, env = "ALLOWED_USER_IDS")]
    allowed_users: String,

    #[arg(long, env = "GORK_TELEGRAM_CWD", default_value = ".")]
    cwd: PathBuf,

    #[arg(long, env = "GORK_BIN")]
    agent: Option<PathBuf>,

    #[arg(long, env = "GORK_TELEGRAM_SESSION")]
    session: Option<String>,

    #[arg(long, env = "ORCHESTRATOR_HOST_ID", default_value = "this-pc")]
    host_id: String,

    /// Sessions root (default ~/.grok/sessions)
    #[arg(long, env = "GROK_SESSIONS_ROOT")]
    sessions_root: Option<PathBuf>,

    /// Data dir for notes + inbound media (default ~/.grok/telegram-native)
    #[arg(long, env = "GORK_TELEGRAM_DATA")]
    data_dir: Option<PathBuf>,
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
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let sessions_root = args
        .sessions_root
        .unwrap_or_else(|| home.join(".grok/sessions"));
    let data_dir = args
        .data_dir
        .unwrap_or_else(|| home.join(".grok/telegram-native"));

    run_telegram_remote(RuntimeConfig {
        bot_token: args.token,
        allowed_user_ids: args.allowed_users,
        cwd,
        agent_bin: args.agent,
        session_id: args.session,
        host_id: args.host_id,
        sessions_root,
        data_dir,
    })
    .await
}
