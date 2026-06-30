use std::collections::HashMap;
use std::convert::Infallible;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_stream::stream;
use axum::extract::{Path, Query, State, rejection::JsonRejection};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};

use crate::browser::BrowserConfig;
use crate::commands::command::{Command, CommandMatcher, CommandTarget};
use crate::commands::matcher::resolve_command_target as resolve_snapshot_target;
use crate::config_store::{
    AegisConfigStore, AegisSecretStore, CredentialInput, CredentialsSettings,
};
use crate::dom::node::{DomNode, DomSnapshot};
use crate::events::stream::{EventReadWindow, EventType, RuntimeEvent, SequencedEvent};
use crate::host::{LoadedAegisClient, RuntimeCancelHandle};
use crate::runtime::executor::{
    ExecutionReport, PageBootstrapDiagnostics, PageResearchData, PageResearchHeading,
    PageResearchLink, RuntimeStatus,
};
use crate::session::cookies::SessionState;
use crate::session::profile::{SessionProfileInfo, SessionProfileStore};
use crate::transport::bridge::AegisError;
use crate::transport::protocol::{DownloadState, HostRuntimeState, PROTOCOL_VERSION};

const HEADLESS_IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(10);
const HEADFUL_IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(2);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_EVENT_STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MIN_EVENT_STREAM_POLL_INTERVAL_MS: u64 = 25;
const MAX_EVENT_STREAM_POLL_INTERVAL_MS: u64 = 1_000;
static TELEMETRY_START: OnceLock<Instant> = OnceLock::new();
static PROCESS_STARTED_AT_MS: OnceLock<u64> = OnceLock::new();
static API_ERROR_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct ApiState {
    tx: mpsc::Sender<ApiCommand>,
    cancel: Arc<Mutex<Option<RuntimeCancelHandle>>>,
    host_library: PathBuf,
    browser: BrowserConfig,
    startup: Arc<Mutex<ServeStartupMetrics>>,
    profile: SessionProfileInfo,
    diagnostics: Arc<Mutex<ServeDiagnostics>>,
}

#[derive(Clone)]
pub struct ServeRootState {
    host_library: PathBuf,
    browser: BrowserConfig,
    default_context_id: String,
    contexts: Arc<Mutex<HashMap<String, ApiState>>>,
    manager_tx: mpsc::Sender<ManagerCommand>,
}

struct MainThreadContext {
    api: ApiState,
    rx: mpsc::Receiver<ApiCommand>,
    client: LoadedAegisClient,
    profile_store: SessionProfileStore,
    credential_settings: CredentialsSettings,
    credential_store: AegisSecretStore,
    credential_capture: AutoCredentialCapture,
    pending_startup_session: Option<SessionState>,
    idle_pump_interval: Duration,
    last_pump_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServeStartupMetrics {
    client_connect_ms: u64,
    api_bind_ms: u64,
    total_ready_ms: u64,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    version: &'static str,
    protocol_version: u16,
    control_plane_up: bool,
    runtime_state: RuntimeOperationalState,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    browser_process_up: bool,
    page_attached: bool,
    renderer_attached: bool,
    dom_snapshot_available: bool,
    event_decoder_ok: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
}

#[derive(Debug, Deserialize)]
pub struct NavigateBody {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteBody {
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextSummary {
    pub id: String,
    pub default: bool,
    pub host_library: PathBuf,
    pub browser: BrowserConfig,
    pub profile: SessionProfileInfo,
    pub runtime_state: String,
    pub command_ready: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateContextBody {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub seed_from_context: Option<String>,
    #[serde(default)]
    pub mode: Option<crate::browser::BrowserMode>,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default)]
    pub download_dir: Option<PathBuf>,
    #[serde(default)]
    pub upload_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct TraceBody {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct SearchBody {
    pub query: String,
    #[serde(default)]
    pub engine: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PageFindBody {
    pub text: String,
    #[serde(default)]
    pub exact: bool,
}

#[derive(Debug, Deserialize)]
pub struct PageOpenLinkBody {
    pub text: String,
    #[serde(default)]
    pub exact: bool,
    #[serde(default)]
    pub href_contains: Option<String>,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub engine: String,
    pub query: String,
    pub url: String,
    pub events: Vec<SequencedEvent>,
}

#[derive(Debug, Serialize)]
pub struct PageTextResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub text: String,
    pub likely_not_found: bool,
    pub not_found_signals: Vec<String>,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageMarkdownResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub markdown: String,
    pub likely_not_found: bool,
    pub not_found_signals: Vec<String>,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageLinksResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub links: Vec<PageResearchLink>,
    pub likely_not_found: bool,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageHeadingsResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub headings: Vec<PageResearchHeading>,
    pub likely_not_found: bool,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageFindMatch {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageFindResponse {
    pub query: String,
    pub exact: bool,
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub match_count: usize,
    pub matches: Vec<PageFindMatch>,
    pub likely_not_found: bool,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageOpenLinkResponse {
    pub text: String,
    pub exact: bool,
    pub href_contains: Option<String>,
    pub candidate_count: usize,
    pub chosen: PageResearchLink,
    pub events: Vec<SequencedEvent>,
}

#[derive(Debug, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub since: u64,
    #[serde(default, rename = "type")]
    pub event_types: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct EventStreamQuery {
    #[serde(default)]
    pub since: u64,
    #[serde(default, rename = "type")]
    pub event_types: Vec<String>,
    #[serde(default)]
    pub poll_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct EventFilter {
    matchers: Vec<EventMatcher>,
}

#[derive(Debug, Clone)]
enum EventMatcher {
    Category(EventType),
    Exact(String),
}

impl EventFilter {
    #[allow(clippy::result_large_err)]
    fn from_query(raw_filters: &[String]) -> Result<Option<Self>, ApiError> {
        let mut matchers = Vec::new();
        for raw in raw_filters {
            for token in raw.split(',') {
                let normalized = normalize_text(token);
                if normalized.is_empty() {
                    continue;
                }
                let matcher = match normalized.as_str() {
                    "dom" | "dom_mutation" => EventMatcher::Category(EventType::DomMutation),
                    "navigation" | "navigate" => EventMatcher::Category(EventType::Navigation),
                    "network" => EventMatcher::Category(EventType::Network),
                    "download" | "downloads" => EventMatcher::Category(EventType::Download),
                    "websocket" | "ws" => EventMatcher::Category(EventType::WebSocket),
                    "log" | "logs" => EventMatcher::Category(EventType::Log),
                    "websocket_open"
                    | "websocket_handshake"
                    | "websocket_frame"
                    | "websocket_close"
                    | "unknown" => EventMatcher::Exact(normalized),
                    other => {
                        return Err(ApiError::bad_request(
                            "invalid_event_filter",
                            format!(
                                "unsupported event filter `{other}`; use dom_mutation, navigation, network, download, websocket, log, websocket_open, websocket_handshake, websocket_frame, websocket_close, or unknown"
                            ),
                            Some("filter the request with documented event types from /manifest or omit the type query entirely".into()),
                        ));
                    }
                };
                matchers.push(matcher);
            }
        }
        if matchers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Self { matchers }))
        }
    }

    fn matches(&self, event: &RuntimeEvent) -> bool {
        self.matchers.iter().any(|matcher| match matcher {
            EventMatcher::Category(category) => event.event_type() == *category,
            EventMatcher::Exact(name) => {
                if name == "unknown" {
                    matches!(event, RuntimeEvent::Unknown { .. })
                } else {
                    event.event_name() == name
                }
            }
        })
    }
}

fn filter_event_window(window: EventReadWindow, filter: Option<&EventFilter>) -> EventReadWindow {
    let Some(filter) = filter else {
        return window;
    };
    EventReadWindow {
        requested_since: window.requested_since,
        oldest_available_sequence: window.oldest_available_sequence,
        latest_sequence: window.latest_sequence,
        gap_detected: window.gap_detected,
        events: window
            .events
            .into_iter()
            .filter(|entry| filter.matches(&entry.event))
            .collect(),
    }
}

enum ApiCommand {
    InjectSession(SessionState, oneshot::Sender<Result<(), AegisError>>),
    SnapshotSession(oneshot::Sender<Result<SessionState, AegisError>>),
    SaveSessionProfile(oneshot::Sender<Result<SessionProfileInfo, AegisError>>),
    LoadSessionProfile(oneshot::Sender<Result<SessionProfileInfo, AegisError>>),
    ActivateBrowser(i32, oneshot::Sender<Result<HostRuntimeState, AegisError>>),
    Navigate(
        String,
        oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>,
    ),
    Execute(
        Vec<Command>,
        oneshot::Sender<Result<ExecutionReport, AegisError>>,
    ),
    PageResearch(oneshot::Sender<Result<PageResearchData, AegisError>>),
    SnapshotDom(oneshot::Sender<Result<DomSnapshot, AegisError>>),
    Events(u64, oneshot::Sender<Result<EventReadWindow, AegisError>>),
    EnableTrace(PathBuf, oneshot::Sender<Result<(), AegisError>>),
}

enum ManagerCommand {
    CreateContext {
        context_id: String,
        profile: String,
        browser: BrowserConfig,
        seed_from_context: Option<String>,
        reply: oneshot::Sender<Result<ContextSummary, AegisError>>,
    },
    DeleteContext {
        context_id: String,
        reply: oneshot::Sender<Result<(), AegisError>>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RuntimeOperationalState {
    Starting,
    Ready,
    Busy,
    Degraded,
    Cancelling,
    Wedged,
}

#[derive(Debug, Clone, Serialize)]
struct OperationSnapshot {
    id: u64,
    name: String,
    stage: String,
    started_at_ms: u64,
    elapsed_ms: u64,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
struct FailureSnapshot {
    operation: String,
    stage: String,
    message: String,
    elapsed_ms: u64,
    timed_out: bool,
    restart_recommended: bool,
    first_seen_at_ms: u64,
    last_seen_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ActiveOperation {
    id: u64,
    name: String,
    stage: String,
    started_at_ms: u64,
    started_at: Instant,
    timed_out: bool,
}

#[derive(Debug, Clone)]
struct ServeDiagnostics {
    runtime: RuntimeStatus,
    active_operation: Option<ActiveOperation>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    next_operation_id: u64,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeDiagnosticsResponse {
    version: &'static str,
    protocol_version: u16,
    state: RuntimeOperationalState,
    control_plane_up: bool,
    command_ready: bool,
    inspectable_dom_ready: bool,
    document_loaded: bool,
    module_scripts_present: bool,
    module_bootstrap_observed: bool,
    app_dom_mutated_after_load: bool,
    synthetic_shell_active: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    browser_process_up: bool,
    page_attached: bool,
    renderer_attached: bool,
    dom_snapshot_available: bool,
    event_decoder_ok: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    runtime: RuntimeStatus,
}

#[derive(Debug, Serialize)]
struct DownloadsResponse {
    download_dir: Option<PathBuf>,
    downloads: Vec<DownloadState>,
}

#[derive(Debug, Deserialize)]
struct NativeOperationError {
    kind: String,
    operation: String,
    stage: String,
    message: String,
    elapsed_ms: u64,
    timed_out: bool,
    restart_recommended: bool,
}

#[derive(Debug, Clone, Default)]
struct AutoCredentialCapture {
    username: Option<CapturedCredentialField>,
    password: Option<CapturedCredentialField>,
    origin: Option<String>,
}

#[derive(Debug, Clone)]
struct CapturedCredentialField {
    value: String,
    field_name: Option<String>,
    label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialFieldKind {
    Username,
    Password,
}

fn create_context_runtime(
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<MainThreadContext, AegisError> {
    emit_telemetry(
        "context_start",
        json!({
            "browser_mode": browser_config.mode,
            "start_url": browser_config.start_url,
            "profile": profile_name,
        }),
    );
    let serve_started = std::time::Instant::now();
    let profile_store = SessionProfileStore::new(profile_name).map_err(AegisError::Bridge)?;
    let credential_settings = AegisConfigStore::detect()
        .and_then(|store| store.load_credentials_settings())
        .map_err(AegisError::Bridge)?;
    let credential_store = AegisSecretStore::detect().map_err(AegisError::Bridge)?;
    let credential_capture = AutoCredentialCapture::default();
    let pending_startup_session = profile_store.load().map_err(AegisError::Bridge)?;
    let (tx, rx) = mpsc::channel::<ApiCommand>();
    let startup = Arc::new(Mutex::new(ServeStartupMetrics {
        client_connect_ms: 0,
        api_bind_ms: 0,
        total_ready_ms: 0,
    }));
    let diagnostics = Arc::new(Mutex::new(ServeDiagnostics::new(default_runtime_status())));
    let cancel = Arc::new(Mutex::new(None));
    let state = ApiState {
        tx,
        cancel: cancel.clone(),
        host_library: host_library.clone(),
        browser: browser_config.clone(),
        startup: startup.clone(),
        profile: profile_store.info(),
        diagnostics: diagnostics.clone(),
    };
    let idle_pump_interval = match browser_config.mode {
        crate::browser::BrowserMode::Headful => HEADFUL_IDLE_PUMP_INTERVAL,
        crate::browser::BrowserMode::Headless => HEADLESS_IDLE_PUMP_INTERVAL,
    };
    let client_connect_started = Instant::now();
    let client = LoadedAegisClient::connect(host_library.clone(), browser_config.clone())?;
    emit_telemetry(
        "serve_client_connected",
        json!({
            "latency_ms": client_connect_started.elapsed().as_millis() as u64,
            "browser_mode": browser_config.mode,
            "runtime": client.runtime_status(),
        }),
    );
    if let Ok(mut shared_cancel) = cancel.lock() {
        *shared_cancel = Some(client.cancel_handle());
    }
    let startup_metrics = ServeStartupMetrics {
        client_connect_ms: client_connect_started.elapsed().as_millis() as u64,
        api_bind_ms: 0,
        total_ready_ms: serve_started.elapsed().as_millis() as u64,
    };
    if let Ok(mut shared) = startup.lock() {
        *shared = startup_metrics.clone();
    }
    emit_telemetry(
        "serve_ready",
        json!({
            "client_connect_ms": startup_metrics.client_connect_ms,
            "api_bind_ms": startup_metrics.api_bind_ms,
            "total_ready_ms": startup_metrics.total_ready_ms,
            "browser_mode": browser_config.mode,
        }),
    );
    eprintln!(
        "Aegis context ready for profile {} ({:?}, host: {})",
        profile_store.info().profile,
        browser_config.mode,
        host_library.display()
    );

    Ok(MainThreadContext {
        api: state,
        rx,
        client,
        profile_store,
        credential_settings,
        credential_store,
        credential_capture,
        pending_startup_session,
        idle_pump_interval,
        last_pump_at: Instant::now(),
    })
}

enum ContextTickOutcome {
    Running,
    Stop,
}

impl MainThreadContext {
    fn handle_startup_session(&mut self) -> Result<(), AegisError> {
        let Some(session) = self.pending_startup_session.take() else {
            return Ok(());
        };
        let started_at = Instant::now();
        record_operation_started(
            &self.api.diagnostics,
            "startup_restore_session",
            "restoring persisted startup session",
        );
        emit_telemetry(
            "startup_session_restore_started",
            json!({
                "browser_mode": self.api.browser.mode,
                "profile": self.profile_store.info().profile,
            }),
        );
        let result = self.client.inject_session(session);
        record_operation_finished(
            &self.api.diagnostics,
            "startup_restore_session",
            &self.client,
            &result,
        );
        emit_operation_telemetry(
            "startup_restore_session",
            started_at,
            &result,
            self.client.runtime_status(),
        );
        if let Err(error) = result {
            emit_telemetry(
                "startup_session_restore_failed",
                json!({
                    "browser_mode": self.api.browser.mode,
                    "profile": self.profile_store.info().profile,
                    "error": error.to_string(),
                    "runtime": self.client.runtime_status(),
                }),
            );
            return Err(error);
        }
        emit_telemetry(
            "startup_session_restore_completed",
            json!({
                "browser_mode": self.api.browser.mode,
                "profile": self.profile_store.info().profile,
                "latency_ms": started_at.elapsed().as_millis() as u64,
                "runtime": self.client.runtime_status(),
            }),
        );
        Ok(())
    }

    fn tick(&mut self) -> ContextTickOutcome {
        if let Err(error) = self.handle_startup_session() {
            record_operation_failure(
                &self.api.diagnostics,
                "startup_restore_session",
                failure_from_error(
                    "startup_restore_session",
                    "restoring persisted startup session",
                    &error,
                ),
                Some(self.client.runtime_status()),
            );
            return ContextTickOutcome::Stop;
        }

        let mut processed_command = false;
        loop {
            match self.rx.try_recv() {
                Ok(command) => {
                    processed_command = true;
                    if matches!(self.handle_command(command), ContextTickOutcome::Stop) {
                        return ContextTickOutcome::Stop;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return ContextTickOutcome::Stop,
            }
        }

        if !processed_command && self.last_pump_at.elapsed() >= self.idle_pump_interval {
            match self.client.pump() {
                Ok(()) => {
                    record_heartbeat(&self.api.diagnostics, &self.client);
                    self.last_pump_at = Instant::now();
                }
                Err(error) => {
                    emit_telemetry(
                        "runtime_pump_failure",
                        json!({
                            "error": error.to_string(),
                            "runtime": self.client.runtime_status(),
                        }),
                    );
                    record_operation_failure(
                        &self.api.diagnostics,
                        "pump",
                        failure_from_error("pump", "pumping browser event loop", &error),
                        Some(self.client.runtime_status()),
                    );
                    return ContextTickOutcome::Stop;
                }
            }
        }

        ContextTickOutcome::Running
    }

    fn handle_command(&mut self, command: ApiCommand) -> ContextTickOutcome {
        match command {
            ApiCommand::InjectSession(session, reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "inject_session",
                    "injecting session",
                );
                let result = self.client.inject_session(session.clone()).and_then(|_| {
                    self.profile_store
                        .save(&session)
                        .map(|_| ())
                        .map_err(AegisError::Bridge)
                });
                record_operation_finished(
                    &self.api.diagnostics,
                    "inject_session",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "inject_session",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::SnapshotSession(reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "snapshot_session",
                    "capturing session state",
                );
                let result = self.client.snapshot_session();
                record_operation_finished(
                    &self.api.diagnostics,
                    "snapshot_session",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "snapshot_session",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::SaveSessionProfile(reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "save_session_profile",
                    "persisting session profile",
                );
                let result = self.client.snapshot_session().and_then(|session| {
                    self.profile_store
                        .save(&session)
                        .map(|_| self.profile_store.info())
                        .map_err(AegisError::Bridge)
                });
                record_operation_finished(
                    &self.api.diagnostics,
                    "save_session_profile",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "save_session_profile",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::LoadSessionProfile(reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "load_session_profile",
                    "loading session profile",
                );
                let result = self
                    .profile_store
                    .load()
                    .map_err(AegisError::Bridge)
                    .and_then(|maybe_session| match maybe_session {
                        Some(session) => self
                            .client
                            .inject_session(session)
                            .map(|_| self.profile_store.info()),
                        None => Ok(self.profile_store.info()),
                    });
                record_operation_finished(
                    &self.api.diagnostics,
                    "load_session_profile",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "load_session_profile",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::ActivateBrowser(browser_id, reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "activate_browser",
                    &format!("activating attached browser {browser_id}"),
                );
                let result = self.client.activate_browser(browser_id);
                record_operation_finished(
                    &self.api.diagnostics,
                    "activate_browser",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "activate_browser",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::Navigate(url, reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "navigate",
                    &format!("navigating to {url}"),
                );
                self.credential_capture.reset_on_explicit_navigation(&url);
                let result = self.client.navigate(url);
                record_operation_finished(&self.api.diagnostics, "navigate", &self.client, &result);
                emit_operation_telemetry(
                    "navigate",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::Execute(commands, reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "execute",
                    "executing browser command batch",
                );
                let maybe_snapshot = if self.credential_settings.auto_store
                    && commands.iter().any(|command| {
                        matches!(command, Command::SetValue { .. } | Command::Click { .. })
                    }) {
                    match self.client.snapshot_dom() {
                        Ok(snapshot) => Some(snapshot),
                        Err(error) => {
                            record_operation_failure(
                                &self.api.diagnostics,
                                "execute",
                                failure_from_error(
                                    "execute",
                                    "capturing pre-execution DOM snapshot",
                                    &error,
                                ),
                                Some(self.client.runtime_status()),
                            );
                            emit_operation_telemetry(
                                "execute",
                                started_at,
                                &Err::<ExecutionReport, AegisError>(AegisError::Bridge(
                                    error.to_string(),
                                )),
                                self.client.runtime_status(),
                            );
                            let _ = reply.send(Err(error));
                            return ContextTickOutcome::Running;
                        }
                    }
                } else {
                    None
                };
                if let Some(snapshot) = maybe_snapshot.as_ref() {
                    self.credential_capture.capture_fields(
                        snapshot,
                        self.client.runtime().current_url(),
                        &commands,
                    );
                }
                let should_persist = self.credential_settings.auto_store
                    && maybe_snapshot.as_ref().is_some_and(|snapshot| {
                        self.credential_capture.should_persist(snapshot, &commands)
                    });
                let persist_origin = if should_persist {
                    self.client.runtime().current_url().map(origin_key)
                } else {
                    None
                };
                let result = self.client.execute(&commands).and_then(|report| {
                    if let Some(origin) = persist_origin {
                        self.credential_capture.persist(
                            &self.credential_store,
                            &self.profile_store.info().profile,
                            &origin,
                        )?;
                    }
                    Ok(report)
                });
                record_operation_finished(&self.api.diagnostics, "execute", &self.client, &result);
                emit_operation_telemetry(
                    "execute",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::SnapshotDom(reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "snapshot_dom",
                    "capturing DOM snapshot",
                );
                let result = self.client.snapshot_dom();
                record_operation_finished(
                    &self.api.diagnostics,
                    "snapshot_dom",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "snapshot_dom",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::PageResearch(reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "page_research",
                    "capturing page research snapshot",
                );
                let result = self.client.page_research_data();
                record_operation_finished(
                    &self.api.diagnostics,
                    "page_research",
                    &self.client,
                    &result,
                );
                emit_operation_telemetry(
                    "page_research",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::Events(since, reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "events",
                    "draining runtime events",
                );
                let result = self.client.events_since(since);
                record_operation_finished(&self.api.diagnostics, "events", &self.client, &result);
                emit_operation_telemetry(
                    "events",
                    started_at,
                    &result,
                    self.client.runtime_status(),
                );
                let _ = reply.send(result);
            }
            ApiCommand::EnableTrace(path, reply) => {
                let started_at = Instant::now();
                record_operation_started(
                    &self.api.diagnostics,
                    "enable_trace",
                    "enabling trace recording",
                );
                self.client.enable_trace_recording(path);
                record_operation_finished(
                    &self.api.diagnostics,
                    "enable_trace",
                    &self.client,
                    &Ok(()),
                );
                emit_operation_telemetry(
                    "enable_trace",
                    started_at,
                    &Ok(()),
                    self.client.runtime_status(),
                );
                let _ = reply.send(Ok(()));
            }
        }

        ContextTickOutcome::Running
    }

    fn persist_session_best_effort(&mut self) {
        if let Ok(session) = self.client.snapshot_session() {
            let _ = self.profile_store.save(&session);
        }
    }

    fn summary(&self, context_id: String, default: bool) -> ContextSummary {
        let diagnostics = read_diagnostics(&self.api.diagnostics);
        ContextSummary {
            id: context_id,
            default,
            host_library: self.api.host_library.clone(),
            browser: self.api.browser.clone(),
            profile: self.api.profile.clone(),
            runtime_state: serde_json::to_value(&diagnostics.state)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "unknown".into()),
            command_ready: diagnostics.command_ready,
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<(), AegisError> {
    let _ = (addr, host_library, browser_config, profile_name);
    Err(AegisError::Bridge(
        "non-macOS serve bootstrap must be reworked to the new coordinator model".into(),
    ))
}

#[cfg(target_os = "macos")]
pub fn serve_main_thread(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<(), AegisError> {
    let default_context_id = "default".to_string();
    let default_context =
        create_context_runtime(host_library.clone(), browser_config.clone(), profile_name)?;
    if let Ok(mut startup) = default_context.api.startup.lock() {
        startup.api_bind_ms = 0;
    }
    let (manager_tx, manager_rx) = mpsc::channel();
    let root = ServeRootState {
        host_library: host_library.clone(),
        browser: browser_config.clone(),
        default_context_id: default_context_id.clone(),
        contexts: Arc::new(Mutex::new(HashMap::from([(
            default_context_id.clone(),
            default_context.api.clone(),
        )]))),
        manager_tx,
    };
    let mut runtimes = HashMap::from([(default_context_id, default_context)]);

    let (bind_tx, bind_rx) = mpsc::channel::<Result<(), AegisError>>();
    let (server_exit_tx, server_exit_rx) = mpsc::channel::<Result<(), AegisError>>();
    let root_for_server = root.clone();
    let server_thread = thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let error = AegisError::Bridge(error.to_string());
                let _ = bind_tx.send(Err(AegisError::Bridge(error.to_string())));
                let _ = server_exit_tx.send(Err(error));
                return;
            }
        };

        let server_result = runtime.block_on(async move {
            let bind_started = Instant::now();
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|error| AegisError::Bridge(error.to_string()))?;
            if let Some(default_state) = root_for_server.default_context_state()
                && let Ok(mut startup) = default_state.startup.lock()
            {
                startup.api_bind_ms = bind_started.elapsed().as_millis() as u64;
            }
            eprintln!("Aegis serve ready on http://{}", addr);
            let _ = bind_tx.send(Ok(()));
            let app = router(root_for_server.clone());
            axum::serve(listener, app)
                .await
                .map_err(|error| AegisError::Bridge(error.to_string()))
        });
        let _ = server_exit_tx.send(server_result);
    });

    match bind_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(_) => {
            return match server_thread.join() {
                Ok(()) => match server_exit_rx.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                        Err(AegisError::Bridge(
                            "serve HTTP bootstrap exited before signaling readiness".into(),
                        ))
                    }
                },
                Err(_) => Err(AegisError::Bridge(
                    "serve HTTP server thread panicked before signaling readiness".into(),
                )),
            };
        }
    }

    let result = run_main_thread_serve_loop(&root, &mut runtimes, manager_rx, &server_exit_rx);
    shutdown_active_contexts(&root, &mut runtimes);
    let server_result = match server_exit_rx.try_recv() {
        Ok(result) => result,
        Err(_) => match server_thread.join() {
            Ok(()) => Ok(()),
            Err(_) => Err(AegisError::Bridge(
                "serve HTTP server thread panicked".into(),
            )),
        },
    };
    result?;
    server_result?;
    Ok(())
}

impl AutoCredentialCapture {
    fn capture_fields(
        &mut self,
        snapshot: &DomSnapshot,
        current_url: Option<&str>,
        commands: &[Command],
    ) {
        let current_origin = current_url.map(origin_key);
        if let (Some(existing), Some(current)) = (self.origin.as_ref(), current_origin.as_ref())
            && existing != current
        {
            self.clear();
        }
        if self.origin.is_none() {
            self.origin = current_origin;
        }

        for command in commands {
            let Command::SetValue { target, value } = command else {
                continue;
            };
            let Some(node) = resolve_command_target(snapshot, target) else {
                continue;
            };
            let Some(kind) = classify_credential_field(node) else {
                continue;
            };
            let field = CapturedCredentialField {
                value: value.clone(),
                field_name: node.attrs.get("name").cloned(),
                label: node
                    .semantic
                    .as_ref()
                    .and_then(|semantic| semantic.label.clone().or_else(|| semantic.name.clone())),
            };
            match kind {
                CredentialFieldKind::Username => self.username = Some(field),
                CredentialFieldKind::Password => self.password = Some(field),
            }
        }
    }

    fn should_persist(&self, snapshot: &DomSnapshot, commands: &[Command]) -> bool {
        self.username.is_some()
            && self.password.is_some()
            && commands
                .iter()
                .any(|command| command_requests_credential_submit(snapshot, command, self))
    }

    fn persist(
        &mut self,
        store: &AegisSecretStore,
        profile: &str,
        fallback_origin: &str,
    ) -> Result<(), AegisError> {
        let Some(username) = self.username.as_ref() else {
            return Ok(());
        };
        let Some(password) = self.password.as_ref() else {
            return Ok(());
        };
        store
            .upsert_profile_credential(
                profile,
                CredentialInput {
                    origin: self
                        .origin
                        .clone()
                        .unwrap_or_else(|| fallback_origin.to_string()),
                    username: username.value.clone(),
                    password: password.value.clone(),
                    username_field: username.field_name.clone(),
                    password_field: password.field_name.clone(),
                    form_label: password.label.clone().or_else(|| username.label.clone()),
                },
            )
            .map_err(AegisError::Bridge)?;
        self.clear();
        Ok(())
    }

    fn reset_on_explicit_navigation(&mut self, url: &str) {
        let target_origin = origin_key(url);
        if self
            .origin
            .as_ref()
            .is_some_and(|origin| origin != &target_origin)
        {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.username = None;
        self.password = None;
        self.origin = None;
    }
}

fn resolve_command_target<'a>(
    snapshot: &'a DomSnapshot,
    target: &CommandTarget,
) -> Option<&'a DomNode> {
    resolve_snapshot_target(snapshot, target, None)
}

fn classify_credential_field(node: &DomNode) -> Option<CredentialFieldKind> {
    let control_type = node
        .semantic
        .as_ref()
        .and_then(|semantic| semantic.control_type.as_deref())
        .or_else(|| node.attrs.get("type").map(String::as_str))
        .unwrap_or("text");
    if includes_normalized(Some(control_type), "password") {
        return Some(CredentialFieldKind::Password);
    }
    let hint = credential_hint_text(node);
    if matches!(
        control_type,
        "email" | "text" | "searchbox" | "search" | "textbox"
    ) && (hint.contains("user")
        || hint.contains("email")
        || hint.contains("login")
        || hint.contains("account")
        || hint.contains("identifier")
        || hint.contains("member"))
    {
        return Some(CredentialFieldKind::Username);
    }
    None
}

fn is_submit_like_node(node: &DomNode) -> bool {
    let semantic = node.semantic.as_ref();
    if semantic.is_some_and(|semantic| semantic.actions.iter().any(|action| action == "submit")) {
        return true;
    }
    if semantic
        .as_ref()
        .and_then(|semantic| semantic.control_type.as_deref())
        .is_some_and(|control| matches!(control, "submit" | "button"))
    {
        return true;
    }
    let text = credential_hint_text(node);
    text.contains("sign in")
        || text.contains("log in")
        || text.contains("login")
        || text.contains("continue")
        || text.contains("submit")
}

fn command_requests_credential_submit(
    snapshot: &DomSnapshot,
    command: &Command,
    capture: &AutoCredentialCapture,
) -> bool {
    match command {
        Command::Click { target } => submit_target_matches(snapshot, target),
        Command::PressKey {
            target, key, code, ..
        } => {
            is_submit_key(key, code.as_deref())
                && match target.as_ref() {
                    Some(target) => {
                        submit_target_matches(snapshot, target)
                            || command_target_matches_password_field(snapshot, target)
                    }
                    None => capture.password.is_some(),
                }
        }
        _ => false,
    }
}

fn is_submit_key(key: &str, code: Option<&str>) -> bool {
    matches!(
        normalize_text(key).as_str(),
        "enter" | "return" | "numpadenter"
    ) || code
        .map(normalize_text)
        .is_some_and(|code| matches!(code.as_str(), "enter" | "return" | "numpadenter"))
}

fn submit_target_matches(snapshot: &DomSnapshot, target: &CommandTarget) -> bool {
    resolve_command_target(snapshot, target).is_some_and(is_submit_like_node)
        || command_target_submit_hint(target)
        || snapshot_has_single_submit_candidate(snapshot)
}

fn command_target_submit_hint(target: &CommandTarget) -> bool {
    match target {
        CommandTarget::Id { .. } => false,
        CommandTarget::Match { matcher } => matcher_submit_hint(matcher),
    }
}

fn matcher_submit_hint(matcher: &CommandMatcher) -> bool {
    matcher
        .role
        .as_deref()
        .is_some_and(|value| normalize_text(value) == "button")
        || matcher
            .control_type
            .as_deref()
            .is_some_and(|value| matches!(normalize_text(value).as_str(), "button" | "submit"))
        || matcher
            .tag
            .as_deref()
            .is_some_and(|value| matches!(normalize_text(value).as_str(), "button" | "input"))
        || [
            matcher.selector.as_deref(),
            matcher.name.as_deref(),
            matcher.label.as_deref(),
            matcher.text.as_deref(),
            matcher.test_id.as_deref(),
            matcher.placeholder.as_deref(),
            matcher.href_contains.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(is_submit_hint_text)
}

fn command_target_matches_password_field(snapshot: &DomSnapshot, target: &CommandTarget) -> bool {
    resolve_command_target(snapshot, target)
        .and_then(classify_credential_field)
        .is_some_and(|kind| kind == CredentialFieldKind::Password)
        || matches!(
            target,
            CommandTarget::Match { matcher }
                if matcher_matches_password_hint(matcher)
        )
}

fn matcher_matches_password_hint(matcher: &CommandMatcher) -> bool {
    [
        matcher.selector.as_deref(),
        matcher.name.as_deref(),
        matcher.label.as_deref(),
        matcher.text.as_deref(),
        matcher.placeholder.as_deref(),
        matcher.test_id.as_deref(),
        matcher.control_type.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(is_password_hint_text)
}

fn snapshot_has_single_submit_candidate(snapshot: &DomSnapshot) -> bool {
    snapshot
        .nodes
        .iter()
        .filter(|node| is_submit_like_node(node))
        .take(2)
        .count()
        == 1
}

fn is_submit_hint_text(value: &str) -> bool {
    let text = normalize_text(value);
    text.contains("sign in")
        || text.contains("signin")
        || text.contains("log in")
        || text.contains("login")
        || text.contains("continue")
        || text.contains("submit")
        || text.contains("password-next")
        || text.contains("type=submit")
        || text.contains("[type=submit]")
        || text.contains("button[type=submit]")
}

fn is_password_hint_text(value: &str) -> bool {
    let text = normalize_text(value);
    text.contains("password") || text.contains("passcode") || text.contains("current-password")
}

fn credential_hint_text(node: &DomNode) -> String {
    let mut parts = Vec::new();
    if let Some(text) = node.text.as_ref() {
        parts.push(text.as_str());
    }
    for key in [
        "name",
        "type",
        "placeholder",
        "title",
        "autocomplete",
        "aria-label",
        "value",
    ] {
        if let Some(value) = node.attrs.get(key) {
            parts.push(value.as_str());
        }
    }
    if let Some(semantic) = node.semantic.as_ref() {
        if let Some(name) = semantic.name.as_ref() {
            parts.push(name.as_str());
        }
        if let Some(label) = semantic.label.as_ref() {
            parts.push(label.as_str());
        }
        if let Some(control_type) = semantic.control_type.as_ref() {
            parts.push(control_type.as_str());
        }
    }
    normalize_text(&parts.join(" "))
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn includes_normalized(haystack: Option<&str>, needle: &str) -> bool {
    haystack
        .map(normalize_text)
        .is_some_and(|haystack| haystack.contains(&normalize_text(needle)))
}

fn search_engine_name(engine: Option<&str>) -> &'static str {
    match engine.map(normalize_text).as_deref() {
        Some("google") => "google",
        Some("bing") => "bing",
        _ => "duckduckgo",
    }
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            output.push(ch);
        } else if ch == ' ' {
            output.push('+');
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02X}"));
        }
    }
    output
}

fn build_search_url(query: &str, engine: Option<&str>) -> String {
    let encoded = percent_encode(query.trim());
    match search_engine_name(engine) {
        "google" => format!("https://www.google.com/search?q={encoded}"),
        "bing" => format!("https://www.bing.com/search?q={encoded}"),
        _ => format!("https://duckduckgo.com/?q={encoded}"),
    }
}

fn render_page_markdown(page: &PageResearchData) -> String {
    let mut sections = Vec::new();
    if let Some(title) = page.title.as_ref()
        && !title.trim().is_empty()
    {
        sections.push(format!("# {}", title.trim()));
    }
    if let Some(url) = page.url.as_ref() {
        sections.push(format!("Source: {url}"));
    }
    if let Some(canonical_url) = page.canonical_url.as_ref()
        && page.url.as_ref() != Some(canonical_url)
    {
        sections.push(format!("Canonical: {canonical_url}"));
    }
    if !page.headings.is_empty() {
        sections.push(String::from("## Headings"));
        sections.extend(page.headings.iter().map(|heading| {
            let level = heading.level.unwrap_or(2).clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), heading.text)
        }));
    }
    if !page.visible_text.trim().is_empty() {
        sections.push(String::from("## Visible Text"));
        sections.push(page.visible_text.clone());
    }
    if !page.links.is_empty() {
        sections.push(String::from("## Links"));
        sections.extend(page.links.iter().map(|link| {
            let label = if link.text.trim().is_empty() {
                link.href.clone()
            } else {
                link.text.clone()
            };
            format!("- [{label}]({})", link.href)
        }));
    }
    sections.join("\n\n")
}

fn snippet_for_match(text: &str, normalized_query: &str) -> Option<String> {
    if normalized_query.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let start = lower.find(normalized_query)?;
    let end = start.saturating_add(normalized_query.len());
    let snippet_start = start.saturating_sub(80);
    let snippet_end = (end + 80).min(text.len());
    Some(text[snippet_start..snippet_end].trim().to_string())
}

fn page_find_matches(page: &PageResearchData, query: &str, exact: bool) -> Vec<PageFindMatch> {
    let normalized_query = normalize_text(query);
    if normalized_query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for heading in &page.headings {
        let candidate = normalize_text(&heading.text);
        let matched = if exact {
            candidate == normalized_query
        } else {
            candidate.contains(&normalized_query)
        };
        if matched {
            matches.push(PageFindMatch {
                kind: String::from("heading"),
                text: heading.text.clone(),
                level: heading.level,
                href: None,
                index: None,
                snippet: Some(heading.text.clone()),
            });
        }
    }
    for link in &page.links {
        let link_text = normalize_text(&link.text);
        let href_text = normalize_text(&link.href);
        let matched = if exact {
            link_text == normalized_query || href_text == normalized_query
        } else {
            link_text.contains(&normalized_query) || href_text.contains(&normalized_query)
        };
        if matched {
            matches.push(PageFindMatch {
                kind: String::from("link"),
                text: if link.text.trim().is_empty() {
                    link.href.clone()
                } else {
                    link.text.clone()
                },
                level: None,
                href: Some(link.href.clone()),
                index: Some(link.index),
                snippet: link.title.clone(),
            });
        }
    }
    if let Some(snippet) = snippet_for_match(&page.visible_text, &normalized_query) {
        matches.push(PageFindMatch {
            kind: String::from("text"),
            text: query.to_string(),
            level: None,
            href: None,
            index: None,
            snippet: Some(snippet),
        });
    }
    matches
}

fn matching_links(
    page: &PageResearchData,
    text: &str,
    exact: bool,
    href_contains: Option<&str>,
) -> Vec<PageResearchLink> {
    let normalized_text = normalize_text(text);
    let normalized_href = href_contains.map(normalize_text);
    page.links
        .iter()
        .filter(|link| {
            let link_text = if link.text.trim().is_empty() {
                link.href.as_str()
            } else {
                link.text.as_str()
            };
            let link_text = normalize_text(link_text);
            let text_matches = if exact {
                link_text == normalized_text
            } else {
                link_text.contains(&normalized_text)
            };
            let href_matches = normalized_href.as_ref().is_none_or(|needle| {
                normalize_text(&link.href).contains(needle)
            });
            text_matches && href_matches
        })
        .cloned()
        .collect()
}

fn origin_key(url: &str) -> String {
    let trimmed = url.trim();
    if let Some((scheme, rest)) = trimmed.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        return format!("{scheme}://{host}");
    }
    trimmed.to_string()
}

#[allow(clippy::result_large_err)]
fn validate_context_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::from(AegisError::Bridge(
            "context id must not be empty".into(),
        )));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::from(AegisError::Bridge(format!(
            "context id {name:?} must use only letters, numbers, '.', '-', or '_'"
        ))));
    }
    Ok(())
}

impl ServeRootState {
    fn default_context_state(&self) -> Option<ApiState> {
        self.context_state(&self.default_context_id)
    }

    fn context_state(&self, context_id: &str) -> Option<ApiState> {
        self.contexts
            .lock()
            .ok()
            .and_then(|contexts| contexts.get(context_id).cloned())
    }

    #[allow(clippy::result_large_err)]
    fn insert_context(&self, context_id: String, state: ApiState) -> Result<(), ApiError> {
        let mut contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        contexts.insert(context_id, state);
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn remove_context(&self, context_id: &str) -> Result<Option<ApiState>, ApiError> {
        let mut contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        Ok(contexts.remove(context_id))
    }

    #[allow(clippy::result_large_err)]
    fn list_contexts(&self) -> Result<Vec<ContextSummary>, ApiError> {
        let contexts = self.contexts.lock().map_err(|_| {
            ApiError::from(AegisError::Bridge("context registry lock poisoned".into()))
        })?;
        let mut items = contexts
            .iter()
            .map(|(id, state)| {
                let diagnostics = read_diagnostics(&state.diagnostics);
                ContextSummary {
                    id: id.clone(),
                    default: id == &self.default_context_id,
                    host_library: state.host_library.clone(),
                    browser: state.browser.clone(),
                    profile: state.profile.clone(),
                    runtime_state: serde_json::to_value(&diagnostics.state)
                        .ok()
                        .and_then(|value| value.as_str().map(ToOwned::to_owned))
                        .unwrap_or_else(|| "unknown".into()),
                    command_ready: diagnostics.command_ready,
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(items)
    }
}

pub fn router(state: ServeRootState) -> Router {
    Router::new()
        .route("/", get(api_manifest))
        .route("/manifest", get(api_manifest))
        .route("/version", get(api_version))
        .route("/contexts", get(list_contexts).post(create_context))
        .route(
            "/contexts/{context_id}",
            get(get_context).delete(delete_context),
        )
        .route("/contexts/{context_id}/healthz", get(context_health))
        .route("/contexts/{context_id}/readyz", get(context_readiness))
        .route("/contexts/{context_id}/doctor", get(context_doctor))
        .route("/contexts/{context_id}/runtime", get(context_runtime_info))
        .route(
            "/contexts/{context_id}/runtime/cancel",
            post(context_cancel_runtime_operation),
        )
        .route(
            "/contexts/{context_id}/browsers/{browser_id}/activate",
            post(context_activate_browser),
        )
        .route(
            "/contexts/{context_id}/session",
            post(context_inject_session).get(context_snapshot_session),
        )
        .route(
            "/contexts/{context_id}/session/save",
            post(context_save_session_profile),
        )
        .route(
            "/contexts/{context_id}/session/load",
            post(context_load_session_profile),
        )
        .route("/contexts/{context_id}/search", post(context_search))
        .route("/contexts/{context_id}/navigate", post(context_navigate))
        .route("/contexts/{context_id}/execute", post(context_execute))
        .route("/contexts/{context_id}/page", get(context_page_research))
        .route("/contexts/{context_id}/page/text", get(context_page_text))
        .route(
            "/contexts/{context_id}/page/markdown",
            get(context_page_markdown),
        )
        .route("/contexts/{context_id}/page/links", get(context_page_links))
        .route(
            "/contexts/{context_id}/page/headings",
            get(context_page_headings),
        )
        .route("/contexts/{context_id}/page/find", post(context_page_find))
        .route(
            "/contexts/{context_id}/page/open-link",
            post(context_page_open_link),
        )
        .route("/contexts/{context_id}/dom", get(context_snapshot_dom))
        .route("/contexts/{context_id}/events", get(context_events))
        .route("/contexts/{context_id}/downloads", get(context_downloads))
        .route(
            "/contexts/{context_id}/events/live",
            get(context_events_live),
        )
        .route(
            "/contexts/{context_id}/trace/enable",
            post(context_enable_trace),
        )
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/doctor", get(doctor))
        .route("/runtime", get(runtime_info))
        .route("/runtime/cancel", post(cancel_runtime_operation))
        .route("/browsers/{browser_id}/activate", post(activate_browser))
        .route("/session", post(inject_session).get(snapshot_session))
        .route("/session/save", post(save_session_profile))
        .route("/session/load", post(load_session_profile))
        .route("/search", post(search))
        .route("/navigate", post(navigate))
        .route("/execute", post(execute))
        .route("/page", get(page_research))
        .route("/page/text", get(page_text))
        .route("/page/markdown", get(page_markdown))
        .route("/page/links", get(page_links))
        .route("/page/headings", get(page_headings))
        .route("/page/find", post(page_find))
        .route("/page/open-link", post(page_open_link))
        .route("/dom", get(snapshot_dom))
        .route("/events", get(events))
        .route("/downloads", get(downloads))
        .route("/events/live", get(events_live))
        .route("/trace/enable", post(enable_trace))
        .with_state(state)
}

#[allow(clippy::result_large_err)]
fn require_default_context_state(root: &ServeRootState) -> Result<ApiState, ApiError> {
    root.default_context_state()
        .ok_or_else(|| ApiError::from(AegisError::Bridge("default context unavailable".into())))
}

#[allow(clippy::result_large_err)]
fn require_context_state(root: &ServeRootState, context_id: &str) -> Result<ApiState, ApiError> {
    root.context_state(context_id)
        .ok_or_else(|| ApiError::not_found(format!("context `{context_id}` was not found")))
}

async fn manage_context<T>(
    root: &ServeRootState,
    command: ManagerCommand,
    reply_rx: oneshot::Receiver<Result<T, AegisError>>,
    timeout_operation: &str,
) -> Result<T, ApiError> {
    root.manager_tx
        .send(command)
        .map_err(manager_channel_error)?;
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => Err(ApiError::from(error)),
        Ok(Err(error)) => Err(reply_error(error)),
        Err(_) => Err(ApiError::timeout(timeout_operation)),
    }
}

async fn health(State(root): State<ServeRootState>) -> Result<Json<HealthResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    Ok(Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        control_plane_up: true,
        runtime_state: diagnostics.state.clone(),
        command_ready: diagnostics.command_ready,
        bridge_healthy: diagnostics.bridge_healthy,
        browser_backend_healthy: diagnostics.browser_backend_healthy,
        browser_process_up: diagnostics.browser_process_up,
        page_attached: diagnostics.page_attached,
        renderer_attached: diagnostics.renderer_attached,
        dom_snapshot_available: diagnostics.dom_snapshot_available,
        event_decoder_ok: diagnostics.event_decoder_ok,
        active_operation: diagnostics.active_operation,
        last_failure: diagnostics.last_failure,
    }))
}

#[derive(Debug, Serialize)]
struct RuntimeInfo {
    host_library: PathBuf,
    browser: BrowserConfig,
    diagnostics: RuntimeDiagnosticsResponse,
    startup: ServeStartupMetrics,
    profile: SessionProfileInfo,
    runtime_identity: RuntimeIdentity,
}

#[derive(Debug, Serialize)]
struct ApiRouteDoc {
    method: &'static str,
    path: &'static str,
    summary: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    params: Vec<ApiParameterDoc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    query: Vec<ApiParameterDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_body: Option<ApiRequestBodyDoc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    responses: Vec<ApiResponseDoc>,
}

#[derive(Debug, Serialize)]
struct ApiCapabilityStatusDoc {
    name: &'static str,
    supported: bool,
    status: &'static str,
    runtime_validated: bool,
    validated_by: &'static str,
    details: &'static str,
}

#[derive(Debug, Serialize)]
struct RuntimeIdentity {
    process_id: u32,
    executable_path: Option<PathBuf>,
    started_at_ms: u64,
    uptime_ms: u64,
}

#[derive(Debug, Serialize)]
struct ApiSchemaFieldDoc {
    name: &'static str,
    kind: &'static str,
    required: bool,
    description: &'static str,
}

#[derive(Debug, Serialize)]
struct ApiParameterDoc {
    name: &'static str,
    location: &'static str,
    kind: &'static str,
    required: bool,
    description: &'static str,
}

#[derive(Debug, Serialize)]
struct ApiRequestBodyDoc {
    content_type: &'static str,
    schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    example: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ApiResponseDoc {
    status: u16,
    description: &'static str,
    schema: &'static str,
}

#[derive(Debug, Serialize)]
struct ApiNamedSchemaDoc {
    name: &'static str,
    kind: &'static str,
    fields: Vec<ApiSchemaFieldDoc>,
}

#[derive(Debug, Serialize)]
struct ApiCommandDoc {
    name: &'static str,
    summary: &'static str,
    targeting: &'static str,
    fields: Vec<ApiSchemaFieldDoc>,
}

#[derive(Debug, Serialize)]
struct ApiManifest {
    service: &'static str,
    version: &'static str,
    protocol_version: u16,
    discovery: &'static str,
    runtime_identity: RuntimeIdentity,
    routes: Vec<ApiRouteDoc>,
    capabilities: Vec<&'static str>,
    capability_status: Vec<ApiCapabilityStatusDoc>,
    event_filters: Vec<&'static str>,
    commands: Vec<ApiCommandDoc>,
    schemas: Vec<ApiNamedSchemaDoc>,
    error_schema: ApiNamedSchemaDoc,
}

async fn api_manifest(State(root): State<ServeRootState>) -> Json<ApiManifest> {
    let runtime_validated = root
        .default_context_state()
        .map(|state| {
            let diagnostics = read_diagnostics(&state.diagnostics);
            diagnostics.command_ready
                && diagnostics.bridge_healthy
                && diagnostics.browser_backend_healthy
        })
        .unwrap_or(false);
    Json(api_manifest_document(runtime_validated))
}

fn api_manifest_document(runtime_validated: bool) -> ApiManifest {
    ApiManifest {
        service: "aegis",
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        discovery: "/manifest",
        runtime_identity: runtime_identity(),
        routes: vec![
            ApiRouteDoc {
                method: "GET",
                path: "/",
                summary: "JSON API discovery document",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Machine-readable API manifest",
                    "ApiManifest",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/manifest",
                summary: "JSON API discovery document",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Machine-readable API manifest",
                    "ApiManifest",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/version",
                summary: "Version and protocol metadata",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Version metadata", "ApiVersion")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts",
                summary: "List all named browser contexts",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Context list", "ContextSummary[]")],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts",
                summary: "Create a new isolated browser context",
                params: vec![],
                query: vec![],
                request_body: Some(json_request("CreateContextBody")),
                responses: vec![
                    json_response(201, "Created context summary", "ContextSummary"),
                    json_response(400, "Invalid create-context request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}",
                summary: "Read one browser context summary",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![
                    json_response(200, "Context summary", "ContextSummary"),
                    json_response(404, "Context not found", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "DELETE",
                path: "/contexts/{context_id}",
                summary: "Delete one non-default browser context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![
                    empty_response(204, "Context deleted"),
                    json_response(400, "Default context cannot be deleted", "ApiErrorBody"),
                    json_response(404, "Context not found", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/healthz",
                summary: "Context-scoped health",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![
                    json_response(200, "Context health snapshot", "HealthResponse"),
                    json_response(404, "Context not found", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/readyz",
                summary: "Context-scoped readiness",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![
                    json_response(
                        200,
                        "Context readiness snapshot",
                        "RuntimeDiagnosticsResponse",
                    ),
                    json_response(503, "Context is not ready", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/doctor",
                summary: "Context-scoped diagnostics",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Context diagnostics",
                    "RuntimeDiagnosticsResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/runtime",
                summary: "Context runtime config and live state",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Context runtime metadata",
                    "RuntimeInfo",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/runtime/cancel",
                summary: "Cancel the active operation in one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Context diagnostics after cancellation request",
                    "RuntimeDiagnosticsResponse",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/browsers/{browser_id}/activate",
                summary: "Activate one attached browser inside a context",
                params: vec![
                    path_param("context_id", "string", "Named browser context id"),
                    path_param("browser_id", "i32", "Attached browser id to activate"),
                ],
                query: vec![],
                request_body: None,
                responses: vec![
                    json_response(
                        200,
                        "Context diagnostics after activating a browser",
                        "RuntimeDiagnosticsResponse",
                    ),
                    json_response(404, "Context not found", "ApiErrorBody"),
                    json_response(
                        502,
                        "Activation failed in the native bridge",
                        "ApiErrorBody",
                    ),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/session",
                summary: "Snapshot one context session",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Serialized session state",
                    "SessionState",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/session",
                summary: "Inject session state into one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request("SessionState")),
                responses: vec![
                    empty_response(204, "Session injected"),
                    json_response(400, "Session payload is invalid", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/session/save",
                summary: "Persist one context profile session",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Saved session profile metadata",
                    "SessionProfileInfo",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/session/load",
                summary: "Load one context profile session",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Loaded session profile metadata",
                    "SessionProfileInfo",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/navigate",
                summary: "Navigate one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "NavigateBody",
                    navigate_body_example(),
                )),
                responses: vec![
                    json_response(200, "Emitted navigation events", "SequencedEvent[]"),
                    json_response(400, "Invalid navigation request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/search",
                summary: "Run a first-class web search in one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "SearchBody",
                    search_body_example(),
                )),
                responses: vec![
                    json_response(200, "Search navigation metadata and events", "SearchResponse"),
                    json_response(400, "Invalid search request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/execute",
                summary: "Execute commands in one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "ExecuteBody",
                    execute_body_example(),
                )),
                responses: vec![
                    json_response(200, "Execution report", "ExecutionReport"),
                    json_response(400, "Invalid command batch", "ApiErrorBody"),
                    json_response(502, "Runtime command execution failed", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/page",
                summary: "Read a structured page research snapshot for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Page research snapshot", "PageResearchData")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/page/text",
                summary: "Read visible page text for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Page text snapshot", "PageTextResponse")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/page/markdown",
                summary: "Read a markdown projection of the current page for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Page markdown projection",
                    "PageMarkdownResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/page/links",
                summary: "List page links for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Page link inventory", "PageLinksResponse")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/page/headings",
                summary: "List page headings for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Page heading inventory",
                    "PageHeadingsResponse",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/page/find",
                summary: "Find text, headings, or links on the current page for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "PageFindBody",
                    page_find_body_example(),
                )),
                responses: vec![
                    json_response(200, "Page find results", "PageFindResponse"),
                    json_response(400, "Invalid page-find request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/page/open-link",
                summary: "Open a page link by text match for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "PageOpenLinkBody",
                    page_open_link_body_example(),
                )),
                responses: vec![
                    json_response(200, "Chosen link navigation result", "PageOpenLinkResponse"),
                    json_response(400, "Invalid or ambiguous link-open request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/dom",
                summary: "Fetch one context DOM snapshot",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "DOM snapshot", "DomSnapshot")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/events",
                summary: "Read buffered events for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: event_query_schema(),
                request_body: None,
                responses: vec![
                    json_response(200, "Buffered event window", "EventReadWindow"),
                    json_response(400, "Invalid event filter query", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/downloads",
                summary: "List browser-triggered downloads for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Download state list",
                    "DownloadsResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/contexts/{context_id}/events/live",
                summary: "Stream events for one context over SSE",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: event_stream_query_schema(),
                request_body: None,
                responses: vec![
                    sse_response(
                        200,
                        "Server-sent event stream carrying EventReadWindow payloads",
                    ),
                    json_response(400, "Invalid event stream query", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/contexts/{context_id}/trace/enable",
                summary: "Enable trace recording for one context",
                params: vec![path_param(
                    "context_id",
                    "string",
                    "Named browser context id",
                )],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "TraceBody",
                    trace_body_example(),
                )),
                responses: vec![
                    empty_response(204, "Trace recording enabled"),
                    json_response(400, "Invalid trace path request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/healthz",
                summary: "Control-plane and runtime health",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Runtime health snapshot",
                    "HealthResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/readyz",
                summary: "Command readiness gate",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![
                    json_response(
                        200,
                        "Runtime readiness snapshot",
                        "RuntimeDiagnosticsResponse",
                    ),
                    json_response(503, "Runtime is not ready", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/doctor",
                summary: "Detailed runtime diagnostics",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Detailed runtime diagnostics",
                    "RuntimeDiagnosticsResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/runtime",
                summary: "Runtime config and live state",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Runtime metadata and diagnostics",
                    "RuntimeInfo",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/runtime/cancel",
                summary: "Cancel the active runtime operation",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Runtime diagnostics after cancellation request",
                    "RuntimeDiagnosticsResponse",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/browsers/{browser_id}/activate",
                summary: "Activate one attached browser in the default context",
                params: vec![path_param(
                    "browser_id",
                    "i32",
                    "Attached browser id to activate",
                )],
                query: vec![],
                request_body: None,
                responses: vec![
                    json_response(
                        200,
                        "Runtime diagnostics after activating a browser",
                        "RuntimeDiagnosticsResponse",
                    ),
                    json_response(
                        502,
                        "Activation failed in the native bridge",
                        "ApiErrorBody",
                    ),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/navigate",
                summary: "Navigate the active browser page",
                params: vec![],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "NavigateBody",
                    navigate_body_example(),
                )),
                responses: vec![
                    json_response(200, "Emitted navigation events", "SequencedEvent[]"),
                    json_response(400, "Invalid navigation request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/search",
                summary: "Run a first-class web search in the active browser",
                params: vec![],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "SearchBody",
                    search_body_example(),
                )),
                responses: vec![
                    json_response(200, "Search navigation metadata and events", "SearchResponse"),
                    json_response(400, "Invalid search request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/execute",
                summary: "Execute a command batch including file upload and media diagnostics",
                params: vec![],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "ExecuteBody",
                    execute_body_example(),
                )),
                responses: vec![
                    json_response(200, "Execution report", "ExecutionReport"),
                    json_response(400, "Invalid command batch", "ApiErrorBody"),
                    json_response(502, "Runtime command execution failed", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/page",
                summary: "Read a structured page research snapshot",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Page research snapshot", "PageResearchData")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/page/text",
                summary: "Read visible page text",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Page text snapshot", "PageTextResponse")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/page/markdown",
                summary: "Read a markdown projection of the current page",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Page markdown projection",
                    "PageMarkdownResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/page/links",
                summary: "List page links",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "Page link inventory", "PageLinksResponse")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/page/headings",
                summary: "List page headings",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Page heading inventory",
                    "PageHeadingsResponse",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/page/find",
                summary: "Find text, headings, or links on the current page",
                params: vec![],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "PageFindBody",
                    page_find_body_example(),
                )),
                responses: vec![
                    json_response(200, "Page find results", "PageFindResponse"),
                    json_response(400, "Invalid page-find request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/page/open-link",
                summary: "Open a page link by text match",
                params: vec![],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "PageOpenLinkBody",
                    page_open_link_body_example(),
                )),
                responses: vec![
                    json_response(200, "Chosen link navigation result", "PageOpenLinkResponse"),
                    json_response(400, "Invalid or ambiguous link-open request", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/dom",
                summary: "Fetch a fresh DOM snapshot",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(200, "DOM snapshot", "DomSnapshot")],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/events",
                summary: "Read buffered runtime events",
                params: vec![],
                query: event_query_schema(),
                request_body: None,
                responses: vec![
                    json_response(200, "Buffered event window", "EventReadWindow"),
                    json_response(400, "Invalid event filter query", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/downloads",
                summary: "List browser-triggered downloads for the active context",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Download state list",
                    "DownloadsResponse",
                )],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/events/live",
                summary: "Stream runtime events over SSE",
                params: vec![],
                query: event_stream_query_schema(),
                request_body: None,
                responses: vec![
                    sse_response(
                        200,
                        "Server-sent event stream carrying EventReadWindow payloads",
                    ),
                    json_response(400, "Invalid event stream query", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "GET",
                path: "/session",
                summary: "Snapshot the active session",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Serialized session state",
                    "SessionState",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/session",
                summary: "Inject session state",
                params: vec![],
                query: vec![],
                request_body: Some(json_request("SessionState")),
                responses: vec![
                    empty_response(204, "Session injected"),
                    json_response(400, "Session payload is invalid", "ApiErrorBody"),
                ],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/session/save",
                summary: "Persist the active profile session",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Saved session profile metadata",
                    "SessionProfileInfo",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/session/load",
                summary: "Load the active profile session",
                params: vec![],
                query: vec![],
                request_body: None,
                responses: vec![json_response(
                    200,
                    "Loaded session profile metadata",
                    "SessionProfileInfo",
                )],
            },
            ApiRouteDoc {
                method: "POST",
                path: "/trace/enable",
                summary: "Enable trace recording",
                params: vec![],
                query: vec![],
                request_body: Some(json_request_with_example(
                    "TraceBody",
                    trace_body_example(),
                )),
                responses: vec![
                    empty_response(204, "Trace recording enabled"),
                    json_response(400, "Invalid trace path request", "ApiErrorBody"),
                ],
            },
        ],
        capabilities: vec![
            "semantic_dom_snapshot",
            "event_stream",
            "network_event_capture",
            "first_class_file_upload",
            "first_class_file_download",
            "media_diagnostics",
            "embedded_audio_playback",
            "named_multi_context_control_plane",
            "research_workflow_primitives",
            "session_snapshotting",
            "trace_recording",
        ],
        event_filters: vec![
            "dom_mutation",
            "navigation",
            "network",
            "download",
            "websocket",
            "websocket_open",
            "websocket_handshake",
            "websocket_frame",
            "websocket_close",
            "log",
            "unknown",
        ],
        capability_status: vec![
            ApiCapabilityStatusDoc {
                name: "semantic_dom_snapshot",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/readyz + /dom",
                details: "DOM snapshots are execute-grade when the active runtime is command-ready.",
            },
            ApiCapabilityStatusDoc {
                name: "event_stream",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/readyz + /events",
                details: "Buffered and live events are validated against the active runtime readiness state.",
            },
            ApiCapabilityStatusDoc {
                name: "network_event_capture",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/readyz + /events",
                details: "Network capture depends on the active runtime being command-ready.",
            },
            ApiCapabilityStatusDoc {
                name: "first_class_file_upload",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/readyz + /execute(set_files)",
                details: "File uploads are staged into the Aegis-owned upload area before injection and support hidden file inputs.",
            },
            ApiCapabilityStatusDoc {
                name: "first_class_file_download",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/readyz + /downloads",
                details: "Browser-triggered downloads are saved into the configured download directory and surfaced through runtime diagnostics and events.",
            },
            ApiCapabilityStatusDoc {
                name: "media_diagnostics",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/readyz + /execute(media_state)",
                details: "Media diagnostics are validated when the browser runtime is attached and command-ready. This does not imply codec-complete playback support.",
            },
            ApiCapabilityStatusDoc {
                name: "embedded_audio_playback",
                supported: true,
                status: "experimental",
                runtime_validated: false,
                validated_by: "per-page media_state codec probe",
                details: "Actual audio playback depends on the embedded browser build, codecs, and response semantics. Inspect media_state.source_codec_support and likely_failure_cause before treating playback as production-ready.",
            },
            ApiCapabilityStatusDoc {
                name: "named_multi_context_control_plane",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/contexts + /contexts/{context_id}/readyz",
                details: "Named contexts are available when the default runtime is healthy.",
            },
            ApiCapabilityStatusDoc {
                name: "research_workflow_primitives",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/search + /page/text + /page/find + /page/links + /page/open-link",
                details: "First-class search, page text extraction, page find, and link opening are exposed without requiring direct /execute usage.",
            },
            ApiCapabilityStatusDoc {
                name: "session_snapshotting",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/session + /session/save + /session/load",
                details: "Session snapshot and restore are validated against the active runtime surface.",
            },
            ApiCapabilityStatusDoc {
                name: "trace_recording",
                supported: true,
                status: if runtime_validated {
                    "validated"
                } else {
                    "supported"
                },
                runtime_validated,
                validated_by: "/trace/enable",
                details: "Trace recording is available when the runtime reaches command-ready state.",
            },
        ],
        commands: vec![
            ApiCommandDoc {
                name: "click",
                summary: "Click one actionable node",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "hover",
                summary: "Hover one node",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "set_value",
                summary: "Set the value of one form control",
                targeting: "id or match",
                fields: vec![ApiSchemaFieldDoc {
                    name: "value",
                    kind: "string",
                    required: true,
                    description: "Text value to apply",
                }],
            },
            ApiCommandDoc {
                name: "set_files",
                summary: "Attach one or more local files to a file input",
                targeting: "id or match",
                fields: vec![ApiSchemaFieldDoc {
                    name: "paths",
                    kind: "string[]",
                    required: true,
                    description: "Absolute local file paths to stage into Aegis and attach",
                }],
            },
            ApiCommandDoc {
                name: "press_key",
                summary: "Dispatch keyboard interaction",
                targeting: "optional id or match",
                fields: vec![ApiSchemaFieldDoc {
                    name: "key",
                    kind: "string",
                    required: true,
                    description: "Key value such as Enter or Space",
                }],
            },
            ApiCommandDoc {
                name: "wait_for",
                summary: "Wait for URL, DOM, text, scroll, or media conditions",
                targeting: "optional id or match",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "timeout_ms",
                        kind: "u64",
                        required: false,
                        description: "Overall wait deadline in milliseconds",
                    },
                    ApiSchemaFieldDoc {
                        name: "selector",
                        kind: "string",
                        required: false,
                        description: "CSS selector that must exist",
                    },
                ],
            },
            ApiCommandDoc {
                name: "scroll",
                summary: "Scroll the page viewport",
                targeting: "none",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "drag",
                summary: "Perform pointer drag gestures, including range controls",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "geometry",
                summary: "Read geometry for a matched node",
                targeting: "id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "media_state",
                summary: "Read diagnostics for all media nodes or one matched media node",
                targeting: "optional id or match",
                fields: vec![],
            },
            ApiCommandDoc {
                name: "eval",
                summary: "Execute JavaScript and return the result",
                targeting: "none",
                fields: vec![],
            },
        ],
        schemas: vec![
            ApiNamedSchemaDoc {
                name: "NavigateBody",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "url",
                    kind: "string",
                    required: true,
                    description: "Absolute URL to load in the active browser",
                }],
            },
            ApiNamedSchemaDoc {
                name: "ExecuteBody",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "commands",
                    kind: "Command[]",
                    required: true,
                    description: "Command batch executed in-order against the active runtime",
                }],
            },
            ApiNamedSchemaDoc {
                name: "SearchBody",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "query",
                        kind: "string",
                        required: true,
                        description: "Search query text to submit through the active browser",
                    },
                    ApiSchemaFieldDoc {
                        name: "engine",
                        kind: "string",
                        required: false,
                        description: "Optional search engine name such as duckduckgo, google, or bing",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "PageFindBody",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "text",
                        kind: "string",
                        required: true,
                        description: "Query text to find in visible page text, headings, or links",
                    },
                    ApiSchemaFieldDoc {
                        name: "exact",
                        kind: "bool",
                        required: false,
                        description: "Require exact field equality for headings and links",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "PageOpenLinkBody",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "text",
                        kind: "string",
                        required: true,
                        description: "Link text to match against the current page",
                    },
                    ApiSchemaFieldDoc {
                        name: "exact",
                        kind: "bool",
                        required: false,
                        description: "Require exact normalized link-text equality",
                    },
                    ApiSchemaFieldDoc {
                        name: "href_contains",
                        kind: "string",
                        required: false,
                        description: "Optional href substring filter applied before disambiguation",
                    },
                    ApiSchemaFieldDoc {
                        name: "index",
                        kind: "usize",
                        required: false,
                        description: "Optional zero-based candidate index when multiple links match",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "CreateContextBody",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "id",
                        kind: "string",
                        required: false,
                        description: "Explicit context id; defaults to a generated context-N name",
                    },
                    ApiSchemaFieldDoc {
                        name: "profile",
                        kind: "string",
                        required: false,
                        description: "Persistent session profile backing this context",
                    },
                    ApiSchemaFieldDoc {
                        name: "seed_from_context",
                        kind: "string",
                        required: false,
                        description: "Optional existing context id to clone session state from",
                    },
                    ApiSchemaFieldDoc {
                        name: "mode",
                        kind: "BrowserMode",
                        required: false,
                        description: "headless or headful browser mode override",
                    },
                    ApiSchemaFieldDoc {
                        name: "start_url",
                        kind: "string",
                        required: false,
                        description: "Initial page URL for the new context",
                    },
                    ApiSchemaFieldDoc {
                        name: "download_dir",
                        kind: "path",
                        required: false,
                        description: "Optional browser download directory override",
                    },
                    ApiSchemaFieldDoc {
                        name: "upload_dir",
                        kind: "path",
                        required: false,
                        description: "Optional browser upload staging directory override",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "TraceBody",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "path",
                    kind: "path",
                    required: true,
                    description: "Destination trace file path on the local machine",
                }],
            },
            ApiNamedSchemaDoc {
                name: "SearchResponse",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "engine",
                        kind: "string",
                        required: true,
                        description: "Resolved search engine used for navigation",
                    },
                    ApiSchemaFieldDoc {
                        name: "query",
                        kind: "string",
                        required: true,
                        description: "Original caller-provided search query",
                    },
                    ApiSchemaFieldDoc {
                        name: "url",
                        kind: "string",
                        required: true,
                        description: "Final search URL loaded into the browser",
                    },
                    ApiSchemaFieldDoc {
                        name: "events",
                        kind: "SequencedEvent[]",
                        required: true,
                        description: "Navigation events emitted by the search navigation",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "PageResearchData",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "title",
                        kind: "string|null",
                        required: true,
                        description: "Current document title",
                    },
                    ApiSchemaFieldDoc {
                        name: "url",
                        kind: "string|null",
                        required: true,
                        description: "Current page URL",
                    },
                    ApiSchemaFieldDoc {
                        name: "canonical_url",
                        kind: "string|null",
                        required: true,
                        description: "Canonical link target when present",
                    },
                    ApiSchemaFieldDoc {
                        name: "visible_text",
                        kind: "string",
                        required: true,
                        description: "Normalized visible page text for research workflows",
                    },
                    ApiSchemaFieldDoc {
                        name: "headings",
                        kind: "PageResearchHeading[]",
                        required: true,
                        description: "Heading inventory extracted from the current page",
                    },
                    ApiSchemaFieldDoc {
                        name: "links",
                        kind: "PageResearchLink[]",
                        required: true,
                        description: "Link inventory extracted from the current page",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "PageTextResponse",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "text",
                    kind: "string",
                    required: true,
                    description: "Normalized visible page text",
                }],
            },
            ApiNamedSchemaDoc {
                name: "PageMarkdownResponse",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "markdown",
                    kind: "string",
                    required: true,
                    description: "Markdown projection of the current page research snapshot",
                }],
            },
            ApiNamedSchemaDoc {
                name: "PageLinksResponse",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "links",
                    kind: "PageResearchLink[]",
                    required: true,
                    description: "Page links captured from the current page",
                }],
            },
            ApiNamedSchemaDoc {
                name: "PageHeadingsResponse",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "headings",
                    kind: "PageResearchHeading[]",
                    required: true,
                    description: "Page headings captured from the current page",
                }],
            },
            ApiNamedSchemaDoc {
                name: "PageFindResponse",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "matches",
                    kind: "PageFindMatch[]",
                    required: true,
                    description: "Matched headings, links, or text snippets",
                }],
            },
            ApiNamedSchemaDoc {
                name: "PageOpenLinkResponse",
                kind: "object",
                fields: vec![ApiSchemaFieldDoc {
                    name: "chosen",
                    kind: "PageResearchLink",
                    required: true,
                    description: "Chosen link opened by the route",
                }],
            },
            ApiNamedSchemaDoc {
                name: "EventReadWindow",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "requested_since",
                        kind: "u64",
                        required: true,
                        description: "Original caller-provided cursor sequence",
                    },
                    ApiSchemaFieldDoc {
                        name: "oldest_available_sequence",
                        kind: "u64|null",
                        required: true,
                        description: "Oldest retained event sequence still available in the runtime buffer",
                    },
                    ApiSchemaFieldDoc {
                        name: "latest_sequence",
                        kind: "u64",
                        required: true,
                        description: "Latest retained event sequence at read time",
                    },
                    ApiSchemaFieldDoc {
                        name: "gap_detected",
                        kind: "bool",
                        required: true,
                        description: "True when the requested sequence is older than the retained event window",
                    },
                    ApiSchemaFieldDoc {
                        name: "events",
                        kind: "SequencedEvent[]",
                        required: true,
                        description: "Buffered events newer than the requested cursor and matching any route filter",
                    },
                ],
            },
            ApiNamedSchemaDoc {
                name: "SequencedEvent",
                kind: "object",
                fields: vec![
                    ApiSchemaFieldDoc {
                        name: "sequence",
                        kind: "u64",
                        required: true,
                        description: "Monotonic runtime event cursor",
                    },
                    ApiSchemaFieldDoc {
                        name: "timestamp_ms",
                        kind: "u64",
                        required: true,
                        description: "Runtime-side event timestamp in milliseconds since the Unix epoch",
                    },
                    ApiSchemaFieldDoc {
                        name: "event",
                        kind: "RuntimeEvent",
                        required: true,
                        description: "Tagged event payload including navigation, network, websocket, download, log, or unknown events",
                    },
                ],
            },
        ],
        error_schema: ApiNamedSchemaDoc {
            name: "ApiErrorBody",
            kind: "object",
            fields: vec![
                ApiSchemaFieldDoc {
                    name: "request_id",
                    kind: "string",
                    required: true,
                    description: "Server-generated correlation id for this failed request",
                },
                ApiSchemaFieldDoc {
                    name: "error",
                    kind: "string",
                    required: true,
                    description: "Operator-facing failure summary",
                },
                ApiSchemaFieldDoc {
                    name: "code",
                    kind: "string",
                    required: true,
                    description: "Stable machine-readable error code",
                },
                ApiSchemaFieldDoc {
                    name: "operation",
                    kind: "string|null",
                    required: true,
                    description: "Optional high-level runtime operation name associated with the failure",
                },
                ApiSchemaFieldDoc {
                    name: "stage",
                    kind: "string|null",
                    required: true,
                    description: "Optional internal stage where the failure occurred",
                },
                ApiSchemaFieldDoc {
                    name: "elapsed_ms",
                    kind: "u64|null",
                    required: true,
                    description: "Optional elapsed time for the failed operation",
                },
                ApiSchemaFieldDoc {
                    name: "timed_out",
                    kind: "bool",
                    required: true,
                    description: "True when the failure represents a timeout",
                },
                ApiSchemaFieldDoc {
                    name: "restart_recommended",
                    kind: "bool",
                    required: true,
                    description: "True when the runtime should be restarted before retrying",
                },
                ApiSchemaFieldDoc {
                    name: "retriable",
                    kind: "bool",
                    required: true,
                    description: "True when retrying the same request may succeed after recovery",
                },
                ApiSchemaFieldDoc {
                    name: "suggested_action",
                    kind: "string|null",
                    required: true,
                    description: "Operator guidance for the next recovery step",
                },
            ],
        },
    }
}

#[derive(Debug, Serialize)]
struct ApiVersion {
    version: &'static str,
    protocol_version: u16,
    runtime_identity: RuntimeIdentity,
}

async fn api_version() -> Json<ApiVersion> {
    Json(ApiVersion {
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        runtime_identity: runtime_identity(),
    })
}

async fn list_contexts(
    State(root): State<ServeRootState>,
) -> Result<Json<Vec<ContextSummary>>, ApiError> {
    Ok(Json(root.list_contexts()?))
}

async fn get_context(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<ContextSummary>, ApiError> {
    let contexts = root.list_contexts()?;
    contexts
        .into_iter()
        .find(|context| context.id == context_id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("context `{context_id}` was not found")))
}

async fn create_context(
    State(root): State<ServeRootState>,
    Json(body): Json<CreateContextBody>,
) -> Result<(StatusCode, Json<ContextSummary>), ApiError> {
    let CreateContextBody {
        id,
        profile,
        seed_from_context,
        mode,
        start_url,
        download_dir,
        upload_dir,
    } = body;
    let existing = root.list_contexts()?;
    let next_index = existing.len() + 1;
    let context_id = id
        .clone()
        .or_else(|| profile.clone())
        .unwrap_or_else(|| format!("context-{next_index}"));
    validate_context_name(&context_id)?;
    if root.context_state(&context_id).is_some() {
        return Err(ApiError::from(AegisError::Bridge(format!(
            "context `{context_id}` already exists"
        ))));
    }
    let profile = profile.unwrap_or_else(|| context_id.clone());
    let mut browser = root.browser.clone();
    if let Some(mode) = mode {
        browser.mode = mode;
    }
    if start_url.is_some() {
        browser.start_url = start_url;
    }
    if download_dir.is_some() {
        browser.download_dir = download_dir;
    }
    if upload_dir.is_some() {
        browser.upload_dir = upload_dir;
    }
    let (reply_tx, reply_rx) = oneshot::channel();
    let summary = manage_context(
        &root,
        ManagerCommand::CreateContext {
            context_id: context_id.clone(),
            profile,
            browser,
            seed_from_context,
            reply: reply_tx,
        },
        reply_rx,
        "create_context",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn delete_context(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if context_id == root.default_context_id {
        return Err(ApiError::from(AegisError::Bridge(
            "default context cannot be deleted".into(),
        )));
    }
    if root.context_state(&context_id).is_none() {
        return Err(ApiError::not_found(format!(
            "context `{context_id}` was not found"
        )));
    }
    let (reply_tx, reply_rx) = oneshot::channel();
    manage_context(
        &root,
        ManagerCommand::DeleteContext {
            context_id,
            reply: reply_tx,
        },
        reply_rx,
        "delete_context",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn context_health(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<HealthResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    Ok(Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION"),
        protocol_version: PROTOCOL_VERSION,
        control_plane_up: true,
        runtime_state: diagnostics.state.clone(),
        command_ready: diagnostics.command_ready,
        bridge_healthy: diagnostics.bridge_healthy,
        browser_backend_healthy: diagnostics.browser_backend_healthy,
        browser_process_up: diagnostics.browser_process_up,
        page_attached: diagnostics.page_attached,
        renderer_attached: diagnostics.renderer_attached,
        dom_snapshot_available: diagnostics.dom_snapshot_available,
        event_decoder_ok: diagnostics.event_decoder_ok,
        active_operation: diagnostics.active_operation,
        last_failure: diagnostics.last_failure,
    }))
}

async fn runtime_info(State(root): State<ServeRootState>) -> Result<Json<RuntimeInfo>, ApiError> {
    let state = require_default_context_state(&root)?;
    Ok(Json(RuntimeInfo {
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        diagnostics: read_diagnostics(&state.diagnostics),
        profile: state.profile.clone(),
        runtime_identity: runtime_identity(),
        startup: state
            .startup
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            }),
    }))
}

async fn readiness(
    State(root): State<ServeRootState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    let inspectable_gate_passed = !diagnostics.document_loaded
        || (!diagnostics.module_scripts_present && !diagnostics.synthetic_shell_active)
        || (diagnostics.inspectable_dom_ready && !diagnostics.synthetic_shell_active);
    if diagnostics.command_ready && inspectable_gate_passed {
        Ok(Json(diagnostics))
    } else {
        Err(ApiError::readiness(diagnostics))
    }
}

async fn doctor(
    State(root): State<ServeRootState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn cancel_runtime_operation(
    State(root): State<ServeRootState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    request_runtime_cancel(&state);
    mark_operation_cancel_requested(&state.diagnostics);
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn activate_browser(
    State(root): State<ServeRootState>,
    Path(browser_id): Path<i32>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::ActivateBrowser(browser_id, reply_tx))
        .map_err(channel_error)?;
    await_command("activate_browser", &state, reply_rx).await??;
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn save_session_profile(
    State(root): State<ServeRootState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("save_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn load_session_profile(
    State(root): State<ServeRootState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("load_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn inject_session(
    State(root): State<ServeRootState>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .map_err(channel_error)?;
    await_command("inject_session", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn snapshot_session(
    State(root): State<ServeRootState>,
) -> Result<Json<SessionState>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_session", &state, reply_rx).await??,
    ))
}

async fn snapshot_page_research(state: &ApiState) -> Result<PageResearchData, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::PageResearch(reply_tx))
        .map_err(channel_error)?;
    Ok(await_command("page_research", state, reply_rx).await??)
}

async fn open_matching_link(
    state: &ApiState,
    body: PageOpenLinkBody,
) -> Result<Json<PageOpenLinkResponse>, ApiError> {
    let page = snapshot_page_research(state).await?;
    let candidates = matching_links(&page, &body.text, body.exact, body.href_contains.as_deref());
    if candidates.is_empty() {
        return Err(ApiError::bad_request(
            "link_not_found",
            format!("no link matched `{}` on the current page", body.text),
            Some(String::from(
                "call `GET /page/links` or `POST /page/find` first, then retry with a more specific link label",
            )),
        ));
    }
    let chosen = if let Some(index) = body.index {
        candidates.get(index).cloned().ok_or_else(|| {
            ApiError::bad_request(
                "link_index_out_of_range",
                format!(
                    "link candidate index {index} is out of range for {} matches",
                    candidates.len()
                ),
                Some(String::from(
                    "use `GET /page/links` to inspect candidate ordering before retrying",
                )),
            )
        })?
    } else if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        return Err(ApiError::bad_request(
            "ambiguous_link_match",
            format!(
                "link text `{}` matched {} links; provide `index` or a narrower `href_contains` filter",
                body.text,
                candidates.len()
            ),
            Some(String::from(
                "call `GET /page/links` to inspect candidates, then retry with `index`, `exact`, or `href_contains`",
            )),
        ));
    };
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(chosen.href.clone(), reply_tx))
        .map_err(channel_error)?;
    let events = await_command("page_open_link", state, reply_rx).await??;
    Ok(Json(PageOpenLinkResponse {
        text: body.text,
        exact: body.exact,
        href_contains: body.href_contains,
        candidate_count: candidates.len(),
        chosen,
        events,
    }))
}

fn invalid_json_request(
    operation: &str,
    example: serde_json::Value,
    rejection: JsonRejection,
) -> ApiError {
    let example_json =
        serde_json::to_string_pretty(&example).unwrap_or_else(|_| example.to_string());
    ApiError::bad_request(
        "invalid_json_body",
        format!("invalid JSON body for `{operation}`: {}", rejection.body_text()),
        Some(format!(
            "send `content-type: application/json` with a body like:\n{example_json}"
        )),
    )
}

async fn navigate(
    State(root): State<ServeRootState>,
    body: Result<Json<NavigateBody>, JsonRejection>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let Json(body) = body.map_err(|rejection| {
        invalid_json_request("navigate", navigate_body_example(), rejection)
    })?;
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("navigate", &state, reply_rx).await??))
}

async fn search(
    State(root): State<ServeRootState>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> Result<Json<SearchResponse>, ApiError> {
    let Json(body) =
        body.map_err(|rejection| invalid_json_request("search", search_body_example(), rejection))?;
    let state = require_default_context_state(&root)?;
    let query = body.query.trim().to_string();
    if query.is_empty() {
        return Err(ApiError::bad_request(
            "invalid_search_query",
            String::from("search query must not be empty"),
            Some(String::from("send a non-empty `query` string")),
        ));
    }
    let url = build_search_url(&query, body.engine.as_deref());
    let engine = search_engine_name(body.engine.as_deref()).to_string();
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(url.clone(), reply_tx))
        .map_err(channel_error)?;
    let events = await_command("search", &state, reply_rx).await??;
    Ok(Json(SearchResponse {
        engine,
        query,
        url,
        events,
    }))
}

async fn execute(
    State(root): State<ServeRootState>,
    body: Result<Json<ExecuteBody>, JsonRejection>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let Json(body) = body.map_err(|rejection| {
        invalid_json_request("execute", execute_body_example(), rejection)
    })?;
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("execute", &state, reply_rx).await??))
}

async fn page_research(
    State(root): State<ServeRootState>,
) -> Result<Json<PageResearchData>, ApiError> {
    let state = require_default_context_state(&root)?;
    Ok(Json(snapshot_page_research(&state).await?))
}

async fn page_text(
    State(root): State<ServeRootState>,
) -> Result<Json<PageTextResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageTextResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        text: page.visible_text.clone(),
        likely_not_found: page.likely_not_found,
        not_found_signals: page.not_found_signals.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_markdown(
    State(root): State<ServeRootState>,
) -> Result<Json<PageMarkdownResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageMarkdownResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        markdown: render_page_markdown(&page),
        likely_not_found: page.likely_not_found,
        not_found_signals: page.not_found_signals.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_links(
    State(root): State<ServeRootState>,
) -> Result<Json<PageLinksResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageLinksResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        links: page.links.clone(),
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_headings(
    State(root): State<ServeRootState>,
) -> Result<Json<PageHeadingsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageHeadingsResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        headings: page.headings.clone(),
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_find(
    State(root): State<ServeRootState>,
    body: Result<Json<PageFindBody>, JsonRejection>,
) -> Result<Json<PageFindResponse>, ApiError> {
    let Json(body) =
        body.map_err(|rejection| invalid_json_request("page_find", page_find_body_example(), rejection))?;
    let state = require_default_context_state(&root)?;
    let page = snapshot_page_research(&state).await?;
    let matches = page_find_matches(&page, &body.text, body.exact);
    Ok(Json(PageFindResponse {
        query: body.text,
        exact: body.exact,
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        match_count: matches.len(),
        matches,
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_open_link(
    State(root): State<ServeRootState>,
    body: Result<Json<PageOpenLinkBody>, JsonRejection>,
) -> Result<Json<PageOpenLinkResponse>, ApiError> {
    let Json(body) = body.map_err(|rejection| {
        invalid_json_request("page_open_link", page_open_link_body_example(), rejection)
    })?;
    let state = require_default_context_state(&root)?;
    open_matching_link(&state, body).await
}

async fn snapshot_dom(State(root): State<ServeRootState>) -> Result<Json<DomSnapshot>, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_dom", &state, reply_rx).await??,
    ))
}

async fn events(
    State(root): State<ServeRootState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventReadWindow>, ApiError> {
    let state = require_default_context_state(&root)?;
    let filter = EventFilter::from_query(&query.event_types)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.since, reply_tx))
        .map_err(channel_error)?;
    let window = await_command("events", &state, reply_rx).await??;
    Ok(Json(filter_event_window(window, filter.as_ref())))
}

async fn downloads(
    State(root): State<ServeRootState>,
) -> Result<Json<DownloadsResponse>, ApiError> {
    let state = require_default_context_state(&root)?;
    Ok(Json(downloads_response(&state)))
}

async fn events_live(
    State(root): State<ServeRootState>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let state = require_default_context_state(&root)?;
    let filter = EventFilter::from_query(&query.event_types)?;
    let poll_ms = query
        .poll_ms
        .unwrap_or(DEFAULT_EVENT_STREAM_POLL_INTERVAL.as_millis() as u64)
        .clamp(
            MIN_EVENT_STREAM_POLL_INTERVAL_MS,
            MAX_EVENT_STREAM_POLL_INTERVAL_MS,
        );
    let poll_interval = Duration::from_millis(poll_ms);
    let mut since = query.since;
    let state = state.clone();

    Ok(Sse::new(stream! {
        yield Ok(Event::default().event("ready").json_data(json!({
            "since": since,
            "poll_ms": poll_ms,
        })).unwrap_or_else(|_| Event::default().event("ready").data("ready")));

        loop {
            let (reply_tx, reply_rx) = oneshot::channel();
            if state.tx.send(ApiCommand::Events(since, reply_tx)).is_err() {
                yield Ok(Event::default().event("error").data("runtime event channel closed"));
                break;
            }

            let window = match timeout(COMMAND_TIMEOUT, reply_rx).await {
                Ok(Ok(Ok(window))) => window,
                Ok(Ok(Err(error))) => {
                    yield Ok(Event::default().event("error").data(error.to_string()));
                    break;
                }
                Ok(Err(_)) => {
                    yield Ok(Event::default().event("error").data("runtime event stream cancelled"));
                    break;
                }
                Err(_) => {
                    yield Ok(Event::default().event("error").data("runtime event stream timed out"));
                    break;
                }
            };

            since = window.latest_sequence;
            let filtered = filter_event_window(window, filter.as_ref());
            if !filtered.events.is_empty() || filtered.gap_detected {
                yield Ok(Event::default().event("runtime_events").json_data(&filtered).unwrap_or_else(|_| {
                    Event::default().event("runtime_events").data("serialization_error")
                }));
            }

            sleep(poll_interval).await;
        }
    })
    .keep_alive(KeepAlive::default().interval(Duration::from_secs(15)).text("keep-alive")))
}
async fn enable_trace(
    State(root): State<ServeRootState>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let state = require_default_context_state(&root)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(body.path, reply_tx))
        .map_err(channel_error)?;
    await_command("enable_trace", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn context_runtime_info(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeInfo>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    Ok(Json(RuntimeInfo {
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        diagnostics: read_diagnostics(&state.diagnostics),
        profile: state.profile.clone(),
        runtime_identity: runtime_identity(),
        startup: state
            .startup
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            }),
    }))
}

async fn context_readiness(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let diagnostics = read_diagnostics(&state.diagnostics);
    let inspectable_gate_passed = !diagnostics.document_loaded
        || (!diagnostics.module_scripts_present && !diagnostics.synthetic_shell_active)
        || (diagnostics.inspectable_dom_ready && !diagnostics.synthetic_shell_active);
    if diagnostics.command_ready && inspectable_gate_passed {
        Ok(Json(diagnostics))
    } else {
        Err(ApiError::readiness(diagnostics))
    }
}

async fn context_doctor(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn context_cancel_runtime_operation(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    request_runtime_cancel(&state);
    mark_operation_cancel_requested(&state.diagnostics);
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn context_activate_browser(
    State(root): State<ServeRootState>,
    Path((context_id, browser_id)): Path<(String, i32)>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::ActivateBrowser(browser_id, reply_tx))
        .map_err(channel_error)?;
    await_command("activate_browser", &state, reply_rx).await??;
    Ok(Json(read_diagnostics(&state.diagnostics)))
}

async fn context_save_session_profile(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("save_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn context_load_session_profile(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("load_session_profile", &state, reply_rx).await??;
    Ok(Json(profile))
}

async fn context_inject_session(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .map_err(channel_error)?;
    await_command("inject_session", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn context_snapshot_session(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<SessionState>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_session", &state, reply_rx).await??,
    ))
}

async fn context_navigate(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    body: Result<Json<NavigateBody>, JsonRejection>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let Json(body) = body.map_err(|rejection| {
        invalid_json_request("navigate", navigate_body_example(), rejection)
    })?;
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("navigate", &state, reply_rx).await??))
}

async fn context_search(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> Result<Json<SearchResponse>, ApiError> {
    let Json(body) =
        body.map_err(|rejection| invalid_json_request("search", search_body_example(), rejection))?;
    let state = require_context_state(&root, &context_id)?;
    let query = body.query.trim().to_string();
    if query.is_empty() {
        return Err(ApiError::bad_request(
            "invalid_search_query",
            String::from("search query must not be empty"),
            Some(String::from("send a non-empty `query` string")),
        ));
    }
    let url = build_search_url(&query, body.engine.as_deref());
    let engine = search_engine_name(body.engine.as_deref()).to_string();
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(url.clone(), reply_tx))
        .map_err(channel_error)?;
    let events = await_command("search", &state, reply_rx).await??;
    Ok(Json(SearchResponse {
        engine,
        query,
        url,
        events,
    }))
}

async fn context_execute(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    body: Result<Json<ExecuteBody>, JsonRejection>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let Json(body) = body.map_err(|rejection| {
        invalid_json_request("execute", execute_body_example(), rejection)
    })?;
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(await_command("execute", &state, reply_rx).await??))
}

async fn context_page_research(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<PageResearchData>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    Ok(Json(snapshot_page_research(&state).await?))
}

async fn context_page_text(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<PageTextResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageTextResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        text: page.visible_text.clone(),
        likely_not_found: page.likely_not_found,
        not_found_signals: page.not_found_signals.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn context_page_markdown(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<PageMarkdownResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageMarkdownResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        markdown: render_page_markdown(&page),
        likely_not_found: page.likely_not_found,
        not_found_signals: page.not_found_signals.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn context_page_links(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<PageLinksResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageLinksResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        links: page.links.clone(),
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn context_page_headings(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<PageHeadingsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let page = snapshot_page_research(&state).await?;
    Ok(Json(PageHeadingsResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        headings: page.headings.clone(),
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn context_page_find(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    body: Result<Json<PageFindBody>, JsonRejection>,
) -> Result<Json<PageFindResponse>, ApiError> {
    let Json(body) =
        body.map_err(|rejection| invalid_json_request("page_find", page_find_body_example(), rejection))?;
    let state = require_context_state(&root, &context_id)?;
    let page = snapshot_page_research(&state).await?;
    let matches = page_find_matches(&page, &body.text, body.exact);
    Ok(Json(PageFindResponse {
        query: body.text,
        exact: body.exact,
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        match_count: matches.len(),
        matches,
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn context_page_open_link(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    body: Result<Json<PageOpenLinkBody>, JsonRejection>,
) -> Result<Json<PageOpenLinkResponse>, ApiError> {
    let Json(body) = body.map_err(|rejection| {
        invalid_json_request("page_open_link", page_open_link_body_example(), rejection)
    })?;
    let state = require_context_state(&root, &context_id)?;
    open_matching_link(&state, body).await
}

async fn context_snapshot_dom(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<DomSnapshot>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_dom", &state, reply_rx).await??,
    ))
}

async fn context_events(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventReadWindow>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let filter = EventFilter::from_query(&query.event_types)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.since, reply_tx))
        .map_err(channel_error)?;
    let window = await_command("events", &state, reply_rx).await??;
    Ok(Json(filter_event_window(window, filter.as_ref())))
}

async fn context_downloads(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
) -> Result<Json<DownloadsResponse>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    Ok(Json(downloads_response(&state)))
}

async fn context_events_live(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let filter = EventFilter::from_query(&query.event_types)?;
    let poll_ms = query
        .poll_ms
        .unwrap_or(DEFAULT_EVENT_STREAM_POLL_INTERVAL.as_millis() as u64)
        .clamp(
            MIN_EVENT_STREAM_POLL_INTERVAL_MS,
            MAX_EVENT_STREAM_POLL_INTERVAL_MS,
        );
    let poll_interval = Duration::from_millis(poll_ms);
    let mut since = query.since;
    let state = state.clone();

    Ok(Sse::new(stream! {
        yield Ok(Event::default().event("ready").json_data(json!({
            "since": since,
            "poll_ms": poll_ms,
        })).unwrap_or_else(|_| Event::default().event("ready").data("ready")));

        loop {
            let (reply_tx, reply_rx) = oneshot::channel();
            if state.tx.send(ApiCommand::Events(since, reply_tx)).is_err() {
                yield Ok(Event::default().event("error").data("runtime event channel closed"));
                break;
            }

            let window = match timeout(COMMAND_TIMEOUT, reply_rx).await {
                Ok(Ok(Ok(window))) => window,
                Ok(Ok(Err(error))) => {
                    yield Ok(Event::default().event("error").data(error.to_string()));
                    break;
                }
                Ok(Err(_)) => {
                    yield Ok(Event::default().event("error").data("runtime event stream cancelled"));
                    break;
                }
                Err(_) => {
                    yield Ok(Event::default().event("error").data("runtime event stream timed out"));
                    break;
                }
            };

            since = window.latest_sequence;
            let filtered = filter_event_window(window, filter.as_ref());
            if !filtered.events.is_empty() || filtered.gap_detected {
                yield Ok(Event::default().event("runtime_events").json_data(&filtered).unwrap_or_else(|_| {
                    Event::default().event("runtime_events").data("serialization_error")
                }));
            }

            sleep(poll_interval).await;
        }
    })
    .keep_alive(KeepAlive::default().interval(Duration::from_secs(15)).text("keep-alive")))
}

async fn context_enable_trace(
    State(root): State<ServeRootState>,
    Path(context_id): Path<String>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let state = require_context_state(&root, &context_id)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(body.path, reply_tx))
        .map_err(channel_error)?;
    await_command("enable_trace", &state, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

fn run_main_thread_serve_loop(
    root: &ServeRootState,
    runtimes: &mut HashMap<String, MainThreadContext>,
    manager_rx: mpsc::Receiver<ManagerCommand>,
    server_exit_rx: &mpsc::Receiver<Result<(), AegisError>>,
) -> Result<(), AegisError> {
    loop {
        match server_exit_rx.try_recv() {
            Ok(result) => return result,
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(AegisError::Bridge(
                    "serve HTTP server status channel disconnected".into(),
                ));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        loop {
            match manager_rx.try_recv() {
                Ok(command) => handle_manager_command(root, runtimes, command)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(AegisError::Bridge(
                        "serve manager channel disconnected".into(),
                    ));
                }
            }
        }

        let context_ids = runtimes.keys().cloned().collect::<Vec<_>>();
        let mut stopped = Vec::new();
        for context_id in context_ids {
            let outcome = match runtimes.get_mut(&context_id) {
                Some(runtime) => runtime.tick(),
                None => continue,
            };
            if matches!(outcome, ContextTickOutcome::Stop) {
                stopped.push(context_id);
            }
        }
        for context_id in stopped {
            if let Some(mut runtime) = runtimes.remove(&context_id) {
                runtime.persist_session_best_effort();
            }
        }

        thread::sleep(Duration::from_millis(1));
    }
}

fn handle_manager_command(
    root: &ServeRootState,
    runtimes: &mut HashMap<String, MainThreadContext>,
    command: ManagerCommand,
) -> Result<(), AegisError> {
    match command {
        ManagerCommand::CreateContext {
            context_id,
            profile,
            browser,
            seed_from_context,
            reply,
        } => {
            let result = (|| -> Result<ContextSummary, AegisError> {
                let mut runtime =
                    create_context_runtime(root.host_library.clone(), browser, profile)?;
                if let Some(source_context_id) = seed_from_context {
                    let source = runtimes.get_mut(&source_context_id).ok_or_else(|| {
                        AegisError::Bridge(format!(
                            "context `{source_context_id}` is not available for seeding"
                        ))
                    })?;
                    let session = source.client.snapshot_session()?;
                    runtime.client.inject_session(session)?;
                }
                let summary = runtime.summary(context_id.clone(), false);
                root.insert_context(context_id.clone(), runtime.api.clone())
                    .map_err(|error| AegisError::Bridge(error.body.error.clone()))?;
                runtimes.insert(context_id, runtime);
                Ok(summary)
            })();
            let _ = reply.send(result);
        }
        ManagerCommand::DeleteContext { context_id, reply } => {
            root.remove_context(&context_id)
                .map_err(|error| AegisError::Bridge(error.body.error.clone()))?;
            if let Some(mut runtime) = runtimes.remove(&context_id) {
                runtime.persist_session_best_effort();
            }
            let _ = reply.send(Ok(()));
        }
    }
    Ok(())
}

fn shutdown_active_contexts(
    root: &ServeRootState,
    runtimes: &mut HashMap<String, MainThreadContext>,
) {
    if let Ok(mut contexts) = root.contexts.lock() {
        contexts.clear();
    }
    for (_, mut runtime) in runtimes.drain() {
        request_runtime_cancel(&runtime.api);
        runtime.persist_session_best_effort();
    }
}

fn manager_channel_error(error: mpsc::SendError<ManagerCommand>) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

fn channel_error(error: mpsc::SendError<ApiCommand>) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    fn bad_request(code: &str, message: String, suggested_action: Option<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ApiErrorBody::new(code, message)
                .with_retriable(false)
                .with_suggested_action(suggested_action),
        }
    }

    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: ApiErrorBody::new("not_found", message).with_retriable(false),
        }
    }

    fn timeout(operation: &str) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            body: ApiErrorBody::new(
                "operation_cancel_requested",
                format!(
                    "operation `{operation}` exceeded the server timeout and a runtime cancellation request was sent"
                ),
            )
            .with_operation(operation)
            .with_stage("awaiting_control_plane_reply")
            .with_elapsed_ms(COMMAND_TIMEOUT.as_millis() as u64)
            .with_timed_out(true)
            .with_retriable(true)
            .with_suggested_action(Some(
                "retry the request after the cancellation settles; if repeated timeouts occur, inspect /doctor or restart the runtime".into(),
            )),
        }
    }

    fn readiness(diagnostics: RuntimeDiagnosticsResponse) -> Self {
        let not_inspectable = diagnostics.command_ready
            && (diagnostics.synthetic_shell_active
                || (diagnostics.document_loaded
                    && diagnostics.module_scripts_present
                    && !diagnostics.inspectable_dom_ready));
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            body: ApiErrorBody::new(
                "not_ready",
                if not_inspectable {
                    if diagnostics.synthetic_shell_active {
                        String::from(
                            "runtime is attached, but is still on the synthetic bootstrap shell",
                        )
                    } else {
                        String::from("runtime is attached, but the page is not yet inspectable")
                    }
                } else {
                    String::from("runtime is not command-ready")
                },
            )
            .with_operation_option(
                diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.name.clone()),
            )
            .with_stage_option(
                diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.stage.clone())
                    .or_else(|| {
                        if not_inspectable {
                            Some(if diagnostics.synthetic_shell_active {
                                "stuck_on_bootstrap_shell".into()
                            } else {
                                "awaiting_page_bootstrap".into()
                            })
                        } else {
                            None
                        }
                    }),
            )
            .with_elapsed_ms_option(
                diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.elapsed_ms),
            )
            .with_timed_out(
                diagnostics
                    .active_operation
                    .as_ref()
                    .is_some_and(|op| op.timed_out),
            )
            .with_restart_recommended(
                diagnostics
                    .last_failure
                    .as_ref()
                    .is_some_and(|failure| failure.restart_recommended),
            )
            .with_retriable(true)
            .with_suggested_action(Some(
                "wait for /readyz to succeed, or inspect /doctor and /runtime if readiness does not recover".into(),
            )),
        }
    }
}

impl From<AegisError> for ApiError {
    fn from(value: AegisError) -> Self {
        let message = value.to_string();
        if let Some(native) = parse_native_operation_error(&message) {
            return Self {
                status: if native.timed_out {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                },
                body: ApiErrorBody::new("native_operation_error", native.message)
                    .with_operation(native.operation)
                    .with_stage(native.stage)
                    .with_elapsed_ms(native.elapsed_ms)
                    .with_timed_out(native.timed_out)
                    .with_restart_recommended(native.restart_recommended)
                    .with_retriable(true)
                    .with_suggested_action(Some(if native.restart_recommended {
                        "restart the runtime before retrying this request".into()
                    } else if native.timed_out {
                        "retry the request once the runtime finishes cancelling the timed out operation".into()
                    } else {
                        "inspect /doctor for the last runtime failure details, then retry if the browser state is still healthy".into()
                    })),
            };
        }

        let status = match value {
            AegisError::InvalidSession(_) => StatusCode::BAD_REQUEST,
            AegisError::Serialize(_)
            | AegisError::Deserialize(_)
            | AegisError::Io(_)
            | AegisError::Utf8(_)
            | AegisError::Protocol(_)
            | AegisError::Bridge(_) => StatusCode::BAD_GATEWAY,
        };
        let (retriable, suggested_action) = match value {
            AegisError::InvalidSession(_) => (
                false,
                Some("fix the session payload and retry the request".into()),
            ),
            AegisError::Serialize(_) | AegisError::Deserialize(_) | AegisError::Protocol(_) => (
                false,
                Some(
                    "check the request and response schema against /manifest before retrying"
                        .into(),
                ),
            ),
            AegisError::Io(_) => (
                true,
                Some("verify local filesystem access, then retry the request".into()),
            ),
            AegisError::Utf8(_) => (
                false,
                Some(
                    "inspect the runtime payload for invalid text encoding before retrying".into(),
                ),
            ),
            AegisError::Bridge(_) => (
                true,
                Some(
                    "inspect /doctor for bridge health and retry once the runtime is healthy"
                        .into(),
                ),
            ),
        };
        Self {
            status,
            body: ApiErrorBody::new("aegis_error", message)
                .with_retriable(retriable)
                .with_suggested_action(suggested_action),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub request_id: String,
    pub error: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    pub timed_out: bool,
    pub restart_recommended: bool,
    pub retriable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
}

impl ApiErrorBody {
    fn new(code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            request_id: next_api_request_id(),
            error: error.into(),
            code: code.into(),
            operation: None,
            stage: None,
            elapsed_ms: None,
            timed_out: false,
            restart_recommended: false,
            retriable: false,
            suggested_action: None,
        }
    }

    fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    fn with_operation_option(mut self, operation: Option<String>) -> Self {
        self.operation = operation;
        self
    }

    fn with_stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = Some(stage.into());
        self
    }

    fn with_stage_option(mut self, stage: Option<String>) -> Self {
        self.stage = stage;
        self
    }

    fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self
    }

    fn with_elapsed_ms_option(mut self, elapsed_ms: Option<u64>) -> Self {
        self.elapsed_ms = elapsed_ms;
        self
    }

    fn with_timed_out(mut self, timed_out: bool) -> Self {
        self.timed_out = timed_out;
        self
    }

    fn with_restart_recommended(mut self, restart_recommended: bool) -> Self {
        self.restart_recommended = restart_recommended;
        self
    }

    fn with_retriable(mut self, retriable: bool) -> Self {
        self.retriable = retriable;
        self
    }

    fn with_suggested_action(mut self, suggested_action: Option<String>) -> Self {
        self.suggested_action = suggested_action;
        self
    }
}

async fn await_command<T>(
    operation: &str,
    state: &ApiState,
    reply_rx: oneshot::Receiver<Result<T, AegisError>>,
) -> Result<Result<T, AegisError>, ApiError> {
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(result) => result.map_err(reply_error),
        Err(_) => {
            request_runtime_cancel(state);
            mark_operation_timeout(&state.diagnostics, operation);
            Err(ApiError::timeout(operation))
        }
    }
}

fn reply_error(error: oneshot::error::RecvError) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

fn emit_telemetry(event: &str, payload: serde_json::Value) {
    let Some(path) = std::env::var_os("AEGIS_DEBUG_LOG") else {
        return;
    };
    let start = TELEMETRY_START.get_or_init(Instant::now);
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let line = json!({
        "source": "serve",
        "event": event,
        "elapsed_ms": elapsed_ms,
        "payload": payload,
    });
    if let Ok(mut output) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(output, "telemetry: {}", line);
    }
}

fn emit_operation_telemetry<T>(
    operation: &str,
    started_at: Instant,
    result: &Result<T, AegisError>,
    runtime: RuntimeStatus,
) {
    emit_telemetry(
        "operation_complete",
        json!({
            "operation": operation,
            "latency_ms": started_at.elapsed().as_millis() as u64,
            "ok": result.is_ok(),
            "error": result.as_ref().err().map(ToString::to_string),
            "runtime": runtime,
        }),
    );
}

fn record_operation_started(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    stage: &str,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.begin_operation(operation, stage);
    }
}

fn record_operation_finished<T>(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    client: &LoadedAegisClient,
    result: &Result<T, AegisError>,
) {
    let runtime = client.runtime_status();
    match result {
        Ok(_) => {
            if let Ok(mut diagnostics) = diagnostics.lock() {
                diagnostics.complete_success(operation, runtime);
            }
        }
        Err(error) => record_operation_failure(
            diagnostics,
            operation,
            failure_from_error(operation, "native operation failed", error),
            Some(runtime),
        ),
    }
}

fn record_operation_failure(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    failure: FailureSnapshot,
    runtime: Option<RuntimeStatus>,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.complete_failure(operation, failure, runtime);
    }
}

fn record_heartbeat(diagnostics: &Arc<Mutex<ServeDiagnostics>>, client: &LoadedAegisClient) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.record_runtime_snapshot(client.runtime_status());
    }
}

fn mark_operation_timeout(diagnostics: &Arc<Mutex<ServeDiagnostics>>, operation: &str) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.mark_timeout(operation, COMMAND_TIMEOUT.as_millis() as u64);
    }
}

fn mark_operation_cancel_requested(diagnostics: &Arc<Mutex<ServeDiagnostics>>) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.mark_cancel_requested();
    }
}

fn request_runtime_cancel(state: &ApiState) {
    if let Ok(cancel) = state.cancel.lock()
        && let Some(cancel) = cancel.as_ref()
    {
        cancel.request_cancel();
    }
}

fn read_diagnostics(diagnostics: &Arc<Mutex<ServeDiagnostics>>) -> RuntimeDiagnosticsResponse {
    diagnostics
        .lock()
        .map(|diagnostics| diagnostics.snapshot())
        .unwrap_or_else(|_| ServeDiagnostics::new(default_runtime_status()).snapshot())
}

fn downloads_response(state: &ApiState) -> DownloadsResponse {
    let diagnostics = read_diagnostics(&state.diagnostics);
    DownloadsResponse {
        download_dir: diagnostics.runtime.host.download_dir.clone(),
        downloads: diagnostics.runtime.host.downloads.clone(),
    }
}

fn path_param(
    name: &'static str,
    kind: &'static str,
    description: &'static str,
) -> ApiParameterDoc {
    ApiParameterDoc {
        name,
        location: "path",
        kind,
        required: true,
        description,
    }
}

fn query_param(
    name: &'static str,
    kind: &'static str,
    required: bool,
    description: &'static str,
) -> ApiParameterDoc {
    ApiParameterDoc {
        name,
        location: "query",
        kind,
        required,
        description,
    }
}

fn json_request(schema: &'static str) -> ApiRequestBodyDoc {
    ApiRequestBodyDoc {
        content_type: "application/json",
        schema,
        example: None,
    }
}

fn json_request_with_example(
    schema: &'static str,
    example: serde_json::Value,
) -> ApiRequestBodyDoc {
    ApiRequestBodyDoc {
        content_type: "application/json",
        schema,
        example: Some(example),
    }
}

fn json_response(status: u16, description: &'static str, schema: &'static str) -> ApiResponseDoc {
    ApiResponseDoc {
        status,
        description,
        schema,
    }
}

fn empty_response(status: u16, description: &'static str) -> ApiResponseDoc {
    ApiResponseDoc {
        status,
        description,
        schema: "empty",
    }
}

fn navigate_body_example() -> serde_json::Value {
    json!({
        "url": "https://example.com"
    })
}

fn execute_body_example() -> serde_json::Value {
    json!({
        "commands": [
            {
                "type": "eval",
                "code": "document.title"
            }
        ]
    })
}

fn search_body_example() -> serde_json::Value {
    json!({
        "query": "shopify app review",
        "engine": "duckduckgo"
    })
}

fn page_find_body_example() -> serde_json::Value {
    json!({
        "text": "Submit an app version for review",
        "exact": false
    })
}

fn page_open_link_body_example() -> serde_json::Value {
    json!({
        "text": "Release an app version",
        "exact": false
    })
}

fn trace_body_example() -> serde_json::Value {
    json!({
        "path": "traces/run.json"
    })
}

fn sse_response(status: u16, description: &'static str) -> ApiResponseDoc {
    ApiResponseDoc {
        status,
        description,
        schema: "text/event-stream<EventReadWindow>",
    }
}

fn event_query_schema() -> Vec<ApiParameterDoc> {
    vec![
        query_param(
            "since",
            "u64",
            false,
            "Return only events with sequence greater than this cursor",
        ),
        query_param(
            "type",
            "string[]",
            false,
            "Repeated or comma-separated event filters such as network, websocket_frame, download, log, or unknown",
        ),
    ]
}

fn event_stream_query_schema() -> Vec<ApiParameterDoc> {
    let mut params = event_query_schema();
    params.push(query_param(
        "poll_ms",
        "u64",
        false,
        "Polling interval for the SSE bridge loop, clamped to the documented runtime bounds",
    ));
    params
}

fn default_runtime_status() -> RuntimeStatus {
    RuntimeStatus {
        bootstrapped: false,
        bootstrap_duration_ms: None,
        dom_nodes: 0,
        dom_snapshot_available: false,
        retained_event_count: 0,
        latest_event_sequence: 0,
        oldest_retained_event_sequence: None,
        current_url: None,
        current_title: None,
        document_ready_state: None,
        page_bootstrap: PageBootstrapDiagnostics::default(),
        media: Vec::new(),
        last_dom_refresh_at_ms: None,
        last_live_state_refresh_at_ms: None,
        last_event_at_ms: None,
        last_successful_command_at_ms: None,
        last_successful_bridge_roundtrip_at_ms: None,
        host: Default::default(),
    }
}

fn failure_from_error(operation: &str, stage: &str, error: &AegisError) -> FailureSnapshot {
    if let Some(native) = parse_native_operation_error(&error.to_string()) {
        return FailureSnapshot {
            operation: native.operation,
            stage: native.stage,
            message: native.message,
            elapsed_ms: native.elapsed_ms,
            timed_out: native.timed_out,
            restart_recommended: native.restart_recommended,
            first_seen_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
        };
    }

    FailureSnapshot {
        operation: operation.to_string(),
        stage: stage.to_string(),
        message: error.to_string(),
        elapsed_ms: 0,
        timed_out: false,
        restart_recommended: false,
        first_seen_at_ms: now_ms(),
        last_seen_at_ms: now_ms(),
    }
}

fn parse_native_operation_error(message: &str) -> Option<NativeOperationError> {
    let payload = message.strip_prefix("bridge error: ").unwrap_or(message);
    let parsed: NativeOperationError = serde_json::from_str(payload).ok()?;
    (parsed.kind == "operation_error").then_some(parsed)
}

impl ServeDiagnostics {
    fn new(runtime: RuntimeStatus) -> Self {
        Self {
            runtime,
            active_operation: None,
            last_failure: None,
            total_operations: 0,
            successful_operations: 0,
            timed_out_operations: 0,
            next_operation_id: 1,
        }
    }

    fn begin_operation(&mut self, name: &str, stage: &str) {
        self.total_operations += 1;
        self.active_operation = Some(ActiveOperation {
            id: self.next_operation_id,
            name: name.to_string(),
            stage: stage.to_string(),
            started_at_ms: now_ms(),
            started_at: Instant::now(),
            timed_out: false,
        });
        self.next_operation_id += 1;
    }

    fn complete_success(&mut self, _name: &str, runtime: RuntimeStatus) {
        self.successful_operations += 1;
        self.runtime = runtime;
        self.active_operation = None;
        self.last_failure = None;
    }

    fn complete_failure(
        &mut self,
        _name: &str,
        mut failure: FailureSnapshot,
        runtime: Option<RuntimeStatus>,
    ) {
        if let Some(runtime) = runtime {
            self.runtime = runtime;
        }
        if let Some(previous) = self.last_failure.as_ref() {
            failure.first_seen_at_ms = previous.first_seen_at_ms;
        }
        self.last_failure = Some(failure);
        self.active_operation = None;
    }

    fn mark_timeout(&mut self, operation: &str, elapsed_ms: u64) {
        self.timed_out_operations += 1;
        self.runtime.host.cancel_requested = true;
        if let Some(active) = self.active_operation.as_mut() {
            active.timed_out = true;
            active.stage = "awaiting_control_plane_reply".into();
        }
        let now = now_ms();
        self.last_failure = Some(FailureSnapshot {
            operation: operation.to_string(),
            stage: "awaiting_control_plane_reply".into(),
            message: "the API timed out waiting for the runtime owner thread to reply and requested cancellation".into(),
            elapsed_ms,
            timed_out: true,
            restart_recommended: false,
            first_seen_at_ms: self
                .last_failure
                .as_ref()
                .map(|failure| failure.first_seen_at_ms)
                .unwrap_or(now),
            last_seen_at_ms: now,
        });
    }

    fn record_runtime_snapshot(&mut self, runtime: RuntimeStatus) {
        self.runtime = runtime;
        let host = &self.runtime.host;
        if self.runtime.bootstrapped
            && host.browser_available
            && host.page_ready
            && host.renderer_ready
            && host.runtime_ready
            && !host.load_in_progress
        {
            self.last_failure = None;
        }
    }

    fn mark_cancel_requested(&mut self) {
        self.runtime.host.cancel_requested = true;
        if let Some(active) = self.active_operation.as_mut() {
            active.stage = "cancelling_active_operation".into();
        }
    }

    fn snapshot(&self) -> RuntimeDiagnosticsResponse {
        let active_operation = self
            .active_operation
            .as_ref()
            .map(|operation| OperationSnapshot {
                id: operation.id,
                name: operation.name.clone(),
                stage: operation.stage.clone(),
                started_at_ms: operation.started_at_ms,
                elapsed_ms: operation.started_at.elapsed().as_millis() as u64,
                timed_out: operation.timed_out,
            });
        let host = &self.runtime.host;
        let runtime_operational = self.runtime.bootstrapped
            && host.browser_available
            && host.renderer_ready
            && host.runtime_ready;
        let state = if host.cancel_requested {
            RuntimeOperationalState::Cancelling
        } else if active_operation.as_ref().is_some_and(|op| op.timed_out) {
            RuntimeOperationalState::Wedged
        } else if !self.runtime.bootstrapped {
            RuntimeOperationalState::Starting
        } else if self
            .last_failure
            .as_ref()
            .is_some_and(|failure| failure.timed_out && failure.restart_recommended)
        {
            RuntimeOperationalState::Wedged
        } else if active_operation.is_some() || host.load_in_progress {
            RuntimeOperationalState::Busy
        } else if host.browser_closed
            || !host.browser_available
            || !host.renderer_ready
            || !host.runtime_ready
            || self.last_failure.is_some()
        {
            RuntimeOperationalState::Degraded
        } else if runtime_operational
            && self
                .runtime
                .last_successful_bridge_roundtrip_at_ms
                .is_some()
        {
            RuntimeOperationalState::Ready
        } else {
            RuntimeOperationalState::Degraded
        };
        let command_ready = matches!(state, RuntimeOperationalState::Ready);
        let page_bootstrap = &self.runtime.page_bootstrap;
        RuntimeDiagnosticsResponse {
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: PROTOCOL_VERSION,
            state,
            control_plane_up: true,
            command_ready,
            inspectable_dom_ready: page_bootstrap.inspectable_dom_ready,
            document_loaded: page_bootstrap.document_loaded,
            module_scripts_present: page_bootstrap.module_scripts_present,
            module_bootstrap_observed: page_bootstrap.module_bootstrap_observed,
            app_dom_mutated_after_load: page_bootstrap.app_dom_mutated_after_load,
            synthetic_shell_active: page_bootstrap.synthetic_shell_active,
            bridge_healthy: self
                .runtime
                .last_successful_bridge_roundtrip_at_ms
                .is_some()
                && runtime_operational
                && self.last_failure.is_none(),
            browser_backend_healthy: self.runtime.bootstrapped
                && host.browser_available
                && !host.browser_closed
                && self
                    .last_failure
                    .as_ref()
                    .is_none_or(|failure| !failure.restart_recommended),
            browser_process_up: host.browser_available && !host.browser_closed,
            page_attached: host.page_ready,
            renderer_attached: host.renderer_ready,
            dom_snapshot_available: self.runtime.dom_snapshot_available,
            event_decoder_ok: self
                .last_failure
                .as_ref()
                .is_none_or(|failure| !failure.message.contains("unknown variant")),
            active_operation,
            last_failure: self.last_failure.clone(),
            total_operations: self.total_operations,
            successful_operations: self.successful_operations,
            timed_out_operations: self.timed_out_operations,
            runtime: self.runtime.clone(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn next_api_request_id() -> String {
    let ordinal = API_ERROR_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("aegis-{}-{ordinal}", now_ms())
}

fn process_started_at_ms() -> u64 {
    *PROCESS_STARTED_AT_MS.get_or_init(now_ms)
}

fn runtime_identity() -> RuntimeIdentity {
    let started_at_ms = process_started_at_ms();
    RuntimeIdentity {
        process_id: std::process::id(),
        executable_path: std::env::current_exe().ok(),
        started_at_ms,
        uptime_ms: now_ms().saturating_sub(started_at_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::command::{CommandMatcher, CommandTarget};
    use crate::dom::node::{DomNode, DomNodeSemantics, DomSnapshot};
    use crate::events::stream::SequencedEvent;
    use serde_json::json;
    use std::collections::HashMap;

    fn dummy_api_state(profile: &str) -> ApiState {
        let (tx, _rx) = mpsc::channel();
        ApiState {
            tx,
            cancel: Arc::new(Mutex::new(None)),
            host_library: PathBuf::from("/tmp/aegis_host.dylib"),
            browser: BrowserConfig::default(),
            startup: Arc::new(Mutex::new(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            })),
            profile: SessionProfileInfo {
                profile: profile.into(),
                path: PathBuf::from(format!("/tmp/{profile}.json")),
            },
            diagnostics: Arc::new(Mutex::new(ServeDiagnostics::new(default_runtime_status()))),
        }
    }

    fn ready_diagnostics() -> RuntimeDiagnosticsResponse {
        let mut runtime = default_runtime_status();
        runtime.bootstrapped = true;
        runtime.last_successful_bridge_roundtrip_at_ms = Some(now_ms());
        runtime.host.browser_available = true;
        runtime.host.page_ready = true;
        runtime.host.renderer_ready = true;
        runtime.host.runtime_ready = true;
        ServeDiagnostics::new(runtime).snapshot()
    }

    #[test]
    fn validate_context_name_rejects_invalid_characters() {
        assert!(validate_context_name("guest-1").is_ok());
        assert!(validate_context_name("guest/admin").is_err());
        assert!(validate_context_name("").is_err());
    }

    #[test]
    fn list_contexts_sorts_and_marks_default() {
        let root = ServeRootState {
            host_library: PathBuf::from("/tmp/aegis_host.dylib"),
            browser: BrowserConfig::default(),
            default_context_id: "default".into(),
            manager_tx: mpsc::channel().0,
            contexts: Arc::new(Mutex::new(HashMap::from([
                ("guest".into(), dummy_api_state("guest")),
                ("default".into(), dummy_api_state("default")),
            ]))),
        };

        let contexts = root
            .list_contexts()
            .unwrap_or_else(|error| panic!("contexts should list: {error:?}"));
        assert_eq!(contexts.len(), 2);
        assert_eq!(contexts[0].id, "default");
        assert!(contexts[0].default);
        assert_eq!(contexts[0].runtime_state, "starting");
        assert!(!contexts[0].command_ready);
        assert_eq!(contexts[1].id, "guest");
        assert!(!contexts[1].default);
    }

    #[test]
    fn readiness_error_reports_bootstrap_shell_truthfully() {
        let mut diagnostics = ready_diagnostics();
        diagnostics.document_loaded = true;
        diagnostics.synthetic_shell_active = true;
        diagnostics.inspectable_dom_ready = false;
        diagnostics.module_scripts_present = false;

        let error = ApiError::readiness(diagnostics);
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.body.code, "not_ready");
        assert_eq!(
            error.body.error,
            "runtime is attached, but is still on the synthetic bootstrap shell"
        );
        assert_eq!(
            error.body.stage.as_deref(),
            Some("stuck_on_bootstrap_shell")
        );
    }

    #[test]
    fn readiness_error_reports_uninspectable_module_page_truthfully() {
        let mut diagnostics = ready_diagnostics();
        diagnostics.document_loaded = true;
        diagnostics.module_scripts_present = true;
        diagnostics.inspectable_dom_ready = false;
        diagnostics.synthetic_shell_active = false;

        let error = ApiError::readiness(diagnostics);
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.body.code, "not_ready");
        assert_eq!(
            error.body.error,
            "runtime is attached, but the page is not yet inspectable"
        );
        assert_eq!(error.body.stage.as_deref(), Some("awaiting_page_bootstrap"));
    }

    #[test]
    fn bad_request_errors_include_correlation_and_recovery_guidance() {
        let error = ApiError::bad_request(
            "invalid_event_filter",
            "unsupported event filter `bogus`".into(),
            Some("use a documented type query".into()),
        );

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.code, "invalid_event_filter");
        assert!(error.body.request_id.starts_with("aegis-"));
        assert!(!error.body.retriable);
        assert_eq!(
            error.body.suggested_action.as_deref(),
            Some("use a documented type query")
        );
    }

    #[test]
    fn manifest_exposes_machine_readable_route_and_error_schema_metadata() {
        let manifest = api_manifest_document(true);
        let events_route = manifest
            .routes
            .iter()
            .find(|route| route.path == "/events/live")
            .expect("events/live route should exist");

        assert_eq!(events_route.method, "GET");
        assert_eq!(
            events_route
                .query
                .iter()
                .find(|param| param.name == "type")
                .map(|param| param.kind),
            Some("string[]")
        );
        assert_eq!(
            events_route
                .responses
                .first()
                .map(|response| response.schema),
            Some("text/event-stream<EventReadWindow>")
        );
        assert!(manifest.event_filters.contains(&"websocket_frame"));
        assert_eq!(manifest.error_schema.name, "ApiErrorBody");
        assert!(
            manifest
                .error_schema
                .fields
                .iter()
                .any(|field| field.name == "request_id")
        );
        let execute_route = manifest
            .routes
            .iter()
            .find(|route| route.path == "/execute")
            .expect("execute route should exist");
        assert!(
            execute_route
                .request_body
                .as_ref()
                .and_then(|body| body.example.as_ref())
                .is_some()
        );
        assert!(manifest.capabilities.contains(&"research_workflow_primitives"));
        assert!(
            manifest
                .routes
                .iter()
                .any(|route| route.path == "/page/text" && route.method == "GET")
        );
    }

    #[test]
    fn search_url_defaults_to_duckduckgo() {
        assert_eq!(
            build_search_url("shopify app review", None),
            "https://duckduckgo.com/?q=shopify+app+review"
        );
    }

    #[test]
    fn page_find_matches_links_and_visible_text() {
        let page = PageResearchData {
            title: Some("Docs".into()),
            url: Some("https://example.com/docs".into()),
            canonical_url: None,
            visible_text: "Submit an app version for review from the releases page.".into(),
            headings: vec![PageResearchHeading {
                level: Some(2),
                text: "Release an app version".into(),
                id: None,
            }],
            links: vec![PageResearchLink {
                index: 0,
                text: "Release an app version".into(),
                href: "https://example.com/releases".into(),
                title: None,
            }],
            likely_not_found: false,
            not_found_signals: Vec::new(),
            suggested_search_query: None,
        };
        let matches = page_find_matches(&page, "release an app version", false);
        assert!(matches.iter().any(|entry| entry.kind == "heading"));
        assert!(matches.iter().any(|entry| entry.kind == "link"));
    }

    #[test]
    fn event_filter_matches_categories_and_exact_wire_types() {
        let filter = EventFilter::from_query(&[
            "network".into(),
            "websocket_frame".into(),
            "unknown".into(),
        ])
        .expect("filter parses")
        .expect("filter exists");
        let filtered = filter_event_window(
            EventReadWindow {
                requested_since: 0,
                oldest_available_sequence: Some(1),
                latest_sequence: 4,
                gap_detected: false,
                events: vec![
                    SequencedEvent {
                        sequence: 1,
                        timestamp_ms: 1,
                        event: RuntimeEvent::Navigation {
                            url: "https://example.com".into(),
                        },
                    },
                    SequencedEvent {
                        sequence: 2,
                        timestamp_ms: 2,
                        event: RuntimeEvent::Network {
                            request_id: "req-1".into(),
                            url: "https://example.com/api".into(),
                            method: Some("GET".into()),
                            resource_type: None,
                            phase: None,
                            status: Some(200),
                            status_text: None,
                            mime_type: None,
                            from_cache: None,
                            error_text: None,
                        },
                    },
                    SequencedEvent {
                        sequence: 3,
                        timestamp_ms: 3,
                        event: RuntimeEvent::WebSocketFrame {
                            request_id: "ws-1".into(),
                            url: "wss://example.com/socket".into(),
                            direction: crate::events::stream::WebSocketFrameDirection::Received,
                            opcode: Some(1),
                            mask: Some(false),
                            payload_preview: "hello".into(),
                            payload_length: 5,
                            truncated: false,
                        },
                    },
                    SequencedEvent {
                        sequence: 4,
                        timestamp_ms: 4,
                        event: RuntimeEvent::Unknown {
                            event_type: "service_worker".into(),
                            payload: json!({"type":"service_worker","scope":"https://example.com"}),
                        },
                    },
                ],
            },
            Some(&filter),
        );

        assert_eq!(filtered.latest_sequence, 4);
        assert_eq!(filtered.events.len(), 3);
        assert_eq!(filtered.events[0].event.event_name(), "network");
        assert_eq!(filtered.events[1].event.event_name(), "websocket_frame");
        assert_eq!(filtered.events[2].event.event_name(), "service_worker");
    }

    #[test]
    fn event_filter_rejects_unknown_filter_names() {
        let error = EventFilter::from_query(&["bogus".into()]).expect_err("filter should fail");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.code, "invalid_event_filter");
    }

    fn password_node(id: u64) -> DomNode {
        DomNode {
            id,
            tag: "input".into(),
            attrs: HashMap::from([
                ("type".into(), "password".into()),
                ("name".into(), "password".into()),
                ("autocomplete".into(), "current-password".into()),
            ]),
            text: None,
            semantic: Some(DomNodeSemantics {
                role: Some("textbox".into()),
                name: Some("Password".into()),
                label: Some("Password".into()),
                control_type: Some("password".into()),
                actionable: true,
                visible: true,
                ..Default::default()
            }),
            children: Vec::new(),
        }
    }

    fn submit_node(id: u64) -> DomNode {
        DomNode {
            id,
            tag: "button".into(),
            attrs: HashMap::from([("type".into(), "submit".into())]),
            text: Some("Log in".into()),
            semantic: Some(DomNodeSemantics {
                role: Some("button".into()),
                name: Some("Log in".into()),
                label: Some("Log in".into()),
                control_type: Some("button".into()),
                actionable: true,
                visible: true,
                actions: vec!["click".into(), "submit".into()],
                ..Default::default()
            }),
            children: Vec::new(),
        }
    }

    fn captured_credentials() -> AutoCredentialCapture {
        AutoCredentialCapture {
            username: Some(CapturedCredentialField {
                value: "saint@example.com".into(),
                field_name: Some("email".into()),
                label: Some("Email".into()),
            }),
            password: Some(CapturedCredentialField {
                value: "super-secret".into(),
                field_name: Some("password".into()),
                label: Some("Password".into()),
            }),
            origin: Some("https://accounts.shopify.com".into()),
        }
    }

    #[test]
    fn credential_submit_detection_accepts_selector_login_clicks() {
        let snapshot = DomSnapshot {
            nodes: vec![password_node(1), submit_node(2)],
        };
        let capture = captured_credentials();
        let commands = vec![Command::Click {
            target: CommandTarget::Match {
                matcher: CommandMatcher {
                    selector: Some("#login".into()),
                    test_id: None,
                    role: None,
                    name: None,
                    label: None,
                    control_type: None,
                    tag: None,
                    text: None,
                    placeholder: None,
                    href_contains: None,
                    actionable: None,
                    disabled: None,
                    exact: None,
                },
            },
        }];

        assert!(capture.should_persist(&snapshot, &commands));
    }

    #[test]
    fn credential_submit_detection_accepts_enter_on_password_field() {
        let snapshot = DomSnapshot {
            nodes: vec![password_node(1), submit_node(2)],
        };
        let capture = captured_credentials();
        let commands = vec![Command::PressKey {
            target: Some(CommandTarget::Id { id: 1 }),
            key: "Enter".into(),
            code: Some("Enter".into()),
            alt_key: false,
            ctrl_key: false,
            meta_key: false,
            shift_key: false,
        }];

        assert!(capture.should_persist(&snapshot, &commands));
    }
}
