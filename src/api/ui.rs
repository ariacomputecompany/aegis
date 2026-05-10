use std::net::SocketAddr;

use axum::Json;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::display::DashboardBootstrap;

use super::server::ApiState;

pub async fn dashboard_bootstrap(
    State(state): State<ApiState>,
) -> Result<Json<DashboardBootstrap>, StatusCode> {
    state
        .dashboard_bootstrap()
        .map(Json)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

pub async fn vnc_websocket(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
) -> Result<impl IntoResponse, StatusCode> {
    let target = state.vnc_addr().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(ws.on_upgrade(move |socket| proxy_vnc(socket, target)))
}

async fn proxy_vnc(socket: WebSocket, target: SocketAddr) {
    let Ok(stream) = tokio::net::TcpStream::connect(target).await else {
        return;
    };

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (mut tcp_read, mut tcp_write) = stream.into_split();

    let websocket_to_tcp = tokio::spawn(async move {
        while let Some(message) = ws_rx.next().await {
            let Ok(message) = message else {
                break;
            };
            match message {
                Message::Binary(data) => {
                    if tcp_write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Message::Text(_) => {}
                Message::Ping(_) => {}
                Message::Pong(_) => {}
                Message::Close(_) => break,
            }
        }
    });

    let tcp_to_websocket = tokio::spawn(async move {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let Ok(read) = tcp_read.read(&mut buffer).await else {
                break;
            };
            if read == 0 {
                break;
            }
            if ws_tx
                .send(Message::Binary(buffer[..read].to_vec().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let _ = tokio::join!(websocket_to_tcp, tcp_to_websocket);
}
