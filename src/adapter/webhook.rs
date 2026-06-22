use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use tracing::{error, info};

use crate::core::Router as CoreRouter;
use crate::protocol::*;

/// Shared state for adapter HTTP handlers.
#[derive(Clone)]
pub struct AdapterState {
    pub router: Arc<CoreRouter>,
}

/// Build the adapter routes.
pub fn adapter_routes(state: AdapterState) -> Router {
    Router::new()
        .route("/api/v1/message/{platform}", post(webhook_handler))
        .with_state(state)
}

/// Unified webhook handler for all platforms.
/// Platform adapters normalize their format before sending to this endpoint,
/// or the platform-specific route handles the raw webhook format.
pub async fn webhook_handler(
    State(state): State<AdapterState>,
    axum::extract::Path(platform): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Parse the incoming message (expects normalized format)
    let conv_id = body.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let user_id = body.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");

    if conv_id.is_empty() || text.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing conversation_id or text".into());
    }

    // Find or create route for this conversation
    let session_id = match state.router.pick_cli().await {
        Some(id) => id,
        None => return (StatusCode::SERVICE_UNAVAILABLE, "no connected CLI".into()),
    };

    // Bind conversation to this CLI session
    state
        .router
        .bind_conversation(session_id, &platform, user_id, conv_id)
        .await;

    // Build and send AssignTask envelope
    let task = AssignTask {
        task_id: uuid::Uuid::new_v4(),
        platform: platform.clone(),
        platform_user_id: user_id.to_string(),
        conversation_id: conv_id.to_string(),
        message: TaskMessage {
            text: text.to_string(),
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
        Ok(()) => {
            info!("routed message from {platform}/{conv_id} to CLI {session_id}");
            (StatusCode::OK, "routed".into())
        }
        Err(e) => {
            error!("failed to route to CLI {session_id}: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, e)
        }
    }
}
