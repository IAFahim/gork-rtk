//! Minimal ACP client over `gork agent stdio` (JSON-RPC lines).

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Server reverse-request the phone must answer.
#[derive(Debug, Clone)]
pub enum ServerRequest {
    Permission {
        id: Value,
        tool_title: String,
        options: Vec<(String, String)>, // (optionId, name)
    },
    AskUser {
        id: Value,
        question: String,
        options: Vec<(String, String)>, // (id, label)
    },
    ExitPlan {
        id: Value,
        plan_excerpt: String,
    },
    Other {
        id: Value,
        method: String,
    },
}

/// Outbound events for Telegram UX.
#[derive(Debug, Clone)]
pub enum AcpEvent {
    AgentText(String),
    ToolTitle(String),
    ServerRequest(ServerRequest),
    TurnDone { stop_reason: String },
    Error(String),
}

pub struct AcpClient {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
    events: mpsc::UnboundedSender<AcpEvent>,
    session_id: Arc<Mutex<Option<String>>>,
}

impl AcpClient {
    pub async fn spawn(agent_bin: &Path, cwd: &Path) -> Result<(Self, mpsc::UnboundedReceiver<AcpEvent>)> {
        let mut child = Command::new(agent_bin)
            .args(["agent", "stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn {} agent stdio", agent_bin.display()))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let session_id = Arc::new(Mutex::new(None));

        let pending_r = pending.clone();
        let events_r = events_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = events_r.send(AcpEvent::Error(format!("bad json: {e}")));
                        continue;
                    }
                };
                if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                    if msg.get("method").is_some() {
                        // reverse-request
                        if let Some(req) = parse_server_request(&msg) {
                            let _ = events_r.send(AcpEvent::ServerRequest(req));
                        }
                        continue;
                    }
                    // response
                    let mut map = pending_r.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        if let Some(err) = msg.get("error") {
                            let _ = tx.send(Err(anyhow!("rpc error: {err}")));
                        } else {
                            let _ = tx.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
                        }
                    }
                    continue;
                }
                // notification
                if msg.get("method").and_then(|m| m.as_str()) == Some("session/update") {
                    handle_session_update(&msg, &events_r);
                }
            }
            let _ = events_r.send(AcpEvent::Error("agent stdout closed".into()));
        });

        let client = Self {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicU64::new(1),
            pending,
            events: events_tx,
            session_id,
        };

        // initialize
        let _ = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": false, "writeTextFile": false },
                        "terminal": false
                    },
                    "clientInfo": { "name": "gork-telegram", "version": "0.1.0" }
                }),
            )
            .await
            .context("initialize")?;

        Ok((client, events_rx))
    }

    async fn write_line(&self, v: &Value) -> Result<()> {
        let mut line = serde_json::to_string(v)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id, tx);
        }
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => bail!("response channel closed for {method}"),
            Err(_) => bail!("timeout waiting for {method}"),
        }
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .await
    }

    pub async fn session_new(&self, cwd: &Path) -> Result<String> {
        let result = self
            .request(
                "session/new",
                json!({
                    "cwd": cwd.display().to_string(),
                    "mcpServers": []
                }),
            )
            .await?;
        let sid = result
            .get("sessionId")
            .or_else(|| result.get("session_id"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow!("session/new missing sessionId"))?
            .to_string();
        *self.session_id.lock().await = Some(sid.clone());
        Ok(sid)
    }

    pub async fn session_load(&self, session_id: &str, cwd: &Path) -> Result<()> {
        let result = self
            .request(
                "session/load",
                json!({
                    "sessionId": session_id,
                    "cwd": cwd.display().to_string(),
                    "mcpServers": []
                }),
            )
            .await;
        if result.is_err() {
            // try resume
            let _ = self
                .request(
                    "session/resume",
                    json!({
                        "sessionId": session_id,
                        "cwd": cwd.display().to_string(),
                        "mcpServers": []
                    }),
                )
                .await
                .context("session/load and session/resume failed")?;
        }
        *self.session_id.lock().await = Some(session_id.to_string());
        Ok(())
    }

    pub async fn session_prompt(&self, text: &str) -> Result<String> {
        let sid = self
            .session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow!("no session"))?;
        let result = self
            .request(
                "session/prompt",
                json!({
                    "sessionId": sid,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await?;
        let stop = result
            .get("stopReason")
            .or_else(|| result.get("stop_reason"))
            .and_then(|s| s.as_str())
            .unwrap_or("ok")
            .to_string();
        let _ = self.events.send(AcpEvent::TurnDone {
            stop_reason: stop.clone(),
        });
        Ok(stop)
    }

    pub fn session_id(&self) -> Option<String> {
        // sync peek via try_lock
        self.session_id.try_lock().ok().and_then(|g| g.clone())
    }

    pub async fn shutdown(mut self) {
        let _ = self.child.kill().await;
    }
}

fn handle_session_update(msg: &Value, events: &mpsc::UnboundedSender<AcpEvent>) {
    let update = msg
        .pointer("/params/update")
        .or_else(|| msg.get("params"))
        .cloned()
        .unwrap_or(Value::Null);
    let kind = update
        .get("sessionUpdate")
        .or_else(|| update.get("session_update"))
        .and_then(|k| k.as_str())
        .unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            if let Some(t) = update
                .pointer("/content/text")
                .and_then(|t| t.as_str())
                .or_else(|| update.get("text").and_then(|t| t.as_str()))
            {
                let _ = events.send(AcpEvent::AgentText(t.to_string()));
            }
        }
        "tool_call" => {
            if let Some(t) = update
                .get("title")
                .or_else(|| update.get("toolCallId"))
                .and_then(|t| t.as_str())
            {
                let _ = events.send(AcpEvent::ToolTitle(t.to_string()));
            }
        }
        _ => {}
    }
}

fn parse_server_request(msg: &Value) -> Option<ServerRequest> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method")?.as_str()?.to_string();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    if method == "session/request_permission" {
        let tool = params.get("toolCall").or_else(|| params.get("tool_call"));
        let title = tool
            .and_then(|t| t.get("title").or_else(|| t.get("toolCallId")))
            .and_then(|t| t.as_str())
            .unwrap_or("tool")
            .to_string();
        let mut options = Vec::new();
        if let Some(arr) = params.get("options").and_then(|o| o.as_array()) {
            for o in arr {
                let oid = o
                    .get("optionId")
                    .or_else(|| o.get("option_id"))
                    .or_else(|| o.get("id"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("opt")
                    .to_string();
                let name = o
                    .get("name")
                    .or_else(|| o.get("label"))
                    .and_then(|x| x.as_str())
                    .unwrap_or(&oid)
                    .to_string();
                options.push((oid, name));
            }
        }
        return Some(ServerRequest::Permission {
            id,
            tool_title: title,
            options,
        });
    }
    if method.contains("ask_user_question") {
        let q = params
            .pointer("/questions/0/question")
            .or_else(|| params.get("question"))
            .and_then(|q| q.as_str())
            .unwrap_or("Choose:")
            .to_string();
        let mut options = Vec::new();
        let opts = params
            .pointer("/questions/0/options")
            .or_else(|| params.get("options"))
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();
        for (i, o) in opts.iter().enumerate() {
            let label = o
                .get("label")
                .or_else(|| o.get("name"))
                .and_then(|x| x.as_str())
                .unwrap_or("opt")
                .to_string();
            options.push((format!("{i}"), label));
        }
        return Some(ServerRequest::AskUser {
            id,
            question: q,
            options,
        });
    }
    if method.contains("exit_plan_mode") {
        let plan = params
            .get("planContent")
            .or_else(|| params.get("plan_content"))
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .chars()
            .take(1500)
            .collect();
        return Some(ServerRequest::ExitPlan {
            id,
            plan_excerpt: plan,
        });
    }
    Some(ServerRequest::Other { id, method })
}

/// Resolve agent binary: env, same dir as current exe, or PATH.
pub fn resolve_agent_bin(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(p) = std::env::var("GORK_BIN").or_else(|_| std::env::var("GROK_BINARY")) {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["gork", "grok"] {
                let cand = dir.join(name);
                if cand.is_file() {
                    return cand;
                }
            }
        }
        // gork-telegram next to agent: try parent downloads
        return exe;
    }
    PathBuf::from("gork")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_permission_fixture() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "session/request_permission",
            "params": {
                "toolCall": { "title": "run_terminal_command" },
                "options": [
                    { "optionId": "allow-once", "name": "Allow once" },
                    { "optionId": "reject-once", "name": "Deny" }
                ]
            }
        });
        match parse_server_request(&msg).unwrap() {
            ServerRequest::Permission {
                tool_title,
                options,
                ..
            } => {
                assert_eq!(tool_title, "run_terminal_command");
                assert_eq!(options.len(), 2);
                assert_eq!(options[0].0, "allow-once");
            }
            _ => panic!("expected permission"),
        }
    }
}
