use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::core::router::{CliSession, Router};
use crate::protocol::*;

/// Handle a new RingCLI WebSocket connection.
pub async fn handle_ws(mut ws: WebSocket, router: Arc<Router>, expected_token: Option<String>) {
    let register = match recv_register(&mut ws).await {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            send_error(&mut ws, "invalid_register", &e).await;
            return;
        }
        None => return,
    };

    // Auth check
    if let Some(ref expected) = expected_token {
        let provided = register.auth_token.as_deref().unwrap_or("");
        if provided != expected {
            send_error(&mut ws, "auth_failed", "invalid auth token").await;
            return;
        }
    }

    let session_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel::<Envelope>();

    let session = CliSession {
        client_id: register.client_id,
        version: register.version,
        capabilities: register.capabilities,
        labels: register.labels.unwrap_or_default(),
        tx,
        connected_at: chrono::Utc::now().timestamp_millis(),
    };
    router.register(session_id, session).await;

    // Send RegisterAck
    let ack = build_envelope(
        MsgType::RegisterAck,
        Direction::Downstream,
        serde_json::to_value(RegisterAck {
            session_id,
            heartbeat_interval_secs: 30,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap(),
    );
    if send_envelope(&mut ws, &ack).await.is_err() {
        router.unregister(&session_id).await;
        return;
    }

    info!("CLI registered: {} (session={session_id})", router.list_clients().await.len());

    // Bidirectional relay loop
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(env) => {
                        if send_envelope(&mut ws, &env).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            ws_msg = ws.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<Envelope>(&text) {
                            Ok(env) => handle_cli_message(env, &router, &session_id).await,
                            Err(e) => error!("parse error from {session_id}: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        error!("WS error from {session_id}: {e}");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("CLI disconnected: {session_id}");
    router.unregister(&session_id).await;
}

/// Handle a message received from RingCLI.
async fn handle_cli_message(env: Envelope, router: &Arc<Router>, session_id: &Uuid) {
    match env.msg_type {
        MsgType::Heartbeat => {
            let pong = build_envelope(
                MsgType::HeartbeatAck,
                Direction::Downstream,
                env.payload,
            );
            let _ = router.send_to_cli(session_id, pong).await;
        }
        MsgType::TaskResult => {
            if let Ok(result) = serde_json::from_value::<TaskResult>(env.payload.clone()) {
                debug!("Task result from {session_id}: {:?}", result.status);
                router.dispatch_result(session_id, &result).await;
            }
        }
        MsgType::Register => {
            warn!("duplicate register from {session_id}");
        }
        _ => {
            debug!("unhandled msg type from cli: {:?}", env.msg_type);
        }
    }
}

/// Wait for the first Register message from a new WS connection.
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
    let env = build_envelope(
        MsgType::Error,
        Direction::Downstream,
        serde_json::to_value(Error {
            code: code.to_string(),
            message: message.to_string(),
            task_id: None,
        })
        .unwrap(),
    );
    let _ = send_envelope(ws, &env).await;
}

fn build_envelope(msg_type: MsgType, direction: Direction, payload: serde_json::Value) -> Envelope {
    Envelope {
        id: Uuid::new_v4(),
        msg_type,
        payload,
        timestamp: chrono::Utc::now().timestamp_millis(),
        direction,
    }
}
