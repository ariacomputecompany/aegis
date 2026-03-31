use std::convert::Infallible;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

use crate::transport::bridge::BrowserChromeState;

use super::server::{ApiCommand, ApiState};

#[derive(Debug, Deserialize)]
pub struct ChromeNavigateBody {
    pub url: String,
}

pub async fn chrome_state_sse(
    State(state): State<ApiState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.chrome_rx();
    let stream = WatchStream::new(rx).map(|chrome_state| {
        let json = serde_json::to_string(&chrome_state).unwrap_or_default();
        Ok(Event::default().data(json))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn chrome_state_snapshot(State(state): State<ApiState>) -> Json<BrowserChromeState> {
    Json(state.chrome_state_snapshot())
}

pub async fn chrome_back(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::GoBack)
}

pub async fn chrome_forward(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::GoForward)
}

pub async fn chrome_reload(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::Reload)
}

pub async fn chrome_stop(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::StopLoad)
}

pub async fn chrome_navigate(
    State(state): State<ApiState>,
    Json(body): Json<ChromeNavigateBody>,
) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::ChromeNavigate(body.url))
}

fn send_chrome_command(state: &ApiState, command: ApiCommand) -> StatusCode {
    match state.send_command(command) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
