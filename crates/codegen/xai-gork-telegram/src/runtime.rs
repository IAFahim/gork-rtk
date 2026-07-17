//! Complete phone remote loop: sessions, notes, media, live ACP.

use crate::acp_client::{resolve_agent_bin, AcpClient, AcpEvent, ServerRequest};
use crate::auth::AuthPolicy;
use crate::notes::NoteQueue;
use crate::sessions::{list_sessions, peek_history, SessionEntry};
use crate::telegram::{esc, inline_keyboard, TelegramBot};
use crate::ui::card_for_request;
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub bot_token: String,
    pub allowed_user_ids: String,
    pub cwd: PathBuf,
    pub agent_bin: Option<PathBuf>,
    pub session_id: Option<String>,
    pub host_id: String,
    pub sessions_root: PathBuf,
    pub data_dir: PathBuf,
}

/// Shared with the ACP event fan-out task (must run during long session/prompt).
struct SharedUi {
    bot: TelegramBot,
    last_chat: Option<i64>,
    pending_rpc: HashMap<String, (serde_json::Value, ServerRequest)>,
    agent_buf: String,
}

struct LiveSlot {
    /// Shared so mid-turn permission responds while prompt is in-flight.
    acp: Arc<AcpClient>,
    prompt_busy: Arc<tokio::sync::Mutex<bool>>,
}

struct App {
    cfg: RuntimeConfig,
    bot: TelegramBot,
    auth: AuthPolicy,
    notes: NoteQueue,
    /// live agent (None = notes-only)
    live: Option<LiveSlot>,
    shared: Arc<tokio::sync::Mutex<SharedUi>>,
    active_session: Option<String>,
    active_cwd: PathBuf,
    event_task: Option<tokio::task::JoinHandle<()>>,
}

pub async fn run_telegram_remote(cfg: RuntimeConfig) -> Result<()> {
    let auth = AuthPolicy::from_csv(&cfg.allowed_user_ids);
    if auth.is_empty() {
        bail!("ALLOWED_USER_IDS is empty — fail-closed.");
    }
    if cfg.bot_token.trim().is_empty() || cfg.bot_token.contains("REPLACE") {
        bail!("TELEGRAM_BOT_TOKEN missing or placeholder");
    }

    std::fs::create_dir_all(&cfg.data_dir)?;
    let notes = NoteQueue::open(&cfg.data_dir)?;
    let bot = TelegramBot::new(cfg.bot_token.clone());
    let me = bot.get_me().await.context("getMe")?;
    info!(?me, "telegram bot ok");

    let shared = Arc::new(tokio::sync::Mutex::new(SharedUi {
        bot: bot.clone(),
        last_chat: None,
        pending_rpc: HashMap::new(),
        agent_buf: String::new(),
    }));

    let mut app = App {
        active_cwd: cfg.cwd.clone(),
        active_session: cfg.session_id.clone(),
        cfg,
        bot,
        auth,
        notes,
        live: None,
        shared,
        event_task: None,
    };

    let mut offset: i64 = 0;
    info!(host = %app.cfg.host_id, "gork telegram COMPLETE remote running");

    loop {
        let updates = match app.bot.get_updates(offset, 20).await {
            Ok(u) => u,
            Err(e) => {
                warn!(%e, "getUpdates");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        for u in updates {
            offset = u.update_id + 1;
            if let Some(cq) = u.callback_query {
                if !app.auth.authorize(Some(cq.from.id)) {
                    let _ = app.bot.answer_callback(&cq.id, "Not authorized").await;
                    continue;
                }
                if let Some(ref m) = cq.message {
                    app.shared.lock().await.last_chat = Some(m.chat.id);
                }
                let data = cq.data.clone().unwrap_or_default();
                let chat_id = cq.message.as_ref().map(|m| m.chat.id);
                app.handle_callback(&cq.id, chat_id, &data).await;
                continue;
            }
            if let Some(msg) = u.message {
                let uid = msg.from.as_ref().map(|u| u.id);
                if !app.auth.authorize(uid) {
                    let _ = app
                        .bot
                        .send_message(
                            msg.chat.id,
                            &format!(
                                "⛔ Not authorized.\nYour id: <code>{}</code>",
                                uid.unwrap_or(0)
                            ),
                            None,
                        )
                        .await;
                    continue;
                }
                app.shared.lock().await.last_chat = Some(msg.chat.id);
                app.handle_message(msg).await;
            }
        }
    }
}

async fn fanout_acp_events(
    mut rx: mpsc::UnboundedReceiver<AcpEvent>,
    shared: Arc<tokio::sync::Mutex<SharedUi>>,
) {
    while let Some(ev) = rx.recv().await {
        let mut s = shared.lock().await;
        match ev {
            AcpEvent::AgentText(t) => s.agent_buf.push_str(&t),
            AcpEvent::ToolTitle(t) => {
                if let Some(cid) = s.last_chat {
                    let _ = s
                        .bot
                        .send_message(cid, &format!("🛠 <b>{}</b>", esc(&t)), None)
                        .await;
                }
            }
            AcpEvent::ServerRequest(req) => {
                if let Some(cid) = s.last_chat {
                    let (text, markup) = card_for_request(&req);
                    let key = match &req {
                        ServerRequest::Permission { .. } => "perm",
                        ServerRequest::AskUser { .. } => "opt",
                        ServerRequest::ExitPlan { .. } => "plan",
                        ServerRequest::Other { .. } => "other",
                    };
                    let id = match &req {
                        ServerRequest::Permission { id, .. }
                        | ServerRequest::AskUser { id, .. }
                        | ServerRequest::ExitPlan { id, .. }
                        | ServerRequest::Other { id, .. } => id.clone(),
                    };
                    s.pending_rpc.insert(key.to_string(), (id, req));
                    let _ = s.bot.send_message(cid, &text, markup).await;
                }
            }
            AcpEvent::TurnDone { stop_reason } => {
                if let Some(cid) = s.last_chat {
                    let body = s.agent_buf.trim().to_string();
                    s.agent_buf.clear();
                    let msg = if body.is_empty() {
                        format!("🟢 <b>Turn finished</b>\n<i>{}</i>", esc(&stop_reason))
                    } else {
                        let clipped = if body.len() > 3500 {
                            format!("{}…", &body[..3499])
                        } else {
                            body
                        };
                        format!("🟢 <b>Gork</b>\n{}", esc(&clipped))
                    };
                    let _ = s.bot.send_message(cid, &msg, None).await;
                }
            }
            AcpEvent::Error(e) => {
                error!(%e, "acp");
                if let Some(cid) = s.last_chat {
                    let _ = s
                        .bot
                        .send_message(cid, &format!("❌ Agent: {}", esc(&e)), None)
                        .await;
                }
            }
        }
    }
}

impl App {

    fn home_keyboard(&self) -> serde_json::Value {
        let live_label = if self.live.is_some() {
            "⏹ Stop live"
        } else {
            "⚡ Go live"
        };
        let live_cb = if self.live.is_some() {
            "nav:stop"
        } else {
            "nav:live"
        };
        inline_keyboard(vec![
            vec![
                ("📚 Sessions".into(), "nav:sessions".into()),
                ("✨ New".into(), "nav:new".into()),
            ],
            vec![
                ("📋 Notes".into(), "nav:queue".into()),
                (live_label.into(), live_cb.into()),
            ],
            vec![
                ("▶️ Drain→live".into(), "nav:drain".into()),
                ("📜 History".into(), "nav:history".into()),
            ],
            vec![("🏠 Home".into(), "nav:home".into())],
        ])
    }

    fn help_text(&self) -> String {
        format!(
            "<b>✦ Gork phone remote (native)</b>\n\
             host <code>{}</code>\n\
             live: <b>{}</b>\n\
             session <code>{}</code>\n\
             cwd <code>{}</code>\n\
             notes: <b>{}</b>\n\n\
             <b>Loop</b>\n\
             1. Sessions → Use a chat (or New)\n\
             2. Type freely (offline → Notes; live → agent)\n\
             3. Go live for tools / plans / replies\n\
             4. Photo/file: saved + live path prompt\n\
             5. Drain: send parked notes into live\n\n\
             /home /sessions /new /live /stop /queue /drain /history /status /help",
            esc(&self.cfg.host_id),
            if self.live.is_some() { "ON" } else { "off" },
            esc(self.active_session.as_deref().unwrap_or("—")),
            esc(&self.active_cwd.display().to_string()),
            self.notes.count(),
        )
    }

    async fn handle_message(&mut self, msg: crate::telegram::Message) {
        let chat_id = msg.chat.id;

        // Media first
        if msg.photo.is_some() || msg.document.is_some() {
            self.handle_media(chat_id, &msg).await;
            return;
        }

        let text = msg.text.unwrap_or_default();
        if text.is_empty() {
            return;
        }
        let cmd = text.split_whitespace().next().unwrap_or("");
        let cmd = cmd.split('@').next().unwrap_or(cmd); // /start@bot

        match cmd {
            "/start" | "/help" => {
                let _ = self
                    .bot
                    .send_message(chat_id, &self.help_text(), Some(self.home_keyboard()))
                    .await;
            }
            "/home" | "/status" => {
                let _ = self
                    .bot
                    .send_message(chat_id, &self.help_text(), Some(self.home_keyboard()))
                    .await;
            }
            "/sessions" => self.send_sessions(chat_id).await,
            "/new" => self.cmd_new(chat_id).await,
            "/live" => self.cmd_live(chat_id).await,
            "/stop" => self.cmd_stop(chat_id).await,
            "/queue" | "/notes" => self.send_queue(chat_id).await,
            "/drain" => self.cmd_drain(chat_id).await,
            "/history" => self.cmd_history(chat_id).await,
            _ if text.starts_with('/') => {
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        "Unknown command. /help for the phone loop.",
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
            _ => self.handle_free_text(chat_id, &text).await,
        }
    }

    async fn handle_free_text(&mut self, chat_id: i64, text: &str) {
        // Free-text answers a held multi-option question if present
        {
            let pending = {
                let mut s = self.shared.lock().await;
                s.pending_rpc.remove("opt")
            };
            if let Some((id, ServerRequest::AskUser { question, .. })) = pending {
                if let Some(slot) = self.live.as_ref() {
                    let result = json!({
                        "outcome": "accepted",
                        "answers": { question.clone(): ["Other"] },
                        "annotations": { question: { "notes": text } }
                    });
                    match slot.acp.respond(id, result).await {
                        Ok(()) => {
                            let _ = self
                                .bot
                                .send_message(chat_id, "✅ Free-text answer sent", None)
                                .await;
                        }
                        Err(e) => {
                            let _ = self
                                .bot
                                .send_message(chat_id, &format!("❌ {}", esc(&e.to_string())), None)
                                .await;
                        }
                    }
                    return;
                }
            }
        }

        if let Some(slot) = self.live.as_ref() {
            {
                let mut busy = slot.prompt_busy.lock().await;
                if *busy {
                    let _ = self
                        .bot
                        .send_message(
                            chat_id,
                            "⏳ Still working on the previous turn — wait or answer a button.",
                            None,
                        )
                        .await;
                    return;
                }
                *busy = true;
            }
            self.shared.lock().await.agent_buf.clear();
            let _ = self.bot.send_message(chat_id, "⏳ Working…", None).await;
            // Spawn so getUpdates keeps running for permission buttons mid-turn
            let acp = slot.acp.clone();
            let busy = slot.prompt_busy.clone();
            let bot = self.bot.clone();
            let text = text.to_string();
            tokio::spawn(async move {
                let res = acp.session_prompt(&text).await;
                *busy.lock().await = false;
                if let Err(e) = res {
                    let _ = bot
                        .send_message(
                            chat_id,
                            &format!("❌ Prompt failed: {}", esc(&e.to_string())),
                            None,
                        )
                        .await;
                }
            });
            return;
        }

        // Offline: park note
        match self
            .notes
            .enqueue(text, self.active_session.clone(), None, false)
        {
            Ok(n) => {
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        &format!(
                            "📋 <b>Parked note</b> <code>{}</code>\n\
                             Live is off — Go live then Drain, or /live then type again.\n\
                             Notes waiting: <b>{}</b>",
                            esc(&n.id.chars().take(8).collect::<String>()),
                            self.notes.count()
                        ),
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .bot
                    .send_message(chat_id, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
            }
        }
    }

    async fn handle_media(&mut self, chat_id: i64, msg: &crate::telegram::Message) {
        let (file_id, filename, is_image) = if let Some(ref photos) = msg.photo {
            let best = photos.last().unwrap();
            (best.file_id.clone(), "photo.jpg".to_string(), true)
        } else if let Some(ref doc) = msg.document {
            (
                doc.file_id.clone(),
                doc.file_name.clone().unwrap_or_else(|| "document.bin".into()),
                false,
            )
        } else {
            return;
        };

        let inbound = self.cfg.data_dir.join("inbound");
        let dest = inbound.join(format!(
            "{}_{}",
            uuid::Uuid::new_v4(),
            sanitize_name(&filename)
        ));
        let _ = self
            .bot
            .send_message(chat_id, "📥 Saving…", None)
            .await;
        match self.bot.get_file_path(&file_id).await {
            Ok(fp) => {
                if let Err(e) = self.bot.download_file(&fp, &dest).await {
                    let _ = self
                        .bot
                        .send_message(chat_id, &format!("❌ download: {}", esc(&e.to_string())), None)
                        .await;
                    return;
                }
            }
            Err(e) => {
                let _ = self
                    .bot
                    .send_message(chat_id, &format!("❌ getFile: {}", esc(&e.to_string())), None)
                    .await;
                return;
            }
        }

        let kind = if is_image { "image" } else { "document" };
        let prompt = format!(
            "[user uploaded {kind}]\nfilename: {filename}\npath: {}\nUse this file as context.",
            dest.display()
        );

        if self.live.is_some() {
            self.shared.lock().await.agent_buf.clear();
            let _ = self
                .bot
                .send_message(
                    chat_id,
                    &format!(
                        "📎 Saved <code>{}</code>\n⏳ Sending path to live agent…",
                        esc(&dest.display().to_string())
                    ),
                    None,
                )
                .await;
            let slot = self.live.as_ref().unwrap();
            let acp = slot.acp.clone();
            let bot = self.bot.clone();
            let notes_path = self.cfg.data_dir.clone();
            let session = self.active_session.clone();
            let dest_s = dest.display().to_string();
            tokio::spawn(async move {
                if let Err(e) = acp.session_prompt(&prompt).await {
                    if let Ok(q) = NoteQueue::open(&notes_path) {
                        let _ = q.enqueue(prompt, session, Some(dest_s), is_image);
                    }
                    let _ = bot
                        .send_message(
                            chat_id,
                            &format!(
                                "❌ Live send failed — parked note.\n{}",
                                esc(&e.to_string())
                            ),
                            None,
                        )
                        .await;
                }
            });
            return;
        }

        match self.notes.enqueue(
            prompt,
            self.active_session.clone(),
            Some(dest.display().to_string()),
            is_image,
        ) {
            Ok(n) => {
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        &format!(
                            "📎 Saved + parked note <code>{}</code>\n<code>{}</code>\n\
                             Go live + Drain to feed the agent.",
                            esc(&n.id.chars().take(8).collect::<String>()),
                            esc(&dest.display().to_string())
                        ),
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .bot
                    .send_message(chat_id, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
            }
        }
    }

    async fn send_sessions(&self, chat_id: i64) {
        let list = list_sessions(&self.cfg.sessions_root, 12);
        if list.is_empty() {
            let _ = self
                .bot
                .send_message(
                    chat_id,
                    "No sessions under ~/.grok/sessions yet. Tap ✨ New.",
                    Some(self.home_keyboard()),
                )
                .await;
            return;
        }
        let mut lines = vec!["<b>📚 Sessions</b> (newest first)".to_string(), String::new()];
        let mut rows = Vec::new();
        for (i, s) in list.iter().enumerate() {
            if i >= 8 {
                break;
            }
            let mark = if Some(&s.session_id) == self.active_session.as_ref() {
                "▸ "
            } else {
                ""
            };
            lines.push(format!(
                "{}<b>{}</b>\n   <code>{}</code> · {}",
                mark,
                esc(&s.title.chars().take(40).collect::<String>()),
                esc(&s.session_id.chars().take(12).collect::<String>()),
                esc(Path::new(&s.cwd).file_name().and_then(|n| n.to_str()).unwrap_or(&s.cwd))
            ));
            let cb = format!("use:{}", s.session_id);
            if cb.len() <= 64 {
                let label = format!("▶️ {}", s.title.chars().take(28).collect::<String>());
                rows.push(vec![(label, cb)]);
            }
        }
        let _ = self
            .bot
            .send_message(
                chat_id,
                &lines.join("\n"),
                Some(inline_keyboard(rows)),
            )
            .await;
    }

    async fn send_queue(&self, chat_id: i64) {
        let notes = self.notes.list().unwrap_or_default();
        if notes.is_empty() {
            let _ = self
                .bot
                .send_message(
                    chat_id,
                    "📋 <b>Notes</b>\n\nEmpty. Type while live is off to park work.",
                    Some(self.home_keyboard()),
                )
                .await;
            return;
        }
        let mut lines = vec![
            format!("📋 <b>Notes</b> · {}", notes.len()),
            String::new(),
        ];
        for (i, n) in notes.iter().take(10).enumerate() {
            let t = n.text.replace('\n', " ");
            let t = if t.len() > 60 {
                format!("{}…", &t[..59])
            } else {
                t
            };
            lines.push(format!(
                "{}. <code>{}</code> {}",
                i + 1,
                esc(&n.id.chars().take(8).collect::<String>()),
                esc(&t)
            ));
        }
        lines.push("\n▶️ Drain sends the next note into live Gork.".into());
        let _ = self
            .bot
            .send_message(
                chat_id,
                &lines.join("\n"),
                Some(inline_keyboard(vec![
                    vec![("▶️ Drain next→live".into(), "nav:drain".into())],
                    vec![("🧹 Clear all".into(), "nav:clearq".into())],
                    vec![("🏠 Home".into(), "nav:home".into())],
                ])),
            )
            .await;
    }

    async fn cmd_new(&mut self, chat_id: i64) {
        // Prefer create while live; else just set cwd intent + park message
        if self.live.is_none() {
            // start live+new
            if let Err(e) = self.start_live(None).await {
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        &format!("❌ Could not start live for /new: {}", esc(&e.to_string())),
                        None,
                    )
                    .await;
                return;
            }
        }
        let acp = self.live.as_ref().unwrap().acp.clone();
        match acp.session_new(&self.active_cwd).await {
            Ok(sid) => {
                self.active_session = Some(sid.clone());
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        &format!(
                            "✨ <b>New session</b>\n<code>{}</code>\nLive is on — type freely.",
                            esc(&sid)
                        ),
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .bot
                    .send_message(chat_id, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
            }
        }
    }

    async fn cmd_live(&mut self, chat_id: i64) {
        if self.live.is_some() {
            let _ = self
                .bot
                .send_message(
                    chat_id,
                    "🟢 Live already on. Type freely or /stop.",
                    Some(self.home_keyboard()),
                )
                .await;
            return;
        }
        let sid = self.active_session.clone();
        match self.start_live(sid.as_deref()).await {
            Ok(sid) => {
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        &format!(
                            "🟢 <b>Live on</b>\nsession <code>{}</code>\ncwd <code>{}</code>\n\
                             Type freely. Tool/plan holds show as buttons.",
                            esc(&sid),
                            esc(&self.active_cwd.display().to_string())
                        ),
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .bot
                    .send_message(
                        chat_id,
                        &format!(
                            "❌ Go live failed: {}\n\
                             Check GORK_BIN / gork login / cwd.",
                            esc(&e.to_string())
                        ),
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
        }
    }

    async fn cmd_stop(&mut self, chat_id: i64) {
        self.stop_live().await;
        let _ = self
            .bot
            .send_message(
                chat_id,
                "⏹ Live off. Free-text parks in Notes.",
                Some(self.home_keyboard()),
            )
            .await;
    }

    async fn cmd_drain(&mut self, chat_id: i64) {
        if self.live.is_none() {
            let _ = self
                .bot
                .send_message(
                    chat_id,
                    "❌ Live is off — /live first, then Drain.",
                    Some(self.home_keyboard()),
                )
                .await;
            return;
        }
        let note = match self.notes.pop_next() {
            Ok(Some(n)) => n,
            Ok(None) => {
                let _ = self
                    .bot
                    .send_message(chat_id, "📋 No notes to drain.", Some(self.home_keyboard()))
                    .await;
                return;
            }
            Err(e) => {
                let _ = self
                    .bot
                    .send_message(chat_id, &format!("❌ {}", esc(&e.to_string())), None)
                    .await;
                return;
            }
        };
        self.shared.lock().await.agent_buf.clear();
        let _ = self
            .bot
            .send_message(
                chat_id,
                &format!(
                    "▶️ Draining note <code>{}</code>…",
                    esc(&note.id.chars().take(8).collect::<String>())
                ),
                None,
            )
            .await;
        let slot = self.live.as_ref().unwrap();
        let acp = slot.acp.clone();
        let bot = self.bot.clone();
        let notes = self.notes.clone();
        let note_text = note.text.clone();
        let note_sid = note.session_id.clone();
        let note_media = note.media_path.clone();
        let note_img = note.is_image;
        let home_kb = self.home_keyboard();
        tokio::spawn(async move {
            match acp.session_prompt(&note_text).await {
                Ok(_) => {
                    let _ = bot
                        .send_message(
                            chat_id,
                            &format!(
                                "✅ Note drained. Remaining: <b>{}</b>",
                                notes.count()
                            ),
                            Some(home_kb),
                        )
                        .await;
                }
                Err(e) => {
                    let _ = notes.enqueue(note_text, note_sid, note_media, note_img);
                    let _ = bot
                        .send_message(
                            chat_id,
                            &format!(
                                "❌ Drain failed — note restored.\n{}",
                                esc(&e.to_string())
                            ),
                            None,
                        )
                        .await;
                }
            }
        });
    }

    async fn cmd_history(&self, chat_id: i64) {
        let Some(sid) = &self.active_session else {
            let _ = self
                .bot
                .send_message(
                    chat_id,
                    "No active session. /sessions → Use one.",
                    Some(self.home_keyboard()),
                )
                .await;
            return;
        };
        let list = list_sessions(&self.cfg.sessions_root, 50);
        let entry = list.iter().find(|s| &s.session_id == sid);
        let body = if let Some(e) = entry {
            peek_history(&e.disk_path, 8)
        } else {
            // try direct path under cwd group
            let group = urlencoding_encode(&self.active_cwd.display().to_string());
            let p = self.cfg.sessions_root.join(group).join(sid);
            peek_history(&p, 8)
        };
        let _ = self
            .bot
            .send_message(
                chat_id,
                &format!(
                    "<b>📜 History</b> <code>{}</code>\n\n{}",
                    esc(&sid.chars().take(12).collect::<String>()),
                    esc(&body)
                ),
                Some(self.home_keyboard()),
            )
            .await;
    }

    async fn start_live(&mut self, session_id: Option<&str>) -> Result<String> {
        self.stop_live().await;
        let agent_bin = resolve_agent_bin(self.cfg.agent_bin.clone());
        let (acp, rx) = AcpClient::spawn(&agent_bin, &self.active_cwd)
            .await
            .context("spawn agent")?;
        let shared = self.shared.clone();
        self.event_task = Some(tokio::spawn(fanout_acp_events(rx, shared)));
        let sid = if let Some(want) = session_id {
            match acp.session_load(want, &self.active_cwd).await {
                Ok(()) => want.to_string(),
                Err(e) => {
                    warn!(%e, "load failed; session/new");
                    acp.session_new(&self.active_cwd).await?
                }
            }
        } else if let Some(ref want) = self.active_session {
            match acp.session_load(want, &self.active_cwd).await {
                Ok(()) => want.clone(),
                Err(_) => acp.session_new(&self.active_cwd).await?,
            }
        } else {
            acp.session_new(&self.active_cwd).await?
        };
        self.active_session = Some(sid.clone());
        self.live = Some(LiveSlot {
            acp: Arc::new(acp),
            prompt_busy: Arc::new(tokio::sync::Mutex::new(false)),
        });
        {
            let mut s = self.shared.lock().await;
            s.pending_rpc.clear();
            s.agent_buf.clear();
        }
        Ok(sid)
    }

    async fn stop_live(&mut self) {
        if let Some(slot) = self.live.take() {
            // shutdown needs ownership — try Arc::try_unwrap
            match Arc::try_unwrap(slot.acp) {
                Ok(c) => c.shutdown().await,
                Err(arc) => {
                    // still borrowed by in-flight prompt; drop and let kill_on_drop
                    drop(arc);
                }
            }
        }
        if let Some(t) = self.event_task.take() {
            t.abort();
        }
        let mut s = self.shared.lock().await;
        s.pending_rpc.clear();
        s.agent_buf.clear();
    }

    async fn use_session(&mut self, chat_id: i64, session_id: &str) {
        let list = list_sessions(&self.cfg.sessions_root, 80);
        let entry: Option<SessionEntry> = list
            .into_iter()
            .find(|s| s.session_id == session_id || s.session_id.starts_with(session_id));
        let Some(entry) = entry else {
            let _ = self
                .bot
                .send_message(chat_id, "Session not found. /sessions", None)
                .await;
            return;
        };
        let was_live = self.live.is_some();
        if was_live {
            self.stop_live().await;
        }
        self.active_session = Some(entry.session_id.clone());
        self.active_cwd = PathBuf::from(&entry.cwd);
        let mut msg = format!(
            "✅ <b>Using</b> {}\n<code>{}</code>\n📁 <code>{}</code>",
            esc(&entry.title),
            esc(&entry.session_id),
            esc(&entry.cwd)
        );
        if was_live {
            match self.start_live(Some(&entry.session_id)).await {
                Ok(_) => msg.push_str("\n🟢 Live re-attached to this session."),
                Err(e) => msg.push_str(&format!(
                    "\n⚪ Live stopped (re-attach failed: {})",
                    esc(&e.to_string())
                )),
            }
        } else {
            msg.push_str("\nLive is off — /live when ready.");
        }
        let _ = self
            .bot
            .send_message(chat_id, &msg, Some(self.home_keyboard()))
            .await;
    }

    async fn handle_callback(&mut self, callback_id: &str, chat_id: Option<i64>, data: &str) {
        let _ = self.bot.answer_callback(callback_id, "OK").await;
        let Some(cid) = chat_id else { return };

        if let Some(sid) = data.strip_prefix("use:") {
            self.use_session(cid, sid).await;
            return;
        }
        match data {
            "nav:home" | "nav:status" => {
                let _ = self
                    .bot
                    .send_message(cid, &self.help_text(), Some(self.home_keyboard()))
                    .await;
            }
            "nav:sessions" => self.send_sessions(cid).await,
            "nav:new" => self.cmd_new(cid).await,
            "nav:live" => self.cmd_live(cid).await,
            "nav:stop" => self.cmd_stop(cid).await,
            "nav:queue" => self.send_queue(cid).await,
            "nav:drain" => self.cmd_drain(cid).await,
            "nav:history" => self.cmd_history(cid).await,
            "nav:clearq" => {
                let n = self.notes.clear().unwrap_or(0);
                let _ = self
                    .bot
                    .send_message(
                        cid,
                        &format!("🧹 Cleared {n} notes."),
                        Some(self.home_keyboard()),
                    )
                    .await;
            }
            _ => {
                // permission / opt / plan
                self.handle_acp_callback(cid, data).await;
            }
        }
    }

    async fn handle_acp_callback(&mut self, cid: i64, data: &str) {
        if let Some(oid) = data.strip_prefix("p:") {
            let pending = self.shared.lock().await.pending_rpc.remove("perm");
            if let Some((id, _)) = pending {
                if let Some(slot) = self.live.as_ref() {
                    let result = json!({
                        "outcome": { "outcome": "selected", "optionId": oid }
                    });
                    if let Err(e) = slot.acp.respond(id, result).await {
                        let _ = self
                            .bot
                            .send_message(cid, &format!("❌ {}", esc(&e.to_string())), None)
                            .await;
                    } else {
                        let _ = self
                            .bot
                            .send_message(
                                cid,
                                &format!("✅ Permission: <code>{}</code>", esc(oid)),
                                None,
                            )
                            .await;
                    }
                }
            } else {
                let _ = self
                    .bot
                    .send_message(cid, "No pending permission.", None)
                    .await;
            }
            return;
        }
        if let Some(qid) = data.strip_prefix("q:") {
            let pending = self.shared.lock().await.pending_rpc.remove("opt");
            if let Some((id, ServerRequest::AskUser { question, options, .. })) = pending {
                let label = options
                    .iter()
                    .find(|(i, _)| i == qid)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_else(|| qid.to_string());
                if let Some(slot) = self.live.as_ref() {
                    let result = json!({
                        "outcome": "accepted",
                        "answers": { question: [label] }
                    });
                    if let Err(e) = slot.acp.respond(id, result).await {
                        let _ = self
                            .bot
                            .send_message(cid, &format!("❌ {}", esc(&e.to_string())), None)
                            .await;
                    } else {
                        let _ = self.bot.send_message(cid, "✅ Answered", None).await;
                    }
                }
            }
            return;
        }
        if let Some(action) = data.strip_prefix("pl:") {
            let pending = self.shared.lock().await.pending_rpc.remove("plan");
            if let Some((id, _)) = pending {
                let outcome = match action {
                    "approve" => json!({ "outcome": "approved" }),
                    "quit" => json!({ "outcome": "abandoned" }),
                    _ => json!({ "outcome": "cancelled", "feedback": "revise from phone" }),
                };
                if let Some(slot) = self.live.as_ref() {
                    if let Err(e) = slot.acp.respond(id, outcome).await {
                        let _ = self
                            .bot
                            .send_message(cid, &format!("❌ {}", esc(&e.to_string())), None)
                            .await;
                    } else {
                        let _ = self
                            .bot
                            .send_message(
                                cid,
                                &format!("✅ Plan: <code>{}</code>", esc(action)),
                                None,
                            )
                            .await;
                    }
                }
            }
        }
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect()
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
