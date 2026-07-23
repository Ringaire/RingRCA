use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::core::Router as CoreRouter;
use crate::protocol::*;

const TG_API: &str = "https://api.telegram.org/bot";

#[derive(Clone)]
pub struct TgState {
    pub router: Arc<CoreRouter>,
    pub http: HttpClient,
    pub bot_token: String,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    chat: TgChat,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    from: Option<TgUser>,
    #[serde(default)]
    message_id: i64,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    id: i64,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

/// Outgoing message to Telegram.
#[derive(Debug, Serialize)]
struct TgSendMessage {
    chat_id: i64,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<i64>,
}

pub fn tg_routes(state: TgState) -> Router {
    // Start long polling in background
    let poll_state = state.clone();
    tokio::spawn(async move {
        tg_poll_loop(poll_state).await;
    });

    Router::new()
        .route("/api/v1/adapter/tg/update", post(webhook_handler))
        .route("/api/v1/adapter/tg/set-webhook", post(set_webhook_handler))
        .with_state(state)
}

/// Long polling loop: pull updates from Telegram every 1s.
async fn tg_poll_loop(state: TgState) {
    let api_url = format!("{}{}", TG_API, state.bot_token);
    let mut offset = 0i64;

    info!("[tg] polling started");
    loop {
        let url = format!("{}/getUpdates?offset={}&timeout=30", api_url, offset);
        match state.http.get(&url).send().await {
            Ok(resp) => {
                if let Ok(updates) = resp.json::<TgUpdatesList>().await {
                    for update in updates.result {
                        offset = update.update_id + 1;
                        if let Some(msg) = update.message.or(update.edited_message) {
                            handle_tg_message(&state, msg).await;
                        }
                    }
                }
            }
            Err(e) => error!("[tg] poll error: {e}"),
        }
        // Small delay between polling cycles
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, Deserialize)]
struct TgUpdatesList {
    result: Vec<TgUpdateItem>,
}

#[derive(Debug, Deserialize)]
struct TgUpdateItem {
    update_id: i64,
    #[serde(default)]
    message: Option<TgMessage>,
    #[serde(default)]
    edited_message: Option<TgMessage>,
}

/// Handle incoming Telegram update (webhook from Telegram).
async fn webhook_handler(
    State(state): State<TgState>,
    Json(body): Json<TgUpdateItem>,
) -> impl IntoResponse {
    let msg = match body.message.or(body.edited_message) {
        Some(m) => m,
        None => return (StatusCode::OK, "ok").into_response(),
    };
    handle_tg_message(&state, msg).await;
    (StatusCode::OK, "ok").into_response()
}

/// Core message handler: shared by webhook and long polling.
async fn handle_tg_message(state: &TgState, msg: TgMessage) {
    let text = match msg.text {
        Some(ref t) if !t.trim().is_empty() => t.clone(),
        _ => return,
    };

    // Built-in Telegram command handling
    if text == "/start" || text == "/help" {
        let help = "\u{1F43E} *RingCLI - Telegram*\n\n\
Send any message to chat with the AI agent.\n\n\
Commands:\n\
/help  - Show this\n\
/status - Connection status\n\n\
Messages are routed to a connected RingCLI.";
        let _ = send_tg_message(&state.http, &state.bot_token, msg.chat.id, help, None).await;
        return;
    }
    if text == "/status" || text == "/ping" {
        let n = state.router.list_clients().await.len();
        let s = if n > 0 { format!("\u{2705} {n} CLI connected") } else { "\u{274C} No CLI connected".into() };
        let _ = send_tg_message(&state.http, &state.bot_token, msg.chat.id, &s, None).await;
        return;
    }

    let user_id = msg.from.as_ref().map(|u| u.id.to_string()).unwrap_or_default();
    let conv_id = format!("tg:{}", msg.chat.id);

    let session_id = match state.router.pick_cli().await {
        Some(id) => id,
        None => {
            let _ = send_tg_message(&state.http, &state.bot_token, msg.chat.id, "No agent connected. Start RingCLI and connect first.", None).await;
            return;
        }
    };

    state.router.bind_conversation(session_id, "telegram", &user_id, &conv_id).await;

    let task = AssignTask {
        task_id: uuid::Uuid::new_v4(),
        platform: "telegram".to_string(),
        platform_user_id: user_id,
        conversation_id: conv_id,
        message: TaskMessage {
            text,
            attachments: None,
        },
        context: None,
    };

    let env = Envelope {
        id: uuid::Uuid::new_v4(),
        msg_type: MsgType::AssignTask,
        payload: serde_json::to_value(task).unwrap(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        direction: Direction::Downstream,
    };

    match state.router.send_to_cli(&session_id, env).await {
        Ok(()) => info!("[tg] routed message to CLI {session_id}"),
        Err(e) => {
            error!("[tg] route failed: {e}");
            let _ = send_tg_message(&state.http, &state.bot_token, msg.chat.id, &format!("Error: {e}"), None).await;
        }
    }
}

/// Set Telegram webhook.
async fn set_webhook_handler(
    State(state): State<TgState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing url").into_response();
    }
    let api_url = format!("{}{}/setWebhook", TG_API, state.bot_token);
    match state.http.post(&api_url).json(&serde_json::json!({ "url": url })).send().await {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            info!("[tg] setWebhook -> {status}: {text}");
            (status, text).into_response()
        }
        Err(e) => {
            error!("[tg] setWebhook failed: {e}");
            (StatusCode::BAD_GATEWAY, e.to_string()).into_response()
        }
    }
}

/// Send a message to Telegram chat.
pub async fn send_tg_message(
    http: &HttpClient,
    token: &str,
    chat_id: i64,
    text: &str,
    reply_to: Option<i64>,
) -> Result<(), String> {
    let url = format!("{}{}/sendMessage", TG_API, token);
    let payload = TgSendMessage {
        chat_id,
        text: text.to_string(),
        reply_to_message_id: reply_to,
    };
    let resp = http.post(&url).json(&payload).send().await.map_err(|e| format!("tg send failed: {e}"))?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("tg api error: {body}"));
    }
    Ok(())
}

/// Send a task result back to a Telegram chat.
pub async fn send_task_result(
    http: &HttpClient,
    token: &str,
    conversation_id: &str,
    result: &TaskResult,
) -> Result<(), String> {
    let chat_id: i64 = conversation_id.strip_prefix("tg:").unwrap_or(conversation_id).parse().map_err(|_| "invalid chat_id".to_string())?;
    if let Some(ref output) = result.output {
        send_tg_message(http, token, chat_id, &output.text, None).await?;
    }
    if let Some(ref err) = result.error {
        send_tg_message(http, token, chat_id, &format!("Error: {err}"), None).await.ok();
    }
    Ok(())
}
