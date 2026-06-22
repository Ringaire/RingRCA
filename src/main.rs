mod adapter;
mod cli;
mod core;
mod protocol;

use std::sync::Arc;

use axum::{Router, extract::ws::WebSocketUpgrade, routing::get};
use clap::Parser;
use reqwest::Client as HttpClient;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::adapter::webhook::{AdapterState, adapter_routes};

#[derive(Parser, Debug)]
#[command(name = "nekorca", about = "Neko Remote Control Adapter")]
struct Args {
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    #[arg(long, default_value = "8080")]
    port: u16,
    #[arg(long, env = "NEKO_RCA_TOKEN")]
    auth_token: Option<String>,
    #[arg(long, env = "NEKO_TG_TOKEN")]
    tg_token: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nekorca=info".into()),
        )
        .init();

    let args = Args::parse();
    let expected_token: Option<String> = args.auth_token.clone();
    let router = Arc::new(core::Router::new());
    let router_for_adapter = router.clone();
    let http = HttpClient::new();

    // ── CLI WS 路由 ──
    let cli_ws_router = Router::new()
        .route("/cli/ws", get(move |ws: WebSocketUpgrade| async move {
            let r = router.clone();
            let tok = expected_token.clone();
            ws.on_upgrade(move |socket| cli::handle_ws(socket, r, tok))
        }));

    // ── Telegram 适配器 ──
    let tg_router = if let Some(ref token) = args.tg_token {
        let tg_state = crate::adapter::tg::TgState {
            router: router_for_adapter.clone(),
            http: http.clone(),
            bot_token: token.clone(),
        };

        // Register result dispatcher
        let t = token.clone();
        let h = http.clone();
        let sender: crate::adapter::dispatcher::BoxedSender = Box::new(move |_token, conv_id, result| {
            let h = h.clone();
            let t = t.clone();
            Box::pin(async move {
                let _ = crate::adapter::tg::send_task_result(&h, &t, &conv_id, &result).await;
            })
        });
        router_for_adapter.dispatcher.register("telegram", sender).await;
        info!("[tg] adapter registered");
        crate::adapter::tg::tg_routes(tg_state)
    } else {
        info!("[tg] no NEKO_TG_TOKEN, skipping");
        Router::new()
    };

    // ── Webhook 路由 ──
    let adapter_state = AdapterState { router: router_for_adapter };
    let adapter_router = adapter_routes(adapter_state);

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(cli_ws_router)
        .merge(tg_router)
        .merge(adapter_router)
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", args.host, args.port);
    info!("NekoRCA listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
