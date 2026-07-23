use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::core::router::Router;
use crate::protocol::*;

/// App WS session state — tracks the result channel for pushing back to the app.
pub struct AppSession {
    pub result_tx: mpsc::UnboundedSender<Envelope>,
}

/// Shared registry of connected app sessions, keyed by a derived platform name.
pub type AppSessions = Arc<RwLock<HashMap<String, AppSession>>>;

/// Handle a RingApp WebSocket connection (platform-style: submit tasks, receive results).
pub async fn handle_app_ws(mut ws: WebSocket, router: Arc<Router>, expected_token: Option<String>, app_sessions: AppSessions) {
    let register = match recv_register(&mut ws).await {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            send_error(&mut ws, "invalid_register", &e).await;
            return;
        }
        None => return,
    };

    if let Some(ref expected) = expected_token {
        let provided = register.auth_token.as_deref().unwrap_or("");
        if provided != expected {
            send_error(&mut ws, "auth_failed", "invalid auth token").await;
            return;
        }
    }

    // Each app instance gets a unique platform key for result routing.
    let app_id = format!("ringapp-{}", Uuid::new_v4());
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<Envelope>();

    // Register this app session so the dispatcher can route results back.
    app_sessions.write().await.insert(app_id.clone(), AppSession { result_tx: result_tx.clone() });

    // Register a dispatcher sender: when a TaskResult's conversation was bound
    // with platform = app_id, the dispatcher calls this closure, which pushes
    // the result envelope onto the channel → the main loop forwards it to the WS.
    let tx_for_dispatcher = result_tx.clone();
    let app_id_for_dispatcher = app_id.clone();
    let sender: crate::adapter::dispatcher::BoxedSender = Box::new(
        move |_token: String, _conv_id: String, result: TaskResult| {
            let tx = tx_for_dispatcher.clone();
            let aid = app_id_for_dispatcher.clone();
            Box::pin(async move {
                let env = Envelope {
                    id: Uuid::new_v4(),
                    msg_type: MsgType::TaskResult,
                    payload: serde_json::to_value(&result).unwrap_or_default(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    direction: Direction::Upstream,
                };
                if tx.send(env).is_err() {
                    debug!("[app:{aid}] result channel closed");
                }
            })
        },
    );
    router.dispatcher.register(&app_id, sender).await;

    // Send RegisterAck
    let ack = Envelope {
        id: Uuid::new_v4(),
        msg_type: MsgType::RegisterAck,
        payload: serde_json::to_value(RegisterAck {
            session_id: Uuid::new_v4(),
            heartbeat_interval_secs: 30,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        direction: Direction::Downstream,
    };
    if send_envelope(&mut ws, &ack).await.is_err() {
        cleanup(&app_sessions, &router, &app_id).await;
        return;
    }

    info!("[app:{app_id}] connected");

    // Bidirectional loop
    loop {
        tokio::select! {
            // Push results back to the app
            env = result_rx.recv() => {
                match env {
                    Some(env) => {
                        if send_envelope(&mut ws, &env).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            // Read messages from the app
            ws_msg = ws.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<Envelope>(&text) {
                            Ok(env) => handle_app_message(env, &router, &app_id, &result_tx).await,
                            Err(e) => error!("[app:{app_id}] parse error: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        error!("[app:{app_id}] WS error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("[app:{app_id}] disconnected");
    cleanup(&app_sessions, &router, &app_id).await;
}

/// Process a message received from the app.
async fn handle_app_message(env: Envelope, router: &Arc<Router>, app_id: &str, result_tx: &mpsc::UnboundedSender<Envelope>) {
    match env.msg_type {
        MsgType::AssignTask => {
            // Parse the task
            let task = match serde_json::from_value::<AssignTask>(env.payload.clone()) {
                Ok(t) => t,
                Err(e) => {
                    error!("[app:{app_id}] bad AssignTask payload: {e}");
                    let err = build_result_envelope(task_id_from_payload(&env.payload), TaskStatus::Failed, None, Some(format!("bad payload: {e}")));
                    let _ = result_tx.send(err);
                    return;
                }
            };

            // Pick an available CLI worker
            let session_id = match router.pick_cli().await {
                Some(id) => id,
                None => {
                    warn!("[app:{app_id}] no CLI worker available");
                    let err = build_result_envelope(task.task_id, TaskStatus::Failed, None, Some("no CLI worker connected".into()));
                    let _ = result_tx.send(err);
                    return;
                }
            };

            // Bind conversation so results route back to this app
            router
                .bind_conversation(session_id, app_id, &task.platform_user_id, &task.conversation_id)
                .await;

            // Forward the AssignTask to the CLI worker
            let forward = Envelope {
                id: Uuid::new_v4(),
                msg_type: MsgType::AssignTask,
                payload: serde_json::to_value(&task).unwrap_or_default(),
                timestamp: chrono::Utc::now().timestamp_millis(),
                direction: Direction::Downstream,
            };
            if let Err(e) = router.send_to_cli(&session_id, forward).await {
                error!("[app:{app_id}] failed to forward to CLI {session_id}: {e}");
                let err = build_result_envelope(task.task_id, TaskStatus::Failed, None, Some(format!("forward failed: {e}")));
                let _ = result_tx.send(err);
            } else {
                debug!("[app:{app_id}] routed task to CLI {session_id}");
            }
        }
        MsgType::Heartbeat => {
            let pong = Envelope {
                id: Uuid::new_v4(),
                msg_type: MsgType::HeartbeatAck,
                payload: env.payload,
                timestamp: chrono::Utc::now().timestamp_millis(),
                direction: Direction::Downstream,
            };
            let _ = result_tx.send(pong);
        }
        _ => {
            debug!("[app:{app_id}] unhandled msg type: {:?}", env.msg_type);
        }
    }
}

/// Extract task_id from a payload that might be partially parsed.
fn task_id_from_payload(payload: &serde_json::Value) -> Uuid {
    payload.get("task_id").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// Build a TaskResult envelope for error replies.
fn build_result_envelope(task_id: Uuid, status: TaskStatus, output: Option<TaskOutput>, error: Option<String>) -> Envelope {
    Envelope {
        id: Uuid::new_v4(),
        msg_type: MsgType::TaskResult,
        payload: serde_json::to_value(TaskResult { task_id, status, output, error }).unwrap_or_default(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        direction: Direction::Upstream,
    }
}

/// Wait for the first Register message.
async fn recv_register(ws: &mut WebSocket) -> Option<Result<Register, String>> {
    loop {
        match ws.recv().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<Envelope>(&text) {
                Ok(env) if env.msg_type == MsgType::Register => {
                    match serde_json::from_value::<Register>(env.payload) {
                        Ok(r) => return Some(Ok(r)),
                        Err(e) => return Some(Err(format!("bad register payload: {e}"))),
                    }
                }
                Ok(_) => return Some(Err("expected register as first message".into())),
                Err(e) => return Some(Err(format!("parse error: {e}"))),
            },
            Some(Ok(Message::Close(_))) | None => return None,
            _ => continue,
        }
    }
}

async fn send_envelope(ws: &mut WebSocket, env: &Envelope) -> Result<(), ()> {
    let text = serde_json::to_string(env).map_err(|_| ())?;
    ws.send(Message::Text(text.into())).await.map_err(|_| ())
}

async fn send_error(ws: &mut WebSocket, code: &str, message: &str) {
    let env = Envelope {
        id: Uuid::new_v4(),
        msg_type: MsgType::Error,
        payload: serde_json::to_value(Error {
            code: code.to_string(),
            message: message.to_string(),
            task_id: None,
        })
        .unwrap(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        direction: Direction::Downstream,
    };
    let _ = send_envelope(ws, &env).await;
}

async fn cleanup(app_sessions: &AppSessions, router: &Arc<Router>, app_id: &str) {
    app_sessions.write().await.remove(app_id);
    router.dispatcher.unregister(app_id).await;
}
