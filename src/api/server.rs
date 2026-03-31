use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::browser::BrowserConfig;
use crate::commands::command::{Command, CommandTarget};
use crate::commands::matcher::resolve_command_target as resolve_snapshot_target;
use crate::config_store::{AegisConfigStore, AegisSecretStore, CredentialInput};
use crate::dom::node::{DomNode, DomSnapshot};
use crate::events::stream::{EventReadWindow, SequencedEvent};
use crate::host::LoadedAegisClient;
use crate::runtime::executor::{ExecutionReport, RuntimeStatus};
use crate::session::cookies::SessionState;
use crate::session::profile::{SessionProfileInfo, SessionProfileStore};
use crate::transport::bridge::{AegisError, BrowserChromeState};

const IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct ApiState {
    tx: mpsc::Sender<ApiCommand>,
    host_library: PathBuf,
    browser: BrowserConfig,
    startup: Arc<Mutex<ServeStartupMetrics>>,
    profile: SessionProfileInfo,
    diagnostics: Arc<Mutex<ServeDiagnostics>>,
    chrome_tx: Arc<tokio::sync::watch::Sender<BrowserChromeState>>,
}

impl ApiState {
    pub fn chrome_rx(&self) -> tokio::sync::watch::Receiver<BrowserChromeState> {
        self.chrome_tx.subscribe()
    }

    pub fn chrome_state_snapshot(&self) -> BrowserChromeState {
        self.chrome_tx.borrow().clone()
    }

    pub fn send_command(&self, command: ApiCommand) -> Result<(), mpsc::SendError<ApiCommand>> {
        self.tx.send(command)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServeStartupMetrics {
    client_connect_ms: u64,
    api_bind_ms: u64,
    total_ready_ms: u64,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    control_plane_up: bool,
    runtime_state: RuntimeOperationalState,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
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

#[derive(Debug, Deserialize)]
pub struct TraceBody {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub since: u64,
}

pub enum ApiCommand {
    InjectSession(SessionState, oneshot::Sender<Result<(), AegisError>>),
    SnapshotSession(oneshot::Sender<Result<SessionState, AegisError>>),
    SaveSessionProfile(oneshot::Sender<Result<SessionProfileInfo, AegisError>>),
    LoadSessionProfile(oneshot::Sender<Result<SessionProfileInfo, AegisError>>),
    Navigate(
        String,
        oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>,
    ),
    Execute(
        Vec<Command>,
        oneshot::Sender<Result<ExecutionReport, AegisError>>,
    ),
    SnapshotDom(oneshot::Sender<Result<DomSnapshot, AegisError>>),
    Events(u64, oneshot::Sender<Result<EventReadWindow, AegisError>>),
    EnableTrace(PathBuf, oneshot::Sender<Result<(), AegisError>>),
    GoBack,
    GoForward,
    Reload,
    StopLoad,
    ChromeNavigate(String),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RuntimeOperationalState {
    Starting,
    Ready,
    Busy,
    Degraded,
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
    state: RuntimeOperationalState,
    control_plane_up: bool,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    dom_snapshot_available: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    runtime: RuntimeStatus,
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

pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
) -> Result<(), AegisError> {
    let serve_started = std::time::Instant::now();
    let client_connect_started = std::time::Instant::now();
    let mut client = LoadedAegisClient::connect(host_library.clone(), browser_config.clone())?;
    let profile_store = SessionProfileStore::new(profile_name).map_err(AegisError::Bridge)?;
    let credential_settings = AegisConfigStore::detect()
        .and_then(|store| store.load_credentials_settings())
        .map_err(AegisError::Bridge)?;
    let credential_store = AegisSecretStore::detect().map_err(AegisError::Bridge)?;
    let mut credential_capture = AutoCredentialCapture::default();
    if let Some(session) = profile_store.load().map_err(AegisError::Bridge)? {
        client.inject_session(session)?;
    }
    let client_connect_ms = client_connect_started.elapsed().as_millis() as u64;
    let api_bind_started = std::time::Instant::now();
    let (tx, rx) = mpsc::channel::<ApiCommand>();
    let startup = Arc::new(Mutex::new(ServeStartupMetrics {
        client_connect_ms,
        api_bind_ms: 0,
        total_ready_ms: 0,
    }));
    let diagnostics = Arc::new(Mutex::new(ServeDiagnostics::new(client.runtime_status())));
    let (chrome_tx, _chrome_rx) = tokio::sync::watch::channel(BrowserChromeState::default());
    let chrome_tx = Arc::new(chrome_tx);
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
    let state = ApiState {
        tx,
        host_library,
        browser: browser_config.clone(),
        startup: startup.clone(),
        profile: profile_store.info(),
        diagnostics: diagnostics.clone(),
        chrome_tx: chrome_tx.clone(),
    };
    let startup_host_library = state.host_library.clone();

    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string()));
                return;
            }
        };

        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let _ = startup_tx.send(Ok(()));
                    listener
                }
                Err(error) => {
                    let _ = startup_tx.send(Err(error.to_string()));
                    return;
                }
            };

            let app = router(state);
            let _ = axum::serve(listener, app).await;
        });
    });

    match startup_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(AegisError::Bridge(error)),
        Err(error) => return Err(AegisError::Bridge(error.to_string())),
    }

    let startup_metrics = ServeStartupMetrics {
        client_connect_ms,
        api_bind_ms: api_bind_started.elapsed().as_millis() as u64,
        total_ready_ms: serve_started.elapsed().as_millis() as u64,
    };
    if let Ok(mut shared) = startup.lock() {
        *shared = startup_metrics;
    }

    eprintln!(
        "Aegis serve ready on http://{} ({:?}, host: {})",
        addr,
        browser_config.mode,
        startup_host_library.display()
    );

    loop {
        match rx.recv_timeout(IDLE_PUMP_INTERVAL) {
            Ok(command) => match command {
                ApiCommand::InjectSession(session, reply) => {
                    record_operation_started(&diagnostics, "inject_session", "injecting session");
                    let result = client.inject_session(session.clone()).and_then(|_| {
                        profile_store
                            .save(&session)
                            .map(|_| ())
                            .map_err(AegisError::Bridge)
                    });
                    record_operation_finished(&diagnostics, "inject_session", &client, &result);
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotSession(reply) => {
                    record_operation_started(
                        &diagnostics,
                        "snapshot_session",
                        "capturing session state",
                    );
                    let result = client.snapshot_session();
                    record_operation_finished(&diagnostics, "snapshot_session", &client, &result);
                    let _ = reply.send(result);
                }
                ApiCommand::SaveSessionProfile(reply) => {
                    record_operation_started(
                        &diagnostics,
                        "save_session_profile",
                        "persisting session profile",
                    );
                    let result = client.snapshot_session().and_then(|session| {
                        profile_store
                            .save(&session)
                            .map(|_| profile_store.info())
                            .map_err(AegisError::Bridge)
                    });
                    record_operation_finished(
                        &diagnostics,
                        "save_session_profile",
                        &client,
                        &result,
                    );
                    let _ = reply.send(result);
                }
                ApiCommand::LoadSessionProfile(reply) => {
                    record_operation_started(
                        &diagnostics,
                        "load_session_profile",
                        "loading session profile",
                    );
                    let result = profile_store.load().map_err(AegisError::Bridge).and_then(
                        |maybe_session| match maybe_session {
                            Some(session) => {
                                client.inject_session(session).map(|_| profile_store.info())
                            }
                            None => Ok(profile_store.info()),
                        },
                    );
                    record_operation_finished(
                        &diagnostics,
                        "load_session_profile",
                        &client,
                        &result,
                    );
                    let _ = reply.send(result);
                }
                ApiCommand::Navigate(url, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "navigate",
                        &format!("navigating to {url}"),
                    );
                    credential_capture.reset_on_explicit_navigation(&url);
                    let result = client.navigate(url);
                    record_operation_finished(&diagnostics, "navigate", &client, &result);
                    let _ = reply.send(result);
                }
                ApiCommand::Execute(commands, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "execute",
                        "executing browser command batch",
                    );
                    let maybe_snapshot = if credential_settings.auto_store
                        && commands.iter().any(|command| {
                            matches!(command, Command::SetValue { .. } | Command::Click { .. })
                        }) {
                        Some(client.snapshot_dom()?)
                    } else {
                        None
                    };
                    if let Some(snapshot) = maybe_snapshot.as_ref() {
                        credential_capture.capture_fields(
                            snapshot,
                            client.runtime().current_url(),
                            &commands,
                        );
                    }
                    let should_persist = credential_settings.auto_store
                        && maybe_snapshot.as_ref().is_some_and(|snapshot| {
                            credential_capture.should_persist(snapshot, &commands)
                        });
                    let persist_origin = if should_persist {
                        client.runtime().current_url().map(origin_key)
                    } else {
                        None
                    };
                    let result = client.execute(&commands).and_then(|report| {
                        if let Some(origin) = persist_origin {
                            credential_capture.persist(
                                &credential_store,
                                &profile_store.info().profile,
                                &origin,
                            )?;
                        }
                        Ok(report)
                    });
                    record_operation_finished(&diagnostics, "execute", &client, &result);
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotDom(reply) => {
                    record_operation_started(
                        &diagnostics,
                        "snapshot_dom",
                        "capturing DOM snapshot",
                    );
                    let result = client.snapshot_dom();
                    record_operation_finished(&diagnostics, "snapshot_dom", &client, &result);
                    let _ = reply.send(result);
                }
                ApiCommand::Events(since, reply) => {
                    record_operation_started(&diagnostics, "events", "draining runtime events");
                    let result = client.events_since(since);
                    record_operation_finished(&diagnostics, "events", &client, &result);
                    let _ = reply.send(result);
                }
                ApiCommand::EnableTrace(path, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "enable_trace",
                        "enabling trace recording",
                    );
                    client.enable_trace_recording(path);
                    record_operation_finished(&diagnostics, "enable_trace", &client, &Ok(()));
                    let _ = reply.send(Ok(()));
                }
                ApiCommand::GoBack => {
                    let _ = client.go_back();
                }
                ApiCommand::GoForward => {
                    let _ = client.go_forward();
                }
                ApiCommand::Reload => {
                    let _ = client.reload_page();
                }
                ApiCommand::StopLoad => {
                    let _ = client.stop_load();
                }
                ApiCommand::ChromeNavigate(url) => {
                    let _ = client.navigate(url);
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => match client.pump() {
                Ok(()) => {
                    record_heartbeat(&diagnostics, &client);
                    if let Ok(state) = client.snapshot_chrome_state() {
                        let _ = chrome_tx.send_if_modified(|current| {
                            if *current != state {
                                *current = state;
                                true
                            } else {
                                false
                            }
                        });
                    }
                }
                Err(error) => {
                    record_operation_failure(
                        &diagnostics,
                        "pump",
                        failure_from_error("pump", "pumping browser event loop", &error),
                        Some(client.runtime_status()),
                    );
                    return Err(error);
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if let Ok(session) = client.snapshot_session() {
        let _ = profile_store.save(&session);
    }

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
            && commands.iter().any(|command| {
                let Command::Click { target } = command else {
                    return false;
                };
                resolve_command_target(snapshot, target).is_some_and(is_submit_like_node)
            })
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

fn origin_key(url: &str) -> String {
    let trimmed = url.trim();
    if let Some((scheme, rest)) = trimmed.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        return format!("{scheme}://{host}");
    }
    trimmed.to_string()
}

pub fn router(state: ApiState) -> Router {
    use super::chrome;
    use tower_http::cors::CorsLayer;
    use tower_http::services::ServeDir;

    let mut app = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/doctor", get(doctor))
        .route("/runtime", get(runtime_info))
        .route("/session", post(inject_session).get(snapshot_session))
        .route("/session/save", post(save_session_profile))
        .route("/session/load", post(load_session_profile))
        .route("/navigate", post(navigate))
        .route("/execute", post(execute))
        .route("/dom", get(snapshot_dom))
        .route("/events", get(events))
        .route("/trace/enable", post(enable_trace))
        .route("/chrome/state", get(chrome::chrome_state_sse))
        .route("/chrome/state/snapshot", get(chrome::chrome_state_snapshot))
        .route("/chrome/back", post(chrome::chrome_back))
        .route("/chrome/forward", post(chrome::chrome_forward))
        .route("/chrome/reload", post(chrome::chrome_reload))
        .route("/chrome/stop", post(chrome::chrome_stop))
        .route("/chrome/navigate", post(chrome::chrome_navigate))
        .layer(CorsLayer::permissive());

    let web_ui_dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web-ui/dist");
    if web_ui_dist.is_dir() {
        app = app.nest_service("/web-ui", ServeDir::new(web_ui_dist));
    }

    app.with_state(state)
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    let diagnostics = read_diagnostics(&state.diagnostics);
    Json(HealthResponse {
        control_plane_up: true,
        runtime_state: diagnostics.state.clone(),
        command_ready: diagnostics.command_ready,
        bridge_healthy: diagnostics.bridge_healthy,
        browser_backend_healthy: diagnostics.browser_backend_healthy,
        active_operation: diagnostics.active_operation,
        last_failure: diagnostics.last_failure,
    })
}

#[derive(Debug, Serialize)]
struct RuntimeInfo {
    host_library: PathBuf,
    browser: BrowserConfig,
    diagnostics: RuntimeDiagnosticsResponse,
    startup: ServeStartupMetrics,
    profile: SessionProfileInfo,
}

async fn runtime_info(State(state): State<ApiState>) -> Result<Json<RuntimeInfo>, ApiError> {
    Ok(Json(RuntimeInfo {
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        diagnostics: read_diagnostics(&state.diagnostics),
        profile: state.profile.clone(),
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
    State(state): State<ApiState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let diagnostics = read_diagnostics(&state.diagnostics);
    if diagnostics.command_ready {
        Ok(Json(diagnostics))
    } else {
        Err(ApiError::readiness(diagnostics))
    }
}

async fn doctor(State(state): State<ApiState>) -> Json<RuntimeDiagnosticsResponse> {
    Json(read_diagnostics(&state.diagnostics))
}

async fn save_session_profile(
    State(state): State<ApiState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("save_session_profile", &state.diagnostics, reply_rx).await??;
    Ok(Json(profile))
}

async fn load_session_profile(
    State(state): State<ApiState>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("load_session_profile", &state.diagnostics, reply_rx).await??;
    Ok(Json(profile))
}

async fn inject_session(
    State(state): State<ApiState>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(body, reply_tx))
        .map_err(channel_error)?;
    await_command("inject_session", &state.diagnostics, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn snapshot_session(State(state): State<ApiState>) -> Result<Json<SessionState>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_session", &state.diagnostics, reply_rx).await??,
    ))
}

async fn navigate(
    State(state): State<ApiState>,
    Json(body): Json<NavigateBody>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("navigate", &state.diagnostics, reply_rx).await??,
    ))
}

async fn execute(
    State(state): State<ApiState>,
    Json(body): Json<ExecuteBody>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("execute", &state.diagnostics, reply_rx).await??,
    ))
}

async fn snapshot_dom(State(state): State<ApiState>) -> Result<Json<DomSnapshot>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_dom", &state.diagnostics, reply_rx).await??,
    ))
}

async fn events(
    State(state): State<ApiState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventReadWindow>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.since, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("events", &state.diagnostics, reply_rx).await??,
    ))
}

async fn enable_trace(
    State(state): State<ApiState>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(body.path, reply_tx))
        .map_err(channel_error)?;
    await_command("enable_trace", &state.diagnostics, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

fn channel_error(error: mpsc::SendError<ApiCommand>) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    fn timeout(operation: &str) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            body: ApiErrorBody {
                error: format!(
                    "operation `{operation}` exceeded the server timeout and the runtime is now marked wedged"
                ),
                code: "operation_timeout".into(),
                operation: Some(operation.to_string()),
                stage: Some("awaiting_control_plane_reply".into()),
                elapsed_ms: Some(COMMAND_TIMEOUT.as_millis() as u64),
                timed_out: true,
                restart_recommended: true,
            },
        }
    }

    fn readiness(diagnostics: RuntimeDiagnosticsResponse) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            body: ApiErrorBody {
                error: "runtime is not command-ready".into(),
                code: "not_ready".into(),
                operation: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.name.clone()),
                stage: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.stage.clone()),
                elapsed_ms: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.elapsed_ms),
                timed_out: diagnostics
                    .active_operation
                    .as_ref()
                    .is_some_and(|op| op.timed_out),
                restart_recommended: diagnostics
                    .last_failure
                    .as_ref()
                    .is_some_and(|failure| failure.restart_recommended),
            },
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
                body: ApiErrorBody {
                    error: native.message,
                    code: "native_operation_error".into(),
                    operation: Some(native.operation),
                    stage: Some(native.stage),
                    elapsed_ms: Some(native.elapsed_ms),
                    timed_out: native.timed_out,
                    restart_recommended: native.restart_recommended,
                },
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
        Self {
            status,
            body: ApiErrorBody {
                error: message,
                code: "aegis_error".into(),
                operation: None,
                stage: None,
                elapsed_ms: None,
                timed_out: false,
                restart_recommended: false,
            },
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
}

async fn await_command<T>(
    operation: &str,
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    reply_rx: oneshot::Receiver<Result<T, AegisError>>,
) -> Result<Result<T, AegisError>, ApiError> {
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(result) => result.map_err(reply_error),
        Err(_) => {
            mark_operation_timeout(diagnostics, operation);
            Err(ApiError::timeout(operation))
        }
    }
}

fn reply_error(error: oneshot::error::RecvError) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
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

fn read_diagnostics(diagnostics: &Arc<Mutex<ServeDiagnostics>>) -> RuntimeDiagnosticsResponse {
    diagnostics
        .lock()
        .map(|diagnostics| diagnostics.snapshot())
        .unwrap_or_else(|_| {
            ServeDiagnostics::new(RuntimeStatus {
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
                last_dom_refresh_at_ms: None,
                last_live_state_refresh_at_ms: None,
                last_event_at_ms: None,
                last_successful_command_at_ms: None,
                last_successful_bridge_roundtrip_at_ms: None,
            })
            .snapshot()
        })
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
        if let Some(active) = self.active_operation.as_mut() {
            active.timed_out = true;
            active.stage = "awaiting_control_plane_reply".into();
        }
        let now = now_ms();
        self.last_failure = Some(FailureSnapshot {
            operation: operation.to_string(),
            stage: "awaiting_control_plane_reply".into(),
            message: "the API timed out waiting for the runtime owner thread to reply".into(),
            elapsed_ms,
            timed_out: true,
            restart_recommended: true,
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
        let state = if active_operation.as_ref().is_some_and(|op| op.timed_out) {
            RuntimeOperationalState::Wedged
        } else if active_operation.is_some() {
            RuntimeOperationalState::Busy
        } else if self
            .last_failure
            .as_ref()
            .is_some_and(|failure| failure.timed_out || failure.restart_recommended)
        {
            RuntimeOperationalState::Wedged
        } else if self.last_failure.is_some() {
            RuntimeOperationalState::Degraded
        } else if self
            .runtime
            .last_successful_bridge_roundtrip_at_ms
            .is_some()
        {
            RuntimeOperationalState::Ready
        } else {
            RuntimeOperationalState::Starting
        };
        let command_ready = !matches!(
            state,
            RuntimeOperationalState::Starting | RuntimeOperationalState::Wedged
        );
        RuntimeDiagnosticsResponse {
            state,
            control_plane_up: true,
            command_ready,
            bridge_healthy: self
                .runtime
                .last_successful_bridge_roundtrip_at_ms
                .is_some()
                && self
                    .last_failure
                    .as_ref()
                    .is_none_or(|failure| !failure.timed_out),
            browser_backend_healthy: self.runtime.bootstrapped
                && self
                    .last_failure
                    .as_ref()
                    .is_none_or(|failure| !failure.restart_recommended),
            dom_snapshot_available: self.runtime.dom_snapshot_available,
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
