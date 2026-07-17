//! Minimal Telegram Bot API long-poll client.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct TelegramBot {
    token: String,
    http: reqwest::Client,
}

// reqwest::Client is already Clone

#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub chat: Chat,
    pub from: Option<User>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub id: i64,
}

impl TelegramBot {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    fn url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    pub async fn get_me(&self) -> Result<Value> {
        let v: Value = self
            .http
            .get(self.url("getMe"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v)
    }

    pub async fn get_updates(&self, offset: i64, timeout: u64) -> Result<Vec<Update>> {
        let v: Value = self
            .http
            .get(self.url("getUpdates"))
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", timeout.to_string()),
                ("allowed_updates", r#"["message","callback_query"]"#.into()),
            ])
            .send()
            .await
            .context("getUpdates")?
            .error_for_status()?
            .json()
            .await?;
        if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            anyhow::bail!("getUpdates not ok: {v}");
        }
        let result = v.get("result").cloned().unwrap_or(json!([]));
        Ok(serde_json::from_value(result)?)
    }

    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        reply_markup: Option<Value>,
    ) -> Result<()> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
            "disable_web_page_preview": true,
        });
        if let Some(m) = reply_markup {
            body["reply_markup"] = m;
        }
        let v: Value = self
            .http
            .post(self.url("sendMessage"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            anyhow::bail!("sendMessage failed: {v}");
        }
        Ok(())
    }

    pub async fn answer_callback(&self, callback_id: &str, text: &str) -> Result<()> {
        let _: Value = self
            .http
            .post(self.url("answerCallbackQuery"))
            .json(&json!({ "callback_query_id": callback_id, "text": text }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(())
    }
}

/// Escape for Telegram HTML.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn inline_keyboard(rows: Vec<Vec<(String, String)>>) -> Value {
    // (text, callback_data)
    let inline_keyboard: Vec<Vec<Value>> = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(text, data)| {
                    let data = if data.len() > 64 {
                        data.chars().take(64).collect()
                    } else {
                        data
                    };
                    json!({ "text": text, "callback_data": data })
                })
                .collect()
        })
        .collect();
    json!({ "inline_keyboard": inline_keyboard })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_html() {
        assert_eq!(esc("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[test]
    fn keyboard_caps_callback() {
        let long = "x".repeat(80);
        let kb = inline_keyboard(vec![vec![("A".into(), long)]]);
        let data = kb["inline_keyboard"][0][0]["callback_data"].as_str().unwrap();
        assert!(data.len() <= 64);
    }
}
