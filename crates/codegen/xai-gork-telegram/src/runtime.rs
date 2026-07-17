//! Main long-poll loop: Telegram ↔ ACP.

use crate::acp_client::{resolve_agent_bin, AcpClient, AcpEvent, ServerRequest};
use crate::auth::AuthPolicy;
use crate::telegram::{esc, TelegramBot};
use crate::ui::card_for_request;
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub bot_token: String,
    pub allowed_user_ids: String,
    pub cwd: PathBuf,
    pub agent_bin: Option<PathBuf>,
    pub session_id: Option<String>,
    pub host_id: String,
}

struct ChatSession {
    /// Pending reverse-request ids keyed by short callback prefix.
    pending_rpc: HashMap<String, (serde_json::Value, ServerRequest)>,
    agent_buf: String,
}

pub async fn run_telegram_remote(cfg: RuntimeConfig) -> Result<()> {
    let auth = AuthPolicy::from_csv(&cfg.allowed_user_ids);
    if auth.is_empty() {
        bail!("ALLOWED_USER_IDS is empty — fail-closed. Set comma-separated Telegram user ids.");
    }
    if cfg.bot_token.trim().is_empty() || cfg.bot_token.contains("REPLACE") {
        bail!("TELEGRAM_BOT_TOKEN missing or placeholder");
    }

    let bot = TelegramBot::new(cfg.bot_token.clone());
    let me = bot.get_me().await.context("getMe — bad token?")?;
    info!(?me, "telegram bot ok");

    let agent_bin = resolve_agent_bin(cfg.agent_bin.clone());
    info!(bin = %agent_bin.display(), cwd = %cfg.cwd.display(), "starting agent");

    let (acp, mut events) = AcpClient::spawn(&agent_bin, &cfg.cwd)
        .await
        .context("spawn agent stdio")?;

    if let Some(sid) = &cfg.session_id {
        match acp.session_load(sid, &cfg.cwd).await {
            Ok(()) => info!(sid, "session loaded"),
            Err(e) => {
                warn!(%e, "session/load failed; creating new");
                let new_id = acp.session_new(&cfg.cwd).await?;
                info!(new_id, "session/new");
            }
        }
    } else {
        let new_id = acp.session_new(&cfg.cwd).await?;
        info!(new_id, "session/new");
    }

    let acp = Arc::new(Mutex::new(acp));
    let sess = Arc::new(Mutex::new(ChatSession {
        pending_rpc: HashMap::new(),
        agent_buf: String::new(),
    }));

    // Event fan-out to last chat that talked to us
    let last_chat = Arc::new(Mutex::new(None::<i64>));
    let bot_ev = bot.clone();
    let sess_ev = sess.clone();
    let last_chat_ev = last_chat.clone();
    let acp_ev = acp.clone();
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            let chat_id = *last_chat_ev.lock().await;
            match ev {
                AcpEvent::AgentText(t) => {
                    let mut s = sess_ev.lock().await;
                    s.agent_buf.push_str(&t);
                }
                AcpEvent::ToolTitle(t) => {
                    if let Some(cid) = chat_id {
                        let _ = bot_ev
                            .send_message(cid, &format!("🛠 <b>{}</b>", esc(&t)), None)
                            .await;
                    }
                }
                AcpEvent::ServerRequest(req) => {
                    if let Some(cid) = chat_id {
                        let (text, markup) = card_for_request(&req);
                        let key = match &req {
                            ServerRequest::Permission { .. } => "perm",
                            ServerRequest::AskUser { .. } => "opt",
                            ServerRequest::ExitPlan { .. } => "plan",
                            ServerRequest::Other { .. } => "other",
                        };
                        {
                            let mut s = sess_ev.lock().await;
                            let id = match &req {
                                ServerRequest::Permission { id, .. }
                                | ServerRequest::AskUser { id, .. }
                                | ServerRequest::ExitPlan { id, .. }
                                | ServerRequest::Other { id, .. } => id.clone(),
                            };
                            s.pending_rpc
                                .insert(key.to_string(), (id, req.clone()));
                        }
                        let _ = bot_ev.send_message(cid, &text, markup).await;
                    }
                }
                AcpEvent::TurnDone { stop_reason } => {
                    if let Some(cid) = chat_id {
                        let mut s = sess_ev.lock().await;
                        let body = s.agent_buf.trim().to_string();
                        s.agent_buf.clear();
                        let msg = if body.is_empty() {
                            format!(
                                "🟢 <b>Turn finished</b>\n<i>{}</i>",
                                esc(&stop_reason)
                            )
                        } else {
                            let clipped = if body.len() > 3500 {
                                format!("{}…", &body[..3499])
                            } else {
                                body
                            };
                            format!("🟢 <b>Gork</b>\n{}", esc(&clipped))
                        };
                        let _ = bot_ev.send_message(cid, &msg, None).await;
                    }
                }
                AcpEvent::Error(e) => {
                    error!(%e, "acp event error");
                    // If agent died, surface once
                    if let Some(cid) = chat_id {
                        let _ = bot_ev
                            .send_message(
                                cid,
                                &format!("❌ Agent: {}", esc(&e)),
                                None,
                            )
                            .await;
                    }
                    // keep loop; outer will fail on next prompt
                    let _ = acp_ev.lock().await;
                }
            }
        }
    });

    let mut offset: i64 = 0;
    info!(host = %cfg.host_id, "gork telegram remote running — message the bot");

    // greet: nothing until first authorized message

    loop {
        let updates = match bot.get_updates(offset, 25).await {
            Ok(u) => u,
            Err(e) => {
                warn!(%e, "getUpdates error; retry");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        for u in updates {
            offset = u.update_id + 1;
            if let Some(cq) = u.callback_query {
                if !auth.authorize(Some(cq.from.id)) {
                    let _ = bot
                        .answer_callback(&cq.id, "Not authorized")
                        .await;
                    continue;
                }
                let chat_id = cq.message.as_ref().map(|m| m.chat.id);
                if let Some(cid) = chat_id {
                    *last_chat.lock().await = Some(cid);
                }
                let data = cq.data.unwrap_or_default();
                handle_callback(&bot, &acp, &sess, &cq.id, chat_id, &data).await;
                continue;
            }
            if let Some(msg) = u.message {
                let uid = msg.from.as_ref().map(|u| u.id);
                if !auth.authorize(uid) {
                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            &format!(
                                "⛔ Not authorized.\nYour id: <code>{}</code>\nAdd to ALLOWED_USER_IDS.",
                                uid.map(|i| i.to_string()).unwrap_or_else(|| "?".into())
                            ),
                            None,
                        )
                        .await;
                    continue;
                }
                *last_chat.lock().await = Some(msg.chat.id);
                let text = msg.text.unwrap_or_default();
                if text.is_empty() {
                    continue;
                }
                if text.starts_with("/start") || text.starts_with("/help") {
                    let sid = acp.lock().await.session_id().unwrap_or_default();
                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            &format!(
                                "<b>✦ Gork phone remote</b>\n\
                                 host <code>{}</code>\n\
                                 session <code>{}</code>\n\
                                 cwd <code>{}</code>\n\n\
                                 Type freely — messages go to the live agent.\n\
                                 Permissions / questions / plans show as buttons.",
                                esc(&cfg.host_id),
                                esc(&sid),
                                esc(&cfg.cwd.display().to_string()),
                            ),
                            None,
                        )
                        .await;
                    continue;
                }
                if text.starts_with('/') {
                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            "Unknown command. Just type a message for the agent, or /help.",
                            None,
                        )
                        .await;
                    continue;
                }

                // free-text → ACP prompt (may hold for permission mid-turn)
                {
                    let mut s = sess.lock().await;
                    s.agent_buf.clear();
                }
                let _ = bot
                    .send_message(msg.chat.id, "⏳ Working…", None)
                    .await;
                let acp_g = acp.lock().await;
                match acp_g.session_prompt(&text).await {
                    Ok(stop) => {
                        // TurnDone event also fires; if buffer empty event handles it
                        tracing::debug!(%stop, "prompt done");
                    }
                    Err(e) => {
                        let _ = bot
                            .send_message(
                                msg.chat.id,
                                &format!("❌ Prompt failed: {}", esc(&e.to_string())),
                                None,
                            )
                            .await;
                    }
                }
            }
        }
    }
}

async fn handle_callback(
    bot: &TelegramBot,
    acp: &Arc<Mutex<AcpClient>>,
    sess: &Arc<Mutex<ChatSession>>,
    callback_id: &str,
    chat_id: Option<i64>,
    data: &str,
) {
    let _ = bot.answer_callback(callback_id, "OK").await;
    let Some(cid) = chat_id else { return };

    if let Some(oid) = data.strip_prefix("p:") {
        let pending = {
            let mut s = sess.lock().await;
            s.pending_rpc.remove("perm")
        };
        if let Some((id, _)) = pending {
            let acp = acp.lock().await;
            let result = json!({
                "outcome": { "outcome": "selected", "optionId": oid }
            });
            if let Err(e) = acp.respond(id, result).await {
                let _ = bot
                    .send_message(cid, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
            } else {
                let _ = bot
                    .send_message(cid, &format!("✅ Permission: <code>{}</code>", esc(oid)), None)
                    .await;
            }
        } else {
            let _ = bot
                .send_message(cid, "No pending permission.", None)
                .await;
        }
        return;
    }

    if let Some(qid) = data.strip_prefix("q:") {
        let pending = {
            let mut s = sess.lock().await;
            s.pending_rpc.remove("opt")
        };
        if let Some((id, ServerRequest::AskUser { question, options, .. })) = pending {
            let label = options
                .iter()
                .find(|(i, _)| i == qid)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| qid.to_string());
            let result = json!({
                "outcome": "accepted",
                "answers": { question: [label] }
            });
            let acp = acp.lock().await;
            if let Err(e) = acp.respond(id, result).await {
                let _ = bot
                    .send_message(cid, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
            } else {
                let _ = bot.send_message(cid, "✅ Answered", None).await;
            }
        }
        return;
    }

    if let Some(action) = data.strip_prefix("pl:") {
        let pending = {
            let mut s = sess.lock().await;
            s.pending_rpc.remove("plan")
        };
        if let Some((id, _)) = pending {
            let outcome = match action {
                "approve" => json!({ "outcome": "approved" }),
                "quit" => json!({ "outcome": "abandoned" }),
                _ => json!({ "outcome": "cancelled", "feedback": "revise from phone" }),
            };
            let acp = acp.lock().await;
            if let Err(e) = acp.respond(id, outcome).await {
                let _ = bot
                    .send_message(cid, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
            } else {
                let _ = bot
                    .send_message(cid, &format!("✅ Plan: <code>{}</code>", esc(action)), None)
                    .await;
            }
        }
    }
}


