use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use rusmpp::{
    pdus::SubmitSm,
    types::{COctetString, OctetString},
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc};
use std::str::FromStr;
use tracing::info;

use crate::queue::MessageQueue;

#[derive(Deserialize)]
pub struct SubmitRequest {
    pub source_addr: String,
    pub destination_addr: String,
    pub short_message: String,
}

#[derive(Serialize)]
pub struct SubmitResponse {
    pub message_id: String,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub status: String,
}

pub async fn run_http_server(
    bind_addr: SocketAddr,
    queue: Arc<dyn MessageQueue>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/submit", post(handle_submit))
        .route("/status/:message_id", get(handle_status))
        .with_state(queue);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!("HTTP server listening on {}", bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_submit(
    State(queue): State<Arc<dyn MessageQueue>>,
    Json(payload): Json<SubmitRequest>,
) -> Result<Json<SubmitResponse>, StatusCode> {
    let source_addr = COctetString::from_str(&payload.source_addr)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let destination_addr = COctetString::from_str(&payload.destination_addr)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let short_message = OctetString::from_str(&payload.short_message)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut submit = SubmitSm::default();
    submit.source_addr = source_addr;
    submit.destination_addr = destination_addr;
    submit.set_short_message(short_message);

    match queue.enqueue(&submit) {
        Ok(msg) => Ok(Json(SubmitResponse {
            message_id: msg.message_id_str().to_string(),
        })),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_status(
    State(queue): State<Arc<dyn MessageQueue>>,
    Path(message_id): Path<String>,
) -> Result<Json<StatusResponse>, StatusCode> {
    match queue.status(&message_id) {
        Some(status) => Ok(Json(StatusResponse {
            status: status.to_string(),
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

