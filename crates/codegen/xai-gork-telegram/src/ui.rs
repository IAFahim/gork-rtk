//! Pure Telegram card builders for ACP holds.

use crate::acp_client::ServerRequest;
use crate::telegram::{esc, inline_keyboard};
use serde_json::Value;

pub fn permission_card(tool_title: &str) -> String {
    format!(
        "<b>🔐 Permission needed</b>\nTool: <b>{}</b>\n\nChoose approve or deny — work pauses until you answer.",
        esc(tool_title)
    )
}

pub fn permission_keyboard(options: &[(String, String)]) -> Value {
    let mut rows = Vec::new();
    for (oid, name) in options {
        let label = if name.len() > 40 {
            format!("{}…", &name[..39])
        } else {
            name.clone()
        };
        // g:perm:{optionId}  (jsonrpc id tracked separately in session map)
        rows.push(vec![(label, format!("p:{oid}"))]);
    }
    if rows.is_empty() {
        rows.push(vec![("✅ Allow once".into(), "p:allow-once".into())]);
        rows.push(vec![("❌ Deny".into(), "p:reject-once".into())]);
    }
    inline_keyboard(rows)
}

pub fn question_card(question: &str, options: &[(String, String)]) -> String {
    let mut lines = vec![
        "<b>❓ Gork is asking</b>".to_string(),
        esc(question),
        String::new(),
    ];
    for (i, (_, label)) in options.iter().enumerate() {
        lines.push(format!("{}. <b>{}</b>", i + 1, esc(label)));
    }
    lines.push("\nTap an option, or type a custom answer.".into());
    lines.join("\n")
}

pub fn question_keyboard(options: &[(String, String)]) -> Value {
    let mut rows = Vec::new();
    for (id, label) in options {
        let text = if label.len() > 40 {
            format!("{}…", &label[..39])
        } else {
            label.clone()
        };
        rows.push(vec![(text, format!("q:{id}"))]);
    }
    inline_keyboard(rows)
}

pub fn plan_card(excerpt: &str) -> String {
    let body = if excerpt.len() > 1200 {
        format!("{}…", &excerpt[..1199])
    } else {
        excerpt.to_string()
    };
    format!(
        "<b>📋 Plan ready</b>\n\n{}\n\nApprove to build, request changes, or quit plan mode.",
        esc(&body)
    )
}

pub fn plan_keyboard() -> Value {
    inline_keyboard(vec![
        vec![("✅ Approve".into(), "pl:approve".into())],
        vec![("✏️ Revise".into(), "pl:revise".into())],
        vec![("❌ Quit".into(), "pl:quit".into())],
    ])
}

pub fn card_for_request(req: &ServerRequest) -> (String, Option<Value>) {
    match req {
        ServerRequest::Permission {
            tool_title,
            options,
            ..
        } => (
            permission_card(tool_title),
            Some(permission_keyboard(options)),
        ),
        ServerRequest::AskUser {
            question, options, ..
        } => (question_card(question, options), Some(question_keyboard(options))),
        ServerRequest::ExitPlan { plan_excerpt, .. } => {
            (plan_card(plan_excerpt), Some(plan_keyboard()))
        }
        ServerRequest::Other { method, .. } => {
            (format!("Unhandled reverse-request: <code>{}</code>", esc(method)), None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_has_buttons() {
        let opts = vec![
            ("allow-once".into(), "Allow once".into()),
            ("reject-once".into(), "Deny".into()),
        ];
        let kb = permission_keyboard(&opts);
        let rows = kb["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0][0]["callback_data"]
            .as_str()
            .unwrap()
            .starts_with("p:"));
    }

    #[test]
    fn no_forbidden_lie() {
        let c = permission_card("x");
        assert!(!c.contains("doesn't run Grok for you yet"));
    }
}
