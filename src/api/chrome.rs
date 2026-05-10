use std::convert::Infallible;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;

use crate::transport::bridge::BrowserChromeState;

use super::server::{ApiCommand, ApiState, BrowserUiState, TabCreateBody};

#[derive(Debug, Deserialize)]
pub struct ChromeNavigateBody {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ChromeNewTabBody {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChromeTabActionBody {
    pub tab_id: u64,
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

pub async fn chrome_tabs_sse(
    State(state): State<ApiState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tabs_rx();
    let stream = WatchStream::new(rx).map(|tabs_state| {
        let json = serde_json::to_string(&tabs_state).unwrap_or_default();
        Ok(Event::default().data(json))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn chrome_tabs_snapshot(State(state): State<ApiState>) -> Json<BrowserUiState> {
    Json(state.tabs_state_snapshot())
}

pub async fn chrome_back(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::GoBack(None))
}

pub async fn chrome_forward(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::GoForward(None))
}

pub async fn chrome_reload(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::Reload(None))
}

pub async fn chrome_stop(State(state): State<ApiState>) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::StopLoad(None))
}

pub async fn chrome_navigate(
    State(state): State<ApiState>,
    Json(body): Json<ChromeNavigateBody>,
) -> impl IntoResponse {
    send_chrome_command(&state, ApiCommand::ChromeNavigate(None, body.url))
}

pub async fn chrome_new_tab(
    State(state): State<ApiState>,
    Json(body): Json<ChromeNewTabBody>,
) -> impl IntoResponse {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    match state.send_command(ApiCommand::CreateTab(
        TabCreateBody {
            url: body.url,
            source_tab_id: None,
            activate: true,
            inherit_session: true,
        },
        reply_tx,
    )) {
        Ok(()) => match reply_rx.await {
            Ok(Ok(_)) => StatusCode::NO_CONTENT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

pub async fn chrome_activate_tab(
    State(state): State<ApiState>,
    Json(body): Json<ChromeTabActionBody>,
) -> impl IntoResponse {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    match state.send_command(ApiCommand::ActivateTab(body.tab_id, reply_tx)) {
        Ok(()) => match reply_rx.await {
            Ok(Ok(_)) => StatusCode::NO_CONTENT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

pub async fn chrome_close_tab(
    State(state): State<ApiState>,
    Json(body): Json<ChromeTabActionBody>,
) -> impl IntoResponse {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    match state.send_command(ApiCommand::CloseTab(body.tab_id, reply_tx)) {
        Ok(()) => match reply_rx.await {
            Ok(Ok(_)) => StatusCode::NO_CONTENT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn send_chrome_command(state: &ApiState, command: ApiCommand) -> StatusCode {
    match state.send_command(command) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
